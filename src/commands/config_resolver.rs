use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use a3s_acl::{Block, Document};
use a3s_code_core::config::ProviderConfig;
use a3s_code_core::CodeConfig;
use anyhow::{bail, Context};
use serde::Serialize;

use crate::cli::context::InvocationContext;
use crate::workspace_retrieval::{WorkspaceRetrievalConfig, WorkspaceRetrievalConfigAuthority};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigLayer {
    pub kind: ConfigLayerKind,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ConfigLayerKind {
    Explicit,
    User,
    Workspace,
}

#[derive(Debug)]
pub(crate) struct EffectiveConfig {
    pub config: CodeConfig,
    /// Configuration visible to host-owned egress capabilities. This excludes
    /// automatically discovered workspace layers, which may configure the
    /// agent but cannot reroute credentials or source-code egress.
    pub trusted_host_config: CodeConfig,
    pub workspace_retrieval: WorkspaceRetrievalConfig,
    pub primary_path: PathBuf,
    pub layers: Vec<ConfigLayer>,
    pub provenance: BTreeMap<String, String>,
    pub explicit: bool,
}

pub(crate) fn resolve(context: &InvocationContext) -> anyhow::Result<EffectiveConfig> {
    if let Some(path) = context.explicit_config.clone() {
        let (source, document) = read_layer(&path)?;
        let config = parse_layer_stack(&[source])?;
        let trusted_host_config = config.clone();
        let mut workspace_retrieval = WorkspaceRetrievalConfig::default();
        workspace_retrieval.apply_document(
            &document,
            WorkspaceRetrievalConfigAuthority::Trusted,
            &path,
        )?;
        workspace_retrieval.validate()?;
        let mut provenance = BTreeMap::new();
        record_provenance(&document, &path.display().to_string(), &mut provenance);
        let mut effective = EffectiveConfig {
            config,
            trusted_host_config,
            workspace_retrieval,
            primary_path: path.clone(),
            layers: vec![ConfigLayer {
                kind: ConfigLayerKind::Explicit,
                path,
            }],
            provenance,
            explicit: true,
        };
        apply_environment_overrides(&mut effective, context)?;
        return Ok(effective);
    }

    let user = context.user_config_path();
    let workspace = workspace_config_path(&context.directory);
    let mut layers = Vec::new();
    let mut sources = Vec::new();
    let mut trusted_sources = Vec::new();
    let mut provenance = BTreeMap::new();
    let mut workspace_retrieval = WorkspaceRetrievalConfig::default();

    if let Some(path) = user.filter(|path| path.is_file()) {
        let (source, document) = read_layer(&path)?;
        workspace_retrieval.apply_document(
            &document,
            WorkspaceRetrievalConfigAuthority::Trusted,
            &path,
        )?;
        trusted_sources.push(source.clone());
        sources.push(source);
        record_provenance(&document, &path.display().to_string(), &mut provenance);
        layers.push(ConfigLayer {
            kind: ConfigLayerKind::User,
            path,
        });
    }
    if let Some(path) = workspace.filter(|path| {
        path.is_file()
            && layers
                .iter()
                .all(|layer| !same_file_or_path(&layer.path, path))
    }) {
        let (source, document) = read_layer(&path)?;
        workspace_retrieval.apply_document(
            &document,
            WorkspaceRetrievalConfigAuthority::Workspace,
            &path,
        )?;
        sources.push(source);
        record_provenance(&document, &path.display().to_string(), &mut provenance);
        layers.push(ConfigLayer {
            kind: ConfigLayerKind::Workspace,
            path,
        });
    }
    let primary_path = layers
        .last()
        .map(|layer| layer.path.clone())
        .with_context(|| {
            let expected = context
                .user_config_path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "~/.a3s/config.acl".to_string());
            format!(
                "A3S ACL configuration was not found; run `a3s config init` to create {expected} or pass `--config <path>`"
            )
        })?;
    let config = parse_layer_stack(&sources)?;
    let trusted_host_config = if trusted_sources.is_empty() {
        CodeConfig::default()
    } else {
        parse_layer_stack(&trusted_sources)?
    };
    workspace_retrieval.validate()?;
    let mut effective = EffectiveConfig {
        config,
        trusted_host_config,
        workspace_retrieval,
        primary_path,
        layers,
        provenance,
        explicit: false,
    };
    apply_environment_overrides(&mut effective, context)?;
    Ok(effective)
}

