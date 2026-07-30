use std::process::ExitCode;

use a3s::plugin_manager::{
    PluginInstallationSnapshot, PluginManager, PluginManagerError, PluginManagerPolicy,
    PluginMarketplaceItem, PluginMarketplaceSnapshot, PluginPackageReadiness,
};
use serde_json::json;

use crate::cli::args::{OutputMode, PluginCommand, PluginInspectArgs, PluginSearchArgs};
use crate::cli::context::InvocationContext;
use crate::cli::output;

mod lifecycle;

pub(crate) async fn run(
    command: PluginCommand,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    if context.output_mode() == OutputMode::Jsonl {
        return Err(output::usage_error(
            "`a3s plugin` commands do not support JSONL output",
        ));
    }
    let config_path = crate::commands::config::active_config_path(context)?;
    let manager = PluginManager::from_host_with_policy(
        &config_path,
        &context.directory,
        PluginManagerPolicy {
            offline: context.network.offline,
        },
    )
    .map_err(manager_error)?;
    match command {
        PluginCommand::Search(args) => search(&manager, args, context).await,
        PluginCommand::Inspect(args) => inspect(&manager, args, context).await,
        PluginCommand::List => list(&manager, context).await,
        PluginCommand::Install(args) => lifecycle::install(&manager, args, context).await,
        PluginCommand::Upgrade(args) => lifecycle::upgrade(&manager, args, context).await,
        PluginCommand::Apply(args) => lifecycle::apply(&manager, args, context).await,
        PluginCommand::Enable(args) => lifecycle::set_enabled(&manager, args, true, context).await,
        PluginCommand::Disable(args) => {
            lifecycle::set_enabled(&manager, args, false, context).await
        }
        PluginCommand::Uninstall(args) => lifecycle::uninstall(&manager, args, context).await,
    }
}

async fn search(
    manager: &PluginManager,
    args: PluginSearchArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let query = args.query.trim();
    if query.is_empty() {
        return Err(output::usage_error("plugin search query cannot be empty"));
    }
    let installed = manager.installation_snapshot().await;
    let marketplace = manager
        .marketplace(&installed.index())
        .await
        .map_err(manager_error)?;
    let mut items = marketplace
        .items
        .iter()
        .filter(|item| search_match(item, &args))
        .cloned()
        .collect::<Vec<_>>();
    let total_matches = items.len();
    let truncated = marketplace.truncated || total_matches > args.limit;
    items.truncate(args.limit);
    let warnings = marketplace_warnings(&marketplace, &installed);
    let human_items = items.clone();
    let query_value = json!({
        "text": query,
        "surface": args.surface.map(|surface| surface.as_str()),
        "channel": args.channel.map(|channel| channel.as_str()),
        "publisher": args.publisher,
        "category": args.category,
        "limit": args.limit,
        "offline": context.network.offline,
    });
    let data = json!({
        "schemaVersion": marketplace.schema_version,
        "verifiedAt": marketplace.verified_at,
        "query": query_value,
        "registries": marketplace.registries,
        "items": items,
        "totalMatches": total_matches,
        "returnedItems": human_items.len(),
        "truncated": truncated,
    });
    let human_warnings = warnings.clone();
    output::render_value_with_warnings(
        context.output_mode(),
        "plugin.search",
        data,
        warnings,
        || {
            for warning in human_warnings {
                eprintln!("warning: {}", single_line(&warning, 300));
            }
            if human_items.is_empty() {
                println!("No verified plugins matched.");
                return;
            }
            for item in &human_items {
                println!(
                    "{}  {}  {}  [{}]{}",
                    item.package_id,
                    item.version,
                    single_line(&item.display_name, 80),
                    item.surface_kinds.join(","),
                    installation_suffix(item)
                );
                if let Some(description) = &item.description {
                    println!("  {}", single_line(description, 180));
                }
            }
            if truncated {
                println!("Results were truncated; narrow the query or filters.");
            }
        },
    )?;
    Ok(ExitCode::SUCCESS)
}

