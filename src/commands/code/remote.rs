use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use a3s_code_core::{
    AgentProtocolChangeSetV1, AGENT_PROTOCOL_MAX_CHANGE_SET_BYTES,
    AGENT_PROTOCOL_MAX_CHANGE_SET_RESPONSE_BYTES,
};
use anyhow::Context;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::cli::args::{CodeRemoteArgs, CodeRemoteCommand, CodeRemoteExecutionArgs, OutputMode};
use crate::cli::context::InvocationContext;
use crate::cli::output::{render_value, write_jsonl, CliError, ExitClass};

const CLOUD_ENVELOPE_OVERHEAD_BYTES: usize = 256 * 1024;
const CLOUD_RESPONSE_BYTES: usize =
    AGENT_PROTOCOL_MAX_CHANGE_SET_RESPONSE_BYTES + CLOUD_ENVELOPE_OVERHEAD_BYTES;
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CloudAgentExecutionChangeSet {
    organization_id: String,
    execution_id: String,
    batch_id: String,
    node_id: String,
    change_set: AgentProtocolChangeSetV1,
    recorded_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CloudSuccessEnvelope<T> {
    code: u16,
    message: String,
    data: T,
    request_id: String,
    timestamp: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CloudErrorEnvelope {
    code: u16,
    status_code: String,
    message: String,
    details: Value,
    request_id: String,
    timestamp: String,
}

struct FetchedChangeSet {
    record: CloudAgentExecutionChangeSet,
    patch: Vec<u8>,
}

#[derive(Debug)]
struct ApplyOutcome {
    repository_root: PathBuf,
    applied: bool,
}

pub(crate) async fn run(args: CodeRemoteArgs, context: &InvocationContext) -> anyhow::Result<()> {
    match args.command {
        CodeRemoteCommand::Diff(args) => diff(args, context).await,
        CodeRemoteCommand::Apply(args) => apply(args, context).await,
    }
}

async fn diff(args: CodeRemoteExecutionArgs, context: &InvocationContext) -> anyhow::Result<()> {
    let fetched = fetch_change_set(&args, context).await?;
    match context.output_mode() {
        OutputMode::Human => write_exact_stdout(&fetched.patch),
        OutputMode::Json => {
            let data = serde_json::to_value(&fetched.record)?;
            render_value(OutputMode::Json, "code.remote.diff", data, || {})
        }
        OutputMode::Jsonl => write_jsonl(&json!({
            "schemaVersion": 1,
            "command": "code.remote.diff",
            "type": "result",
            "sequence": 1,
            "ok": true,
            "data": fetched.record,
        })),
    }
}

async fn apply(args: CodeRemoteExecutionArgs, context: &InvocationContext) -> anyhow::Result<()> {
    let fetched = fetch_change_set(&args, context).await?;
    let outcome =
        apply_patch_to_workspace(&context.directory, &fetched.patch, &context.cancellation).await?;
    let data = json!({
        "organizationId": fetched.record.organization_id,
        "executionId": fetched.record.execution_id,
        "state": fetched.record.change_set.state,
        "baseTree": fetched.record.change_set.base_tree,
        "resultTree": fetched.record.change_set.result_tree,
        "patchDigest": fetched.record.change_set.patch_digest,
        "patchBytes": fetched.record.change_set.patch_bytes,
        "repositoryRoot": outcome.repository_root,
        "applied": outcome.applied,
    });
    match context.output_mode() {
        OutputMode::Human => {
            if outcome.applied {
                println!(
                    "applied remote execution {} to {}",
                    args.execution_id,
                    outcome.repository_root.display()
                );
            } else {
                println!("remote execution {} contains no changes", args.execution_id);
            }
            Ok(())
        }
        OutputMode::Json => render_value(OutputMode::Json, "code.remote.apply", data, || {}),
        OutputMode::Jsonl => write_jsonl(&json!({
            "schemaVersion": 1,
            "command": "code.remote.apply",
            "type": "result",
            "sequence": 1,
            "ok": true,
            "data": data,
        })),
    }
}

async fn fetch_change_set(
    args: &CodeRemoteExecutionArgs,
    context: &InvocationContext,
) -> anyhow::Result<FetchedChangeSet> {
    if context.network.offline {
        return Err(CliError::new(
            "code.remote.offline",
            "remote execution changes are unavailable in offline mode",
            ExitClass::Failure,
        )
        .with_suggestion("Rerun without --offline after network access is available.")
        .into());
    }
    let (config_path, config) = crate::commands::config::load_active_config(context)?;
    let os_config = config.os.ok_or_else(|| {
        CliError::new(
            "code.remote.not_configured",
            format!(
                "OS is not configured in {}; add `os = \"https://your-os-host\"`",
                config_path.display()
            ),
            ExitClass::Failure,
        )
    })?;
    let mut session = crate::a3s_os::current_session(&os_config).ok_or_else(|| {
        CliError::new(
            "code.remote.auth_required",
            "no signed-in OS session is available for remote execution changes",
            ExitClass::Failure,
        )
        .with_suggestion("Run `a3s auth login os` and retry.")
    })?;
    if crate::a3s_os::needs_refresh(&session) {
        session = crate::a3s_os::refresh_session(&session)
            .await
            .context("could not refresh the OS session")?;
    }

    let origin = crate::a3s_os::os_origin(&session.address);
    let url = change_set_url(&origin, &args.organization, &args.execution_id)?;
    let client = cloud_client(&origin)?;
    let request = client
        .get(url)
        .header("accept", "application/json")
        .bearer_auth(&session.access_token);
    let response = tokio::select! {
        _ = context.cancellation.cancelled() => return Err(cancelled_error()),
        result = request.send() => result.context("could not request remote execution changes")?,
    };
    let status = response.status();
    let body = bounded_response_body(response, CLOUD_RESPONSE_BYTES, &context.cancellation).await?;
    if !status.is_success() {
        return Err(cloud_status_error(status.as_u16(), &body));
    }
    let envelope: CloudSuccessEnvelope<CloudAgentExecutionChangeSet> =
        serde_json::from_slice(&body).map_err(|_| {
            CliError::new(
                "code.remote.invalid_response",
                "Cloud returned an invalid execution change-set response",
                ExitClass::Failure,
            )
        })?;
    if envelope.code != status.as_u16()
        || envelope.message.trim().is_empty()
        || uuid::Uuid::parse_str(&envelope.request_id).is_err()
        || chrono::DateTime::parse_from_rfc3339(&envelope.timestamp).is_err()
    {
        return Err(invalid_response_error());
    }
    let patch = validate_change_set_record(&envelope.data, &args.organization, &args.execution_id)?;
    Ok(FetchedChangeSet {
        record: envelope.data,
        patch,
    })
}

fn change_set_url(origin: &str, organization_id: &str, execution_id: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(&format!("{}/api/v1/", origin.trim_end_matches('/')))
        .context("configured OS address is not a valid URL")?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("configured OS address cannot be used as an API base URL"))?
        .pop_if_empty()
        .extend([
            "organizations",
            organization_id,
            "agent-executions",
            execution_id,
            "changes",
        ]);
    Ok(url)
}

fn cloud_client(origin: &str) -> anyhow::Result<reqwest::Client> {
    let parsed = Url::parse(origin).context("configured OS address is not a valid URL")?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        anyhow::bail!("configured OS address must use HTTP or HTTPS");
    }
    let mut builder = reqwest::Client::builder()
        .timeout(REMOTE_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .user_agent(format!("a3s/{}", env!("CARGO_PKG_VERSION")));
    if parsed.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    }) {
        builder = builder.no_proxy();
    }
    builder.build().context("could not create Cloud client")
}

