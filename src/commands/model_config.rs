mod contract;
mod projection;

use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use a3s_acl::{Block, Value as AclValue};
use a3s_code_core::config::{
    rewrite_acl_sections, ConfigSection, ModelConfig, ModelCost, ModelLimit, ModelModalities,
    ProviderConfig,
};
use a3s_code_core::CodeConfig;
use anyhow::{bail, Context};
use reqwest::header::{HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::io::AsyncReadExt;
use url::Url;

use self::contract::{
    HeaderInput, ModelInput, MutationDocument, ProviderInput, ProviderTestDocument, SecretMutation,
};
use crate::cli::args::{ConfigScope, ModelConfigArgs, ModelConfigCommand, ModelScopeArgs};
use crate::cli::context::InvocationContext;
use crate::cli::output::render_value;
use crate::model::route::{ModelRoute, ModelSource};

const MAX_INPUT_BYTES: u64 = 256 * 1024;
const MAX_SECRET_BYTES: usize = 64 * 1024;
const PROVIDER_TEST_TIMEOUT: Duration = Duration::from_secs(12);
const ENV_SENTINEL_PREFIX: &str = "__A3S_MODEL_CONFIG_ENV_";
static ENV_SENTINEL_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) async fn run(args: ModelConfigArgs, context: &InvocationContext) -> anyhow::Result<()> {
    match args.command {
        ModelConfigCommand::Show(scope) => show(scope, context),
        ModelConfigCommand::Apply(args) => {
            if !args.input_stdin {
                bail!("model config mutations require --input-stdin");
            }
            apply(args.target, context).await
        }
        ModelConfigCommand::Test(args) => {
            if !args.input_stdin {
                bail!("model config tests require --input-stdin");
            }
            test_provider(args.target, context).await
        }
    }
}

fn show(scope: ModelScopeArgs, context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    let path = super::config::target_config_path(scope.scope, context)?;
    crate::config::persistence::ensure_config_file(&path)
        .map_err(|error| anyhow::anyhow!("could not initialize {}: {error}", path.display()))?;
    let LoadedConfig {
        source,
        config,
        fallbacks,
    } = LoadedConfig::load(&path)?;
    drop(fallbacks);
    let data =
        projection::configuration_projection(&path, scope_name(scope.scope), &source, &config)?;
    render_value(output, "model.config.show", data, || {
        println!("config: {}", path.display());
        println!("providers: {}", config.providers.len());
        println!("models: {}", config.list_models().len());
    })
}

async fn apply(scope: ModelScopeArgs, context: &InvocationContext) -> anyhow::Result<()> {
    let mutation = read_stdin_json::<MutationDocument>().await?;
    let output = context.output_mode();
    let path = super::config::target_config_path(scope.scope, context)?;
    let LoadedConfig {
        source,
        mut config,
        fallbacks,
    } = LoadedConfig::load(&path)?;
    let mut replacements = Vec::new();
    let (operation, section) = apply_mutation(&mut config, mutation, &mut replacements)?;
    validate_complete_config(&config)?;

    let mut rendered = rewrite_acl_sections(&source, &config, &[section])
        .map_err(|error| anyhow::anyhow!("could not render {}: {error}", path.display()))?;
    for (sentinel, variable) in replacements {
        let needle = format!("\"{sentinel}\"");
        let replacement = format!("env(\"{variable}\")");
        if !rendered.contains(&needle) {
            bail!("generated model configuration lost a protected environment reference");
        }
        rendered = rendered.replace(&needle, &replacement);
    }
    let verification_fallbacks = EnvironmentFallbacks::install(&rendered)?;
    let verified = CodeConfig::from_acl(&rendered)
        .map_err(|error| anyhow::anyhow!("generated config is invalid: {error}"))?;
    validate_complete_config(&verified)?;
    crate::config::persistence::write_atomic(&path, rendered.as_bytes())
        .map_err(|error| anyhow::anyhow!("could not update {}: {error}", path.display()))?;
    drop(verification_fallbacks);
    drop(fallbacks);
    let data =
        projection::configuration_projection(&path, scope_name(scope.scope), &rendered, &verified)?;
    render_value(
        output,
        "model.config.apply",
        json!({"operation": operation, "configuration": data}),
        || println!("updated {operation} in {}", path.display()),
    )
}

