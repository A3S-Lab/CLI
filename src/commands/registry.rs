use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use a3s_acl::{Block, Document, Value};
use anyhow::{bail, Context};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::cli::args::{OutputMode, RegistryArgs, RegistryCommand};
use crate::cli::context::InvocationContext;
use crate::cli::output::{coded_error, render_value, CliError, ExitClass};

const OFFICIAL_NAME: &str = "a3s";
const OFFICIAL_URL: &str = "https://components.a3s.dev/";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegistryRecord {
    name: String,
    url: String,
    trust_root: String,
    built_in: bool,
}

pub(crate) async fn run(args: RegistryArgs, context: &InvocationContext) -> anyhow::Result<()> {
    match args.command {
        RegistryCommand::List => list(context),
        RegistryCommand::Show(args) => show(&args.name, context),
        RegistryCommand::Add(args) => add(&args.url, &args.trust_root, args.yes, context),
        RegistryCommand::Remove(args) => remove(&args.name, args.yes, context),
        RegistryCommand::Refresh(args) => refresh(args.name.as_deref(), context).await,
    }
}

fn list(context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    let registries = registries(context)?;
    render_value(
        output,
        "registry.list",
        json!({"registries": registries}),
        || {
            println!("REGISTRY                 TRUST ROOT");
            for registry in &registries {
                println!("{:<24} {}", registry.name, registry.trust_root);
                println!("  {}", registry.url);
            }
        },
    )
}

fn show(name: &str, context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    validate_name(name)?;
    let registry = registries(context)?
        .into_iter()
        .find(|registry| registry.name == name)
        .with_context(|| format!("registry `{name}` is not configured"))?;
    render_value(
        output,
        "registry.show",
        json!({"registry": registry}),
        || {
            println!("name: {}", registry.name);
            println!("url: {}", registry.url);
            println!("trust root: {}", registry.trust_root);
            println!("built in: {}", registry.built_in);
        },
    )
}

fn add(url: &str, trust_root: &str, yes: bool, context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    let parsed = validate_url(url)?;
    let name = registry_name(&parsed)?;
    if name == OFFICIAL_NAME {
        bail!("registry name `a3s` is reserved for the built-in official registry");
    }
    let trust_root = resolve_trust_root(trust_root, context)?;
    if !yes {
        confirm(
            &format!("Trust registry `{name}` at {url} with root {trust_root}?"),
            context,
            "registry enrollment requires `--yes` in non-interactive mode",
        )?;
    }
    let path = registry_path(&name, context)?;
    if path.exists() {
        bail!("registry `{name}` already exists; remove it before changing its trust root");
    }
    let record = RegistryRecord {
        name: name.clone(),
        url: parsed.to_string(),
        trust_root,
        built_in: false,
    };
    write_registry(&path, &record)?;
    render_value(
        output,
        "registry.add",
        json!({"registry": record, "created": true}),
        || println!("added trusted registry `{name}`"),
    )
}

fn remove(name: &str, yes: bool, context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    validate_name(name)?;
    if name == OFFICIAL_NAME {
        bail!("the built-in official registry cannot be removed");
    }
    let path = registry_path(name, context)?;
    let record =
        read_registry(&path)?.with_context(|| format!("registry `{name}` is not configured"))?;
    if !yes {
        confirm(
            &format!("Remove trusted registry `{name}`?"),
            context,
            "registry removal requires `--yes` in non-interactive mode",
        )?;
    }
    std::fs::remove_file(&path)
        .with_context(|| format!("could not remove registry file {}", path.display()))?;
    render_value(
        output,
        "registry.remove",
        json!({"registry": record, "removed": true}),
        || println!("removed registry `{name}`"),
    )
}

async fn refresh(name: Option<&str>, context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    if context.network.offline {
        bail!("registry refresh is unavailable in offline mode");
    }
    let mut selected = registries(context)?;
    if let Some(name) = name {
        validate_name(name)?;
        selected.retain(|registry| registry.name == name);
        if selected.is_empty() {
            bail!("registry `{name}` is not configured");
        }
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let mut results = Vec::new();
    for registry in selected {
        let result = match client.head(&registry.url).send().await {
            Ok(response) => json!({
                "name": registry.name,
                "url": registry.url,
                "reachable": response.status().is_success() || response.status().is_redirection(),
                "status": response.status().as_u16(),
            }),
            Err(error) => json!({
                "name": registry.name,
                "url": registry.url,
                "reachable": false,
                "error": error.to_string(),
            }),
        };
        results.push(result);
    }
    let all_reachable = results
        .iter()
        .all(|result| result["reachable"].as_bool() == Some(true));
    if !all_reachable {
        bail!("one or more registries could not be reached");
    }
    render_value(
        output,
        "registry.refresh",
        json!({"registries": results}),
        || {
            for result in &results {
                println!(
                    "ok {} ({})",
                    result["name"].as_str().unwrap_or_default(),
                    result["status"].as_u64().unwrap_or_default()
                );
            }
        },
    )
}

fn registries(context: &InvocationContext) -> anyhow::Result<Vec<RegistryRecord>> {
    let mut records = vec![RegistryRecord {
        name: OFFICIAL_NAME.to_string(),
        url: OFFICIAL_URL.to_string(),
        trust_root: "built-in TUF root".to_string(),
        built_in: true,
    }];
    let root = registry_root(context)?;
    if root.is_dir() {
        for entry in std::fs::read_dir(root)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("acl") {
                continue;
            }
            if let Some(record) = read_registry(&path)? {
                records.push(record);
            }
        }
    }
    records.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(records)
}