async fn bounded_response_body(
    mut response: reqwest::Response,
    maximum: usize,
    cancellation: &CancellationToken,
) -> anyhow::Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        return Err(response_too_large_error());
    }
    let mut body = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled_error()),
            result = response.chunk() => result.context("could not read Cloud response")?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        if body
            .len()
            .checked_add(chunk.len())
            .is_none_or(|length| length > maximum)
        {
            return Err(response_too_large_error());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn validate_change_set_record(
    record: &CloudAgentExecutionChangeSet,
    expected_organization_id: &str,
    expected_execution_id: &str,
) -> anyhow::Result<Vec<u8>> {
    let organization_id =
        uuid::Uuid::parse_str(&record.organization_id).map_err(|_| invalid_response_error())?;
    let execution_id =
        uuid::Uuid::parse_str(&record.execution_id).map_err(|_| invalid_response_error())?;
    let batch_id = uuid::Uuid::parse_str(&record.batch_id).map_err(|_| invalid_response_error())?;
    let node_id = uuid::Uuid::parse_str(&record.node_id).map_err(|_| invalid_response_error())?;
    if organization_id.is_nil()
        || execution_id.is_nil()
        || batch_id.is_nil()
        || node_id.is_nil()
        || record.organization_id != expected_organization_id
        || record.execution_id != expected_execution_id
        || chrono::DateTime::parse_from_rfc3339(&record.recorded_at).is_err()
    {
        return Err(invalid_response_error());
    }
    record
        .change_set
        .validate()
        .map_err(|_| invalid_response_error())?;
    let patch = base64::engine::general_purpose::STANDARD
        .decode(&record.change_set.patch_base64)
        .map_err(|_| invalid_response_error())?;
    if patch.len() > AGENT_PROTOCOL_MAX_CHANGE_SET_BYTES
        || u64::try_from(patch.len()).ok() != Some(record.change_set.patch_bytes)
        || record.change_set.patch_digest != format!("sha256:{:x}", Sha256::digest(&patch))
    {
        return Err(invalid_response_error());
    }
    Ok(patch)
}

async fn apply_patch_to_workspace(
    directory: &Path,
    patch: &[u8],
    cancellation: &CancellationToken,
) -> anyhow::Result<ApplyOutcome> {
    let repository_root = git_repository_root(directory, cancellation).await?;
    if patch.is_empty() {
        return Ok(ApplyOutcome {
            repository_root,
            applied: false,
        });
    }
    if cancellation.is_cancelled() {
        return Err(cancelled_error());
    }
    let mut patch_file = tempfile::Builder::new()
        .prefix("a3s-remote-")
        .suffix(".patch")
        .tempfile()
        .context("could not create a protected temporary patch file")?;
    patch_file
        .as_file_mut()
        .write_all(patch)
        .context("could not write the temporary patch file")?;
    patch_file
        .as_file_mut()
        .flush()
        .context("could not flush the temporary patch file")?;
    let patch_path = patch_file.path().as_os_str().to_owned();
    let arguments = |check: bool| {
        let mut args = vec![OsString::from("apply")];
        if check {
            args.push(OsString::from("--check"));
        }
        args.extend([
            OsString::from("--binary"),
            OsString::from("--whitespace=nowarn"),
            OsString::from("--"),
            patch_path.clone(),
        ]);
        args
    };
    let checked = run_git(&repository_root, &arguments(true), cancellation).await?;
    if !checked.status.success() {
        return Err(CliError::new(
            "code.remote.apply_conflict",
            "remote execution changes do not apply cleanly to the local workspace",
            ExitClass::Failure,
        )
        .with_suggestion(
            "Review `a3s code remote diff` and resolve local divergence before retrying.",
        )
        .with_details(json!({"git": bounded_stderr(&checked.stderr)}))
        .into());
    }
    if cancellation.is_cancelled() {
        return Err(cancelled_error());
    }
    let applied = run_git(&repository_root, &arguments(false), cancellation).await?;
    if !applied.status.success() {
        return Err(CliError::new(
            "code.remote.apply_failed",
            "Git could not apply the already-validated remote execution changes",
            ExitClass::Failure,
        )
        .with_details(json!({"git": bounded_stderr(&applied.stderr)}))
        .into());
    }
    Ok(ApplyOutcome {
        repository_root,
        applied: true,
    })
}

async fn git_repository_root(
    directory: &Path,
    cancellation: &CancellationToken,
) -> anyhow::Result<PathBuf> {
    let output = run_git(
        directory,
        &[
            OsString::from("rev-parse"),
            OsString::from("--show-toplevel"),
        ],
        cancellation,
    )
    .await?;
    if !output.status.success() {
        return Err(CliError::new(
            "code.remote.workspace_not_git",
            "remote execution changes can only be applied inside a Git workspace",
            ExitClass::Failure,
        )
        .into());
    }
    let root =
        String::from_utf8(output.stdout).context("Git returned a non-UTF-8 repository path")?;
    let root = PathBuf::from(root.trim());
    root.canonicalize()
        .with_context(|| format!("could not resolve Git repository root {}", root.display()))
}

async fn run_git(
    directory: &Path,
    arguments: &[OsString],
    cancellation: &CancellationToken,
) -> anyhow::Result<std::process::Output> {
    let mut command = tokio::process::Command::new("git");
    command
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    tokio::select! {
        _ = cancellation.cancelled() => Err(cancelled_error()),
        result = tokio::time::timeout(GIT_COMMAND_TIMEOUT, command.output()) => {
            result
                .map_err(|_| anyhow::anyhow!("Git command timed out after {} seconds", GIT_COMMAND_TIMEOUT.as_secs()))?
                .context("could not run Git")
        }
    }
}

fn write_exact_stdout(bytes: &[u8]) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    stdout.write_all(bytes)?;
    stdout.flush()?;
    Ok(())
}