async fn test_provider(scope: ModelScopeArgs, context: &InvocationContext) -> anyhow::Result<()> {
    let document = read_stdin_json::<ProviderTestDocument>().await?;
    let output = context.output_mode();
    let path = super::config::target_config_path(scope.scope, context)?;
    let mut loaded = LoadedConfig::load(&path)?;
    let mut replacements = Vec::new();
    let provider = upsert_provider(&mut loaded.config, document.provider, &mut replacements)?;
    if !replacements.is_empty() {
        for (sentinel, variable) in &replacements {
            if let Ok(value) = std::env::var(variable) {
                replace_provider_secret(provider, sentinel, &value);
            } else {
                bail!("environment variable `{variable}` is not set");
            }
        }
    }
    if let Some(variable) = loaded.fallbacks.missing_reference(provider) {
        bail!("environment variable `{variable}` is not set");
    }
    let (endpoint, status, latency_ms) = probe_provider(provider).await?;
    render_value(
        output,
        "model.config.test",
        json!({
            "providerId": provider.name,
            "endpoint": endpoint,
            "status": status,
            "latencyMs": latency_ms,
        }),
        || println!("provider {} is reachable ({latency_ms} ms)", provider.name),
    )
}

fn apply_mutation(
    config: &mut CodeConfig,
    mutation: MutationDocument,
    replacements: &mut Vec<(String, String)>,
) -> anyhow::Result<(&'static str, ConfigSection)> {
    match mutation {
        MutationDocument::UpsertProvider { provider } => {
            upsert_provider(config, provider, replacements)?;
            Ok(("upsertProvider", ConfigSection::Providers))
        }
        MutationDocument::RemoveProvider { provider_id } => {
            validate_provider_id(&provider_id)?;
            reject_default_removal(config, &provider_id, None)?;
            let before = config.providers.len();
            config
                .providers
                .retain(|provider| provider.name != provider_id);
            if config.providers.len() == before {
                bail!("provider `{provider_id}` is not configured");
            }
            Ok(("removeProvider", ConfigSection::Providers))
        }
        MutationDocument::UpsertModel { provider_id, model } => {
            validate_provider_id(&provider_id)?;
            let provider = config
                .providers
                .iter_mut()
                .find(|provider| provider.name == provider_id)
                .ok_or_else(|| anyhow::anyhow!("provider `{provider_id}` is not configured"))?;
            upsert_model(provider, *model, replacements)?;
            Ok(("upsertModel", ConfigSection::Providers))
        }
        MutationDocument::RemoveModel {
            provider_id,
            model_id,
        } => {
            validate_provider_id(&provider_id)?;
            validate_model_id(&model_id)?;
            reject_default_removal(config, &provider_id, Some(&model_id))?;
            let provider = config
                .providers
                .iter_mut()
                .find(|provider| provider.name == provider_id)
                .ok_or_else(|| anyhow::anyhow!("provider `{provider_id}` is not configured"))?;
            let before = provider.models.len();
            provider.models.retain(|model| model.id != model_id);
            if provider.models.len() == before {
                bail!("model `{provider_id}/{model_id}` is not configured");
            }
            Ok(("removeModel", ConfigSection::Providers))
        }
        MutationDocument::UpdateRuntime {
            thinking_budget,
            llm_api_timeout_ms,
        } => {
            if thinking_budget == Some(0) {
                bail!("thinkingBudget must be greater than zero when configured");
            }
            if llm_api_timeout_ms.is_some_and(|value| value < 100) {
                bail!("llmApiTimeoutMs must be at least 100 milliseconds");
            }
            config.thinking_budget = thinking_budget;
            config.llm_api_timeout_ms = llm_api_timeout_ms;
            Ok(("updateRuntime", ConfigSection::ModelRuntime))
        }
    }
}

fn upsert_provider<'a>(
    config: &'a mut CodeConfig,
    input: ProviderInput,
    replacements: &mut Vec<(String, String)>,
) -> anyhow::Result<&'a mut ProviderConfig> {
    validate_provider_input(&input)?;
    let index = config
        .providers
        .iter()
        .position(|provider| provider.name == input.id)
        .unwrap_or_else(|| {
            config.providers.push(ProviderConfig {
                name: input.id.clone(),
                api_key: None,
                base_url: None,
                headers: HashMap::new(),
                session_id_header: None,
                models: Vec::new(),
            });
            config.providers.len() - 1
        });
    let provider = &mut config.providers[index];
    provider.base_url = normalized_optional(input.base_url);
    provider.session_id_header = normalized_optional(input.session_id_header);
    apply_secret(&mut provider.api_key, input.credential, replacements)?;
    provider.headers = desired_headers(&provider.headers, input.headers, replacements)?;
    Ok(provider)
}

