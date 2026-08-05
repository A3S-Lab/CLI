use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use a3s_use_core::{PluginSurfaceRef, MAX_PLUGIN_PLAN_ITEMS};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

use super::{PluginInstallationIndex, PluginManager, MAX_PLUGIN_COMMAND_OUTPUT};

const CAPABILITY_SNAPSHOT_TIMEOUT_SECONDS: u64 = 10;
const MAX_INSTALLED_PLUGINS: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginCapabilityEvidenceStatus {
    Verified,
    Unavailable,
}

/// One bounded observation of the immutable A3S Use capability registry.
///
/// Evidence can be unavailable when A3S Use is absent or unhealthy. That state
/// remains explicit in legacy results. Complete lifecycle plans instead keep
/// their parent cutover pending until a verified post-mutation snapshot exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginCapabilityEvidence {
    pub status: PluginCapabilityEvidenceStatus,
    pub observed_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl PluginCapabilityEvidence {
    pub(super) fn same_registry_state(&self, other: &Self) -> bool {
        self.status == other.status
            && self.generation == other.generation
            && self.revision == other.revision
    }

    fn verified(observed_at_ms: u64, generation: u64, revision: String) -> Self {
        Self {
            status: PluginCapabilityEvidenceStatus::Verified,
            observed_at_ms,
            generation: Some(generation),
            revision: Some(revision),
            error: None,
        }
    }

    fn unavailable(observed_at_ms: u64, error: impl Into<String>) -> Self {
        let error = concise_error(&error.into());
        Self {
            status: PluginCapabilityEvidenceStatus::Unavailable,
            observed_at_ms,
            generation: None,
            revision: None,
            error: Some(if error.is_empty() {
                "capability observation unavailable".to_string()
            } else {
                error
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginPackageReadiness {
    Ready,
    Missing,
    Broken,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginPlannerEvidence {
    pub schema_version: u32,
    pub package_id: String,
    pub package_sha256: String,
    pub manifest_sha256: String,
    pub receipt_digest: String,
    pub catalog_record_digest: String,
    pub desired_enabled: bool,
    pub selected_surfaces: Vec<PluginSurfaceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstalledPackage {
    pub component_id: String,
    pub package_id: String,
    pub route: String,
    pub version: String,
    /// Desired enabled state. This can be true while reconciliation continues.
    pub enabled: bool,
    /// Whether the exact binding is currently callable.
    pub callable: bool,
    pub readiness: PluginPackageReadiness,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconciliation: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planner_evidence: Option<PluginPlannerEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallationSnapshot {
    pub schema_version: u32,
    pub available: bool,
    pub observed_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub items: Vec<PluginInstalledPackage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl PluginInstallationSnapshot {
    pub fn index(&self) -> PluginInstallationIndex {
        self.items
            .iter()
            .map(|item| (item.component_id.clone(), item.enabled))
            .collect()
    }

    fn unavailable(observed_at_ms: u64, error: impl Into<String>) -> Self {
        let error = concise_error(&error.into());
        Self {
            schema_version: 1,
            available: false,
            observed_at_ms,
            generation: None,
            revision: None,
            items: Vec::new(),
            error: Some(if error.is_empty() {
                "capability observation unavailable".to_string()
            } else {
                error
            }),
        }
    }

    pub(super) fn evidence(&self) -> PluginCapabilityEvidence {
        if self.available {
            if let (Some(generation), Some(revision)) = (self.generation, self.revision.clone()) {
                return PluginCapabilityEvidence::verified(
                    self.observed_at_ms,
                    generation,
                    revision,
                );
            }
        }
        PluginCapabilityEvidence::unavailable(
            self.observed_at_ms,
            self.error
                .as_deref()
                .unwrap_or("capability observation unavailable"),
        )
    }
}

pub(super) async fn observe(manager: &PluginManager) -> PluginCapabilityEvidence {
    installation_snapshot(manager).await.evidence()
}

pub(super) async fn installation_snapshot(manager: &PluginManager) -> PluginInstallationSnapshot {
    let observed_at_ms = match unix_time_millis() {
        Ok(observed_at_ms) => observed_at_ms,
        Err(error) => return PluginInstallationSnapshot::unavailable(1, error),
    };
    let executable =
        match crate::components::find_ready_executable_with("use", &manager.component_paths) {
            Ok(Some(executable)) => executable,
            Ok(None) => {
                return PluginInstallationSnapshot::unavailable(
                    observed_at_ms,
                    "A3S Use is not installed and ready",
                );
            }
            Err(error) => {
                return PluginInstallationSnapshot::unavailable(observed_at_ms, error.to_string());
            }
        };
    match read_snapshot(&executable, manager.process.workspace(), observed_at_ms).await {
        Ok(snapshot) => snapshot,
        Err(error) => PluginInstallationSnapshot::unavailable(observed_at_ms, error),
    }
}

async fn read_snapshot(
    executable: &Path,
    workspace: &Path,
    observed_at_ms: u64,
) -> Result<PluginInstallationSnapshot, String> {
    let mut command = Command::new(executable);
    command
        .args(["capability", "snapshot", "--json"])
        .current_dir(workspace)
        .kill_on_drop(true);
    let output = timeout(
        Duration::from_secs(CAPABILITY_SNAPSHOT_TIMEOUT_SECONDS),
        command.output(),
    )
    .await
    .map_err(|_| {
        format!(
            "A3S Use capability snapshot timed out after {CAPABILITY_SNAPSHOT_TIMEOUT_SECONDS} seconds"
        )
    })?
    .map_err(|error| format!("failed to run A3S Use capability snapshot: {error}"))?;
    if output.stdout.len() > MAX_PLUGIN_COMMAND_OUTPUT
        || output.stderr.len() > MAX_PLUGIN_COMMAND_OUTPUT
    {
        return Err("A3S Use capability snapshot exceeded the supported size".to_string());
    }
    if !output.status.success() {
        return Err(format!(
            "A3S Use capability snapshot failed{}",
            stderr_suffix(&output.stderr)
        ));
    }
    parse_snapshot(&output.stdout, observed_at_ms)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotEnvelope {
    schema_version: u32,
    ok: bool,
    data: SnapshotData,
}

#[derive(Debug, Deserialize)]
struct SnapshotData {
    registry: RegistryEvidence,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryEvidence {
    schema_version: u32,
    generation: u64,
    revision: String,
    #[serde(default)]
    capabilities: Vec<RegistryCapability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum CapabilityOrigin {
    BuiltIn,
    Extension,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryCapability {
    id: String,
    route: String,
    version: String,
    origin: CapabilityOrigin,
    enabled: bool,
    #[serde(default)]
    lifecycle_generation: Option<u64>,
    #[serde(default)]
    readiness: PluginPackageReadiness,
    #[serde(default)]
    reconciliation: Option<Value>,
    #[serde(default)]
    planner_evidence: Option<PluginPlannerEvidence>,
}

fn parse_snapshot(input: &[u8], observed_at_ms: u64) -> Result<PluginInstallationSnapshot, String> {
    let envelope: SnapshotEnvelope = serde_json::from_slice(input)
        .map_err(|error| format!("A3S Use returned invalid capability JSON: {error}"))?;
    let registry = envelope.data.registry;
    if envelope.schema_version != 1
        || !envelope.ok
        || registry.schema_version != 1
        || !valid_revision(&registry.revision)
        || registry.capabilities.len() > 10_000
    {
        return Err("A3S Use returned invalid capability registry evidence".to_string());
    }
    let mut identities = BTreeSet::new();
    let mut items = registry
        .capabilities
        .into_iter()
        .filter(|capability| capability.origin == CapabilityOrigin::Extension)
        .map(|capability| {
            let package_id = plugin_package_id(&capability.id)?;
            if !valid_segment(&capability.route) {
                return Err("A3S Use returned an invalid plugin route".to_string());
            }
            semver::Version::parse(&capability.version)
                .map_err(|_| "A3S Use returned an invalid plugin version".to_string())?;
            if capability.lifecycle_generation == Some(0) {
                return Err("A3S Use returned an invalid plugin lifecycle generation".to_string());
            }
            if !identities.insert(capability.id.clone()) {
                return Err("A3S Use returned duplicate installed plugins".to_string());
            }
            let enabled = desired_enabled(capability.enabled, capability.reconciliation.as_ref())?;
            validate_planner_evidence(
                capability.planner_evidence.as_ref(),
                &package_id,
                enabled,
                capability.reconciliation.as_ref(),
            )?;
            if capability.planner_evidence.is_some() && capability.lifecycle_generation.is_none() {
                return Err(
                    "A3S Use plan-ready plugin omitted its lifecycle generation".to_string()
                );
            }
            Ok(PluginInstalledPackage {
                component_id: capability.id,
                package_id,
                route: capability.route,
                version: capability.version,
                enabled,
                callable: capability.enabled,
                readiness: capability.readiness,
                lifecycle_generation: capability.lifecycle_generation,
                reconciliation: capability.reconciliation,
                planner_evidence: capability.planner_evidence,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if items.len() > MAX_INSTALLED_PLUGINS {
        return Err(format!(
            "A3S Use returned more than {MAX_INSTALLED_PLUGINS} installed plugins"
        ));
    }
    items.sort_by(|left, right| left.component_id.cmp(&right.component_id));
    Ok(PluginInstallationSnapshot {
        schema_version: 1,
        available: true,
        observed_at_ms,
        generation: Some(registry.generation),
        revision: Some(registry.revision),
        items,
        error: None,
    })
}

fn plugin_package_id(component_id: &str) -> Result<String, String> {
    let segments = component_id.split('/').collect::<Vec<_>>();
    if segments.len() != 3
        || segments[0] != "use"
        || !segments[1..].iter().copied().all(valid_segment)
    {
        return Err("A3S Use returned an invalid plugin component ID".to_string());
    }
    Ok(format!("{}/{}", segments[1], segments[2]))
}

fn desired_enabled(callable: bool, reconciliation: Option<&Value>) -> Result<bool, String> {
    let Some(reconciliation) = reconciliation else {
        return Ok(callable);
    };
    if reconciliation.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || !reconciliation
            .get("capabilityReady")
            .is_some_and(Value::is_boolean)
        || !reconciliation.get("surfaces").is_some_and(Value::is_array)
    {
        return Err("A3S Use returned invalid plugin reconciliation evidence".to_string());
    }
    match reconciliation.get("desired").and_then(Value::as_str) {
        Some("enabled") => Ok(true),
        Some("installed-disabled" | "absent") => Ok(false),
        _ => Err("A3S Use returned an invalid desired plugin state".to_string()),
    }
}

fn validate_planner_evidence(
    evidence: Option<&PluginPlannerEvidence>,
    package_id: &str,
    desired_enabled: bool,
    reconciliation: Option<&Value>,
) -> Result<(), String> {
    let Some(evidence) = evidence else {
        return Ok(());
    };
    if evidence.schema_version != 1
        || evidence.package_id != package_id
        || evidence.desired_enabled != desired_enabled
        || !valid_sha256_digest(&evidence.package_sha256)
        || !valid_sha256_digest(&evidence.manifest_sha256)
        || !valid_sha256_digest(&evidence.receipt_digest)
        || !valid_sha256_digest(&evidence.catalog_record_digest)
        || evidence.selected_surfaces.is_empty()
        || evidence.selected_surfaces.len() > MAX_PLUGIN_PLAN_ITEMS
        || !evidence.selected_surfaces.iter().all(valid_surface_ref)
        || !strictly_sorted(&evidence.selected_surfaces)
    {
        return Err("A3S Use returned invalid plugin planner evidence".to_string());
    }
    let reconciliation = reconciliation
        .ok_or_else(|| "A3S Use planner evidence omitted reconciliation state".to_string())?;
    let surfaces = reconciliation
        .get("surfaces")
        .and_then(Value::as_array)
        .ok_or_else(|| "A3S Use planner evidence omitted its surface inventory".to_string())?;
    if surfaces.len() > MAX_PLUGIN_PLAN_ITEMS {
        return Err("A3S Use returned too many reconciled plugin surfaces".to_string());
    }
    let mut reconciled = surfaces
        .iter()
        .map(|surface| {
            let surface = surface
                .get("surface")
                .ok_or_else(|| "A3S Use returned invalid reconciled plugin surfaces".to_string())?;
            let reference = serde_json::from_value::<PluginSurfaceRef>(surface.clone())
                .map_err(|_| "A3S Use returned invalid reconciled plugin surfaces".to_string())?;
            if !valid_surface_ref(&reference) {
                return Err("A3S Use returned invalid reconciled plugin surfaces".to_string());
            }
            Ok(reference)
        })
        .collect::<Result<Vec<_>, String>>()?;
    reconciled.sort();
    if reconciled.windows(2).any(|pair| pair[0] == pair[1])
        || reconciled != evidence.selected_surfaces
    {
        return Err("A3S Use planner evidence does not match reconciliation surfaces".to_string());
    }
    Ok(())
}

fn valid_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(valid_revision)
}

fn valid_surface_ref(reference: &PluginSurfaceRef) -> bool {
    reference.id.len() <= 63 && valid_segment(&reference.id)
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn valid_segment(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn valid_revision(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn unix_time_millis() -> Result<u64, String> {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let timestamp = u64::try_from(timestamp)
        .map_err(|error| format!("system time is before the Unix epoch: {error}"))?;
    if timestamp == 0 {
        return Err("system time does not provide a valid observation timestamp".to_string());
    }
    Ok(timestamp)
}

fn concise_error(value: &str) -> String {
    let value = value.trim().replace(['\n', '\r'], " ");
    let character_count = value.chars().count();
    let mut concise = value
        .chars()
        .take(if character_count > 500 { 499 } else { 500 })
        .collect::<String>();
    if character_count > 500 {
        concise.push('…');
    }
    concise
}

fn stderr_suffix(stderr: &[u8]) -> String {
    let stderr = concise_error(&String::from_utf8_lossy(stderr));
    if stderr.is_empty() {
        String::new()
    } else {
        format!(": {stderr}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_snapshot_parser_keeps_only_bounded_state_evidence() {
        let revision = "a".repeat(64);
        let input = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "registry": {
                    "schemaVersion": 1,
                    "generation": 17,
                    "revision": revision,
                    "capabilities": [{
                        "id": "use/browser",
                        "route": "browser",
                        "version": "0.2.1",
                        "origin": "built-in",
                        "enabled": true,
                        "untrustedDescription": "not retained"
                    }]
                }
            }
        }))
        .unwrap();

        let snapshot = parse_snapshot(&input, 42).unwrap();
        let evidence = snapshot.evidence();

        assert_eq!(evidence.status, PluginCapabilityEvidenceStatus::Verified);
        assert_eq!(evidence.observed_at_ms, 42);
        assert_eq!(evidence.generation, Some(17));
        assert_eq!(evidence.revision.as_deref(), Some(revision.as_str()));
        assert_eq!(evidence.error, None);
        assert!(snapshot.items.is_empty());
    }

    #[test]
    fn capability_snapshot_parser_rejects_noncanonical_revision() {
        let input = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "registry": {
                    "schemaVersion": 1,
                    "generation": 17,
                    "revision": "ABC",
                    "capabilities": []
                }
            }
        }))
        .unwrap();

        assert!(parse_snapshot(&input, 42).is_err());
    }

    #[test]
    fn unavailable_evidence_error_stays_within_the_record_limit() {
        let evidence = PluginCapabilityEvidence::unavailable(42, "x".repeat(600));

        assert_eq!(evidence.error.unwrap().chars().count(), 500);
    }

    #[test]
    fn installation_snapshot_separates_desired_enabled_from_callable() {
        let revision = "b".repeat(64);
        let input = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "registry": {
                    "schemaVersion": 1,
                    "generation": 18,
                    "revision": revision,
                    "capabilities": [{
                        "id": "use/acme/research",
                        "route": "research",
                        "version": "2.0.0",
                        "origin": "extension",
                        "enabled": false,
                        "readiness": "unknown",
                        "reconciliation": {
                            "schemaVersion": 1,
                            "desired": "enabled",
                            "observed": "reconciling",
                            "capabilityReady": false,
                            "surfaces": []
                        }
                    }]
                }
            }
        }))
        .unwrap();

        let snapshot = parse_snapshot(&input, 42).unwrap();

        assert!(snapshot.available);
        assert_eq!(snapshot.items.len(), 1);
        assert!(snapshot.items[0].enabled);
        assert!(!snapshot.items[0].callable);
        assert_eq!(snapshot.index().get("use/acme/research"), Some(&true));
    }

    #[test]
    fn installation_snapshot_rejects_invalid_reconciliation_evidence() {
        let input = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "registry": {
                    "schemaVersion": 1,
                    "generation": 18,
                    "revision": "c".repeat(64),
                    "capabilities": [{
                        "id": "use/acme/research",
                        "route": "research",
                        "version": "2.0.0",
                        "origin": "extension",
                        "enabled": false,
                        "reconciliation": {
                            "schemaVersion": 1,
                            "desired": "unknown",
                            "capabilityReady": false,
                            "surfaces": []
                        }
                    }]
                }
            }
        }))
        .unwrap();

        assert!(parse_snapshot(&input, 42).is_err());
    }

    #[test]
    fn installation_snapshot_retains_strict_plugin_planner_evidence() {
        let input = plan_ready_snapshot();

        let snapshot = parse_snapshot(&serde_json::to_vec(&input).unwrap(), 42).unwrap();
        let evidence = snapshot.items[0].planner_evidence.as_ref().unwrap();

        assert_eq!(evidence.package_id, "acme/research");
        assert_eq!(
            evidence.receipt_digest,
            format!("sha256:{}", "d".repeat(64))
        );
        assert_eq!(
            evidence.selected_surfaces,
            vec![PluginSurfaceRef {
                kind: a3s_use_core::PluginSurfaceKind::Skill,
                id: "research".to_string(),
            }]
        );
    }

    #[test]
    fn installation_snapshot_rejects_drifted_plugin_planner_evidence() {
        let mut invalid_digest = plan_ready_snapshot();
        invalid_digest["data"]["registry"]["capabilities"][0]["plannerEvidence"]["packageSha256"] =
            Value::String("sha256:ABC".to_string());
        assert!(parse_snapshot(&serde_json::to_vec(&invalid_digest).unwrap(), 42).is_err());

        let mut invalid_desired = plan_ready_snapshot();
        invalid_desired["data"]["registry"]["capabilities"][0]["plannerEvidence"]
            ["desiredEnabled"] = Value::Bool(false);
        assert!(parse_snapshot(&serde_json::to_vec(&invalid_desired).unwrap(), 42).is_err());

        let mut drifted_surface = plan_ready_snapshot();
        drifted_surface["data"]["registry"]["capabilities"][0]["plannerEvidence"]
            ["selectedSurfaces"][0]["id"] = Value::String("other".to_string());
        assert!(parse_snapshot(&serde_json::to_vec(&drifted_surface).unwrap(), 42).is_err());

        let mut unknown_field = plan_ready_snapshot();
        unknown_field["data"]["registry"]["capabilities"][0]["plannerEvidence"]["untrusted"] =
            Value::Bool(true);
        assert!(parse_snapshot(&serde_json::to_vec(&unknown_field).unwrap(), 42).is_err());
    }

    fn plan_ready_snapshot() -> Value {
        serde_json::json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "registry": {
                    "schemaVersion": 1,
                    "generation": 19,
                    "revision": "e".repeat(64),
                    "capabilities": [{
                        "id": "use/acme/research",
                        "route": "research",
                        "version": "2.0.0",
                        "origin": "extension",
                        "enabled": true,
                        "lifecycleGeneration": 7,
                        "readiness": "ready",
                        "reconciliation": {
                            "schemaVersion": 1,
                            "desired": "enabled",
                            "observed": "ready",
                            "capabilityReady": true,
                            "surfaces": [{
                                "surface": {
                                    "kind": "skill",
                                    "id": "research"
                                }
                            }]
                        },
                        "plannerEvidence": {
                            "schemaVersion": 1,
                            "packageId": "acme/research",
                            "packageSha256": format!("sha256:{}", "a".repeat(64)),
                            "manifestSha256": format!("sha256:{}", "b".repeat(64)),
                            "receiptDigest": format!("sha256:{}", "d".repeat(64)),
                            "catalogRecordDigest": format!("sha256:{}", "c".repeat(64)),
                            "desiredEnabled": true,
                            "selectedSurfaces": [{
                                "kind": "skill",
                                "id": "research"
                            }]
                        }
                    }]
                }
            }
        })
    }
}