fn cloud_status_error(status: u16, body: &[u8]) -> anyhow::Error {
    let parsed = serde_json::from_slice::<CloudErrorEnvelope>(body).ok();
    let server_message = parsed.as_ref().and_then(|error| {
        (error.code == status
            && !error.status_code.trim().is_empty()
            && error.details.is_object()
            && uuid::Uuid::parse_str(&error.request_id).is_ok()
            && chrono::DateTime::parse_from_rfc3339(&error.timestamp).is_ok())
        .then(|| sanitize_message(&error.message))
        .filter(|message| !message.is_empty())
    });
    let (code, message, suggestion) = match status {
        401 => (
            "code.remote.auth_required",
            "Cloud rejected the stored OS session".to_string(),
            Some("Run `a3s auth login os` and retry."),
        ),
        403 => (
            "code.remote.forbidden",
            "Cloud denied access to the requested execution".to_string(),
            None,
        ),
        404 => (
            "code.remote.not_found",
            "the execution change set was not found or is not available yet".to_string(),
            Some("Wait for the execution to reach a terminal state, then retry."),
        ),
        _ => (
            "code.remote.request_failed",
            server_message.unwrap_or_else(|| format!("Cloud returned HTTP {status}")),
            None,
        ),
    };
    let mut error = CliError::new(code, message, ExitClass::Failure)
        .with_details(json!({"httpStatus": status}));
    if let Some(suggestion) = suggestion {
        error = error.with_suggestion(suggestion);
    }
    error.into()
}

