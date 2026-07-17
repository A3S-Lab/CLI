use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitStatus;

use a3s_updater::{fetch_latest_release, parse_version, InstallProvenance};
use anyhow::{bail, Context};
use serde::Serialize;

use super::catalog::{self, ComponentKind, Distribution};
use super::discovery::{discover, find_state};
use super::id::ComponentId;
use super::lifecycle::{
    install_component, uninstall_component, InstallIntent, InstallRequest, InstallSource,
    OperationRecord,
};
use super::paths::ComponentPaths;
use super::state::{ComponentReport, Health, Presence, UpdateState};

mod options;

use options::{
    DoctorOptions, InfoOptions, InstallOptions, ListOptions, UninstallOptions, UpdateOptions,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentFailure {
    pub component: ComponentId,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentBatchFailure {
    pub action: &'static str,
    pub operations: Vec<OperationRecord>,
    pub failures: Vec<ComponentFailure>,
}

impl ComponentBatchFailure {
    pub fn is_partial(&self) -> bool {
        !self.operations.is_empty() && !self.failures.is_empty()
    }
}

impl std::fmt::Display for ComponentBatchFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "component {} completed {} target(s) and failed {} target(s)",
            self.action,
            self.operations.len(),
            self.failures.len()
        )?;
        if let Some(failure) = self.failures.first() {
            write!(
                formatter,
                "; first failure: {}: {}",
                failure.component, failure.message
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for ComponentBatchFailure {}

pub async fn run_list(args: Vec<String>) -> anyhow::Result<()> {
    let paths = ComponentPaths::from_env()?;
    let offline = environment_flag("A3S_OFFLINE");
    run_list_with(args, &paths, offline).await
}

pub async fn run_list_with(
    args: Vec<String>,
    paths: &ComponentPaths,
    offline: bool,
) -> anyhow::Result<()> {
    run_list_with_command(args, paths, offline, "component.list", false).await
}

pub async fn run_upgrade_list_with(
    args: Vec<String>,
    paths: &ComponentPaths,
    offline: bool,
) -> anyhow::Result<()> {
    run_list_with_command(args, paths, offline, "component.upgrade", true).await
}

async fn run_list_with_command(
    args: Vec<String>,
    paths: &ComponentPaths,
    offline: bool,
    command: &'static str,
    managed_upgrades_only: bool,
) -> anyhow::Result<()> {
    let options = ListOptions::parse(&args)?;
    let mut report = discover(paths)?;
    if options.check_updates {
        if offline {
            bail!("component update checks are unavailable in offline mode");
        }
        populate_updates(&mut report).await;
    }
    report.components.retain(|component| {
        (!options.installed || component.presence != Presence::Missing)
            && (!options.available || component.presence == Presence::Missing)
            && (!options.check_updates || component.update == UpdateState::Available)
            && options.kind.is_none_or(|kind| component.kind == kind)
            && (!managed_upgrades_only || is_upgrade_all_candidate(component))
    });
    if managed_upgrades_only {
        report.external_tools.clear();
    }
    if options.json {
        print_json(command, &report, true)?;
    } else {
        print_human_report(&report);
    }
    Ok(())
}

pub async fn run_install(args: Vec<String>) -> anyhow::Result<()> {
    let paths = ComponentPaths::from_env()?;
    let offline = environment_flag("A3S_OFFLINE");
    let progress = !environment_flag("A3S_NO_PROGRESS") && !environment_flag("A3S_QUIET");
    run_install_with(args, &paths, offline, progress).await
}

pub async fn run_install_with(
    args: Vec<String>,
    paths: &ComponentPaths,
    offline: bool,
    progress: bool,
) -> anyhow::Result<()> {
    let options = InstallOptions::parse(&args)?;
    if options.components.is_empty() {
        return print_available(options.json);
    }
    validate_supported_install_policy(&options)?;
    enforce_provenance_policy(&options, paths)?;
    let request = InstallRequest {
        version: options.version.clone(),
        source: options.source,
        intent: InstallIntent::Install,
        package: options.package.clone(),
        force: options.force,
        allow_unsigned: options.allow_unsigned,
        progress,
    };
    if options.dry_run {
        let plans = options
            .components
            .iter()
            .map(|component| install_plan(component, &request, &options, paths))
            .collect::<anyhow::Result<Vec<_>>>()?;
        return print_plans("component.install", &plans, options.json);
    }
    enforce_install_network_policy(&options.components, &request, paths, offline)?;
    let mut operations = Vec::new();
    let mut failures = Vec::new();
    for component in options.components {
        match install_component(&component, &request, paths).await {
            Ok(operation) => operations.push(operation),
            Err(error) => failures.push(ComponentFailure {
                component,
                message: format!("{error:#}"),
            }),
        }
    }
    finish_batch(
        "component.install",
        "install",
        operations,
        failures,
        options.json,
    )
}

pub fn run_uninstall(args: Vec<String>) -> anyhow::Result<()> {
    let paths = ComponentPaths::from_env()?;
    run_uninstall_with(args, &paths)
}

pub fn run_uninstall_with(args: Vec<String>, paths: &ComponentPaths) -> anyhow::Result<()> {
    let options = UninstallOptions::parse(&args)?;
    if options.components.is_empty() {
        bail!("usage: a3s uninstall <component>... [--cascade] [--purge]");
    }
    if options.dry_run {
        let plans = options
            .components
            .iter()
            .map(|component| uninstall_plan(component, options.purge, paths))
            .collect::<anyhow::Result<Vec<_>>>()?;
        return print_plans("component.uninstall", &plans, options.json);
    }
    let mut operations = Vec::new();
    let mut failures = Vec::new();
    for component in options.components {
        match uninstall_component(&component, options.cascade, options.purge, paths) {
            Ok(operation) => operations.push(operation),
            Err(error) => failures.push(ComponentFailure {
                component,
                message: format!("{error:#}"),
            }),
        }
    }
    finish_batch(
        "component.uninstall",
        "uninstall",
        operations,
        failures,
        options.json,
    )
}

pub async fn run_update(args: Vec<String>) -> anyhow::Result<()> {
    let paths = ComponentPaths::from_env()?;
    let offline = environment_flag("A3S_OFFLINE");
    let progress = !environment_flag("A3S_NO_PROGRESS") && !environment_flag("A3S_QUIET");
    run_update_with(args, &paths, offline, progress).await
}

pub async fn run_update_with(
    args: Vec<String>,
    paths: &ComponentPaths,
    offline: bool,
    progress: bool,
) -> anyhow::Result<()> {
    let options = UpdateOptions::parse(&args)?;
    let components = if options.all {
        discover(paths)?
            .components
            .into_iter()
            .filter(is_upgrade_all_candidate)
            .map(|component| component.id)
            .collect::<Vec<_>>()
    } else {
        options.components
    };
    if components.is_empty() {
        bail!("no managed components were selected for update");
    }
    if options.dry_run {
        let plans = components
            .iter()
            .map(|component| upgrade_plan(component, paths))
            .collect::<anyhow::Result<Vec<_>>>()?;
        return print_plans("component.upgrade", &plans, options.json);
    }
    if offline {
        bail!("component upgrades require network access and are unavailable in offline mode");
    }
    let mut operations = Vec::new();
    let mut failures = Vec::new();
    for component in components {
        let operation = async {
            let state = find_state(&component, paths)?;
            if state.presence != Presence::Managed {
                bail!(
                    "component '{}' is not managed by A3S; install it explicitly before updating",
                    component
                );
            }
            if state.provenance == Some(InstallProvenance::Delegated)
                && catalog::find(&component).is_none()
            {
                bail!(
                    "local extension '{}' has no recorded upgrade source; install the new package explicitly with 'a3s install {} --from <package> --force --allow-unsigned'",
                    component,
                    component
                );
            }
            let source = if state.provenance == Some(InstallProvenance::Homebrew) {
                InstallSource::Homebrew
            } else {
                InstallSource::Release
            };
            let request = InstallRequest {
                source,
                intent: InstallIntent::Upgrade,
                force: true,
                progress,
                ..InstallRequest::default()
            };
            install_component(&component, &request, paths).await
        }
        .await;
        match operation {
            Ok(operation) => operations.push(operation),
            Err(error) => failures.push(ComponentFailure {
                component,
                message: format!("{error:#}"),
            }),
        }
    }
    finish_batch(
        "component.upgrade",
        "upgrade",
        operations,
        failures,
        options.json,
    )
}

fn is_upgrade_all_candidate(component: &super::state::ComponentState) -> bool {
    component.presence == Presence::Managed && component.kind == ComponentKind::Product
}

pub async fn run_info(args: Vec<String>) -> anyhow::Result<()> {
    let paths = ComponentPaths::from_env()?;
    let offline = environment_flag("A3S_OFFLINE");
    run_info_with(args, &paths, offline).await
}

pub async fn run_info_with(
    args: Vec<String>,
    paths: &ComponentPaths,
    offline: bool,
) -> anyhow::Result<()> {
    let options = InfoOptions::parse(&args)?;
    let id = ComponentId::parse(&options.component)?;
    let state = find_state(&id, paths)?;
    let spec = catalog::find(&id);
    let sources = spec
        .map(|spec| match spec.distribution {
            Distribution::Bundled => vec!["bundled".to_string()],
            Distribution::Delegated { parent } => vec![format!("delegated:{parent}")],
            Distribution::Release(release) => {
                let mut sources = vec![format!(
                    "release:github.com/{}/{}",
                    release.github_owner, release.github_repo
                )];
                if let Some(formula) = release.homebrew_formula {
                    sources.push(format!("homebrew:{formula}"));
                }
                sources
            }
        })
        .unwrap_or_else(|| vec!["delegated:use".to_string()]);
    let latest = if options.versions {
        if offline {
            bail!("version discovery is unavailable in offline mode");
        }
        match spec.and_then(catalog::release) {
            Some(release) => Some(
                fetch_latest_release(release.github_owner, release.github_repo)
                    .await?
                    .tag_name,
            ),
            None => state.version.clone(),
        }
    } else {
        None
    };
    let data = serde_json::json!({
        "schemaVersion": 1,
        "component": state,
        "sources": sources,
        "latestVersion": latest,
    });
    if options.json {
        print_json("component.info", &data, true)?;
    } else {
        println!("component: {}", state.id);
        println!("description: {}", state.description);
        println!("presence: {:?}", state.presence);
        println!("health: {:?}", state.health);
        println!("version: {}", state.version.as_deref().unwrap_or("-"));
        if let Some(path) = &state.path {
            println!("path: {}", path.display());
        }
        if options.sources || options.versions {
            println!("sources:");
            for source in sources {
                println!("  {source}");
            }
        }
        if let Some(latest) = latest {
            println!("latest version: {latest}");
        }
    }
    Ok(())
}

pub fn run_doctor(args: Vec<String>) -> anyhow::Result<bool> {
    let paths = ComponentPaths::from_env()?;
    run_doctor_with(args, &paths)
}

pub fn run_doctor_with(args: Vec<String>, paths: &ComponentPaths) -> anyhow::Result<bool> {
    let options = DoctorOptions::parse(&args)?;
    let report = discover(paths)?;
    let checks = if let Some(component) = options.component {
        let id = ComponentId::parse(&component)?;
        vec![report
            .components
            .into_iter()
            .find(|state| state.id == id)
            .with_context(|| format!("component '{id}' is not registered"))?]
    } else {
        report
            .components
            .into_iter()
            .filter(|state| {
                state.presence != Presence::Missing || state.kind == ComponentKind::BuiltIn
            })
            .collect()
    };
    let healthy = checks.iter().all(|state| state.is_ready());
    if options.json {
        print_json(
            "component.doctor",
            &serde_json::json!({"healthy": healthy, "checks": checks}),
            healthy,
        )?;
    } else {
        for state in &checks {
            println!(
                "{} {:<18} {:?} / {:?}",
                if state.is_ready() { "ok" } else { "failed" },
                state.id,
                state.presence,
                state.health
            );
            if let Some(message) = &state.message {
                println!("  {message}");
            }
        }
    }
    Ok(healthy)
}

pub async fn run_proxy(component: &str, args: Vec<OsString>) -> anyhow::Result<ExitStatus> {
    let paths = ComponentPaths::from_env()?;
    let allow_auto_install =
        !environment_flag("A3S_NO_AUTO_INSTALL") && !environment_flag("A3S_OFFLINE");
    let progress = !environment_flag("A3S_NO_PROGRESS") && !environment_flag("A3S_QUIET");
    let executable =
        resolve_or_install_with(component, &paths, allow_auto_install, progress).await?;
    let status = tokio::process::Command::new(&executable)
        .args(args)
        .status()
        .await
        .with_context(|| format!("failed to run {}", executable.display()))?;
    Ok(status)
}

pub async fn resolve_or_install(component: &str) -> anyhow::Result<PathBuf> {
    let paths = ComponentPaths::from_env()?;
    let allow_auto_install =
        !environment_flag("A3S_NO_AUTO_INSTALL") && !environment_flag("A3S_OFFLINE");
    let progress = !environment_flag("A3S_NO_PROGRESS") && !environment_flag("A3S_QUIET");
    resolve_or_install_with(component, &paths, allow_auto_install, progress).await
}

/// Resolve an already-ready component without installing or mutating it.
///
/// This is used when one component can optionally expose another component as
/// a route. The component catalog and receipt remain the single source of
/// truth; the consumer receives only the resolved executable path.
pub fn find_ready_executable_with(
    component: &str,
    paths: &ComponentPaths,
) -> anyhow::Result<Option<PathBuf>> {
    let id = ComponentId::parse(component)?;
    let state = find_state(&id, paths)?;
    if !state.is_ready() {
        return Ok(None);
    }
    Ok(state.path)
}

pub async fn resolve_or_install_with(
    component: &str,
    paths: &ComponentPaths,
    allow_auto_install: bool,
    progress: bool,
) -> anyhow::Result<PathBuf> {
    let id = ComponentId::parse(component)?;
    let spec = catalog::find(&id)
        .with_context(|| format!("component '{}' is not registered", component))?;
    let mut state = find_state(&id, paths)?;
    if state.presence == Presence::Missing {
        if !spec.auto_install_on_use {
            bail!(
                "component '{}' is not installed; run 'a3s install {}'",
                id,
                id
            );
        }
        if !allow_auto_install {
            bail!(
                "component '{}' is not installed and first-use installation is disabled; run 'a3s install {}'",
                id,
                id
            );
        }
        super::progress(
            progress,
            format!(
                "a3s: component '{}' is not installed; installing it now...",
                id
            ),
        );
        let request = InstallRequest {
            progress,
            ..InstallRequest::default()
        };
        install_component(&id, &request, paths).await?;
        state = find_state(&id, paths)?;
    }
    if state.health != Health::Ready {
        bail!(
            "component '{}' is present but not healthy; run 'a3s install {} --force'",
            id,
            id
        );
    }
    state
        .path
        .context("resolved component has no executable path")
}

async fn populate_updates(report: &mut ComponentReport) {
    for component in &mut report.components {
        let Some(spec) = catalog::find(&component.id) else {
            continue;
        };
        let Some(release) = catalog::release(spec) else {
            continue;
        };
        let Some(current) = component.version.as_deref() else {
            continue;
        };
        let latest = match fetch_latest_release(release.github_owner, release.github_repo).await {
            Ok(latest) => latest,
            Err(error) => {
                component.message = Some(format!("Update check failed: {error}"));
                continue;
            }
        };
        let Ok(current) = parse_version(current) else {
            continue;
        };
        let Ok(latest) = parse_version(&latest.tag_name) else {
            continue;
        };
        component.update = if latest > current {
            UpdateState::Available
        } else {
            UpdateState::Current
        };
    }
}

fn print_human_report(report: &ComponentReport) {
    println!(
        "{:<18} {:<11} {:<17} {:<12} SOURCE",
        "COMPONENT", "TYPE", "STATUS", "VERSION"
    );
    for component in &report.components {
        let status = match (component.presence, component.health) {
            (Presence::Missing, _) => "missing".to_string(),
            (_, Health::Broken) => "broken".to_string(),
            (Presence::System, Health::Ready) => "ready (system)".to_string(),
            (Presence::External, Health::Ready) => "ready (external)".to_string(),
            (_, Health::Ready) => "ready".to_string(),
            _ => "unknown".to_string(),
        };
        let source = component
            .provenance
            .map(|value| format!("{value:?}").to_ascii_lowercase())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<18} {:<11} {:<17} {:<12} {}",
            component.id,
            format!("{:?}", component.kind).to_ascii_lowercase(),
            status,
            component.version.as_deref().unwrap_or("-"),
            source
        );
        if let Some(message) = &component.message {
            println!("  note: {message}");
        }
    }
    if !report.external_tools.is_empty() {
        println!();
        println!("EXTERNAL TOOLS (discovered, never activated automatically)");
        for tool in &report.external_tools {
            println!(
                "  {:<16} {:<20} {}",
                tool.command,
                tool.binary,
                tool.path.display()
            );
        }
    }
}

fn print_available(json: bool) -> anyhow::Result<()> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Available<'a> {
        schema_version: u32,
        components: Vec<AvailableItem<'a>>,
    }
    #[derive(Serialize)]
    struct AvailableItem<'a> {
        id: &'a str,
        kind: ComponentKind,
        description: &'a str,
    }
    let output = Available {
        schema_version: 1,
        components: catalog::all()
            .iter()
            .map(|component| AvailableItem {
                id: component.id,
                kind: component.kind,
                description: component.description,
            })
            .collect(),
    };
    if json {
        print_json("component.install", &output, true)?;
    } else {
        println!("Available A3S components:");
        for component in catalog::all() {
            println!("  {:<18} {}", component.id, component.description);
        }
    }
    Ok(())
}