fn upsert_model(
    provider: &mut ProviderConfig,
    input: ModelInput,
    replacements: &mut Vec<(String, String)>,
) -> anyhow::Result<()> {
    validate_model_input(&input)?;
    let index = provider
        .models
        .iter()
        .position(|model| model.id == input.id)
        .unwrap_or_else(|| {
            provider.models.push(ModelConfig {
                id: input.id.clone(),
                name: input.id.clone(),
                family: String::new(),
                api_key: None,
                base_url: None,
                headers: HashMap::new(),
                session_id_header: None,
                attachment: false,
                reasoning: false,
                tool_call: true,
                temperature: true,
                release_date: None,
                modalities: ModelModalities::default(),
                cost: ModelCost::default(),
                limit: ModelLimit::default(),
            });
            provider.models.len() - 1
        });
    let model = &mut provider.models[index];
    model.name = input.name.trim().to_string();
    model.family = input.family.trim().to_string();
    model.base_url = normalized_optional(input.base_url);
    model.session_id_header = normalized_optional(input.session_id_header);
    apply_secret(&mut model.api_key, input.credential, replacements)?;
    model.headers = desired_headers(&model.headers, input.headers, replacements)?;
    model.attachment = input.attachment;
    model.reasoning = input.reasoning;
    model.tool_call = input.tool_call;
    model.temperature = input.temperature;
    model.release_date = normalized_optional(input.release_date);
    model.modalities = ModelModalities {
        input: normalized_list(input.modalities.input),
        output: normalized_list(input.modalities.output),
    };
    model.cost = ModelCost {
        input: input.cost.input.unwrap_or_default(),
        output: input.cost.output.unwrap_or_default(),
        cache_read: input.cost.cache_read.unwrap_or_default(),
        cache_write: input.cost.cache_write.unwrap_or_default(),
    };
    model.limit = ModelLimit {
        context: input.limit.context.unwrap_or_default(),
        output: input.limit.output.unwrap_or_default(),
    };
    Ok(())
}

fn apply_secret(
    current: &mut Option<String>,
    mutation: SecretMutation,
    replacements: &mut Vec<(String, String)>,
) -> anyhow::Result<()> {
    match mutation {
        SecretMutation::Unchanged => {}
        SecretMutation::Clear => *current = None,
        SecretMutation::Inline { value } => {
            validate_secret(&value)?;
            *current = Some(value);
        }
        SecretMutation::Environment { variable } => {
            validate_environment_name(&variable)?;
            let sentinel = new_env_sentinel();
            *current = Some(sentinel.clone());
            replacements.push((sentinel, variable));
        }
    }
    Ok(())
}

fn desired_headers(
    current: &HashMap<String, String>,
    inputs: Vec<HeaderInput>,
    replacements: &mut Vec<(String, String)>,
) -> anyhow::Result<HashMap<String, String>> {
    let mut result = HashMap::new();
    let mut seen = HashSet::new();
    for input in inputs {
        let name = input.name.trim().to_string();
        validate_header_name(&name)?;
        if !seen.insert(name.to_ascii_lowercase()) {
            bail!("header `{name}` is configured more than once");
        }
        let mut value = current.iter().find_map(|(existing, value)| {
            existing.eq_ignore_ascii_case(&name).then(|| value.clone())
        });
        apply_secret(&mut value, input.value, replacements)?;
        if let Some(value) = value {
            HeaderValue::from_str(&value).context("header value contains invalid characters")?;
            result.insert(name, value);
        }
    }
    Ok(result)
}

fn validate_provider_input(input: &ProviderInput) -> anyhow::Result<()> {
    validate_provider_id(&input.id)?;
    validate_url(input.base_url.as_deref(), &input.id)?;
    if let Some(header) = input.session_id_header.as_deref() {
        validate_header_name(header)?;
    }
    Ok(())
}