fn sanitize_message(message: &str) -> String {
    message
        .chars()
        .take(512)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn bounded_stderr(stderr: &[u8]) -> String {
    sanitize_message(&String::from_utf8_lossy(
        &stderr[..stderr.len().min(4 * 1024)],
    ))
}

fn invalid_response_error() -> anyhow::Error {
    CliError::new(
        "code.remote.invalid_response",
        "Cloud returned a change set whose identity, bounds, or digest is invalid",
        ExitClass::Failure,
    )
    .into()
}

fn response_too_large_error() -> anyhow::Error {
    CliError::new(
        "code.remote.response_too_large",
        "Cloud execution change-set response exceeds the supported protocol bound",
        ExitClass::Failure,
    )
    .into()
}

fn cancelled_error() -> anyhow::Error {
    CliError::new(
        "code.remote.cancelled",
        "remote execution change operation was cancelled",
        ExitClass::Cancelled,
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORGANIZATION_ID: &str = "019c0000-0000-7000-8000-000000000001";
    const EXECUTION_ID: &str = "019c0000-0000-7000-8000-000000000002";

    fn record(patch: &[u8]) -> CloudAgentExecutionChangeSet {
        CloudAgentExecutionChangeSet {
            organization_id: ORGANIZATION_ID.into(),
            execution_id: EXECUTION_ID.into(),
            batch_id: "019c0000-0000-7000-8000-000000000003".into(),
            node_id: "019c0000-0000-7000-8000-000000000004".into(),
            change_set: AgentProtocolChangeSetV1 {
                schema: AgentProtocolChangeSetV1::SCHEMA.into(),
                identity: a3s_code_core::AgentProtocolRunIdentityV1 {
                    schema: a3s_code_core::AgentProtocolRunIdentityV1::SCHEMA.into(),
                    protocol: a3s_code_core::AGENT_PROTOCOL_V1.into(),
                    agent_release_identity: format!("sha256:{}", "a".repeat(64)),
                    session_id: "session-1".into(),
                    run_id: "run-1".into(),
                },
                state: a3s_code_core::AgentProtocolRunStateV1::Completed,
                format: a3s_code_core::AGENT_PROTOCOL_CHANGE_SET_FORMAT_V1.into(),
                encoding: a3s_code_core::AGENT_PROTOCOL_CHANGE_SET_ENCODING_V1.into(),
                base_tree: format!("git-tree:{}", "a".repeat(40)),
                result_tree: format!("git-tree:{}", "b".repeat(40)),
                patch_digest: format!("sha256:{:x}", Sha256::digest(patch)),
                patch_bytes: u64::try_from(patch.len()).unwrap(),
                patch_base64: base64::engine::general_purpose::STANDARD.encode(patch),
                observed_at_ms: 1_723_000_000_000,
            },
            recorded_at: "2026-08-11T00:00:00Z".into(),
        }
    }

    #[test]
    fn validates_exact_patch_bytes_and_rejects_tampering() {
        let patch = b"diff --git a/a b/a\n";
        let mut value = record(patch);
        assert_eq!(
            validate_change_set_record(&value, ORGANIZATION_ID, EXECUTION_ID).unwrap(),
            patch
        );

        value.change_set.patch_base64 =
            base64::engine::general_purpose::STANDARD.encode(b"tampered");
        assert!(validate_change_set_record(&value, ORGANIZATION_ID, EXECUTION_ID).is_err());
    }

    #[test]
    fn builds_a_tenant_scoped_encoded_cloud_url() {
        let url = change_set_url("https://cloud.example", ORGANIZATION_ID, EXECUTION_ID).unwrap();
        assert_eq!(
            url.as_str(),
            format!(
                "https://cloud.example/api/v1/organizations/{ORGANIZATION_ID}/agent-executions/{EXECUTION_ID}/changes"
            )
        );
    }

    #[tokio::test]
    async fn applies_a_checked_patch_without_staging_or_committing() {
        let workspace = git_workspace("seed\n", "other\n");
        let patch = b"diff --git a/seed.txt b/seed.txt\n--- a/seed.txt\n+++ b/seed.txt\n@@ -1 +1 @@\n-seed\n+changed\n";

        let outcome = apply_patch_to_workspace(workspace.path(), patch, &CancellationToken::new())
            .await
            .unwrap();

        assert!(outcome.applied);
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("seed.txt")).unwrap(),
            "changed\n"
        );
        let status = git(workspace.path(), &["status", "--short"]);
        assert_eq!(status.trim(), "M seed.txt");
    }

    #[tokio::test]
    async fn preflight_conflict_leaves_every_file_unchanged() {
        let workspace = git_workspace("seed\n", "local\n");
        let patch = b"diff --git a/seed.txt b/seed.txt\n--- a/seed.txt\n+++ b/seed.txt\n@@ -1 +1 @@\n-seed\n+changed\ndiff --git a/other.txt b/other.txt\n--- a/other.txt\n+++ b/other.txt\n@@ -1 +1 @@\n-remote\n+changed\n";

        let error = apply_patch_to_workspace(workspace.path(), patch, &CancellationToken::new())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("do not apply cleanly"));
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("seed.txt")).unwrap(),
            "seed\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("other.txt")).unwrap(),
            "local\n"
        );
    }

    fn git_workspace(seed: &str, other: &str) -> tempfile::TempDir {
        let workspace = tempfile::tempdir().unwrap();
        git(workspace.path(), &["init"]);
        git(workspace.path(), &["config", "core.autocrlf", "false"]);
        git(workspace.path(), &["config", "user.name", "A3S Test"]);
        git(
            workspace.path(),
            &["config", "user.email", "test@a3s.invalid"],
        );
        std::fs::write(workspace.path().join("seed.txt"), seed).unwrap();
        std::fs::write(workspace.path().join("other.txt"), other).unwrap();
        git(workspace.path(), &["add", "seed.txt", "other.txt"]);
        git(workspace.path(), &["commit", "-m", "seed"]);
        workspace
    }

    fn git(directory: &Path, arguments: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(arguments)
            .current_dir(directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}
