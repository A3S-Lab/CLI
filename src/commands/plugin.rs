use std::collections::BTreeSet;
use std::process::ExitCode;

use a3s::plugin_manager::{PluginManager, PluginManagerError, PluginManagerPolicy};
use a3s_use::cognitive_package::CognitiveRegistryAccess;
use a3s_use::plugin_manager::{
    PluginManagerInstalledPage, PluginManagerSearchResult, PluginManagerService,
};
use a3s_use_core::{
    PlanScopeKind, PluginDesiredState, PluginManagerInspectInput, PluginManagerListInstalledInput,
    PluginManagerSearchInput, PluginObservedState, PluginPackageId, PluginReleaseChannel,
    PluginSurfaceKind, UseError, VerifiedPluginCatalogRecord,
};

use crate::cli::args::{
    OutputMode, PluginChannelArg, PluginCommand, PluginInspectArgs, PluginSearchArgs,
    PluginSurfaceArg,
};
use crate::cli::context::InvocationContext;
use crate::cli::output;

mod lifecycle;
mod policy;
pub(crate) use policy::{
    load_forwarded_or_host_authorization, load_host_authorization_context, HostPluginAuthorization,
};

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
    let authorization = if matches!(&command, PluginCommand::McpServe) {
        load_forwarded_or_host_authorization(context).await?
    } else {
        load_host_authorization_context(context).await?
    };
    let manager = PluginManager::from_host_with_policy_and_runtime_config(
        &config_path,
        &context.directory,
        authorization.handoff().source(),
        PluginManagerPolicy {
            offline: context.network.offline,
            authorization: authorization.policy().clone(),
        },
    )
    .await
    .map_err(manager_error)?;
    let registry_access = if context.network.offline {
        CognitiveRegistryAccess::Cached
    } else {
        CognitiveRegistryAccess::Refreshed
    };
    let result = match manager.shared_service().map_err(manager_error) {
        Ok(service) => match command {
            PluginCommand::Search(args) => search(&service, args, registry_access, context).await,
            PluginCommand::Inspect(args) => inspect(&service, args, registry_access, context).await,
            PluginCommand::List => list(&service, context).await,
            PluginCommand::Install(args) => lifecycle::install(&service, args, context).await,
            PluginCommand::Upgrade(args) => lifecycle::upgrade(&service, args, context).await,
            PluginCommand::Apply(args) => lifecycle::apply(&service, args, context).await,
            PluginCommand::Enable(args) => {
                lifecycle::set_enabled(&service, args, true, context).await
            }
            PluginCommand::Disable(args) => {
                lifecycle::set_enabled(&service, args, false, context).await
            }
            PluginCommand::Uninstall(args) => lifecycle::uninstall(&service, args, context).await,
            PluginCommand::McpServe => {
                a3s::plugin_manager_mcp::serve_stdio(service, registry_access)
                    .await
                    .map(|()| ExitCode::SUCCESS)
            }
        },
        Err(error) => Err(error),
    };
    manager.shutdown().await;
    result
}

async fn search(
    service: &PluginManagerService,
    args: PluginSearchArgs,
    registry_access: CognitiveRegistryAccess,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let query = args.query.trim();
    if query.is_empty() {
        return Err(output::usage_error("plugin search query cannot be empty"));
    }
    let result = service
        .search(
            PluginManagerSearchInput {
                query: query.to_string(),
                kind: args.surface.map(surface_kind),
                channel: args.channel.map(release_channel),
                cursor: None,
                limit: Some(u16::try_from(args.limit).map_err(|_| {
                    output::usage_error("plugin search limit exceeds the manager contract")
                })?),
            },
            registry_access,
        )
        .await
        .map_err(service_error)?;
    render_search(result, context)
}

fn render_search(
    result: PluginManagerSearchResult,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let data = serde_json::to_value(&result)?;
    output::render_value(context.output_mode(), "plugin.search", data, || {
        if result.plugins.is_empty() {
            println!("No verified plugins matched.");
            return;
        }
        for plugin in &result.plugins {
            print_search_item(plugin);
        }
        if !result.next_cursors.is_empty() {
            println!("More verified releases are available through manager pagination.");
        }
    })?;
    Ok(ExitCode::SUCCESS)
}

