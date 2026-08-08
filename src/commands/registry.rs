use std::io::Write;

use a3s::registry::RegistryStore;
use a3s_use_extension::{
    refresh_remote_registry, RegistrySourceInput, RegistrySourceMutation, VerifiedTargetCachePolicy,
};
use anyhow::{bail, Context};
use serde_json::json;

use crate::cli::args::{OutputMode, RegistryArgs, RegistryCommand};
use crate::cli::context::InvocationContext;
use crate::cli::output::{coded_error, render_value, ExitClass};

pub(crate) async fn run(args: RegistryArgs, context: &InvocationContext) -> anyhow::Result<()> {
    match args.command {
        RegistryCommand::List => list(context).await,
        RegistryCommand::Show(args) => show(&args.name, context).await,
        RegistryCommand::Add(args) => {
            add(
                &args.name,
                &args.url,
                &args.root_sha256,
                args.trusted_root.as_deref(),
                args.yes,
                context,
            )
            .await
        }
        RegistryCommand::Replace(args) => {
            replace(
                &args.name,
                &args.url,
                &args.root_sha256,
                args.trusted_root.as_deref(),
                &args.revision,
                args.yes,
                context,
            )
            .await
        }
        RegistryCommand::Default(args) => {
            set_default(&args.name, &args.revision, args.yes, context).await
        }
        RegistryCommand::Enable(args) => {
            set_enabled(&args.name, true, &args.revision, args.yes, context).await
        }
        RegistryCommand::Disable(args) => {
            set_enabled(&args.name, false, &args.revision, args.yes, context).await
        }
        RegistryCommand::Remove(args) => {
            remove(&args.name, &args.revision, args.yes, context).await
        }
        RegistryCommand::Refresh(args) => refresh(args.name.as_deref(), context).await,
    }
}

async fn list(context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    let snapshot = store(context)?.snapshot().await?;
    let human = snapshot.clone();
    render_value(
        output,
        "registry.list",
        json!({"registrySources": snapshot}),
        || {
            println!("revision: {}", human.revision);
            println!(
                "default: {}",
                human.default_registry.as_deref().unwrap_or("none")
            );
            if human.sources.is_empty() {
                println!("No A3S Use Registry sources configured.");
                return;
            }
            println!();
            println!("REGISTRY                 STATE       ROOT SHA-256");
            for source in &human.sources {
                let state = if source.enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                let default = if human.default_registry.as_deref() == Some(&source.name) {
                    " *"
                } else {
                    ""
                };
                println!(
                    "{:<24} {:<11} {}{}",
                    source.name, state, source.root_sha256, default
                );
                println!("  {}", source.registry_url);
            }
        },
    )
}

async fn show(name: &str, context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    let snapshot = store(context)?.snapshot().await?;
    let source = snapshot
        .sources
        .iter()
        .find(|source| source.name == name)
        .cloned()
        .with_context(|| format!("Registry source '{name}' is not configured"))?;
    let is_default = snapshot.default_registry.as_deref() == Some(name);
    let human = source.clone();
    render_value(
        output,
        "registry.show",
        json!({
            "revision": snapshot.revision,
            "default": is_default,
            "registry": source,
        }),
        || {
            println!("name: {}", human.name);
            println!("url: {}", human.registry_url);
            println!("root SHA-256: {}", human.root_sha256);
            println!("source identity: {}", human.source_identity);
            println!("trusted root imported: {}", human.imported_trusted_root);
            println!("enabled: {}", human.enabled);
            println!("default: {is_default}");
        },
    )
}

async fn add(
    name: &str,
    url: &str,
    root_sha256: &str,
    trusted_root: Option<&std::path::Path>,
    yes: bool,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    require_authority(
        yes,
        &format!("Trust and add Registry source '{name}' at {url} with root {root_sha256}?"),
        "Registry source enrollment requires '--yes' in non-interactive mode",
        context,
    )?;
    let trusted_root = trusted_root.map(|path| context.resolve_path(path));
    let mutation = store(context)?
        .source_store()
        .add(source_input(name, url, root_sha256, trusted_root))
        .await
        .map_err(anyhow::Error::new)?;
    render_mutation("registry.add", mutation, context)
}

#[allow(clippy::too_many_arguments)]
async fn replace(
    name: &str,
    url: &str,
    root_sha256: &str,
    trusted_root: Option<&std::path::Path>,
    revision: &str,
    yes: bool,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    require_authority(
        yes,
        &format!(
            "Replace Registry source '{name}' at reviewed revision {revision} with {url} ({root_sha256})?"
        ),
        "Registry source replacement requires '--yes' in non-interactive mode",
        context,
    )?;
    let trusted_root = trusted_root.map(|path| context.resolve_path(path));
    let mutation = store(context)?
        .source_store()
        .replace(revision, source_input(name, url, root_sha256, trusted_root))
        .await
        .map_err(anyhow::Error::new)?;
    render_mutation("registry.replace", mutation, context)
}