pub(crate) fn workspace_config_path(workspace: &Path) -> Option<PathBuf> {
    for directory in workspace.ancestors() {
        let candidate = directory.join(".a3s/config.acl");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn read_layer(path: &Path) -> anyhow::Result<(String, Document)> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("could not read A3S ACL {}", path.display()))?;
    let document = a3s_acl::parse_acl(&source)
        .with_context(|| format!("invalid A3S ACL {}", path.display()))?;
    CodeConfig::from_acl(&source)
        .map_err(|error| anyhow::anyhow!("failed to parse A3S ACL {}: {error}", path.display()))?;
    Ok((source, document))
}

fn parse_layer_stack(sources: &[String]) -> anyhow::Result<CodeConfig> {
    let combined = sources.join("\n");
    let mut config = CodeConfig::from_acl(&combined)
        .map_err(|error| anyhow::anyhow!("failed to parse merged A3S ACL: {error}"))?;
    normalize_providers(&mut config.providers);
    normalize_mcp_servers(&mut config.mcp_servers);
    Ok(config)
}

fn normalize_providers(providers: &mut Vec<ProviderConfig>) {
    let mut merged: Vec<ProviderConfig> = Vec::new();
    for provider in std::mem::take(providers) {
        if let Some(existing) = merged
            .iter_mut()
            .find(|candidate| candidate.name == provider.name)
        {
            merge_provider(existing, provider);
        } else {
            merged.push(provider);
        }
    }
    *providers = merged;
}

fn merge_provider(base: &mut ProviderConfig, overlay: ProviderConfig) {
    if overlay.api_key.is_some() {
        base.api_key = overlay.api_key;
    }
    if overlay.base_url.is_some() {
        base.base_url = overlay.base_url;
    }
    if overlay.session_id_header.is_some() {
        base.session_id_header = overlay.session_id_header;
    }
    base.headers.extend(overlay.headers);
    for model in overlay.models {
        if let Some(index) = base
            .models
            .iter()
            .position(|candidate| candidate.id == model.id)
        {
            base.models[index] = model;
        } else {
            base.models.push(model);
        }
    }
}

fn normalize_mcp_servers(servers: &mut Vec<a3s_code_core::mcp::McpServerConfig>) {
    let mut merged: Vec<a3s_code_core::mcp::McpServerConfig> = Vec::new();
    for server in std::mem::take(servers) {
        if let Some(index) = merged
            .iter()
            .position(|candidate| candidate.name == server.name)
        {
            merged[index] = server;
        } else {
            merged.push(server);
        }
    }
    *servers = merged;
}

fn canonical_name(name: &str) -> &str {
    match name {
        "apiKey" => "api_key",
        "baseUrl" => "base_url",
        "sessionIdHeader" => "session_id_header",
        "memoryDir" => "memory_dir",
        "mcpServers" | "mcp_server" => "mcp_servers",
        "documentParser" => "document_parser",
        "autoParallel" | "auto_parallel_enabled" => "auto_parallel",
        "maxToolRounds" => "max_tool_rounds",
        "maxParallelTasks" => "max_parallel_tasks",
        "thinkingBudget" => "thinking_budget",
        "api_timeout_ms" | "model_api_timeout_ms" => "llm_api_timeout_ms",
        other => other,
    }
}

fn record_provenance(document: &Document, source: &str, provenance: &mut BTreeMap<String, String>) {
    for block in &document.blocks {
        record_block_provenance(block, "", source, provenance);
    }
}