fn print_operations(
    command: &'static str,
    operations: &[OperationRecord],
    json: bool,
) -> anyhow::Result<()> {
    if json {
        print_json(
            command,
            &serde_json::json!({"operations": operations}),
            true,
        )?;
    } else {
        for operation in operations {
            let marker = if operation.changed { "✓" } else { "=" };
            println!("{marker} {}", operation.message);
        }
    }
    Ok(())
}

fn finish_batch(
    command: &'static str,
    action: &'static str,
    operations: Vec<OperationRecord>,
    failures: Vec<ComponentFailure>,
    json: bool,
) -> anyhow::Result<()> {
    if failures.is_empty() {
        return print_operations(command, &operations, json);
    }
    if !json && !operations.is_empty() {
        print_operations(command, &operations, false)?;
    }
    Err(ComponentBatchFailure {
        action,
        operations,
        failures,
    }
    .into())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationPlan {
    schema_version: u32,
    component: ComponentId,
    action: &'static str,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    migration: Option<bool>,
    target: String,
    ownership: String,
    mutates: bool,
    message: String,
}

fn install_plan(
    id: &ComponentId,
    request: &InstallRequest,
    options: &InstallOptions,
    paths: &ComponentPaths,
) -> anyhow::Result<OperationPlan> {
    let state = find_state(id, paths).or_else(|_| {
        let use_id = ComponentId::parse("use")?;
        if id.is_child_of(&use_id) && id.as_str().split('/').count() >= 3 {
            find_state(&use_id, paths)
        } else {
            bail!("component '{}' is not registered", id)
        }
    })?;
    let source = match request.source {
        InstallSource::Auto => state
            .provenance
            .map(|value| format!("existing:{value:?}").to_ascii_lowercase())
            .unwrap_or_else(|| "auto".to_string()),
        InstallSource::Homebrew => "homebrew".to_string(),
        InstallSource::Release => "release".to_string(),
    };
    Ok(OperationPlan {
        schema_version: 1,
        component: id.clone(),
        action: "install",
        source,
        channel: Some(options.channel.as_str()),
        scope: Some(options.scope.as_str()),
        migration: Some(options.migrate),
        target: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        ownership: "resolved-at-apply".to_string(),
        mutates: !state.is_ready() || request.force,
        message: if state.is_ready() && !request.force {
            "Already healthy; apply would be a no-op.".to_string()
        } else {
            "Apply would resolve, verify, stage, activate, and health-check the component."
                .to_string()
        },
    })
}

fn uninstall_plan(
    id: &ComponentId,
    purge: bool,
    paths: &ComponentPaths,
) -> anyhow::Result<OperationPlan> {
    let state = find_state(id, paths)?;
    if matches!(
        state.presence,
        Presence::External | Presence::System | Presence::Bundled
    ) {
        bail!(
            "component '{}' is not owned by A3S and cannot be uninstalled",
            id
        );
    }
    Ok(OperationPlan {
        schema_version: 1,
        component: id.clone(),
        action: "uninstall",
        source: state
            .provenance
            .map(|value| format!("{value:?}").to_ascii_lowercase())
            .unwrap_or_else(|| "unknown".to_string()),
        channel: None,
        scope: None,
        migration: None,
        target: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        ownership: "receipt-or-parent-owned".to_string(),
        mutates: state.presence != Presence::Missing,
        message: if purge {
            "Apply would remove owned files and component-owned recreatable cache.".to_string()
        } else {
            "Apply would remove only receipt-owned files or delegate to the owning parent."
                .to_string()
        },
    })
}

fn upgrade_plan(id: &ComponentId, paths: &ComponentPaths) -> anyhow::Result<OperationPlan> {
    let state = find_state(id, paths)?;
    if state.presence != Presence::Managed {
        bail!("component '{}' is not managed by A3S", id);
    }
    Ok(OperationPlan {
        schema_version: 1,
        component: id.clone(),
        action: "upgrade",
        source: state
            .provenance
            .map(|value| format!("{value:?}").to_ascii_lowercase())
            .unwrap_or_else(|| "existing-provenance".to_string()),
        channel: None,
        scope: None,
        migration: None,
        target: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        ownership: "existing-provenance".to_string(),
        mutates: true,
        message: "Apply would query and install through the existing provenance.".to_string(),
    })
}

fn print_plans(command: &'static str, plans: &[OperationPlan], json: bool) -> anyhow::Result<()> {
    if json {
        print_json(
            command,
            &serde_json::json!({"dryRun": true, "plans": plans}),
            true,
        )?;
    } else {
        for plan in plans {
            println!("component: {}", plan.component);
            println!("action: {}", plan.action);
            println!("source: {}", plan.source);
            if let Some(channel) = plan.channel {
                println!("channel: {channel}");
            }
            if let Some(scope) = plan.scope {
                println!("scope: {scope}");
            }
            if let Some(migration) = plan.migration {
                println!("migration: {migration}");
            }
            println!("target: {}", plan.target);
            println!("ownership: {}", plan.ownership);
            println!("mutates: {}", plan.mutates);
            println!("{}", plan.message);
        }
    }
    Ok(())
}

fn print_json(command: &'static str, data: &impl Serialize, ok: bool) -> anyhow::Result<()> {
    let value = serde_json::json!({
        "schemaVersion": 1,
        "command": command,
        "ok": ok,
        "data": data,
        "warnings": [],
    });
    let bytes = serde_json::to_vec_pretty(&value)?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    stdout.write_all(&bytes)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn enforce_install_network_policy(
    components: &[ComponentId],
    request: &InstallRequest,
    paths: &ComponentPaths,
    offline: bool,
) -> anyhow::Result<()> {
    if !offline || request.package.is_some() {
        return Ok(());
    }
    for component in components {
        if !find_state(component, paths)?.is_ready() {
            bail!(
                "component '{}' is not available from an installed or explicit local source in offline mode",
                component
            );
        }
    }
    Ok(())
}

fn environment_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| {
        if value.is_empty() {
            return true;
        }
        !matches!(
            value.to_string_lossy().trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ReleaseChannel {
    #[default]
    Stable,
    Beta,
    Nightly,
}

impl ReleaseChannel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Nightly => "nightly",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum InstallScope {
    #[default]
    User,
    System,
}

impl InstallScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::System => "system",
        }
    }
}

fn validate_supported_install_policy(options: &InstallOptions) -> anyhow::Result<()> {
    if let Some(version) = options.version.as_deref() {
        parse_version(version).with_context(|| format!("invalid component version '{version}'"))?;
        if options.source == InstallSource::Homebrew {
            bail!("--version requires --source release because Homebrew does not support exact component version selection");
        }
    }
    if options.version.is_some() && options.channel != ReleaseChannel::Stable {
        bail!("--version cannot be combined with a beta or nightly release channel");
    }
    if options.channel != ReleaseChannel::Stable {
        bail!(
            "the '{}' channel is not declared by the selected component sources",
            options.channel.as_str()
        );
    }
    if options.scope == InstallScope::System {
        bail!(
            "system-scope installation is not declared by the selected component sources; use --scope user"
        );
    }
    Ok(())
}

fn enforce_provenance_policy(
    options: &InstallOptions,
    paths: &ComponentPaths,
) -> anyhow::Result<()> {
    let requested = match options.source {
        InstallSource::Homebrew => Some(InstallProvenance::Homebrew),
        InstallSource::Release => Some(InstallProvenance::GithubRelease),
        InstallSource::Auto => None,
    };
    let Some(requested) = requested else {
        return Ok(());
    };
    for component in &options.components {
        let state = find_state(component, paths)?;
        let Some(current) = state.provenance else {
            continue;
        };
        if state.presence == Presence::Managed && current != requested {
            if !options.migrate {
                bail!(
                    "component '{}' is managed through {:?}; changing to {:?} requires --migrate",
                    component,
                    current,
                    requested
                );
            }
            bail!(
                "component '{}' cannot yet migrate atomically from {:?} to {:?}; keep its current source",
                component,
                current,
                requested
            );
        }
    }
    Ok(())
}

fn parse_kind(value: &str) -> anyhow::Result<ComponentKind> {
    match value {
        "built-in" => Ok(ComponentKind::BuiltIn),
        "product" => Ok(ComponentKind::Product),
        "capability" => Ok(ComponentKind::Capability),
        "extension" => Ok(ComponentKind::Extension),
        _ => bail!("unknown component kind '{value}'"),
    }
}

fn required_value<'a>(args: &'a [String], index: usize, option: &str) -> anyhow::Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .with_context(|| format!("{option} requires a value"))
}

