use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

use super::{PluginManager, MAX_PLUGIN_COMMAND_OUTPUT};

const CAPABILITY_SNAPSHOT_TIMEOUT_SECONDS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginCapabilityEvidenceStatus {
    Verified,
    Unavailable,
}

/// One bounded observation of the immutable A3S Use capability registry.
///
/// Evidence can be unavailable when A3S Use is absent or unhealthy. That state
/// is explicit so a successful package mutation is never reported as failed
/// merely because post-mutation observation could not be collected.
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

pub(super) async fn observe(manager: &PluginManager) -> PluginCapabilityEvidence {
    let observed_at_ms = match unix_time_millis() {
        Ok(observed_at_ms) => observed_at_ms,
        Err(error) => return PluginCapabilityEvidence::unavailable(1, error),
    };
    let executable =
        match crate::components::find_ready_executable_with("use", &manager.component_paths) {
            Ok(Some(executable)) => executable,
            Ok(None) => {
                return PluginCapabilityEvidence::unavailable(
                    observed_at_ms,
                    "A3S Use is not installed and ready",
                );
            }
            Err(error) => {
                return PluginCapabilityEvidence::unavailable(observed_at_ms, error.to_string());
            }
        };
    match read_snapshot(&executable, manager.process.workspace(), observed_at_ms).await {
        Ok(evidence) => evidence,
        Err(error) => PluginCapabilityEvidence::unavailable(observed_at_ms, error),
    }
}

async fn read_snapshot(
    executable: &Path,
    workspace: &Path,
    observed_at_ms: u64,
) -> Result<PluginCapabilityEvidence, String> {
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
    capabilities: Vec<Value>,
}

fn parse_snapshot(input: &[u8], observed_at_ms: u64) -> Result<PluginCapabilityEvidence, String> {
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
    Ok(PluginCapabilityEvidence::verified(
        observed_at_ms,
        registry.generation,
        registry.revision,
    ))
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
                        "id": "use/acme/research",
                        "untrustedDescription": "not retained"
                    }]
                }
            }
        }))
        .unwrap();

        let evidence = parse_snapshot(&input, 42).unwrap();

        assert_eq!(evidence.status, PluginCapabilityEvidenceStatus::Verified);
        assert_eq!(evidence.observed_at_ms, 42);
        assert_eq!(evidence.generation, Some(17));
        assert_eq!(evidence.revision.as_deref(), Some(revision.as_str()));
        assert_eq!(evidence.error, None);
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
}