fn validate_model_input(input: &ModelInput) -> anyhow::Result<()> {
    validate_model_id(&input.id)?;
    if input.name.trim().is_empty() || input.name.len() > 256 {
        bail!("model name must be between 1 and 256 characters");
    }
    if input.family.len() > 128 {
        bail!("model family exceeds 128 characters");
    }
    validate_url(input.base_url.as_deref(), &input.id)?;
    if let Some(header) = input.session_id_header.as_deref() {
        validate_header_name(header)?;
    }
    for value in [
        input.cost.input,
        input.cost.output,
        input.cost.cache_read,
        input.cost.cache_write,
    ]
    .into_iter()
    .flatten()
    {
        if !value.is_finite() || value < 0.0 {
            bail!("model costs must be finite non-negative numbers");
        }
    }
    for modality in input
        .modalities
        .input
        .iter()
        .chain(input.modalities.output.iter())
    {
        if modality.trim().is_empty() || modality.len() > 32 {
            bail!("model modalities must be non-empty and at most 32 characters");
        }
    }
    Ok(())
}

fn validate_provider_id(value: &str) -> anyhow::Result<()> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || value.contains('/')
        || value.chars().any(char::is_whitespace)
    {
        bail!("provider id must be 1-128 non-whitespace characters without `/`");
    }
    Ok(())
}

fn validate_model_id(value: &str) -> anyhow::Result<()> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 256
        || value.chars().any(char::is_whitespace)
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
    {
        bail!("model id must be a valid non-empty route segment");
    }
    Ok(())
}

fn validate_url(value: Option<&str>, field: &str) -> anyhow::Result<()> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let parsed = Url::parse(value).with_context(|| format!("{field} base URL is invalid"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        bail!("{field} base URL must use http or https");
    }
    Ok(())
}

fn validate_header_name(value: &str) -> anyhow::Result<()> {
    value
        .parse::<HeaderName>()
        .with_context(|| format!("invalid HTTP header name `{value}`"))?;
    Ok(())
}

fn validate_secret(value: &str) -> anyhow::Result<()> {
    if value.is_empty() || value.len() > MAX_SECRET_BYTES || value.contains(['\0', '\r', '\n']) {
        bail!("secret value must be non-empty, bounded, and single-line");
    }
    if value.starts_with(ENV_SENTINEL_PREFIX) {
        bail!("secret value uses a reserved internal prefix");
    }
    Ok(())
}

fn validate_environment_name(value: &str) -> anyhow::Result<()> {
    let mut chars = value.chars();
    let valid_first = chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic());
    if !valid_first || !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric()) || value.len() > 128
    {
        bail!("environment variable name is invalid");
    }
    Ok(())
}

fn reject_default_removal(
    config: &CodeConfig,
    provider_id: &str,
    model_id: Option<&str>,
) -> anyhow::Result<()> {
    let Some(route) = config
        .default_model
        .as_deref()
        .and_then(|value| value.parse::<ModelRoute>().ok())
    else {
        return Ok(());
    };
    if route.source != ModelSource::Config {
        return Ok(());
    }
    let Some((provider, model)) = route.model.split_once('/') else {
        return Ok(());
    };
    if provider == provider_id && model_id.is_none_or(|candidate| candidate == model) {
        bail!("select or reset a different default model before removing `{route}`");
    }
    Ok(())
}

fn validate_complete_config(config: &CodeConfig) -> anyhow::Result<()> {
    let issues = crate::config::validation::validate_config(config);
    if !issues.is_empty() {
        bail!("invalid model configuration: {}", issues.join("; "));
    }
    Ok(())
}

async fn probe_provider(provider: &ProviderConfig) -> anyhow::Result<(String, u16, u64)> {
    let base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| default_base_url(&provider.name).to_string());
    let endpoint = format!("{}/models", base.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(PROVIDER_TEST_TIMEOUT)
        .build()
        .context("could not create provider test client")?;
    let mut request = client.get(&endpoint);
    let has_authorization = provider
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("authorization"));
    for (name, value) in &provider.headers {
        request = request.header(name, value);
    }
    if let Some(api_key) = provider.api_key.as_deref() {
        match provider.name.to_ascii_lowercase().as_str() {
            "anthropic" | "claude" => {
                request = request
                    .header("x-api-key", api_key)
                    .header("anthropic-version", "2023-06-01");
            }
            _ if !has_authorization => {
                request = request.bearer_auth(api_key);
            }
            _ => {}
        }
    }
    let started = Instant::now();
    let response = request
        .send()
        .await
        .context("provider connection test failed")?;
    let status = response.status();
    if !status.is_success() {
        bail!("provider returned HTTP {}", status.as_u16());
    }
    Ok((
        endpoint,
        status.as_u16(),
        started.elapsed().as_millis() as u64,
    ))
}