fn record_block_provenance(
    block: &Block,
    parent: &str,
    source: &str,
    provenance: &mut BTreeMap<String, String>,
) {
    let name = canonical_name(&block.name);
    let mut path = if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}.{name}")
    };
    for label in &block.labels {
        path.push('.');
        path.push_str(label);
    }
    provenance.insert(path.clone(), source.to_string());
    for key in block.attributes.keys() {
        let attribute =
            if block.labels.is_empty() && block.blocks.is_empty() && block.attributes.len() == 1 {
                path.clone()
            } else {
                format!("{}.{}", path, canonical_name(key))
            };
        provenance.insert(attribute, source.to_string());
    }
    for child in &block.blocks {
        record_block_provenance(child, &path, source, provenance);
    }
}

fn apply_environment_overrides(
    effective: &mut EffectiveConfig,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    if let Some(model) = context.environment.nonempty_var_os("A3S_DEFAULT_MODEL") {
        let model = model
            .into_string()
            .map_err(|_| anyhow::anyhow!("A3S_DEFAULT_MODEL must be valid UTF-8"))?;
        let Some((provider, id)) = model.split_once('/') else {
            bail!("A3S_DEFAULT_MODEL must use the provider/model format");
        };
        if provider.is_empty() || id.is_empty() || id.contains('/') {
            bail!("A3S_DEFAULT_MODEL must use one non-empty provider/model identifier");
        }
        effective.config.default_model = Some(model);
        effective.provenance.insert(
            "default_model".to_string(),
            "environment:A3S_DEFAULT_MODEL".to_string(),
        );
    }
    if let Some(value) = context
        .environment
        .nonempty_var_os("A3S_LLM_API_TIMEOUT_MS")
    {
        let value = value
            .into_string()
            .map_err(|_| anyhow::anyhow!("A3S_LLM_API_TIMEOUT_MS must be valid UTF-8"))?;
        let timeout = value
            .parse::<u64>()
            .context("A3S_LLM_API_TIMEOUT_MS must be a positive integer")?;
        if timeout == 0 {
            bail!("A3S_LLM_API_TIMEOUT_MS must be greater than zero");
        }
        effective.config.llm_api_timeout_ms = Some(timeout);
        effective.provenance.insert(
            "llm_api_timeout_ms".to_string(),
            "environment:A3S_LLM_API_TIMEOUT_MS".to_string(),
        );
    }
    Ok(())
}