async fn inspect(
    manager: &PluginManager,
    args: PluginInspectArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let package_id = normalize_package_id(&args.package_id)?;
    let installed = manager.installation_snapshot().await;
    let marketplace = manager
        .marketplace(&installed.index())
        .await
        .map_err(manager_error)?;
    let matches = marketplace
        .items
        .iter()
        .filter(|item| {
            item.package_id == package_id
                && args
                    .channel
                    .is_none_or(|channel| item.channel == channel.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Err(output::CliError::new(
            "plugin.not_found",
            format!("verified plugin '{package_id}' was not found"),
            output::ExitClass::Failure,
        )
        .with_suggestion(
            "Run `a3s plugin search <query>` and inspect the returned package identity.",
        )
        .into());
    }
    let warnings = marketplace_warnings(&marketplace, &installed);
    let human_matches = matches.clone();
    let data = json!({
        "schemaVersion": marketplace.schema_version,
        "verifiedAt": marketplace.verified_at,
        "packageId": package_id,
        "registries": marketplace.registries,
        "matches": matches,
        "totalMatches": human_matches.len(),
        "offline": context.network.offline,
    });
    let human_warnings = warnings.clone();
    output::render_value_with_warnings(
        context.output_mode(),
        "plugin.inspect",
        data,
        warnings,
        || {
            for warning in human_warnings {
                eprintln!("warning: {}", single_line(&warning, 300));
            }
            for (index, item) in human_matches.iter().enumerate() {
                if index > 0 {
                    println!();
                }
                print_item(item);
            }
        },
    )?;
    Ok(ExitCode::SUCCESS)
}

async fn list(manager: &PluginManager, context: &InvocationContext) -> anyhow::Result<ExitCode> {
    let snapshot = manager.installation_snapshot().await;
    let data = serde_json::to_value(&snapshot)?;
    let warnings = snapshot
        .error
        .iter()
        .map(|error| format!("A3S Use installation state is unavailable: {error}"))
        .collect::<Vec<_>>();
    let human = snapshot.clone();
    let human_warnings = warnings.clone();
    output::render_value_with_warnings(
        context.output_mode(),
        "plugin.list",
        data,
        warnings,
        || {
            for warning in human_warnings {
                eprintln!("warning: {}", single_line(&warning, 300));
            }
            if human.items.is_empty() {
                println!("No installed plugins.");
                return;
            }
            for item in &human.items {
                println!(
                    "{}  {}  desired={}  callable={}  readiness={}",
                    item.package_id,
                    item.version,
                    if item.enabled { "enabled" } else { "disabled" },
                    item.callable,
                    readiness_name(item.readiness),
                );
            }
        },
    )?;
    Ok(ExitCode::SUCCESS)
}

fn search_match(item: &PluginMarketplaceItem, args: &PluginSearchArgs) -> bool {
    let query = args.query.trim().to_lowercase();
    let text_match = [
        Some(item.package_id.as_str()),
        Some(item.component_id.as_str()),
        Some(item.display_name.as_str()),
        item.description.as_deref(),
        item.publisher.as_deref(),
    ]
    .into_iter()
    .flatten()
    .chain(item.keywords.iter().map(String::as_str))
    .chain(item.categories.iter().map(String::as_str))
    .any(|value| value.to_lowercase().contains(&query));
    text_match
        && args.surface.is_none_or(|surface| {
            item.surface_kinds
                .iter()
                .any(|kind| kind == surface.as_str())
        })
        && args
            .channel
            .is_none_or(|channel| item.channel == channel.as_str())
        && args.publisher.as_deref().is_none_or(|publisher| {
            item.package_id
                .split_once('/')
                .is_some_and(|(id, _)| id.eq_ignore_ascii_case(publisher))
                || item
                    .publisher
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(publisher))
        })
        && args.category.as_deref().is_none_or(|category| {
            item.categories
                .iter()
                .any(|value| value.eq_ignore_ascii_case(category))
        })
}

fn marketplace_warnings(
    marketplace: &PluginMarketplaceSnapshot,
    installed: &PluginInstallationSnapshot,
) -> Vec<String> {
    let mut warnings = marketplace
        .registries
        .iter()
        .filter_map(|source| {
            source
                .error
                .as_ref()
                .map(|error| format!("{}: {error}", source.name))
        })
        .collect::<Vec<_>>();
    if let Some(error) = &installed.error {
        warnings.push(format!(
            "installed plugin state is unavailable; catalog installation flags may be incomplete: {error}"
        ));
    }
    warnings
}

fn normalize_package_id(value: &str) -> anyhow::Result<String> {
    let value = value.trim();
    let segments = value.split('/').collect::<Vec<_>>();
    if segments.len() != 2 || !segments.iter().copied().all(valid_segment) {
        return Err(output::usage_error(
            "plugin package IDs must use publisher/name syntax",
        ));
    }
    Ok(value.to_string())
}

fn valid_segment(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn manager_error(error: PluginManagerError) -> anyhow::Error {
    let (code, class) = match error {
        PluginManagerError::InvalidRequest(_) => {
            ("plugin.request_invalid", output::ExitClass::Usage)
        }
        PluginManagerError::Timeout(_) => ("plugin.timeout", output::ExitClass::Failure),
        PluginManagerError::OperationFailed(_) => {
            ("plugin.operation_failed", output::ExitClass::Failure)
        }
        PluginManagerError::Upstream(_) => ("plugin.upstream_failed", output::ExitClass::Failure),
        PluginManagerError::Infrastructure(_) => {
            ("plugin.infrastructure_failed", output::ExitClass::Failure)
        }
    };
    output::coded_error(code, error.to_string(), class)
}

fn print_item(item: &PluginMarketplaceItem) {
    println!(
        "{} ({})",
        single_line(&item.display_name, 100),
        item.package_id
    );
    println!("version: {} ({})", item.version, item.channel);
    println!(
        "source: {} <{}>",
        single_line(&item.registry_name, 80),
        single_line(&item.registry_url, 200)
    );
    println!("target: {}", item.target);
    println!(
        "archive: {} ({})",
        item.archive_name,
        format_bytes(item.length)
    );
    println!("sha256: {}", item.sha256);
    println!("surfaces: {}", item.surface_kinds.join(", "));
    println!(
        "state: {}",
        if item.installed {
            if item.enabled {
                "installed, enabled"
            } else {
                "installed, disabled"
            }
        } else {
            "not installed"
        }
    );
    if let Some(description) = &item.description {
        println!("description: {}", single_line(description, 300));
    }
    if let Some(digest) = &item.permission_ceiling_digest {
        println!("permission ceiling: {digest}");
    }
    if let Some(repository) = &item.repository {
        println!("repository: {}", single_line(repository, 200));
    }
}

fn installation_suffix(item: &PluginMarketplaceItem) -> &'static str {
    if !item.installed {
        ""
    } else if item.enabled {
        "  installed+enabled"
    } else {
        "  installed+disabled"
    }
}

fn readiness_name(readiness: PluginPackageReadiness) -> &'static str {
    match readiness {
        PluginPackageReadiness::Ready => "ready",
        PluginPackageReadiness::Missing => "missing",
        PluginPackageReadiness::Broken => "broken",
        PluginPackageReadiness::Unknown => "unknown",
    }
}

fn single_line(value: &str, max_chars: usize) -> String {
    let value = value
        .chars()
        .map(|character| {
            if unsafe_terminal_character(character) {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let mut output = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if output.chars().count() > max_chars {
        output = output.chars().take(max_chars.saturating_sub(1)).collect();
        output.push('\u{2026}');
    }
    output
}

fn unsafe_terminal_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{061c}'
        )
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_identity_is_canonical_and_not_component_prefixed() {
        assert_eq!(
            normalize_package_id("acme/research").unwrap(),
            "acme/research"
        );
        assert!(normalize_package_id("use/acme/research").is_err());
        assert!(normalize_package_id("../research").is_err());
        assert!(normalize_package_id("Acme/research").is_err());
    }

    #[test]
    fn human_metadata_is_single_line_and_bounded() {
        assert_eq!(
            single_line("hello\n\u{1b}[31m\u{202e}world", 20),
            "hello [31m world"
        );
        assert_eq!(single_line("abcdef", 4), "abc\u{2026}");
    }
}
