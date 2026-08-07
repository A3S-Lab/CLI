use std::path::PathBuf;
use std::time::Duration;

use a3s_use_core::InstalledPluginPlanEvidence;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

use super::{
    PluginManagerError, PluginManagerResult, MAX_PLUGIN_COMMAND_OUTPUT,
    PLUGIN_OPERATION_TIMEOUT_SECONDS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginLifecycleAction {
    Install,
    Upgrade,
    Uninstall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginPlanRequest {
    pub action: PluginLifecycleAction,
    pub component_id: String,
    pub version: Option<String>,
    pub channel: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginApplyRequest {
    pub operation_id: String,
    pub plan_digest: String,
}

pub(super) struct A3sProcessAdapter {
    executable: PathBuf,
    config_path: PathBuf,
    workspace: PathBuf,
    offline: bool,
}

impl A3sProcessAdapter {
    pub(super) fn new(
        executable: PathBuf,
        config_path: PathBuf,
        workspace: PathBuf,
        offline: bool,
    ) -> Self {
        Self {
            executable,
            config_path,
            workspace,
            offline,
        }
    }

    pub(super) async fn plan(&self, request: &PluginPlanRequest) -> PluginManagerResult<Value> {
        let args = plugin_operation_args(
            request.action,
            &request.component_id,
            request.version.as_deref(),
            request.channel.as_deref(),
        )?;
        self.run_json(args, JsonOutputOwner::Root).await
    }

    pub(super) async fn installed_planning_evidence(
        &self,
        component_id: &str,
    ) -> PluginManagerResult<InstalledPluginPlanEvidence> {
        let component_id = normalize_component_id(component_id)?;
        let package_id = component_id.strip_prefix("use/").ok_or_else(|| {
            PluginManagerError::InvalidRequest("invalid Use package ID".to_string())
        })?;
        let value = self
            .run_json(
                use_extension_planning_evidence_args(package_id),
                JsonOutputOwner::UseProxy,
            )
            .await?;
        let evidence = value.get("planningEvidence").ok_or_else(|| {
            PluginManagerError::Upstream(
                "A3S Use planning response omitted planningEvidence".to_string(),
            )
        })?;
        let bytes = serde_json::to_vec(evidence).map_err(|error| {
            PluginManagerError::Upstream(format!(
                "failed to normalize A3S Use planning evidence: {error}"
            ))
        })?;
        InstalledPluginPlanEvidence::from_json(&bytes).map_err(|error| {
            PluginManagerError::Upstream(format!(
                "A3S Use returned invalid installed plugin planning evidence: {error}"
            ))
        })
    }

    pub(super) fn workspace(&self) -> &std::path::Path {
        &self.workspace
    }

    async fn run_json(
        &self,
        args: Vec<String>,
        output_owner: JsonOutputOwner,
    ) -> PluginManagerResult<Value> {
        let mut command = Command::new(&self.executable);
        command
            .arg("--config")
            .arg(&self.config_path)
            .arg("--directory")
            .arg(&self.workspace);
        if self.offline {
            command.arg("--offline");
        }
        command
            .args(json_invocation_args(output_owner, args))
            .current_dir(&self.workspace)
            .kill_on_drop(true);
        let operation_timeout = Duration::from_secs(PLUGIN_OPERATION_TIMEOUT_SECONDS);
        let output = timeout(operation_timeout, command.output())
            .await
            .map_err(|_| {
                PluginManagerError::Timeout(format!(
                    "plugin operation timed out after {PLUGIN_OPERATION_TIMEOUT_SECONDS} seconds"
                ))
            })?
            .map_err(|error| PluginManagerError::Upstream(format!("failed to run a3s: {error}")))?;
        if output.stdout.len() > MAX_PLUGIN_COMMAND_OUTPUT
            || output.stderr.len() > MAX_PLUGIN_COMMAND_OUTPUT
        {
            return Err(PluginManagerError::Upstream(
                "plugin operation output exceeded the supported size".to_string(),
            ));
        }
        let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            PluginManagerError::Upstream(format!(
                "a3s returned invalid JSON: {error}{}",
                stderr_suffix(&output.stderr)
            ))
        })?;
        let ok = value.get("ok").and_then(Value::as_bool) == Some(true);
        if !output.status.success() || !ok {
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| value.pointer("/error/message").and_then(Value::as_str))
                .unwrap_or("plugin operation failed");
            return Err(PluginManagerError::OperationFailed(concise_error(message)));
        }
        value.get("data").cloned().ok_or_else(|| {
            PluginManagerError::Upstream("a3s JSON response has no data".to_string())
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum JsonOutputOwner {
    Root,
    UseProxy,
}

pub(super) fn json_invocation_args(
    output_owner: JsonOutputOwner,
    args: Vec<String>,
) -> Vec<String> {
    let mut invocation = Vec::with_capacity(args.len() + 4);
    if output_owner == JsonOutputOwner::Root {
        invocation.extend(["--output".to_string(), "json".to_string()]);
    }
    invocation.extend(["--non-interactive".to_string(), "--no-progress".to_string()]);
    invocation.extend(args);
    invocation
}

pub(super) fn use_extension_planning_evidence_args(package_id: &str) -> Vec<String> {
    vec![
        "use".to_string(),
        "extension".to_string(),
        "planning-evidence".to_string(),
        package_id.to_string(),
        "--json".to_string(),
    ]
}

pub(super) fn plugin_operation_args(
    action: PluginLifecycleAction,
    component_id: &str,
    version: Option<&str>,
    channel: Option<&str>,
) -> PluginManagerResult<Vec<String>> {
    let request = normalize_plan_request(&PluginPlanRequest {
        action,
        component_id: component_id.to_string(),
        version: version.map(str::to_string),
        channel: channel.map(str::to_string),
    })?;
    let mut args = match request.action {
        PluginLifecycleAction::Install => {
            vec!["install".to_string(), request.component_id.clone()]
        }
        PluginLifecycleAction::Upgrade => {
            vec!["upgrade".to_string(), request.component_id.clone()]
        }
        PluginLifecycleAction::Uninstall => {
            vec!["uninstall".to_string(), request.component_id.clone()]
        }
    };
    if request.action == PluginLifecycleAction::Install {
        if let Some(version) = request.version {
            args.extend(["--version".to_string(), version]);
        }
        if let Some(channel) = request.channel {
            args.extend(["--channel".to_string(), channel]);
        }
    }
    args.push("--dry-run".to_string());
    Ok(args)
}

pub(super) fn normalize_plan_request(
    request: &PluginPlanRequest,
) -> PluginManagerResult<PluginPlanRequest> {
    let component_id = normalize_component_id(&request.component_id)?;
    let (version, channel) = if request.action == PluginLifecycleAction::Install {
        let version = normalize_optional_value(request.version.as_deref(), "version", 64)?;
        if let Some(version) = &version {
            let parsed = semver::Version::parse(version).map_err(|error| {
                PluginManagerError::InvalidRequest(format!(
                    "plugin version must be canonical semantic version syntax: {error}"
                ))
            })?;
            if parsed.to_string() != *version {
                return Err(PluginManagerError::InvalidRequest(
                    "plugin version must use canonical semantic version syntax".to_string(),
                ));
            }
        }
        let channel = normalize_optional_value(request.channel.as_deref(), "channel", 16)?;
        if channel
            .as_deref()
            .is_some_and(|channel| !matches!(channel, "stable" | "beta" | "nightly"))
        {
            return Err(PluginManagerError::InvalidRequest(
                "channel must be stable, beta, or nightly".to_string(),
            ));
        }
        (version, channel)
    } else {
        if request.version.is_some() || request.channel.is_some() {
            return Err(PluginManagerError::InvalidRequest(format!(
                "{} does not accept version or channel",
                request.action.as_str()
            )));
        }
        (None, None)
    };
    Ok(PluginPlanRequest {
        action: request.action,
        component_id,
        version,
        channel,
    })
}

impl PluginLifecycleAction {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Upgrade => "upgrade",
            Self::Uninstall => "uninstall",
        }
    }
}

pub(super) fn normalize_component_id(value: &str) -> PluginManagerResult<String> {
    let value = value.trim();
    let segments = value.split('/').collect::<Vec<_>>();
    if segments.len() != 3
        || segments[0] != "use"
        || !segments[1..].iter().copied().all(valid_segment)
    {
        return Err(PluginManagerError::InvalidRequest(
            "plugin component IDs must be use/<publisher>/<name>".to_string(),
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

fn normalize_optional_value(
    value: Option<&str>,
    label: &str,
    max_chars: usize,
) -> PluginManagerResult<Option<String>> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.chars().count() > max_chars || value.chars().any(char::is_whitespace) {
                Err(PluginManagerError::InvalidRequest(format!(
                    "invalid plugin {label}"
                )))
            } else {
                Ok(value.to_string())
            }
        })
        .transpose()
}

pub(super) fn normalize_plan_digest(value: &str) -> PluginManagerResult<String> {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(PluginManagerError::InvalidRequest(
            "planDigest must contain 64 lowercase hexadecimal characters".to_string(),
        ));
    }
    Ok(value.to_string())
}

fn concise_error(value: &str) -> String {
    let value = value.trim().replace(['\n', '\r'], " ");
    let mut concise = value.chars().take(500).collect::<String>();
    if value.chars().count() > 500 {
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
    fn installed_planning_evidence_uses_the_bounded_use_proxy_command() {
        assert_eq!(
            use_extension_planning_evidence_args("acme/guide"),
            vec![
                "use",
                "extension",
                "planning-evidence",
                "acme/guide",
                "--json"
            ]
        );
    }
}