#[cfg(test)]
mod tests {
    use super::super::state::{ComponentState, Trust};
    use super::*;

    #[test]
    fn parses_install_options_in_any_order() {
        let args = [
            "use/acme/slack",
            "--from",
            "./package",
            "--allow-unsigned",
            "--json",
            "--force",
        ]
        .map(str::to_string);
        let options = InstallOptions::parse(&args).unwrap();
        assert_eq!(options.components[0].as_str(), "use/acme/slack");
        assert_eq!(options.package, Some(PathBuf::from("./package")));
        assert!(options.allow_unsigned);
        assert!(options.force);
        assert!(options.json);
    }

    #[test]
    fn rejects_conflicting_list_and_update_options() {
        assert!(
            ListOptions::parse(&["--installed".to_string(), "--available".to_string()]).is_err()
        );
        assert!(UpdateOptions::parse(&["--all".to_string(), "use".to_string()]).is_err());
    }

    #[test]
    fn upgrade_all_selects_only_managed_products() {
        let state = |id: &str, kind, presence| ComponentState {
            id: ComponentId::parse(id).unwrap(),
            kind,
            description: String::new(),
            presence,
            health: Health::Ready,
            update: UpdateState::Unknown,
            trust: Trust::FirstParty,
            provenance: Some(InstallProvenance::GithubRelease),
            version: Some("1.0.0".to_string()),
            path: Some(PathBuf::from("/tmp/component")),
            message: None,
        };

        assert!(is_upgrade_all_candidate(&state(
            "use",
            ComponentKind::Product,
            Presence::Managed,
        )));
        assert!(!is_upgrade_all_candidate(&state(
            "use/browser",
            ComponentKind::Capability,
            Presence::Managed,
        )));
        assert!(!is_upgrade_all_candidate(&state(
            "use/acme/slack",
            ComponentKind::Extension,
            Presence::Managed,
        )));
        assert!(!is_upgrade_all_candidate(&state(
            "search",
            ComponentKind::Product,
            Presence::External,
        )));
    }
}
