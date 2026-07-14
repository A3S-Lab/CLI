use a3s_code_core::config::CodeConfig;
use std::path::Path;

use crate::config;

use super::catalog::ModelCatalog;
use super::route::ModelRoute;
use super::selection::{self, ModelSelection};

pub(crate) const USAGE: &str = "usage:\n\
  a3s model list\n\
  a3s model current\n\
  a3s model use <route>\n\
  a3s model reset\n\
  a3s model config\n\n\
Routes from config.acl use provider/model. Account routes use\n\
claude-code/model, codex/model, or a3s-os/model.\n";

pub(crate) async fn run(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        None | Some("list") => {
            ensure_exact_args("model list", &args[usize::from(!args.is_empty())..], 0)?;
            list().await
        }
        Some("current") => {
            ensure_exact_args("model current", &args[1..], 0)?;
            current()
        }
        Some("use") => {
            ensure_exact_args("model use", &args[1..], 1)?;
            use_route(&args[1]).await
        }
        Some("reset") => {
            ensure_exact_args("model reset", &args[1..], 0)?;
            reset()
        }
        Some("config") => {
            ensure_exact_args("model config", &args[1..], 0)?;
            match config::find_config() {
                Some(path) => println!("{path}"),
                None => println!(
                    "{}",
                    config::default_config_path()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "(HOME is not set)".to_string())
                ),
            }
            Ok(())
        }
        Some("-h" | "--help" | "help") if args.len() == 1 => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => anyhow::bail!("unknown model command `{other}`\n\n{USAGE}"),
    }
}

async fn list() -> anyhow::Result<()> {
    let catalog = ModelCatalog::discover(true).await;
    let current = selection::load()
        .and_then(|selection| selection.route().ok())
        .or_else(|| catalog.config_default.as_deref()?.parse().ok());
    if catalog.entries.is_empty() {
        println!("No models available.");
    } else {
        println!("ROUTE\tSOURCE\tNAME\tCAPABILITIES");
        for entry in &catalog.entries {
            let marker = if current.as_ref() == Some(&entry.route) {
                "*"
            } else {
                " "
            };
            let mut capabilities = Vec::new();
            if entry.reasoning {
                capabilities.push("reasoning".to_string());
            }
            if entry.tool_call {
                capabilities.push("tools".to_string());
            }
            if let Some(context) = entry.context_window {
                capabilities.push(format!("context={context}"));
            }
            println!(
                "{marker} {}\t{}\t{}\t{}",
                entry.route,
                entry.route.source.label(),
                entry.display_name,
                capabilities.join(",")
            );
        }
    }
    for warning in catalog.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}

fn current() -> anyhow::Result<()> {
    match selection::load() {
        Some(selection) => println!("{}", selection.route()?),
        None => match configured_default() {
            Some(default) => println!("{default} (config.acl default)"),
            None => println!("(not selected)"),
        },
    }
    Ok(())
}

async fn use_route(value: &str) -> anyhow::Result<()> {
    let route: ModelRoute = value.parse()?;
    if !ModelCatalog::route_available(&route).await {
        anyhow::bail!(
            "model route `{route}` is not available; run `a3s model list` to inspect available routes"
        );
    }
    selection::save(&ModelSelection::from(route.clone()))?;
    println!("Active model: {route}");
    println!("Controller: A3S Code");
    Ok(())
}

fn reset() -> anyhow::Result<()> {
    let removed = selection::reset()?;
    match configured_default() {
        Some(default) => println!("Active model: {default} (config.acl default)"),
        None if removed => println!("Model selection cleared; no config.acl default is set."),
        None => println!("No saved model selection; no config.acl default is set."),
    }
    Ok(())
}

fn configured_default() -> Option<String> {
    let path = config::find_config()?;
    CodeConfig::from_file(Path::new(&path)).ok()?.default_model
}

fn ensure_exact_args(command: &str, args: &[String], expected: usize) -> anyhow::Result<()> {
    if args.len() == expected {
        Ok(())
    } else {
        anyhow::bail!("{command} expects {expected} argument(s)\n\n{USAGE}")
    }
}