async fn inspect(
    service: &PluginManagerService,
    args: PluginInspectArgs,
    registry_access: CognitiveRegistryAccess,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let package_id =
        PluginPackageId::parse(normalize_package_id(&args.package_id)?).map_err(service_error)?;
    let inspection = service
        .inspect(
            PluginManagerInspectInput {
                package_id,
                version: None,
                channel: args.channel.map(release_channel),
            },
            registry_access,
        )
        .await
        .map_err(service_error)?;
    let data = serde_json::to_value(&inspection)?;
    output::render_value(context.output_mode(), "plugin.inspect", data, || {
        print_inspection(&inspection.plugin);
    })?;
    Ok(ExitCode::SUCCESS)
}

async fn list(
    service: &PluginManagerService,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let page = installed_page(service).await?;
    let data = serde_json::to_value(&page)?;
    output::render_value(context.output_mode(), "plugin.list", data, || {
        if page.packages.is_empty() {
            println!("No installed plugins.");
            return;
        }
        for package in &page.packages {
            println!(
                "{}  {}  desired={}  observed={}",
                package.package_id,
                package.state.version.as_deref().unwrap_or("unknown"),
                desired_state_name(package.state.desired),
                observed_state_name(package.state.observed),
            );
        }
    })?;
    Ok(ExitCode::SUCCESS)
}

async fn installed_page(
    service: &PluginManagerService,
) -> anyhow::Result<PluginManagerInstalledPage> {
    const PAGE_LIMIT: u16 = 100;
    const MAX_PACKAGES: usize = 1_000;

    let mut cursor = None;
    let mut snapshot_digest = None;
    let mut scope = None;
    let mut identities = BTreeSet::new();
    let mut packages = Vec::new();
    loop {
        let page = service
            .list_installed(PluginManagerListInstalledInput {
                scope_kind: PlanScopeKind::User,
                scope_id: a3s_use::COGNITIVE_PACKAGE_DEFAULT_SCOPE.to_string(),
                cursor,
                limit: Some(PAGE_LIMIT),
            })
            .await
            .map_err(service_error)?;
        if snapshot_digest
            .as_deref()
            .is_some_and(|digest| digest != page.snapshot_digest)
        {
            return Err(output::coded_error(
                "plugin.list_unstable",
                "installed plugin state changed while the command was loading it",
                output::ExitClass::Failure,
            ));
        }
        snapshot_digest = Some(page.snapshot_digest.clone());
        scope.get_or_insert_with(|| page.scope.clone());
        for package in page.packages {
            if !identities.insert(package.package_id.clone()) {
                return Err(output::coded_error(
                    "plugin.list_invalid",
                    "the shared Plugin Manager returned a duplicate installed package",
                    output::ExitClass::Failure,
                ));
            }
            packages.push(package);
            if packages.len() > MAX_PACKAGES {
                return Err(output::coded_error(
                    "plugin.list_too_large",
                    format!("installed plugin state exceeds the supported {MAX_PACKAGES} packages"),
                    output::ExitClass::Failure,
                ));
            }
        }
        let Some(next_cursor) = page.next_cursor else {
            return Ok(PluginManagerInstalledPage {
                scope: scope.unwrap_or_else(|| service.managed_scope().plan_scope()),
                snapshot_digest: snapshot_digest.unwrap_or_default(),
                packages,
                next_cursor: None,
            });
        };
        cursor = Some(next_cursor);
    }
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

fn service_error(error: UseError) -> anyhow::Error {
    let UseError {
        code,
        message,
        suggestion,
        details,
    } = error;
    let class = match code.as_str() {
        "use.plugin.manager_input_invalid" | "use.plugin.package_id_invalid" => {
            output::ExitClass::Usage
        }
        _ => output::ExitClass::Failure,
    };
    let details = serde_json::to_value(details).unwrap_or_else(|_| serde_json::json!({}));
    let mut error = output::CliError::new(code, message, class).with_details(details);
    if let Some(suggestion) = suggestion {
        error = error.with_suggestion(suggestion);
    }
    error.into()
}

fn print_search_item(plugin: &VerifiedPluginCatalogRecord) {
    let record = &plugin.record;
    let surface_kinds = record
        .surfaces
        .iter()
        .map(|surface| surface_kind_name(surface.kind))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{}  {}  {}  [{}]",
        record.package_id,
        record.version,
        single_line(&record.display_name, 80),
        surface_kinds,
    );
    println!("  {}", single_line(&record.description, 180));
}