fn default_base_url(provider: &str) -> &'static str {
    match provider.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => "https://api.anthropic.com/v1",
        "glm" | "zhipu" | "bigmodel" => "https://open.bigmodel.cn/api/paas/v4",
        _ => "https://api.openai.com/v1",
    }
}

fn replace_provider_secret(provider: &mut ProviderConfig, needle: &str, replacement: &str) {
    if provider.api_key.as_deref() == Some(needle) {
        provider.api_key = Some(replacement.to_string());
    }
    for value in provider.headers.values_mut() {
        if value == needle {
            *value = replacement.to_string();
        }
    }
}

async fn read_stdin_json<T: DeserializeOwned>() -> anyhow::Result<T> {
    if std::io::stdin().is_terminal() {
        bail!("protected model configuration input must be piped through standard input");
    }
    let mut bytes = Vec::new();
    tokio::io::stdin()
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .context("could not read model configuration input")?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        bail!("model configuration input exceeds 256 KiB");
    }
    serde_json::from_slice(&bytes).context("model configuration input is not valid JSON")
}

fn normalized_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalized_list(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}

fn new_env_sentinel() -> String {
    format!(
        "{ENV_SENTINEL_PREFIX}{}_{}__",
        std::process::id(),
        ENV_SENTINEL_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn scope_name(scope: ConfigScope) -> &'static str {
    match scope {
        ConfigScope::Workspace => "workspace",
        ConfigScope::User => "user",
    }
}

struct LoadedConfig {
    source: String,
    config: CodeConfig,
    fallbacks: EnvironmentFallbacks,
}

impl LoadedConfig {
    fn load(path: &Path) -> anyhow::Result<Self> {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("could not read {}", path.display()))
            }
        };
        let fallbacks = EnvironmentFallbacks::install(&source)?;
        let config = CodeConfig::from_acl(&source)
            .map_err(|error| anyhow::anyhow!("failed to parse {}: {error}", path.display()))?;
        Ok(Self {
            source,
            config,
            fallbacks,
        })
    }
}

struct EnvironmentFallbacks {
    inserted: Vec<(String, String)>,
}

impl EnvironmentFallbacks {
    fn install(source: &str) -> anyhow::Result<Self> {
        if source.trim().is_empty() {
            return Ok(Self {
                inserted: Vec::new(),
            });
        }
        let document = a3s_acl::parse_acl(source)?;
        let mut names = HashSet::new();
        for block in &document.blocks {
            collect_environment_names(block, &mut names);
        }
        let mut inserted = Vec::new();
        for name in names {
            if std::env::var_os(&name).is_none() {
                let value = new_env_sentinel();
                std::env::set_var(&name, &value);
                inserted.push((name, value));
            }
        }
        Ok(Self { inserted })
    }

    fn missing_reference(&self, provider: &ProviderConfig) -> Option<&str> {
        self.inserted.iter().find_map(|(name, sentinel)| {
            (provider.api_key.as_deref() == Some(sentinel)
                || provider.headers.values().any(|value| value == sentinel))
            .then_some(name.as_str())
        })
    }
}

impl Drop for EnvironmentFallbacks {
    fn drop(&mut self) {
        for (name, _) in &self.inserted {
            std::env::remove_var(name);
        }
    }
}

fn collect_environment_names(block: &Block, names: &mut HashSet<String>) {
    for value in block.attributes.values() {
        collect_environment_names_from_value(value, names);
    }
    for child in &block.blocks {
        collect_environment_names(child, names);
    }
}

fn collect_environment_names_from_value(value: &AclValue, names: &mut HashSet<String>) {
    match value {
        AclValue::Call(name, args) if name == "env" => {
            if let Some(AclValue::String(variable)) = args.first() {
                names.insert(variable.clone());
            }
        }
        AclValue::List(values) => {
            for value in values {
                collect_environment_names_from_value(value, names);
            }
        }
        AclValue::Object(values) => {
            for (_, value) in values {
                collect_environment_names_from_value(value, names);
            }
        }
        _ => {}
    }
}