async fn set_default(
    name: &str,
    revision: &str,
    yes: bool,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    require_authority(
        yes,
        &format!("Select Registry source '{name}' as default at reviewed revision {revision}?"),
        "Registry default selection requires '--yes' in non-interactive mode",
        context,
    )?;
    let mutation = store(context)?
        .source_store()
        .set_default(name, revision)
        .await
        .map_err(anyhow::Error::new)?;
    render_mutation("registry.default", mutation, context)
}

async fn set_enabled(
    name: &str,
    enabled: bool,
    revision: &str,
    yes: bool,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    let action = if enabled { "enable" } else { "disable" };
    require_authority(
        yes,
        &format!("{action} Registry source '{name}' at reviewed revision {revision}?"),
        &format!("Registry source {action} requires '--yes' in non-interactive mode"),
        context,
    )?;
    let sources = store(context)?;
    let mutation = if enabled {
        sources.source_store().enable(name, revision).await
    } else {
        sources.source_store().disable(name, revision).await
    }
    .map_err(anyhow::Error::new)?;
    render_mutation(
        if enabled {
            "registry.enable"
        } else {
            "registry.disable"
        },
        mutation,
        context,
    )
}

async fn remove(
    name: &str,
    revision: &str,
    yes: bool,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    require_authority(
        yes,
        &format!("Remove Registry source '{name}' at reviewed revision {revision}?"),
        "Registry source removal requires '--yes' in non-interactive mode",
        context,
    )?;
    let mutation = store(context)?
        .source_store()
        .remove(name, revision)
        .await
        .map_err(anyhow::Error::new)?;
    render_mutation("registry.remove", mutation, context)
}

async fn refresh(name: Option<&str>, context: &InvocationContext) -> anyhow::Result<()> {
    if context.network.offline {
        bail!("Registry refresh is unavailable in offline mode");
    }
    let sources = store(context)?;
    let snapshot = sources.snapshot().await?;
    let selected = match name {
        Some(name) => snapshot
            .sources
            .iter()
            .find(|source| source.name == name)
            .map(|source| vec![source.clone()])
            .with_context(|| format!("Registry source '{name}' is not configured"))?,
        None => snapshot.sources.clone(),
    };
    let mut results = Vec::new();
    for source in selected {
        if !source.enabled {
            if name.is_some() {
                bail!(
                    "Registry source '{}' is disabled; enable it before refreshing",
                    source.name
                );
            }
            results.push(json!({
                "name": source.name,
                "url": source.registry_url,
                "enabled": false,
                "verified": false,
            }));
            continue;
        }
        let resolved = sources.resolve(Some(&source.name)).await?;
        let metadata = refresh_remote_registry(resolved.root())
            .await
            .map_err(anyhow::Error::new)?;
        results.push(json!({
            "name": source.name,
            "url": source.registry_url,
            "enabled": true,
            "verified": true,
            "metadata": metadata,
        }));
    }
    let human = results.clone();
    render_value(
        context.output_mode(),
        "registry.refresh",
        json!({"revision": snapshot.revision, "registries": results}),
        || {
            for result in &human {
                if result["enabled"].as_bool() == Some(false) {
                    println!("disabled {}", result["name"].as_str().unwrap_or_default());
                } else {
                    println!(
                        "verified {} (root {}, timestamp {}, snapshot {}, targets {})",
                        result["name"].as_str().unwrap_or_default(),
                        result["metadata"]["rootVersion"]
                            .as_u64()
                            .unwrap_or_default(),
                        result["metadata"]["timestampVersion"]
                            .as_u64()
                            .unwrap_or_default(),
                        result["metadata"]["snapshotVersion"]
                            .as_u64()
                            .unwrap_or_default(),
                        result["metadata"]["targetsVersion"]
                            .as_u64()
                            .unwrap_or_default(),
                    );
                }
            }
        },
    )
}

fn source_input(
    name: &str,
    url: &str,
    root_sha256: &str,
    trusted_root: Option<std::path::PathBuf>,
) -> RegistrySourceInput {
    RegistrySourceInput::new(
        name,
        url,
        root_sha256,
        trusted_root,
        VerifiedTargetCachePolicy::default(),
    )
}

fn render_mutation(
    command: &'static str,
    mutation: RegistrySourceMutation,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    let action = mutation.action.clone();
    let changed = mutation.changed;
    let revision = mutation.snapshot.revision.clone();
    render_value(
        context.output_mode(),
        command,
        json!({"registrySources": mutation}),
        || {
            if changed {
                println!("Registry source {action} committed at revision {revision}");
            } else {
                println!("Registry source {action} made no change; revision {revision}");
            }
        },
    )
}

pub(crate) fn store(context: &InvocationContext) -> anyhow::Result<RegistryStore> {
    Ok(RegistryStore::from_component_paths(
        &context.component_paths,
        context.network.offline,
    ))
}

fn require_authority(
    yes: bool,
    prompt: &str,
    non_interactive: &str,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    if yes {
        return Ok(());
    }
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
            "Registry source operation cancelled",
            ExitClass::Cancelled,
        ))
    }
}