fn print_inspection(plugin: &VerifiedPluginCatalogRecord) {
    let record = &plugin.record;
    let provenance = &plugin.provenance;
    println!(
        "{} ({})",
        single_line(&record.display_name, 100),
        record.package_id
    );
    println!(
        "version: {} ({})",
        record.version,
        release_channel_name(record.channel)
    );
    println!(
        "source: {} <{}>",
        single_line(&provenance.registry_name, 80),
        single_line(&provenance.registry_url, 200)
    );
    println!("target: {}", record.target);
    println!(
        "archive: {} ({})",
        record.archive.target_name,
        format_bytes(record.archive.length)
    );
    println!("sha256: {}", record.archive.sha256);
    println!(
        "surfaces: {}",
        record
            .surfaces
            .iter()
            .map(|surface| format!("{}/{}", surface_kind_name(surface.kind), surface.id))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("description: {}", single_line(&record.description, 300));
    println!("permission ceiling: {}", record.permission_ceiling_digest);
    println!("repository: {}", single_line(&record.repository, 200));
}

pub(super) const fn release_channel(channel: PluginChannelArg) -> PluginReleaseChannel {
    match channel {
        PluginChannelArg::Stable => PluginReleaseChannel::Stable,
        PluginChannelArg::Beta => PluginReleaseChannel::Beta,
        PluginChannelArg::Nightly => PluginReleaseChannel::Nightly,
    }
}

const fn release_channel_name(channel: PluginReleaseChannel) -> &'static str {
    match channel {
        PluginReleaseChannel::Stable => "stable",
        PluginReleaseChannel::Beta => "beta",
        PluginReleaseChannel::Nightly => "nightly",
    }
}

const fn surface_kind(surface: PluginSurfaceArg) -> PluginSurfaceKind {
    match surface {
        PluginSurfaceArg::Flow => PluginSurfaceKind::Flow,
        PluginSurfaceArg::Mcp => PluginSurfaceKind::Mcp,
        PluginSurfaceArg::Okf => PluginSurfaceKind::Okf,
        PluginSurfaceArg::Skill => PluginSurfaceKind::Skill,
        PluginSurfaceArg::Tool => PluginSurfaceKind::Tool,
        PluginSurfaceArg::Ui => PluginSurfaceKind::Ui,
    }
}

const fn surface_kind_name(kind: PluginSurfaceKind) -> &'static str {
    match kind {
        PluginSurfaceKind::Flow => "flow",
        PluginSurfaceKind::Mcp => "mcp",
        PluginSurfaceKind::Okf => "okf",
        PluginSurfaceKind::Skill => "skill",
        PluginSurfaceKind::Tool => "tool",
        PluginSurfaceKind::Ui => "ui",
    }
}

const fn desired_state_name(state: PluginDesiredState) -> &'static str {
    match state {
        PluginDesiredState::Absent => "absent",
        PluginDesiredState::InstalledDisabled => "installed-disabled",
        PluginDesiredState::Enabled => "enabled",
    }
}

const fn observed_state_name(state: PluginObservedState) -> &'static str {
    match state {
        PluginObservedState::Installed => "installed",
        PluginObservedState::Reconciling => "reconciling",
        PluginObservedState::Ready => "ready",
        PluginObservedState::Degraded => "degraded",
        PluginObservedState::Broken => "broken",
        PluginObservedState::Incompatible => "incompatible",
        PluginObservedState::Draining => "draining",
        PluginObservedState::Removed => "removed",
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