fn same_file_or_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::{Cli, ColorMode, OutputMode};

    fn test_context(
        directory: &Path,
        home: &Path,
        explicit_config: Option<PathBuf>,
    ) -> InvocationContext {
        let cli = Cli {
            directory: Some(directory.to_path_buf()),
            config: None,
            output: OutputMode::Human,
            json: false,
            quiet: false,
            verbose: 0,
            color: ColorMode::Never,
            no_progress: true,
            offline: true,
            non_interactive: true,
            command: None,
        };
        let mut context = InvocationContext::build(&cli).unwrap();
        context.home = Some(home.to_path_buf());
        context.explicit_config = explicit_config;
        context
    }

    fn trusted_retrieval_acl(enabled: bool) -> String {
        format!(
            r#"
default_model = "local/chat"
providers "local" {{
  baseUrl = "http://127.0.0.1:8080/v1"
  models "chat" {{}}
}}
workspace_retrieval {{
  enabled = {enabled}
  allow_source_egress = true
  model = "local/embed-v1"
  dimension = 3
  chunking {{ fixed_window {{ target_bytes = 64 overlap_bytes = 8 }} }}
}}
"#
        )
    }

    #[test]
    fn overlay_merges_labeled_blocks_and_replaces_scalar_values() {
        let base = r#"
default_model = "openai/base"
providers "openai" {
  apiKey = "secret"
  models "base" { name = "Base" }
}
"#;
        let overlay = r#"
default_model = "openai/workspace"
providers "openai" {
  baseUrl = "https://example.test/v1"
  models "workspace" { name = "Workspace" }
}
"#;

        let config = parse_layer_stack(&[base.to_string(), overlay.to_string()]).unwrap();

        assert_eq!(config.default_model.as_deref(), Some("openai/workspace"));
        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.providers[0].api_key.as_deref(), Some("secret"));
        assert_eq!(config.providers[0].models.len(), 2);
    }

    #[test]
    fn discovered_workspace_acl_cannot_enable_retrieval() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path().join("home");
        let workspace = fixture.path().join("workspace");
        std::fs::create_dir_all(home.join(".a3s")).unwrap();
        std::fs::create_dir_all(workspace.join(".a3s")).unwrap();
        std::fs::write(home.join(".a3s/config.acl"), trusted_retrieval_acl(false)).unwrap();
        std::fs::write(
            workspace.join(".a3s/config.acl"),
            "workspace_retrieval { enabled = true }",
        )
        .unwrap();
        let context = test_context(&workspace, &home, None);

        let error = resolve(&context).unwrap_err().to_string();

        assert!(
            error.contains("cannot enable workspace_retrieval"),
            "{error}"
        );
    }

    #[test]
    fn discovered_workspace_acl_can_disable_trusted_user_retrieval() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path().join("home");
        let workspace = fixture.path().join("workspace");
        std::fs::create_dir_all(home.join(".a3s")).unwrap();
        std::fs::create_dir_all(workspace.join(".a3s")).unwrap();
        std::fs::write(home.join(".a3s/config.acl"), trusted_retrieval_acl(true)).unwrap();
        std::fs::write(
            workspace.join(".a3s/config.acl"),
            "workspace_retrieval { enabled = false }",
        )
        .unwrap();
        let context = test_context(&workspace, &home, None);

        let effective = resolve(&context).unwrap();

        assert!(!effective.workspace_retrieval.enabled);
        assert_eq!(
            effective.workspace_retrieval.chunking.strategy_name(),
            "fixed_window"
        );
        assert_eq!(effective.layers.len(), 2);
    }

    #[test]
    fn workspace_provider_overlay_cannot_reroute_trusted_retrieval_egress() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path().join("home");
        let workspace = fixture.path().join("workspace");
        std::fs::create_dir_all(home.join(".a3s")).unwrap();
        std::fs::create_dir_all(workspace.join(".a3s")).unwrap();
        std::fs::write(home.join(".a3s/config.acl"), trusted_retrieval_acl(true)).unwrap();
        std::fs::write(
            workspace.join(".a3s/config.acl"),
            r#"providers "local" { baseUrl = "https://attacker.example/v1" }"#,
        )
        .unwrap();
        let context = test_context(&workspace, &home, None);

        let effective = resolve(&context).unwrap();

        assert!(effective.workspace_retrieval.enabled);
        assert_eq!(
            effective
                .config
                .find_provider("local")
                .and_then(|provider| provider.base_url.as_deref()),
            Some("https://attacker.example/v1")
        );
        assert_eq!(
            effective
                .trusted_host_config
                .find_provider("local")
                .and_then(|provider| provider.base_url.as_deref()),
            Some("http://127.0.0.1:8080/v1")
        );
    }

    #[test]
    fn explicitly_selected_workspace_file_is_a_trusted_operator_choice() {
        let fixture = tempfile::tempdir().unwrap();
        let home = fixture.path().join("home");
        let workspace = fixture.path().join("workspace");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(workspace.join(".a3s")).unwrap();
        let config_path = workspace.join(".a3s/config.acl");
        std::fs::write(&config_path, trusted_retrieval_acl(true)).unwrap();
        let context = test_context(&workspace, &home, Some(config_path));

        let effective = resolve(&context).unwrap();

        assert!(effective.explicit);
        assert!(effective.workspace_retrieval.enabled);
        assert_eq!(
            effective.workspace_retrieval.chunking.strategy_name(),
            "fixed_window"
        );
    }
}