fn write_registry(path: &Path, record: &RegistryRecord) -> anyhow::Result<()> {
    let attributes = HashMap::from([
        ("url".to_string(), Value::String(record.url.clone())),
        (
            "trust_root".to_string(),
            Value::String(record.trust_root.clone()),
        ),
    ]);
    let document = Document {
        blocks: vec![Block {
            name: "registry".to_string(),
            labels: vec![record.name.clone()],
            blocks: Vec::new(),
            attributes,
        }],
    };
    let rendered = a3s_acl::generate_acl(&document);
    a3s_acl::parse_acl(&rendered).context("generated registry ACL is invalid")?;
    crate::api::code_web::config::persistence::write_atomic(path, rendered.as_bytes())
        .map_err(|error| anyhow::anyhow!("could not write {}: {error}", path.display()))
}

fn read_registry(path: &Path) -> anyhow::Result<Option<RegistryRecord>> {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let document = a3s_acl::parse_acl(&source)
        .with_context(|| format!("invalid registry ACL {}", path.display()))?;
    let block = document
        .blocks
        .into_iter()
        .find(|block| block.name == "registry")
        .with_context(|| format!("registry ACL {} has no registry block", path.display()))?;
    let name = block
        .labels
        .first()
        .cloned()
        .context("registry block requires one name label")?;
    validate_name(&name)?;
    let url = block
        .attributes
        .get("url")
        .and_then(Value::as_str)
        .context("registry URL is missing")?
        .to_string();
    validate_url(&url)?;
    let trust_root = block
        .attributes
        .get("trust_root")
        .and_then(Value::as_str)
        .context("registry trust_root is missing")?
        .to_string();
    Ok(Some(RegistryRecord {
        name,
        url,
        trust_root,
        built_in: false,
    }))
}

fn registry_root(context: &InvocationContext) -> anyhow::Result<PathBuf> {
    let config = crate::commands::config::active_config_path(context)?;
    let parent = config.parent().unwrap_or_else(|| Path::new("."));
    Ok(parent.join("registries"))
}

fn registry_path(name: &str, context: &InvocationContext) -> anyhow::Result<PathBuf> {
    validate_name(name)?;
    Ok(registry_root(context)?.join(format!("{name}.acl")))
}

fn validate_url(url: &str) -> anyhow::Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url).map_err(|_| {
        invalid_registry_url(
            "registry URL is invalid",
            "Use an absolute HTTPS URL without credentials, query parameters, or fragments.",
        )
    })?;
    if parsed.scheme() != "https" {
        return Err(invalid_registry_url(
            "registry URLs must use HTTPS",
            "Use an https:// registry endpoint and establish trust with --trust-root.",
        ));
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid_registry_url(
            "registry URLs must not contain credentials, query parameters, or fragments",
            "Move authentication out of the URL and use a stable registry identity endpoint.",
        ));
    }
    Ok(parsed)
}

fn invalid_registry_url(message: &str, suggestion: &str) -> anyhow::Error {
    CliError::new("registry.url_invalid", message, ExitClass::Usage)
        .with_suggestion(suggestion)
        .with_details(json!({
            "requiredScheme": "https",
            "credentialsAllowed": false,
            "queryAllowed": false,
            "fragmentAllowed": false,
        }))
        .into()
}

fn registry_name(url: &reqwest::Url) -> anyhow::Result<String> {
    let host = url.host_str().context("registry URL requires a host")?;
    let mut name = host
        .trim_start_matches("www.")
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.retain(|character| character.is_ascii_alphanumeric() || character == '-');
    validate_name(&name)?;
    Ok(name)
}

fn validate_name(name: &str) -> anyhow::Result<()> {
    let mut characters = name.chars();
    if !characters
        .next()
        .is_some_and(|value| value.is_ascii_lowercase())
        || !characters
            .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == '-')
    {
        bail!("registry names use lowercase letters, digits, and hyphens and must start with a letter");
    }
    Ok(())
}

fn resolve_trust_root(value: &str, context: &InvocationContext) -> anyhow::Result<String> {
    if let Some(digest) = value.strip_prefix("sha256:") {
        if digest.len() == 64
            && digest
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Ok(format!("sha256:{}", digest.to_ascii_lowercase()));
        }
        bail!("SHA-256 trust roots require exactly 64 hexadecimal characters");
    }
    let path = context.resolve_path(value);
    let bytes = std::fs::read(&path)
        .with_context(|| format!("could not read trust-root file {}", path.display()))?;
    if bytes.is_empty() {
        bail!("trust-root file is empty");
    }
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn confirm(prompt: &str, context: &InvocationContext, non_interactive: &str) -> anyhow::Result<()> {
    if context.output_mode() != OutputMode::Human
        || context.interaction.non_interactive
        || !context.terminal.stdin
        || !context.terminal.stderr
    {
        bail!("{non_interactive}");
    }
    eprint!("{prompt} [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(coded_error(
            "operation.cancelled",
            "registry operation cancelled",
            ExitClass::Cancelled,
        ))
    }
}
