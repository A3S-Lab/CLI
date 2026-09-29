use std::collections::BTreeMap;

use a3s_updater::{parse_version, ComponentReceipt, InstallProvenance};
use anyhow::{bail, Context};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::catalog::{self, Distribution};
use super::discovery::discover;
use super::id::ComponentId;
use super::lifecycle::{resolve_install_source, InstallRequest, InstallSource};
use super::paths::ComponentPaths;
use super::release_install::{resolve_release, ResolvedRelease};
use super::state::{ComponentState, Health, Presence};

const PLAN_SCHEMA_VERSION: u32 = 1;
const PLAN_DIGEST_DOMAIN: &[u8] = b"a3s-component-plan-v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannedPath {
    display: String,
    identity: String,
}

impl PlannedPath {
    fn new(path: &std::path::Path) -> Self {
        Self {
            display: path.to_string_lossy().into_owned(),
            identity: path_identity(path),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PreparedOperationPlan {
    pub(super) plan: OperationPlan,
    pub(super) resolved_releases: BTreeMap<String, ResolvedRelease>,
    pub(super) resolved_sources: BTreeMap<String, InstallSource>,
    pub(super) apply_force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlannedCurrentState {
    presence: Presence,
    health: Health,
    #[serde(skip_serializing_if = "Option::is_none")]
    provenance: Option<InstallProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<PlannedPath>,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt: Option<PlannedReceipt>,
}

impl From<&ComponentState> for PlannedCurrentState {
    fn from(state: &ComponentState) -> Self {
        Self {
            presence: state.presence,
            health: state.health,
            provenance: state.provenance,
            version: state.version.clone(),
            path: state.path.as_deref().map(PlannedPath::new),
            receipt: None,
        }
    }
}

impl PlannedCurrentState {
    fn with_receipt(state: &ComponentState, paths: &ComponentPaths) -> anyhow::Result<Self> {
        let mut planned = Self::from(state);
        planned.receipt = paths
            .receipt_store()
            .read(state.id.as_str())?
            .as_ref()
            .map(PlannedReceipt::from);
        Ok(planned)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannedReceipt {
    schema_version: u32,
    component_id: String,
    version: String,
    provenance: InstallProvenance,
    install_root: PlannedPath,
    #[serde(skip_serializing_if = "Option::is_none")]
    executable_path: Option<PlannedPath>,
    owned_paths: Vec<PlannedPath>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    artifact_checksums: BTreeMap<String, String>,
}

impl From<&ComponentReceipt> for PlannedReceipt {
    fn from(receipt: &ComponentReceipt) -> Self {
        let mut owned_paths = receipt
            .owned_paths
            .iter()
            .map(|path| PlannedPath::new(path))
            .collect::<Vec<_>>();
        owned_paths.sort_by(|left, right| left.identity.cmp(&right.identity));
        Self {
            schema_version: receipt.schema_version,
            component_id: receipt.component_id.clone(),
            version: receipt.version.clone(),
            provenance: receipt.provenance,
            install_root: PlannedPath::new(&receipt.install_root),
            executable_path: receipt.executable_path.as_deref().map(PlannedPath::new),
            owned_paths,
            source: receipt.source.clone(),
            artifact_checksums: receipt.artifact_checksums.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationPlan {
    schema_version: u32,
    component: ComponentId,
    action: &'static str,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    migration: Option<bool>,
    target: String,
    ownership: String,
    mutates: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_version: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    resolved_sources: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    resolved_releases: BTreeMap<String, ResolvedRelease>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    prerequisites: BTreeMap<String, PlannedCurrentState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    force: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cascade: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    purge: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current: Option<PlannedCurrentState>,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OperationPlanSet {
    pub(super) plan_schema_version: u32,
    pub(super) plan_command: &'static str,
    pub(super) plan_digest: String,
    pub(super) plans: Vec<OperationPlan>,
}

impl OperationPlanSet {
    pub(super) fn new(command: &'static str, plans: Vec<OperationPlan>) -> anyhow::Result<Self> {
        let plan_digest = plan_digest(command, &plans)?;
        Ok(Self {
            plan_schema_version: PLAN_SCHEMA_VERSION,
            plan_command: command,
            plan_digest,
            plans,
        })
    }

    pub(super) fn print_human(&self) {
        println!("plan digest: {}", self.plan_digest);
        for plan in &self.plans {
            println!();
            println!("component: {}", plan.component);
            println!("action: {}", plan.action);
            println!("source: {}", plan.source);
            if let Some(source) = &plan.requested_source {
                println!("requested source: {source}");
            }
            if let Some(channel) = &plan.channel {
                println!("channel: {channel}");
            }
            if let Some(scope) = &plan.scope {
                println!("scope: {scope}");
            }
            if let Some(migration) = plan.migration {
                println!("migration: {migration}");
            }
            if let Some(version) = &plan.requested_version {
                println!("requested version: {version}");
            }
            for (component, release) in &plan.resolved_releases {
                println!(
                    "resolved release: {component} {} {} {}",
                    release.version, release.archive_name, release.sha256
                );
            }
            for (component, state) in &plan.prerequisites {
                println!(
                    "prerequisite: {component} {:?} {:?}",
                    state.presence, state.health
                );
            }
            println!("target: {}", plan.target);
            println!("ownership: {}", plan.ownership);
            println!("mutates: {}", plan.mutates);
            println!("{}", plan.message);
        }
    }

    pub(super) fn digest(&self) -> &str {
        &self.plan_digest
    }

    pub(super) fn verify_expected(&self, expected: Option<&str>) -> anyhow::Result<()> {
        let Some(expected) = expected else {
            return Ok(());
        };
        if expected == self.plan_digest {
            return Ok(());
        }
        Err(ComponentPlanMismatch {
            expected: expected.to_string(),
            actual: self.plan_digest.clone(),
        }
        .into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentPlanMismatch {
    pub expected: String,
    pub actual: String,
}

impl std::fmt::Display for ComponentPlanMismatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "component plan changed after review: expected {}, resolved {}",
            self.expected, self.actual
        )
    }
}

impl std::error::Error for ComponentPlanMismatch {}

pub(super) fn validate_install_plan(
    id: &ComponentId,
    request: &InstallRequest,
) -> anyhow::Result<()> {
    let Some(spec) = catalog::find(id) else {
        bail!("component '{}' is not registered", id);
    };
    if request.version.is_some() && !matches!(spec.distribution, Distribution::Release(_)) {
        bail!(
            "component '{}' does not own a versioned release; --version is not supported",
            id
        );
    }
    if request.source != InstallSource::Auto
        && !matches!(spec.distribution, Distribution::Release(_))
    {
        bail!(
            "component '{}' does not own an install source; --source is not supported",
            id
        );
    }
    Ok(())
}

pub(super) async fn install_plan(
    id: &ComponentId,
    request: &InstallRequest,
    channel: &str,
    scope: &str,
    migrate: bool,
    paths: &ComponentPaths,
) -> anyhow::Result<PreparedOperationPlan> {
    validate_install_plan(id, request)?;
    let state = discover(paths)?
        .components
        .into_iter()
        .find(|component| &component.id == id);
    let spec = catalog::find(id);

    let requested_version_is_ready = request.version.as_deref().is_none_or(|requested| {
        state
            .as_ref()
            .and_then(|state| state.version.as_deref())
            .is_some_and(|installed| parse_version(installed).ok() == parse_version(requested).ok())
    });
    let already_ready = state.as_ref().is_some_and(ComponentState::is_ready)
        && requested_version_is_ready
        && !request.force;
    let mut resolved_releases = BTreeMap::new();
    let mut resolved_sources = BTreeMap::new();
    let mut prerequisites = BTreeMap::new();
    let (source, ownership) = match spec.map(|spec| spec.distribution) {
        Some(Distribution::Bundled) => ("bundled".to_string(), "bundled".to_string()),
        Some(Distribution::Delegated { parent }) => {
            prepare_parent_release(
                &ComponentId::parse(parent)?,
                paths,
                &mut resolved_sources,
                &mut resolved_releases,
                &mut prerequisites,
            )
            .await?;
            (format!("delegated:{parent}"), format!("parent:{parent}"))
        }
        Some(Distribution::Release(_)) if already_ready => (
            state
                .as_ref()
                .and_then(|state| state.provenance)
                .map(|value| format!("existing:{value:?}").to_ascii_lowercase())
                .unwrap_or_else(|| "existing".to_string()),
            state
                .as_ref()
                .and_then(|state| state.provenance)
                .map(|value| match value {
                    InstallProvenance::Homebrew => "package-manager:homebrew".to_string(),
                    _ => "a3s".to_string(),
                })
                .unwrap_or_else(|| "a3s".to_string()),
        ),
        Some(Distribution::Release(release)) => {
            let selected = resolve_install_source(id, release, request)?;
            resolved_sources.insert(id.to_string(), selected);
            match selected {
                InstallSource::Release => {
                    let resolved = resolve_release(id, release, request.version.as_deref()).await?;
                    resolved_releases.insert(id.to_string(), resolved);
                    ("github-release".to_string(), "a3s".to_string())
                }
                InstallSource::Homebrew => {
                    let formula = release
                        .homebrew_formula
                        .with_context(|| format!("component '{}' has no Homebrew formula", id))?;
                    (
                        format!("homebrew:{formula}"),
                        "package-manager:homebrew".to_string(),
                    )
                }
                InstallSource::Auto => bail!("automatic install source was not resolved"),
            }
        }
        None => bail!("component '{}' is not registered", id),
    };
    let current = state
        .as_ref()
        .map(|state| PlannedCurrentState::with_receipt(state, paths))
        .transpose()?;

    let plan = OperationPlan {
        schema_version: PLAN_SCHEMA_VERSION,
        component: id.clone(),
        action: "install",
        source,
        requested_source: Some(install_source_name(request.source).to_string()),
        channel: Some(channel.to_string()),
        scope: Some(scope.to_string()),
        migration: Some(migrate),
        target: host_target(),
        ownership,
        mutates: !already_ready,
        requested_version: request.version.clone(),
        resolved_sources: planned_sources(&resolved_sources)?,
        resolved_releases: resolved_releases.clone(),
        prerequisites,
        force: Some(request.force),
        cascade: None,
        purge: None,
        current,
        message: if already_ready {
            "Already healthy; apply would be a no-op.".to_string()
        } else {
            "Apply would resolve, verify, stage, activate, and health-check the component."
                .to_string()
        },
    };
    Ok(PreparedOperationPlan {
        plan,
        resolved_releases,
        resolved_sources,
        apply_force: request.force,
    })
}

pub(super) fn uninstall_plan(
    id: &ComponentId,
    cascade: bool,
    purge: bool,
    paths: &ComponentPaths,
) -> anyhow::Result<OperationPlan> {
    if catalog::find(id).is_none() {
        bail!("component '{}' is not registered", id);
    }
    let state = super::discovery::find_state(id, paths)?;
    if matches!(
        state.presence,
        Presence::External | Presence::System | Presence::Bundled
    ) {
        bail!(
            "component '{}' is not owned by A3S and cannot be uninstalled",
            id
        );
    }
    let mut prerequisites = BTreeMap::new();
    let parent = match catalog::find(id).map(|spec| spec.distribution) {
        Some(Distribution::Delegated { parent }) => Some(ComponentId::parse(parent)?),
        _ => None,
    };
    if let Some(parent) = parent {
        let parent_state = super::discovery::find_state(&parent, paths)?;
        if !parent_state.is_ready() {
            bail!("parent component '{}' is not ready", parent);
        }
        prerequisites.insert(
            parent.to_string(),
            PlannedCurrentState::with_receipt(&parent_state, paths)?,
        );
    }
    Ok(OperationPlan {
        schema_version: PLAN_SCHEMA_VERSION,
        component: id.clone(),
        action: "uninstall",
        source: state
            .provenance
            .map(|value| format!("{value:?}").to_ascii_lowercase())
            .unwrap_or_else(|| "unknown".to_string()),
        requested_source: None,
        channel: None,
        scope: None,
        migration: None,
        target: host_target(),
        ownership: "receipt-or-parent-owned".to_string(),
        mutates: state.presence != Presence::Missing,
        requested_version: None,
        resolved_sources: BTreeMap::new(),
        resolved_releases: BTreeMap::new(),
        prerequisites,
        force: None,
        cascade: Some(cascade),
        purge: Some(purge),
        current: Some(PlannedCurrentState::with_receipt(&state, paths)?),
        message: if purge {
            "Apply would remove owned files and component-owned recreatable cache.".to_string()
        } else {
            "Apply would remove only receipt-owned files or delegate to the owning parent."
                .to_string()
        },
    })
}

pub(super) async fn upgrade_plan(
    id: &ComponentId,
    paths: &ComponentPaths,
) -> anyhow::Result<PreparedOperationPlan> {
    let state = super::discovery::find_state(id, paths)?;
    if state.presence != Presence::Managed {
        bail!("component '{}' is not managed by A3S", id);
    }
    let Some(spec) = catalog::find(id) else {
        bail!("component '{}' is not registered", id);
    };
    let release = catalog::release(spec)
        .with_context(|| format!("component '{}' has no managed release", id))?;
    let selected = match state.provenance {
        Some(InstallProvenance::Homebrew) => InstallSource::Homebrew,
        Some(InstallProvenance::GithubRelease) => InstallSource::Release,
        Some(provenance) => bail!(
            "component '{}' cannot be upgraded from {:?} provenance",
            id,
            provenance
        ),
        None => bail!("component '{}' has no recorded upgrade provenance", id),
    };
    let resolved_sources = BTreeMap::from([(id.to_string(), selected)]);
    let mut resolved_releases = BTreeMap::new();
    let source = match selected {
        InstallSource::Release => {
            resolved_releases.insert(id.to_string(), resolve_release(id, release, None).await?);
            "github-release".to_string()
        }
        InstallSource::Homebrew => {
            let formula = release
                .homebrew_formula
                .with_context(|| format!("component '{}' has no Homebrew formula", id))?;
            format!("homebrew:{formula}")
        }
        InstallSource::Auto => bail!("automatic install source was not resolved"),
    };
    let plan = OperationPlan {
        schema_version: PLAN_SCHEMA_VERSION,
        component: id.clone(),
        action: "upgrade",
        source,
        requested_source: None,
        channel: None,
        scope: None,
        migration: None,
        target: host_target(),
        ownership: "existing-provenance".to_string(),
        mutates: true,
        requested_version: None,
        resolved_sources: planned_sources(&resolved_sources)?,
        resolved_releases: resolved_releases.clone(),
        prerequisites: BTreeMap::new(),
        force: Some(true),
        cascade: None,
        purge: None,
        current: Some(PlannedCurrentState::with_receipt(&state, paths)?),
        message: "Apply would install the exact resolved artifact through the existing provenance."
            .to_string(),
    };
    Ok(PreparedOperationPlan {
        plan,
        resolved_releases,
        resolved_sources,
        apply_force: true,
    })
}

async fn prepare_parent_release(
    parent: &ComponentId,
    paths: &ComponentPaths,
    resolved_sources: &mut BTreeMap<String, InstallSource>,
    resolved_releases: &mut BTreeMap<String, ResolvedRelease>,
    prerequisites: &mut BTreeMap<String, PlannedCurrentState>,
) -> anyhow::Result<()> {
    let state = super::discovery::find_state(parent, paths)?;
    prerequisites.insert(
        parent.to_string(),
        PlannedCurrentState::with_receipt(&state, paths)?,
    );
    if state.is_ready() {
        return Ok(());
    }
    let spec = catalog::find(parent)
        .with_context(|| format!("parent component '{}' is not registered", parent))?;
    let release = catalog::release(spec)
        .with_context(|| format!("parent component '{}' is not installable", parent))?;
    let request = InstallRequest {
        progress: false,
        ..InstallRequest::default()
    };
    let source = resolve_install_source(parent, release, &request)?;
    resolved_sources.insert(parent.to_string(), source);
    if source == InstallSource::Release {
        resolved_releases.insert(
            parent.to_string(),
            resolve_release(parent, release, None).await?,
        );
    }
    Ok(())
}

fn planned_sources(
    sources: &BTreeMap<String, InstallSource>,
) -> anyhow::Result<BTreeMap<String, String>> {
    sources
        .iter()
        .map(|(component, source)| {
            let id = ComponentId::parse(component)?;
            let release = catalog::find(&id).and_then(catalog::release);
            let source = match (source, release) {
                (InstallSource::Auto, _) => "auto".to_string(),
                (InstallSource::Homebrew, Some(release)) => format!(
                    "homebrew:{}",
                    release.homebrew_formula.with_context(|| {
                        format!("component '{}' has no Homebrew formula", component)
                    })?
                ),
                (InstallSource::Release, Some(release)) => format!(
                    "github-release:{}/{}",
                    release.github_owner, release.github_repo
                ),
                (_, None) => bail!("component '{}' has no resolved install source", component),
            };
            Ok((component.clone(), source))
        })
        .collect()
}

fn install_source_name(source: InstallSource) -> &'static str {
    match source {
        InstallSource::Auto => "auto",
        InstallSource::Homebrew => "homebrew",
        InstallSource::Release => "release",
    }
}

fn host_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(unix)]
fn path_identity(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    format!("unix-hex:{}", encode_hex(path.as_os_str().as_bytes()))
}

#[cfg(windows)]
fn path_identity(path: &std::path::Path) -> String {
    use std::os::windows::ffi::OsStrExt;

    let words = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let mut bytes = Vec::with_capacity(words.len() * 2);
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    format!("windows-utf16le-hex:{}", encode_hex(&bytes))
}

#[cfg(not(any(unix, windows)))]
fn path_identity(path: &std::path::Path) -> String {
    format!("display:{}", path.to_string_lossy())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn plan_digest(command: &'static str, plans: &[OperationPlan]) -> anyhow::Result<String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CanonicalPlanSet<'a> {
        schema_version: u32,
        command: &'a str,
        plans: Vec<CanonicalOperationPlan<'a>>,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct CanonicalOperationPlan<'a> {
        schema_version: u32,
        component: &'a ComponentId,
        action: &'a str,
        source: &'a str,
        requested_source: &'a Option<String>,
        channel: &'a Option<String>,
        scope: &'a Option<String>,
        migration: Option<bool>,
        target: &'a str,
        ownership: &'a str,
        mutates: bool,
        requested_version: &'a Option<String>,
        resolved_sources: &'a BTreeMap<String, String>,
        resolved_releases: &'a BTreeMap<String, ResolvedRelease>,
        prerequisites: &'a BTreeMap<String, PlannedCurrentState>,
        force: Option<bool>,
        cascade: Option<bool>,
        purge: Option<bool>,
        current: &'a Option<PlannedCurrentState>,
    }

    let canonical = CanonicalPlanSet {
        schema_version: PLAN_SCHEMA_VERSION,
        command,
        plans: plans
            .iter()
            .map(|plan| CanonicalOperationPlan {
                schema_version: plan.schema_version,
                component: &plan.component,
                action: plan.action,
                source: &plan.source,
                requested_source: &plan.requested_source,
                channel: &plan.channel,
                scope: &plan.scope,
                migration: plan.migration,
                target: &plan.target,
                ownership: &plan.ownership,
                mutates: plan.mutates,
                requested_version: &plan.requested_version,
                resolved_sources: &plan.resolved_sources,
                resolved_releases: &plan.resolved_releases,
                prerequisites: &plan.prerequisites,
                force: plan.force,
                cascade: plan.cascade,
                purge: plan.purge,
                current: &plan.current,
            })
            .collect(),
    };
    let bytes = serde_json::to_vec(&canonical).context("failed to encode component plan")?;
    let mut digest = Sha256::new();
    digest.update(PLAN_DIGEST_DOMAIN);
    digest.update(bytes);
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::super::catalog::ComponentKind;
    use super::super::state::Trust;
    use super::super::state::UpdateState;
    use super::*;

    fn fixture(message: &str) -> OperationPlan {
        OperationPlan {
            schema_version: PLAN_SCHEMA_VERSION,
            component: ComponentId::parse("box").unwrap(),
            action: "install",
            source: "release".to_string(),
            requested_source: Some("release".to_string()),
            channel: Some("stable".to_string()),
            scope: Some("user".to_string()),
            migration: Some(false),
            target: "linux-x86_64".to_string(),
            ownership: "a3s".to_string(),
            mutates: true,
            requested_version: Some("1.2.3".to_string()),
            resolved_sources: BTreeMap::from([("box".to_string(), "github-release".to_string())]),
            resolved_releases: BTreeMap::new(),
            prerequisites: BTreeMap::new(),
            force: Some(false),
            cascade: None,
            purge: None,
            current: None,
            message: message.to_string(),
        }
    }

    #[test]
    fn digest_is_stable_and_excludes_presentation_text() {
        let first = OperationPlanSet::new("component.install", vec![fixture("first")]).unwrap();
        let second = OperationPlanSet::new("component.install", vec![fixture("second")]).unwrap();
        assert_eq!(first.plan_digest, second.plan_digest);
        assert_eq!(first.plan_digest.len(), 64);
        assert!(first
            .plan_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    }

    #[test]
    fn digest_changes_with_semantics_or_operation_order() {
        let first = fixture("same");
        let mut forced = first.clone();
        forced.force = Some(true);
        let mut different_version = first.clone();
        different_version.requested_version = Some("2.0.0".to_string());
        let mut different_source = first.clone();
        different_source.source = "homebrew:a3s-lab/tap/a3s-box".to_string();
        different_source.requested_source = Some("homebrew".to_string());
        let mut purged = first.clone();
        purged.purge = Some(true);
        assert_ne!(
            plan_digest("component.install", std::slice::from_ref(&first)).unwrap(),
            plan_digest("component.install", &[forced.clone()]).unwrap()
        );
        for changed in [different_version, different_source, purged] {
            assert_ne!(
                plan_digest("component.install", std::slice::from_ref(&first)).unwrap(),
                plan_digest("component.install", &[changed]).unwrap()
            );
        }
        assert_ne!(
            plan_digest("component.install", &[first.clone(), forced.clone()]).unwrap(),
            plan_digest("component.install", &[forced, first]).unwrap()
        );
    }

    #[test]
    fn current_state_is_part_of_the_digest() {
        let mut first = fixture("same");
        first.current = Some(PlannedCurrentState::from(&ComponentState {
            id: ComponentId::parse("box").unwrap(),
            kind: ComponentKind::Product,
            description: String::new(),
            presence: Presence::Managed,
            health: Health::Ready,
            update: UpdateState::Unknown,
            trust: Trust::FirstParty,
            provenance: Some(InstallProvenance::GithubRelease),
            version: Some("1.0.0".to_string()),
            path: Some(PathBuf::from("/components/box")),
            message: None,
        }));
        let mut second = first.clone();
        second.current.as_mut().unwrap().version = Some("2.0.0".to_string());
        assert_ne!(
            plan_digest("component.install", &[first]).unwrap(),
            plan_digest("component.install", &[second]).unwrap()
        );
    }

    #[test]
    fn receipt_ownership_and_checksums_are_part_of_the_digest() {
        let mut first = fixture("same");
        first.current = Some(PlannedCurrentState {
            presence: Presence::Managed,
            health: Health::Ready,
            provenance: Some(InstallProvenance::GithubRelease),
            version: Some("1.0.0".to_string()),
            path: Some(PlannedPath::new(Path::new("/components/box/a3s-box"))),
            receipt: Some(PlannedReceipt {
                schema_version: 1,
                component_id: "box".to_string(),
                version: "1.0.0".to_string(),
                provenance: InstallProvenance::GithubRelease,
                install_root: PlannedPath::new(Path::new("/components/box")),
                executable_path: Some(PlannedPath::new(Path::new("/components/box/a3s-box"))),
                owned_paths: vec![PlannedPath::new(Path::new("/components/box"))],
                source: Some("https://example.invalid/releases/v1.0.0".to_string()),
                artifact_checksums: BTreeMap::from([("box.tar.gz".to_string(), "a".repeat(64))]),
            }),
        });
        let mut second = first.clone();
        second
            .current
            .as_mut()
            .unwrap()
            .receipt
            .as_mut()
            .unwrap()
            .artifact_checksums
            .insert("box.tar.gz".to_string(), "b".repeat(64));
        assert_ne!(
            plan_digest("component.uninstall", &[first]).unwrap(),
            plan_digest("component.uninstall", &[second]).unwrap()
        );
    }

    #[test]
    fn expected_digest_rejects_a_changed_plan() {
        let plan = OperationPlanSet::new("component.install", vec![fixture("same")]).unwrap();
        plan.verify_expected(Some(plan.digest())).unwrap();
        let error = plan.verify_expected(Some(&"0".repeat(64))).unwrap_err();
        let mismatch = error.downcast_ref::<ComponentPlanMismatch>().unwrap();
        assert_eq!(mismatch.expected, "0".repeat(64));
        assert_eq!(mismatch.actual, plan.digest());
    }

    #[test]
    fn resolved_release_is_part_of_the_digest() {
        let mut first = fixture("same");
        first.resolved_releases.insert(
            "box".to_string(),
            ResolvedRelease {
                version: "1.2.3".to_string(),
                tag: "v1.2.3".to_string(),
                target: "linux-x86_64".to_string(),
                archive_name: "a3s-box-v1.2.3-linux-x86_64.tar.gz".to_string(),
                asset_url: "https://example.invalid/box.tar.gz".to_string(),
                sha256: "a".repeat(64),
            },
        );
        let mut second = first.clone();
        second.resolved_releases.get_mut("box").unwrap().sha256 = "b".repeat(64);
        assert_ne!(
            plan_digest("component.install", &[first]).unwrap(),
            plan_digest("component.install", &[second]).unwrap()
        );
    }
}
