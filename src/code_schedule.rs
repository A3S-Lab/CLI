//! Durable, workspace-local scheduling for engineered Code loops.
//!
//! A schedule is metadata beside the existing `.a3s/loops/<id>` contract. A
//! detached singleton worker atomically claims due runs, invokes the ordinary
//! non-interactive Code boundary, and emits bounded result notifications. The
//! worker never invents a second prompt format or bypasses Code permissions.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::tui::loop_engineering::{
    append_run_start, audit_loop, find_loop, loop_run_prompt, LoopSpec,
};

const SCHEDULE_SCHEMA_VERSION: u32 = 1;
const NOTIFICATION_SCHEMA_VERSION: u32 = 1;
const SCHEDULE_FILE: &str = "schedule.json";
const SCHEDULE_LOCK_FILE: &str = ".schedule.lock";
const SCHEDULER_DIRECTORY: &str = ".scheduler";
const WORKER_LOCK_FILE: &str = "worker.lock";
const WORKER_STATUS_FILE: &str = "worker.json";
const WORKER_STOP_FILE: &str = "stop.requested";
const MAX_SCHEDULE_FILE_BYTES: u64 = 256 * 1024;
const MAX_NOTIFICATION_FILE_BYTES: u64 = 512 * 1024;
const MAX_REPORT_ARTIFACT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;
const MAX_NOTIFICATION_SUMMARY_CHARS: usize = 1_200;
const MAX_SCHEDULES: usize = 256;
const MAX_REPORT_ARTIFACTS: usize = 128;
const MIN_SCHEDULED_TOOL_ROUNDS: usize = 12;
const MAX_SCHEDULED_TOOL_ROUNDS: usize = 32;
const TOOL_ROUNDS_PER_LOOP_ITERATION: usize = 8;
const MIN_CADENCE_SECONDS: u64 = 60;
const MAX_CADENCE_SECONDS: u64 = 31 * 24 * 60 * 60;
const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(2);
const WORKER_START_TIMEOUT: Duration = Duration::from_secs(8);
const RUN_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
pub(crate) const SCHEDULE_LOOP_ENV: &str = "A3S_CODE_SCHEDULE_LOOP_ID";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScheduledExecutionPolicy {
    pub(crate) loop_id: String,
    pub(crate) denylist: Vec<String>,
    pub(crate) max_tool_rounds: usize,
    pub(crate) protected_config_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoopScheduleState {
    pub(crate) schema_version: u32,
    pub(crate) loop_id: String,
    pub(crate) workspace: PathBuf,
    pub(crate) cadence_seconds: u64,
    pub(crate) enabled: bool,
    pub(crate) model: Option<String>,
    pub(crate) next_run_at_ms: Option<u64>,
    pub(crate) pending_run_at_ms: Option<u64>,
    pub(crate) active_run: Option<ActiveScheduleRun>,
    pub(crate) last_run: Option<CompletedScheduleRun>,
    pub(crate) created_at_ms: u64,
    pub(crate) updated_at_ms: u64,
    pub(crate) revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActiveScheduleRun {
    pub(crate) run_id: String,
    pub(crate) started_at_ms: u64,
    pub(crate) worker_pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompletedScheduleRun {
    pub(crate) run_id: String,
    pub(crate) outcome: ScheduleRunOutcome,
    pub(crate) started_at_ms: u64,
    pub(crate) finished_at_ms: u64,
    pub(crate) session_id: Option<String>,
    pub(crate) result_path: PathBuf,
    pub(crate) summary: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScheduleRunOutcome {
    Succeeded,
    Failed,
    TimedOut,
    Interrupted,
}

impl ScheduleRunOutcome {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed out",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScheduleNotification {
    pub(crate) schema_version: u32,
    pub(crate) notification_id: String,
    pub(crate) loop_id: String,
    pub(crate) run_id: String,
    pub(crate) outcome: ScheduleRunOutcome,
    pub(crate) summary: String,
    pub(crate) result_path: PathBuf,
    pub(crate) created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScheduleWorkerStatus {
    pub(crate) schema_version: u32,
    pub(crate) pid: u32,
    pub(crate) workspace: PathBuf,
    pub(crate) started_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkerStartOutcome {
    AlreadyRunning,
    Started { pid: u32 },
}

#[derive(Clone, Debug)]
struct ScheduleEntry {
    path: PathBuf,
    state: LoopScheduleState,
}

#[derive(Clone, Debug)]
struct RunClaim {
    schedule_path: PathBuf,
    state: LoopScheduleState,
    run: ActiveScheduleRun,
}

#[derive(Debug)]
struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

#[derive(Debug)]
struct ExecutionResult {
    outcome: ScheduleRunOutcome,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
    session_id: Option<String>,
    summary: String,
}

#[derive(Debug)]
struct LoopArtifactSnapshot {
    state: Vec<u8>,
    run_log: Vec<u8>,
    reports: Vec<(PathBuf, Vec<u8>)>,
}

pub(crate) fn parse_cadence_seconds(value: &str) -> Result<u64, String> {
    let value = value.trim().to_ascii_lowercase();
    if value == "manual" {
        return Err("manual loops cannot be scheduled".to_string());
    }
    let split = value
        .find(|ch: char| !ch.is_ascii_digit())
        .ok_or_else(|| "cadence must include one of s, m, h, or d".to_string())?;
    let (amount, unit) = value.split_at(split);
    if amount.is_empty() || unit.len() != 1 {
        return Err("cadence must look like 15m, 2h, or 1d".to_string());
    }
    let amount = amount
        .parse::<u64>()
        .map_err(|_| "cadence amount is invalid".to_string())?;
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        _ => return Err("cadence unit must be s, m, h, or d".to_string()),
    };
    let seconds = amount
        .checked_mul(multiplier)
        .ok_or_else(|| "cadence is too large".to_string())?;
    if !(MIN_CADENCE_SECONDS..=MAX_CADENCE_SECONDS).contains(&seconds) {
        return Err("cadence must be between 1 minute and 31 days".to_string());
    }
    Ok(seconds)
}

pub(crate) fn scheduled_execution_policy(
    workspace: &Path,
    loop_id: &str,
    config_path: &Path,
) -> anyhow::Result<ScheduledExecutionPolicy> {
    let workspace = canonical_command_path(workspace)
        .with_context(|| format!("could not resolve workspace {}", workspace.display()))?;
    let workspace_text = workspace
        .to_str()
        .context("scheduled loop workspaces must use UTF-8 paths")?;
    let spec = find_loop(workspace_text, loop_id).map_err(anyhow::Error::msg)?;
    validate_schedulable_loop(&spec)?;
    validate_loop_directory(&workspace, &spec.dir, &spec.id)?;
    Ok(ScheduledExecutionPolicy {
        loop_id: spec.id,
        denylist: spec.denylist,
        max_tool_rounds: scheduled_tool_round_budget(spec.max_iterations_per_run),
        protected_config_path: canonical_command_path(config_path).with_context(|| {
            format!(
                "could not resolve active configuration {}",
                config_path.display()
            )
        })?,
    })
}

fn scheduled_tool_round_budget(max_iterations_per_run: usize) -> usize {
    max_iterations_per_run
        .saturating_mul(TOOL_ROUNDS_PER_LOOP_ITERATION)
        .clamp(MIN_SCHEDULED_TOOL_ROUNDS, MAX_SCHEDULED_TOOL_ROUNDS)
}

pub(crate) fn enable_loop_schedule(
    workspace: &Path,
    loop_id: &str,
    cadence_override: Option<&str>,
    model: Option<String>,
) -> anyhow::Result<LoopScheduleState> {
    let workspace = canonical_command_path(workspace)
        .with_context(|| format!("could not resolve workspace {}", workspace.display()))?;
    let workspace_text = workspace
        .to_str()
        .context("scheduled loop workspaces must use UTF-8 paths")?;
    let spec = find_loop(workspace_text, loop_id).map_err(anyhow::Error::msg)?;
    validate_schedulable_loop(&spec)?;
    validate_loop_directory(&workspace, &spec.dir, &spec.id)?;
    let cadence = cadence_override.unwrap_or(&spec.cadence);
    let cadence_seconds = parse_cadence_seconds(cadence).map_err(anyhow::Error::msg)?;
    let model = validate_model(model)?;
    let path = schedule_path(&spec.dir);
    with_schedule_lock(&path, || {
        let now = epoch_ms();
        let existing = read_schedule_if_present(&path)?;
        if let Some(existing) = &existing {
            validate_schedule_identity(&path, existing, &workspace)?;
        }
        let created_at_ms = existing.as_ref().map_or(now, |state| state.created_at_ms);
        let next_run_at_ms = existing
            .as_ref()
            .filter(|state| state.enabled)
            .and_then(|state| state.next_run_at_ms)
            .or_else(|| Some(add_ms(now, cadence_seconds)));
        let state = LoopScheduleState {
            schema_version: SCHEDULE_SCHEMA_VERSION,
            loop_id: spec.id.clone(),
            workspace: workspace.clone(),
            cadence_seconds,
            enabled: true,
            model,
            next_run_at_ms,
            pending_run_at_ms: existing.as_ref().and_then(|state| state.pending_run_at_ms),
            active_run: existing.as_ref().and_then(|state| state.active_run.clone()),
            last_run: existing.as_ref().and_then(|state| state.last_run.clone()),
            created_at_ms,
            updated_at_ms: now,
            revision: existing
                .as_ref()
                .map_or(1, |state| state.revision.saturating_add(1)),
        };
        write_schedule(&path, &state)?;
        Ok(state)
    })
}

pub(crate) fn disable_loop_schedule(
    workspace: &Path,
    loop_id: &str,
) -> anyhow::Result<LoopScheduleState> {
    let workspace = canonical_command_path(workspace)
        .with_context(|| format!("could not resolve workspace {}", workspace.display()))?;
    let spec = find_loop(
        workspace
            .to_str()
            .context("scheduled loop workspaces must use UTF-8 paths")?,
        loop_id,
    )
    .map_err(anyhow::Error::msg)?;
    validate_loop_directory(&workspace, &spec.dir, &spec.id)?;
    let path = schedule_path(&spec.dir);
    with_schedule_lock(&path, || {
        let mut state = read_schedule(&path)?;
        validate_schedule_identity(&path, &state, &workspace)?;
        state.enabled = false;
        state.next_run_at_ms = None;
        state.pending_run_at_ms = None;
        state.updated_at_ms = epoch_ms();
        state.revision = state.revision.saturating_add(1);
        write_schedule(&path, &state)?;
        Ok(state)
    })
}

pub(crate) fn queue_loop_schedule_run(
    workspace: &Path,
    loop_id: &str,
) -> anyhow::Result<LoopScheduleState> {
    let workspace = canonical_command_path(workspace)
        .with_context(|| format!("could not resolve workspace {}", workspace.display()))?;
    let spec = find_loop(
        workspace
            .to_str()
            .context("scheduled loop workspaces must use UTF-8 paths")?,
        loop_id,
    )
    .map_err(anyhow::Error::msg)?;
    validate_loop_directory(&workspace, &spec.dir, &spec.id)?;
    let path = schedule_path(&spec.dir);
    with_schedule_lock(&path, || {
        let mut state = read_schedule(&path)?;
        validate_schedule_identity(&path, &state, &workspace)?;
        state.pending_run_at_ms = Some(epoch_ms());
        state.updated_at_ms = epoch_ms();
        state.revision = state.revision.saturating_add(1);
        write_schedule(&path, &state)?;
        Ok(state)
    })
}

pub(crate) fn list_loop_schedules(workspace: &Path) -> anyhow::Result<Vec<LoopScheduleState>> {
    Ok(list_schedule_entries(workspace)?
        .into_iter()
        .map(|entry| entry.state)
        .collect())
}

pub(crate) fn schedule_worker_running(workspace: &Path) -> anyhow::Result<bool> {
    let path = worker_lock_path(workspace);
    ensure_parent(&path)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("could not open scheduler lock {}", path.display()))?;
    match file.try_lock_exclusive() {
        Ok(()) => {
            FileExt::unlock(&file).ok();
            Ok(false)
        }
        Err(error) if lock_is_contended(&error) => Ok(true),
        Err(error) => Err(error)
            .with_context(|| format!("could not inspect scheduler lock {}", path.display())),
    }
}

pub(crate) fn read_schedule_worker_status(
    workspace: &Path,
) -> anyhow::Result<Option<ScheduleWorkerStatus>> {
    if !schedule_worker_running(workspace)? {
        return Ok(None);
    }
    let path = worker_status_path(workspace);
    let bytes = read_bounded_file(&path, MAX_SCHEDULE_FILE_BYTES)?;
    let status: ScheduleWorkerStatus = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid scheduler status {}", path.display()))?;
    if status.schema_version != SCHEDULE_SCHEMA_VERSION {
        bail!("unsupported scheduler status schema");
    }
    if canonical_command_path(&status.workspace)? != canonical_command_path(workspace)? {
        bail!("scheduler status belongs to another workspace");
    }
    Ok(Some(status))
}

pub(crate) fn start_schedule_worker(
    workspace: &Path,
    config_path: Option<&Path>,
) -> anyhow::Result<WorkerStartOutcome> {
    let workspace = canonical_command_path(workspace)
        .with_context(|| format!("could not resolve workspace {}", workspace.display()))?;
    let start_lock_path = scheduler_directory(&workspace).join("start.lock");
    ensure_parent(&start_lock_path)?;
    let start_lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&start_lock_path)?;
    start_lock.lock_exclusive()?;
    if schedule_worker_running(&workspace)? {
        FileExt::unlock(&start_lock).ok();
        return Ok(WorkerStartOutcome::AlreadyRunning);
    }
    let stop = worker_stop_path(&workspace);
    if stop.exists() {
        fs::remove_file(&stop).with_context(|| {
            format!("could not clear scheduler stop request {}", stop.display())
        })?;
    }
    let executable = canonical_command_path(&std::env::current_exe()?)?;
    let mut command = StdCommand::new(executable);
    command.arg("-C").arg(&workspace).arg("--non-interactive");
    if let Some(config_path) = config_path {
        command.arg("--config").arg(config_path);
    }
    command
        .args(["code", "schedule", "worker"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir(&workspace);
    configure_detached(&mut command);
    let mut child = command
        .spawn()
        .context("could not launch the detached schedule worker")?;
    let pid = child.id();
    let deadline = std::time::Instant::now() + WORKER_START_TIMEOUT;
    loop {
        if schedule_worker_running(&workspace)?
            && read_schedule_worker_status(&workspace)
                .ok()
                .flatten()
                .is_some_and(|status| status.pid == pid)
        {
            FileExt::unlock(&start_lock).ok();
            return Ok(WorkerStartOutcome::Started { pid });
        }
        if let Some(status) = child.try_wait()? {
            FileExt::unlock(&start_lock).ok();
            bail!("schedule worker exited before becoming ready ({status})");
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            FileExt::unlock(&start_lock).ok();
            bail!("schedule worker did not become ready within 8 seconds");
        }
        std::thread::sleep(Duration::from_millis(80));
    }
}

pub(crate) fn request_schedule_worker_stop(workspace: &Path) -> anyhow::Result<bool> {
    if !schedule_worker_running(workspace)? {
        return Ok(false);
    }
    let path = worker_stop_path(workspace);
    write_new_or_identical_private_file(&path, b"stop\n")?;
    Ok(true)
}

pub(crate) async fn run_schedule_worker(
    workspace: &Path,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    let workspace = canonical_command_path(workspace)
        .with_context(|| format!("could not resolve workspace {}", workspace.display()))?;
    let lock_path = worker_lock_path(&workspace);
    ensure_parent(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("could not open scheduler lock {}", lock_path.display()))?;
    match lock.try_lock_exclusive() {
        Ok(()) => {}
        Err(error) if lock_is_contended(&error) => return Ok(()),
        Err(error) => return Err(error).context("could not acquire scheduler singleton lock"),
    }
    let _guard = WorkerGuard {
        lock,
        status_path: worker_status_path(&workspace),
        stop_path: worker_stop_path(&workspace),
    };
    let status = ScheduleWorkerStatus {
        schema_version: SCHEDULE_SCHEMA_VERSION,
        pid: std::process::id(),
        workspace: workspace.clone(),
        started_at_ms: epoch_ms(),
    };
    write_json_atomic(&_guard.status_path, &status, MAX_SCHEDULE_FILE_BYTES)?;
    if _guard.stop_path.exists() {
        fs::remove_file(&_guard.stop_path).ok();
    }
    recover_interrupted_runs(&workspace)?;
    reconcile_schedule_notifications(&workspace)?;

    loop {
        if _guard.stop_path.exists() {
            break;
        }
        let entries = list_schedule_entries(&workspace)?;
        let mut keep_alive = false;
        for entry in entries {
            keep_alive |= entry.state.enabled
                || entry.state.pending_run_at_ms.is_some()
                || entry.state.active_run.is_some();
            if let Some(claim) = claim_due_run(&workspace, &entry.path, epoch_ms())? {
                execute_claim(&workspace, config_path, claim).await?;
            }
        }
        if !keep_alive {
            break;
        }
        tokio::time::sleep(WORKER_POLL_INTERVAL).await;
    }
    Ok(())
}

pub(crate) fn list_pending_schedule_notifications(
    workspace: &Path,
    limit: usize,
) -> anyhow::Result<Vec<ScheduleNotification>> {
    let pending = notification_pending_directory(workspace);
    match fs::symlink_metadata(&pending) {
        Ok(_) => validate_directory_chain(&pending)?,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not inspect {}", pending.display()))
        }
    }
    let mut files = fs::read_dir(&pending)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension() == Some(OsStr::new("json")))
        .collect::<Vec<_>>();
    files.sort();
    let mut notifications = Vec::new();
    for path in files.into_iter().take(limit.min(64)) {
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let bytes = read_bounded_file(&path, MAX_NOTIFICATION_FILE_BYTES)?;
        let notification: ScheduleNotification = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid schedule notification {}", path.display()))?;
        validate_schedule_notification(workspace, &notification)?;
        if path.file_name() != Some(OsStr::new(&notification_file_name(&notification)?)) {
            bail!(
                "schedule notification filename does not match its identity: {}",
                path.display()
            );
        }
        notifications.push(notification);
    }
    Ok(notifications)
}

pub(crate) fn acknowledge_schedule_notifications(
    workspace: &Path,
    notifications: &[ScheduleNotification],
) -> anyhow::Result<()> {
    if notifications.is_empty() {
        return Ok(());
    }
    let pending = notification_pending_directory(workspace);
    let delivered = notification_delivered_directory(workspace);
    ensure_directory(&delivered)?;
    for notification in notifications {
        validate_schedule_notification(workspace, notification)?;
        let file_name = notification_file_name(notification)?;
        let source = pending.join(&file_name);
        let target = delivered.join(&file_name);
        match fs::rename(&source, &target) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound && target.is_file() => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let source_bytes = read_bounded_file(&source, MAX_NOTIFICATION_FILE_BYTES)?;
                let target_bytes = read_bounded_file(&target, MAX_NOTIFICATION_FILE_BYTES)?;
                if source_bytes != target_bytes {
                    return Err(error).with_context(|| {
                        format!(
                            "schedule notification identity collided at {}",
                            target.display()
                        )
                    });
                }
                fs::remove_file(&source).with_context(|| {
                    format!(
                        "could not remove duplicate schedule notification {}",
                        source.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "could not acknowledge schedule notification {}",
                        source.display()
                    )
                })
            }
        }
    }
    Ok(())
}

fn validate_schedulable_loop(spec: &LoopSpec) -> anyhow::Result<()> {
    if spec.level != "L1" {
        bail!(
            "only L1 report-only loops can run unattended; `{}` is {}",
            spec.id,
            spec.level
        );
    }
    let audit = audit_loop(spec);
    if !audit.missing.is_empty() {
        bail!(
            "loop `{}` failed its readiness audit: {}",
            spec.id,
            audit.missing.join("; ")
        );
    }
    validate_denylist(&spec.denylist)?;
    Ok(())
}

fn validate_denylist(denylist: &[String]) -> anyhow::Result<()> {
    if denylist.len() > 128 {
        bail!("loop denylist contains more than 128 entries");
    }
    for pattern in denylist {
        let normalized = pattern.replace('\\', "/");
        let wildcard = normalized.find('*');
        let supported = match wildcard {
            None => true,
            Some(index) if normalized.ends_with("/**") => index == normalized.len() - 2,
            Some(index) if normalized.ends_with('*') => index == normalized.len() - 1,
            Some(_) => false,
        };
        if normalized.is_empty()
            || normalized.len() > 512
            || normalized.starts_with('/')
            || normalized.contains(':')
            || normalized.chars().any(char::is_control)
            || normalized.split('/').any(|part| part == "..")
            || !supported
        {
            bail!("loop denylist contains an unsupported pattern: {pattern}");
        }
    }
    Ok(())
}

fn validate_model(model: Option<String>) -> anyhow::Result<Option<String>> {
    model
        .map(|model| {
            let model = model.trim().to_string();
            if model.is_empty()
                || model.len() > 256
                || model.starts_with('-')
                || model.chars().any(char::is_control)
            {
                bail!("scheduled model identity is invalid");
            }
            Ok(model)
        })
        .transpose()
}

fn list_schedule_entries(workspace: &Path) -> anyhow::Result<Vec<ScheduleEntry>> {
    let workspace = canonical_command_path(workspace)
        .with_context(|| format!("could not resolve workspace {}", workspace.display()))?;
    let root = workspace.join(".a3s").join("loops");
    match fs::symlink_metadata(&root) {
        Ok(_) => validate_directory_chain(&root)?,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not inspect {}", root.display()))
        }
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(&root)
        .with_context(|| format!("could not list loop directory {}", root.display()))?
    {
        let entry =
            entry.with_context(|| format!("could not read an entry in {}", root.display()))?;
        let entry_path = entry.path();
        let metadata = fs::symlink_metadata(&entry_path)
            .with_context(|| format!("could not inspect {}", entry_path.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let path = entry_path.join(SCHEDULE_FILE);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                paths.push(path)
            }
            Ok(_) => bail!("loop schedule is not a regular file: {}", path.display()),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("could not inspect {}", path.display()))
            }
        }
    }
    paths.sort();
    if paths.len() > MAX_SCHEDULES {
        bail!("workspace contains more than {MAX_SCHEDULES} loop schedules");
    }
    let mut entries = Vec::with_capacity(paths.len());
    for path in paths {
        let state = with_schedule_lock(&path, || read_schedule(&path))?;
        validate_schedule_identity(&path, &state, &workspace)?;
        entries.push(ScheduleEntry { path, state });
    }
    entries.sort_by(|left, right| left.state.loop_id.cmp(&right.state.loop_id));
    Ok(entries)
}

fn validate_loop_directory(
    workspace: &Path,
    directory: &Path,
    loop_id: &str,
) -> anyhow::Result<()> {
    if !valid_loop_id(loop_id) {
        bail!("loop id is invalid for scheduling");
    }
    let workspace = canonical_command_path(workspace).context("could not resolve workspace")?;
    validate_directory_chain(&workspace)?;
    let root_path = workspace.join(".a3s").join("loops");
    validate_directory_chain(&root_path)?;
    let root =
        canonical_command_path(&root_path).context("could not resolve engineered-loop root")?;
    if root.parent().and_then(Path::parent) != Some(workspace.as_path()) {
        bail!("engineered-loop root escapes this workspace");
    }
    validate_directory_chain(directory)?;
    let directory = canonical_command_path(directory)
        .with_context(|| format!("could not resolve loop directory {}", directory.display()))?;
    if directory.parent() != Some(root.as_path())
        || directory.file_name() != Some(OsStr::new(loop_id))
    {
        bail!("loop directory does not belong directly to this workspace");
    }
    for name in ["loop.toml", "STATE.md", "RUN_LOG.md", "budget.toml"] {
        validate_regular_file(&directory.join(name))?;
    }
    for name in ["skills", "reports"] {
        validate_directory_chain(&directory.join(name))?;
    }
    Ok(())
}

fn validate_schedule_identity(
    path: &Path,
    state: &LoopScheduleState,
    workspace: &Path,
) -> anyhow::Result<()> {
    validate_loop_directory(
        workspace,
        path.parent().context("schedule has no loop directory")?,
        &state.loop_id,
    )?;
    let recorded_workspace = canonical_command_path(&state.workspace).with_context(|| {
        format!(
            "could not resolve recorded schedule workspace {}",
            state.workspace.display()
        )
    })?;
    if recorded_workspace != canonical_command_path(workspace)? {
        bail!("loop schedule belongs to another workspace");
    }
    Ok(())
}

fn recover_interrupted_runs(workspace: &Path) -> anyhow::Result<()> {
    for entry in list_schedule_entries(workspace)? {
        let recovered = with_schedule_lock(&entry.path, || {
            let mut state = read_schedule(&entry.path)?;
            validate_schedule_identity(&entry.path, &state, workspace)?;
            let Some(active) = state.active_run.take() else {
                return Ok(None);
            };
            let finished_at_ms = epoch_ms();
            let result_path = run_directory(&state, &active.run_id).join("result.json");
            let completed = CompletedScheduleRun {
                run_id: active.run_id.clone(),
                outcome: ScheduleRunOutcome::Interrupted,
                started_at_ms: active.started_at_ms,
                finished_at_ms,
                session_id: None,
                result_path,
                summary: "the previous schedule worker stopped before recording a terminal result"
                    .to_string(),
            };
            state.last_run = Some(completed.clone());
            // At-most-once recovery: preserve an explicit interrupted result,
            // but do not silently replay a run whose external effects are
            // unknown. The next recurring cadence or explicit `run` can retry.
            state.pending_run_at_ms = None;
            state.updated_at_ms = finished_at_ms;
            state.revision = state.revision.saturating_add(1);
            write_schedule(&entry.path, &state)?;
            Ok(Some((state, completed)))
        })?;
        if let Some((state, completed)) = recovered {
            write_notification(&state, &completed)?;
        }
    }
    Ok(())
}

fn reconcile_schedule_notifications(workspace: &Path) -> anyhow::Result<()> {
    for entry in list_schedule_entries(workspace)? {
        if let Some(last_run) = &entry.state.last_run {
            write_notification(&entry.state, last_run)?;
        }
    }
    Ok(())
}

fn claim_due_run(workspace: &Path, path: &Path, now: u64) -> anyhow::Result<Option<RunClaim>> {
    with_schedule_lock(path, || {
        let mut state = read_schedule(path)?;
        validate_schedule_identity(path, &state, workspace)?;
        if state.active_run.is_some() {
            return Ok(None);
        }
        let recurring_due = state.enabled
            && state
                .next_run_at_ms
                .is_some_and(|next_run_at_ms| next_run_at_ms <= now);
        let one_shot_due = state
            .pending_run_at_ms
            .is_some_and(|pending_run_at_ms| pending_run_at_ms <= now);
        if !recurring_due && !one_shot_due {
            return Ok(None);
        }
        let run = ActiveScheduleRun {
            run_id: uuid::Uuid::new_v4().to_string(),
            started_at_ms: now,
            worker_pid: std::process::id(),
        };
        state.active_run = Some(run.clone());
        state.pending_run_at_ms = None;
        if state.enabled {
            state.next_run_at_ms = Some(next_after(
                state.next_run_at_ms.unwrap_or(now),
                now,
                state.cadence_seconds,
            ));
        } else {
            state.next_run_at_ms = None;
        }
        state.updated_at_ms = now;
        state.revision = state.revision.saturating_add(1);
        write_schedule(path, &state)?;
        Ok(Some(RunClaim {
            schedule_path: path.to_path_buf(),
            state,
            run,
        }))
    })
}

async fn execute_claim(
    workspace: &Path,
    config_path: Option<&Path>,
    claim: RunClaim,
) -> anyhow::Result<()> {
    let workspace_text = workspace
        .to_str()
        .context("scheduled loop workspaces must use UTF-8 paths")?;
    let spec = find_loop(workspace_text, &claim.state.loop_id).map_err(anyhow::Error::msg)?;
    validate_schedulable_loop(&spec)?;
    append_run_start(&spec, false).map_err(anyhow::Error::msg)?;
    let prompt = loop_run_prompt(&spec, workspace_text, false);
    let execution = match capture_loop_artifacts(&spec) {
        Ok(before) => {
            let mut execution = execute_code_process(workspace, config_path, &claim, &prompt)
                .await
                .unwrap_or_else(failed_execution);
            enforce_loop_completion_contract(&spec, &before, &mut execution);
            execution
        }
        Err(error) => failed_execution(error.context("could not snapshot loop artifacts")),
    };
    let completed = persist_execution_result(&claim, execution)?;
    let state = with_schedule_lock(&claim.schedule_path, || {
        let mut state = read_schedule(&claim.schedule_path)?;
        validate_schedule_identity(&claim.schedule_path, &state, workspace)?;
        if state.active_run.as_ref().map(|run| run.run_id.as_str())
            != Some(claim.run.run_id.as_str())
        {
            bail!("scheduled run identity changed before completion");
        }
        state.active_run = None;
        state.last_run = Some(completed.clone());
        state.updated_at_ms = completed.finished_at_ms;
        state.revision = state.revision.saturating_add(1);
        write_schedule(&claim.schedule_path, &state)?;
        Ok(state)
    })?;
    write_notification(&state, &completed)
}

fn failed_execution(error: anyhow::Error) -> ExecutionResult {
    let message = format!("{error:#}");
    ExecutionResult {
        outcome: ScheduleRunOutcome::Failed,
        stdout: CapturedOutput {
            bytes: Vec::new(),
            truncated: false,
        },
        stderr: CapturedOutput {
            bytes: message.as_bytes().to_vec(),
            truncated: false,
        },
        session_id: None,
        summary: truncate_chars(&message, MAX_NOTIFICATION_SUMMARY_CHARS),
    }
}

fn capture_loop_artifacts(spec: &LoopSpec) -> anyhow::Result<LoopArtifactSnapshot> {
    Ok(LoopArtifactSnapshot {
        state: read_bounded_file(&spec.dir.join("STATE.md"), MAX_SCHEDULE_FILE_BYTES)
            .context("could not read loop STATE.md")?,
        run_log: read_bounded_file(&spec.dir.join("RUN_LOG.md"), MAX_SCHEDULE_FILE_BYTES)
            .context("could not read loop RUN_LOG.md")?,
        reports: read_loop_reports(&spec.dir.join("reports"))?,
    })
}

fn read_loop_reports(directory: &Path) -> anyhow::Result<Vec<(PathBuf, Vec<u8>)>> {
    validate_directory_chain(directory)?;
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("could not list loop reports {}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();
    let mut reports = Vec::new();
    for path in entries {
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || !matches!(
                path.extension().and_then(OsStr::to_str),
                Some(extension)
                    if extension.eq_ignore_ascii_case("md")
                        || extension.eq_ignore_ascii_case("html")
            )
        {
            continue;
        }
        if reports.len() >= MAX_REPORT_ARTIFACTS {
            bail!("loop reports exceeded the bounded artifact count");
        }
        let name = path
            .file_name()
            .map(PathBuf::from)
            .context("loop report path had no filename")?;
        let bytes = read_bounded_file(&path, MAX_REPORT_ARTIFACT_BYTES)
            .with_context(|| format!("could not read loop report {}", path.display()))?;
        reports.push((name, bytes));
    }
    Ok(reports)
}

fn enforce_loop_completion_contract(
    spec: &LoopSpec,
    before: &LoopArtifactSnapshot,
    execution: &mut ExecutionResult,
) {
    if execution.outcome != ScheduleRunOutcome::Succeeded {
        return;
    }
    let failure = match loop_completion_contract_error(spec, before) {
        Ok(Some(failure)) => failure,
        Ok(None) => return,
        Err(error) => format!("could not verify loop completion artifacts: {error:#}"),
    };
    execution.outcome = ScheduleRunOutcome::Failed;
    execution.summary = truncate_chars(
        &format!("scheduled loop completion contract failed: {failure}"),
        MAX_NOTIFICATION_SUMMARY_CHARS,
    );
}

fn loop_completion_contract_error(
    spec: &LoopSpec,
    before: &LoopArtifactSnapshot,
) -> anyhow::Result<Option<String>> {
    let after = capture_loop_artifacts(spec)?;
    let mut missing = Vec::new();
    if after.state == before.state {
        missing.push("STATE.md was not updated");
    }
    let appended_log = after.run_log.len() > before.run_log.len()
        && after.run_log.starts_with(&before.run_log)
        && {
            let suffix = String::from_utf8_lossy(&after.run_log[before.run_log.len()..])
                .to_ascii_lowercase();
            ["finish", "complete", "succeed"]
                .iter()
                .any(|marker| suffix.contains(marker))
        };
    if !appended_log {
        missing.push("RUN_LOG.md has no appended terminal entry");
    }
    if !changed_report_with_extension(before, &after, "md") {
        missing.push("no Markdown report was created or updated");
    }
    if !changed_report_with_extension(before, &after, "html") {
        missing.push("no HTML report was created or updated");
    }
    Ok((!missing.is_empty()).then(|| missing.join("; ")))
}

fn changed_report_with_extension(
    before: &LoopArtifactSnapshot,
    after: &LoopArtifactSnapshot,
    extension: &str,
) -> bool {
    after.reports.iter().any(|(path, bytes)| {
        path.extension()
            .and_then(OsStr::to_str)
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(extension))
            && before
                .reports
                .iter()
                .find(|(before_path, _)| before_path == path)
                .is_none_or(|(_, before_bytes)| before_bytes != bytes)
    })
}

async fn execute_code_process(
    workspace: &Path,
    config_path: Option<&Path>,
    claim: &RunClaim,
    prompt: &str,
) -> anyhow::Result<ExecutionResult> {
    let run_dir = run_directory(&claim.state, &claim.run.run_id);
    ensure_directory(&run_dir)
        .with_context(|| format!("could not create scheduled run {}", run_dir.display()))?;
    let prompt_path = run_dir.join("prompt.txt");
    write_private_file(&prompt_path, prompt.as_bytes())?;
    let executable = canonical_command_path(&std::env::current_exe()?)?;
    let mut command = Command::new(executable);
    command
        .arg("-C")
        .arg(workspace)
        .args(["--output", "json", "--non-interactive"]);
    if let Some(config_path) = config_path {
        command.arg("--config").arg(config_path);
    }
    command
        .args(["code", "exec", "--prompt-file"])
        .arg(&prompt_path)
        .args(["--mode", "auto", "--tool-policy", "scheduled-report"]);
    command.env(SCHEDULE_LOOP_ENV, &claim.state.loop_id);
    if let Some(model) = &claim.state.model {
        command.arg("--model").arg(model);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .current_dir(workspace);
    configure_child_process_group(&mut command);
    let mut child = command
        .spawn()
        .context("could not start scheduled Code run")?;
    let stdout = child
        .stdout
        .take()
        .context("scheduled stdout was unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("scheduled stderr was unavailable")?;
    let stdout_task = tokio::spawn(capture_bounded(stdout, MAX_CAPTURE_BYTES));
    let stderr_task = tokio::spawn(capture_bounded(stderr, MAX_CAPTURE_BYTES));
    let status = match tokio::time::timeout(RUN_TIMEOUT, child.wait()).await {
        Ok(status) => Some(status.context("could not wait for scheduled Code run")?),
        Err(_) => {
            terminate_child_process_group(&mut child).await;
            None
        }
    };
    let stdout = stdout_task
        .await
        .context("scheduled stdout reader failed")??;
    let stderr = stderr_task
        .await
        .context("scheduled stderr reader failed")??;
    let (session_id, model_summary) = parse_code_exec_result(&stdout.bytes);
    let outcome = match status {
        Some(status) if status.success() => ScheduleRunOutcome::Succeeded,
        Some(_) => ScheduleRunOutcome::Failed,
        None => ScheduleRunOutcome::TimedOut,
    };
    let summary = model_summary.unwrap_or_else(|| {
        if outcome == ScheduleRunOutcome::TimedOut {
            "scheduled Code run exceeded the two-hour deadline".to_string()
        } else if !stderr.bytes.is_empty() {
            String::from_utf8_lossy(&stderr.bytes).into_owned()
        } else {
            format!("scheduled Code run {}", outcome.label())
        }
    });
    Ok(ExecutionResult {
        outcome,
        stdout,
        stderr,
        session_id,
        summary: truncate_chars(&summary, MAX_NOTIFICATION_SUMMARY_CHARS),
    })
}

fn persist_execution_result(
    claim: &RunClaim,
    execution: ExecutionResult,
) -> anyhow::Result<CompletedScheduleRun> {
    let run_dir = run_directory(&claim.state, &claim.run.run_id);
    ensure_directory(&run_dir)?;
    let result_path = run_dir.join("result.json");
    let stdout_path = run_dir.join("stdout.json");
    let stderr_path = run_dir.join("stderr.log");
    write_private_file(&stdout_path, &execution.stdout.bytes)?;
    write_private_file(&stderr_path, &execution.stderr.bytes)?;
    let finished_at_ms = epoch_ms();
    let result = serde_json::json!({
        "schemaVersion": 1,
        "runId": claim.run.run_id,
        "outcome": execution.outcome,
        "summary": execution.summary,
        "stdoutTruncated": execution.stdout.truncated,
        "stderrTruncated": execution.stderr.truncated,
        "sessionId": execution.session_id,
        "startedAtMs": claim.run.started_at_ms,
        "finishedAtMs": finished_at_ms,
        "stdoutPath": stdout_path,
        "stderrPath": stderr_path,
    });
    write_json_atomic(&result_path, &result, MAX_SCHEDULE_FILE_BYTES)?;
    Ok(CompletedScheduleRun {
        run_id: claim.run.run_id.clone(),
        outcome: execution.outcome,
        started_at_ms: claim.run.started_at_ms,
        finished_at_ms,
        session_id: execution.session_id,
        result_path,
        summary: execution.summary,
    })
}

fn parse_code_exec_result(stdout: &[u8]) -> (Option<String>, Option<String>) {
    let Ok(value) = serde_json::from_slice::<Value>(stdout) else {
        return (None, None);
    };
    let session_id = value
        .pointer("/data/sessionId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let summary = value
        .pointer("/data/text")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/error/message").and_then(Value::as_str))
        .map(str::to_string);
    (session_id, summary)
}

async fn capture_bounded<R>(mut reader: R, limit: usize) -> std::io::Result<CapturedOutput>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 16 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(bytes.len());
        let retained = read.min(remaining);
        bytes.extend_from_slice(&buffer[..retained]);
        truncated |= retained < read;
    }
    Ok(CapturedOutput { bytes, truncated })
}

fn write_notification(
    state: &LoopScheduleState,
    completed: &CompletedScheduleRun,
) -> anyhow::Result<()> {
    let notification = ScheduleNotification {
        schema_version: NOTIFICATION_SCHEMA_VERSION,
        notification_id: completed.run_id.clone(),
        loop_id: state.loop_id.clone(),
        run_id: completed.run_id.clone(),
        outcome: completed.outcome,
        summary: completed.summary.clone(),
        result_path: completed.result_path.clone(),
        created_at_ms: completed.finished_at_ms,
    };
    validate_schedule_notification(&state.workspace, &notification)?;
    let file_name = notification_file_name(&notification)?;
    let pending = notification_pending_directory(&state.workspace);
    let delivered = notification_delivered_directory(&state.workspace);
    ensure_directory(&pending)?;
    ensure_directory(&delivered)?;
    let path = pending.join(&file_name);
    let delivered_path = delivered.join(&file_name);
    let mut bytes = serde_json::to_vec_pretty(&notification)?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_NOTIFICATION_FILE_BYTES {
        bail!("schedule notification exceeded its storage bound");
    }
    if delivered_path.exists() {
        let existing = read_bounded_file(&delivered_path, MAX_NOTIFICATION_FILE_BYTES)?;
        if existing == bytes {
            return Ok(());
        }
        bail!(
            "delivered schedule notification identity has different content: {}",
            delivered_path.display()
        );
    }
    write_new_or_identical_private_file(&path, &bytes)
}

fn notification_file_name(notification: &ScheduleNotification) -> anyhow::Result<String> {
    if uuid::Uuid::parse_str(&notification.notification_id).is_err() {
        bail!("schedule notification has an invalid identity");
    }
    Ok(format!(
        "{:020}-{}.json",
        notification.created_at_ms, notification.notification_id
    ))
}

fn validate_schedule_notification(
    workspace: &Path,
    notification: &ScheduleNotification,
) -> anyhow::Result<()> {
    if notification.schema_version != NOTIFICATION_SCHEMA_VERSION {
        bail!("unsupported schedule notification schema");
    }
    validate_run_id(&notification.notification_id)?;
    validate_run_id(&notification.run_id)?;
    if notification.notification_id != notification.run_id
        || !valid_loop_id(&notification.loop_id)
        || notification.created_at_ms == 0
        || notification.summary.chars().count() > MAX_NOTIFICATION_SUMMARY_CHARS + 1
    {
        bail!("schedule notification metadata is invalid");
    }
    let workspace = canonical_command_path(workspace)
        .with_context(|| format!("could not resolve workspace {}", workspace.display()))?;
    let expected = workspace
        .join(".a3s")
        .join("loops")
        .join(&notification.loop_id)
        .join("runs")
        .join(&notification.run_id)
        .join("result.json");
    if notification.result_path != expected {
        bail!("schedule notification result path escapes its run directory");
    }
    Ok(())
}

fn with_schedule_lock<T>(
    schedule_path: &Path,
    operation: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let lock_path = schedule_path.with_file_name(SCHEDULE_LOCK_FILE);
    ensure_parent(&lock_path)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("could not open schedule lock {}", lock_path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("could not lock schedule {}", schedule_path.display()))?;
    let result = operation();
    FileExt::unlock(&file).ok();
    result
}

fn read_schedule_if_present(path: &Path) -> anyhow::Result<Option<LoopScheduleState>> {
    match fs::metadata(path) {
        Ok(_) => read_schedule(path).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("could not inspect {}", path.display())),
    }
}

fn read_schedule(path: &Path) -> anyhow::Result<LoopScheduleState> {
    let bytes = read_bounded_file(path, MAX_SCHEDULE_FILE_BYTES)?;
    let state: LoopScheduleState = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid loop schedule {}", path.display()))?;
    validate_schedule_state(&state)?;
    Ok(state)
}

fn write_schedule(path: &Path, state: &LoopScheduleState) -> anyhow::Result<()> {
    validate_schedule_state(state)?;
    write_json_atomic(path, state, MAX_SCHEDULE_FILE_BYTES)
}

fn validate_schedule_state(state: &LoopScheduleState) -> anyhow::Result<()> {
    if state.schema_version != SCHEDULE_SCHEMA_VERSION {
        bail!("unsupported loop schedule schema {}", state.schema_version);
    }
    if !valid_loop_id(&state.loop_id) {
        bail!("loop schedule has an invalid loop id");
    }
    if !(MIN_CADENCE_SECONDS..=MAX_CADENCE_SECONDS).contains(&state.cadence_seconds) {
        bail!("loop schedule cadence is out of bounds");
    }
    if !state.workspace.is_absolute() {
        bail!("loop schedule workspace must be absolute");
    }
    if state.revision == 0 || state.created_at_ms == 0 || state.updated_at_ms < state.created_at_ms
    {
        bail!("loop schedule revision or timestamps are invalid");
    }
    if !state.enabled && state.next_run_at_ms.is_some() {
        bail!("disabled loop schedule cannot retain a recurring deadline");
    }
    if let Some(active) = &state.active_run {
        validate_run_id(&active.run_id)?;
        if active.started_at_ms == 0 || active.worker_pid == 0 {
            bail!("active scheduled run metadata is invalid");
        }
    }
    if let Some(completed) = &state.last_run {
        validate_run_id(&completed.run_id)?;
        if completed.started_at_ms == 0
            || completed.finished_at_ms < completed.started_at_ms
            || completed.summary.chars().count() > MAX_NOTIFICATION_SUMMARY_CHARS + 1
        {
            bail!("completed scheduled run metadata is invalid");
        }
        let expected = run_directory(state, &completed.run_id).join("result.json");
        if completed.result_path != expected {
            bail!("completed scheduled run result path escapes its run directory");
        }
    }
    validate_model(state.model.clone())?;
    Ok(())
}

fn validate_run_id(value: &str) -> anyhow::Result<()> {
    if uuid::Uuid::parse_str(value).is_err() {
        bail!("scheduled run has an invalid identity");
    }
    Ok(())
}

fn valid_loop_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && value != "."
        && value != ".."
}

fn schedule_path(loop_directory: &Path) -> PathBuf {
    loop_directory.join(SCHEDULE_FILE)
}

fn scheduler_directory(workspace: &Path) -> PathBuf {
    workspace
        .join(".a3s")
        .join("loops")
        .join(SCHEDULER_DIRECTORY)
}

fn worker_lock_path(workspace: &Path) -> PathBuf {
    scheduler_directory(workspace).join(WORKER_LOCK_FILE)
}

fn worker_status_path(workspace: &Path) -> PathBuf {
    scheduler_directory(workspace).join(WORKER_STATUS_FILE)
}

fn worker_stop_path(workspace: &Path) -> PathBuf {
    scheduler_directory(workspace).join(WORKER_STOP_FILE)
}

fn notification_pending_directory(workspace: &Path) -> PathBuf {
    workspace
        .join(".a3s")
        .join("notifications")
        .join("schedules")
        .join("v1")
        .join("pending")
}

fn notification_delivered_directory(workspace: &Path) -> PathBuf {
    workspace
        .join(".a3s")
        .join("notifications")
        .join("schedules")
        .join("v1")
        .join("delivered")
}

fn run_directory(state: &LoopScheduleState, run_id: &str) -> PathBuf {
    state
        .workspace
        .join(".a3s")
        .join("loops")
        .join(&state.loop_id)
        .join("runs")
        .join(run_id)
}

fn next_after(scheduled: u64, now: u64, cadence_seconds: u64) -> u64 {
    let step = cadence_seconds.saturating_mul(1_000).max(1);
    if scheduled > now {
        return scheduled;
    }
    let missed = now.saturating_sub(scheduled) / step;
    scheduled.saturating_add(missed.saturating_add(1).saturating_mul(step))
}

fn add_ms(now: u64, cadence_seconds: u64) -> u64 {
    now.saturating_add(cadence_seconds.saturating_mul(1_000))
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn lock_is_contended(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::WouldBlock
        || (cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)))
}

fn truncate_chars(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let mut output = chars.by_ref().take(max).collect::<String>();
    if chars.next().is_some() {
        output.push('…');
    }
    output
}

fn ensure_parent(path: &Path) -> anyhow::Result<()> {
    let parent = path.parent().context("scheduler path has no parent")?;
    ensure_directory(parent)
}

fn ensure_directory(path: &Path) -> anyhow::Result<()> {
    let mut missing = Vec::new();
    let mut cursor = path;
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    bail!("scheduler directory path is unsafe: {}", cursor.display());
                }
                validate_directory_chain(cursor)?;
                break;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                missing.push(cursor.to_path_buf());
                cursor = cursor
                    .parent()
                    .context("scheduler directory has no existing ancestor")?;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not inspect {}", cursor.display()))
            }
        }
    }
    for directory in missing.into_iter().rev() {
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "could not create scheduler directory {}",
                        directory.display()
                    )
                })
            }
        }
        let metadata = fs::symlink_metadata(&directory)
            .with_context(|| format!("could not inspect {}", directory.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!(
                "scheduler directory path is unsafe: {}",
                directory.display()
            );
        }
    }
    Ok(())
}

fn validate_directory_chain(path: &Path) -> anyhow::Result<()> {
    for directory in path.ancestors() {
        if directory.as_os_str().is_empty() {
            continue;
        }
        let metadata = fs::symlink_metadata(directory)
            .with_context(|| format!("could not inspect {}", directory.display()))?;
        if metadata.file_type().is_symlink() {
            if is_trusted_platform_directory_alias(directory) {
                continue;
            }
            bail!(
                "scheduler directory path is unsafe: {}",
                directory.display()
            );
        }
        if !metadata.is_dir() {
            bail!(
                "scheduler directory path is unsafe: {}",
                directory.display()
            );
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn is_trusted_platform_directory_alias(path: &Path) -> bool {
    let expected = match path.to_str() {
        Some("/etc") => Path::new("/private/etc"),
        Some("/tmp") => Path::new("/private/tmp"),
        Some("/var") => Path::new("/private/var"),
        _ => return false,
    };
    path.canonicalize()
        .is_ok_and(|resolved| resolved == expected)
        && fs::metadata(expected).is_ok_and(|metadata| metadata.is_dir())
}

#[cfg(not(target_os = "macos"))]
fn is_trusted_platform_directory_alias(_path: &Path) -> bool {
    false
}

fn validate_regular_file(path: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("could not inspect {}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!(
            "scheduled loop contract is not a regular file: {}",
            path.display()
        );
    }
    Ok(())
}

fn read_bounded_file(path: &Path, max_bytes: u64) -> anyhow::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("could not inspect {}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > max_bytes {
        bail!(
            "scheduler file is not a bounded regular file: {}",
            path.display()
        );
    }
    fs::read(path).with_context(|| format!("could not read {}", path.display()))
}

fn write_json_atomic(path: &Path, value: &impl Serialize, max_bytes: u64) -> anyhow::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    if bytes.len() as u64 > max_bytes {
        bail!("scheduler state exceeded its storage bound");
    }
    ensure_parent(path)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!(
                "refusing to replace unsafe scheduler state {}",
                path.display()
            );
        }
    }
    let parent = path.parent().context("scheduler path has no parent")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    set_private_file(temporary.as_file())?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("could not replace scheduler state {}", path.display()))?;
    sync_directory(parent);
    Ok(())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    ensure_parent(path)?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("could not write {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn write_new_or_identical_private_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    ensure_parent(path)?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let existing = read_bounded_file(path, MAX_NOTIFICATION_FILE_BYTES)?;
            if existing == bytes {
                return Ok(());
            }
            bail!(
                "schedule notification identity has different content: {}",
                path.display()
            );
        }
        Err(error) => {
            return Err(error).with_context(|| format!("could not create {}", path.display()))
        }
    };
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn set_private_file(file: &File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_file(_file: &File) -> std::io::Result<()> {
    Ok(())
}

fn sync_directory(path: &Path) {
    if let Ok(directory) = File::open(path) {
        directory.sync_all().ok();
    }
}

fn canonical_command_path(path: &Path) -> std::io::Result<PathBuf> {
    let path = path.canonicalize()?;
    #[cfg(windows)]
    {
        let value = path.as_os_str().to_string_lossy();
        if value
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
        {
            return Ok(PathBuf::from(format!(r"\\{}", &value[8..])));
        }
        if let Some(value) = value.strip_prefix(r"\\?\") {
            return Ok(PathBuf::from(value));
        }
    }
    Ok(path)
}

fn scheduled_artifact_relative(path: &Path) -> bool {
    let components = path.components().collect::<Vec<_>>();
    if components.len() < 4
        || components[0] != Component::Normal(OsStr::new(".a3s"))
        || components[1] != Component::Normal(OsStr::new("loops"))
        || !matches!(components[2], Component::Normal(_))
    {
        return false;
    }
    match components[3] {
        Component::Normal(value)
            if value == OsStr::new("STATE.md") || value == OsStr::new("RUN_LOG.md") =>
        {
            components.len() == 4
        }
        Component::Normal(value) if value == OsStr::new("reports") => {
            components.len() > 4
                && components[4..]
                    .iter()
                    .all(|component| matches!(component, Component::Normal(_)))
        }
        _ => false,
    }
}

pub(crate) fn is_scheduled_loop_artifact(
    workspace: &Path,
    loop_id: &str,
    args: &serde_json::Value,
) -> bool {
    let Some(value) = args.get("file_path").and_then(Value::as_str) else {
        return false;
    };
    let requested = Path::new(value);
    let relative = if requested.is_absolute() {
        let Ok(relative) = requested.strip_prefix(workspace) else {
            return false;
        };
        relative
    } else {
        requested
    };
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) || !scheduled_artifact_relative(relative)
        || !relative
            .components()
            .nth(2)
            .is_some_and(|component| component == Component::Normal(OsStr::new(loop_id)))
    {
        return false;
    }

    // Require every existing component to be a real path inside the canonical
    // loop directory. New report files may not exist yet, but their nearest
    // existing ancestor must still be the loop's reports tree.
    let Ok(workspace) = canonical_command_path(workspace) else {
        return false;
    };
    let loop_directory = workspace.join(".a3s").join("loops").join(loop_id);
    if validate_loop_directory(&workspace, &loop_directory, loop_id).is_err() {
        return false;
    }
    let target = workspace.join(relative);
    let mut existing = target.as_path();
    loop {
        match fs::symlink_metadata(existing) {
            Ok(metadata) if metadata.file_type().is_symlink() => return false,
            Ok(_) => break,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let Some(parent) = existing.parent() else {
                    return false;
                };
                existing = parent;
            }
            Err(_) => return false,
        }
    }
    let Ok(existing) = canonical_command_path(existing) else {
        return false;
    };
    let direct_state =
        target == loop_directory.join("STATE.md") || target == loop_directory.join("RUN_LOG.md");
    if direct_state {
        return existing == target
            && fs::symlink_metadata(&target)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());
    }
    let reports = loop_directory.join("reports");
    let Ok(reports) = canonical_command_path(&reports) else {
        return false;
    };
    existing == reports || existing.starts_with(&reports)
}

struct WorkerGuard {
    lock: File,
    status_path: PathBuf,
    stop_path: PathBuf,
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        fs::remove_file(&self.status_path).ok();
        fs::remove_file(&self.stop_path).ok();
        FileExt::unlock(&self.lock).ok();
    }
}

#[cfg(unix)]
fn configure_detached(command: &mut StdCommand) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn configure_detached(command: &mut StdCommand) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(not(any(unix, windows)))]
fn configure_detached(_command: &mut StdCommand) {}

#[cfg(unix)]
fn configure_child_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.as_std_mut().process_group(0);
}

#[cfg(not(unix))]
fn configure_child_process_group(_command: &mut Command) {}

async fn terminate_child_process_group(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id().and_then(|pid| libc::pid_t::try_from(pid).ok()) {
        // SAFETY: the child was made leader of a fresh process group above.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    child.kill().await.ok();
    child.wait().await.ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cadence_parser_is_bounded_and_rejects_manual() {
        assert_eq!(parse_cadence_seconds("15m"), Ok(900));
        assert_eq!(parse_cadence_seconds("1d"), Ok(86_400));
        assert!(parse_cadence_seconds("manual").is_err());
        assert!(parse_cadence_seconds("30s").is_err());
        assert!(parse_cadence_seconds("32d").is_err());
        assert!(parse_cadence_seconds("1 month").is_err());
    }

    #[test]
    fn scheduler_recognizes_nonblocking_lock_contention() {
        assert!(lock_is_contended(&std::io::Error::from(
            ErrorKind::WouldBlock
        )));
        #[cfg(windows)]
        assert!(lock_is_contended(&std::io::Error::from_raw_os_error(33)));
    }

    #[test]
    fn next_run_preserves_cadence_without_replaying_missed_intervals() {
        assert_eq!(next_after(1_000, 1_000, 60), 61_000);
        assert_eq!(next_after(1_000, 122_000, 60), 181_000);
        assert_eq!(next_after(200_000, 100_000, 60), 200_000);
    }

    #[test]
    fn scheduled_tool_rounds_scale_loop_iterations_with_hard_bounds() {
        assert_eq!(scheduled_tool_round_budget(0), 12);
        assert_eq!(scheduled_tool_round_budget(1), 12);
        assert_eq!(scheduled_tool_round_budget(2), 16);
        assert_eq!(scheduled_tool_round_budget(3), 24);
        assert_eq!(scheduled_tool_round_budget(4), 32);
        assert_eq!(scheduled_tool_round_budget(usize::MAX), 32);
    }

    #[test]
    fn successful_loop_contract_requires_state_log_and_both_report_formats() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace_text = workspace.path().to_str().unwrap();
        let spec = crate::tui::loop_engineering::init_loop(workspace_text, "daily-triage").unwrap();
        append_run_start(&spec, false).unwrap();
        let before = capture_loop_artifacts(&spec).unwrap();

        let incomplete = loop_completion_contract_error(&spec, &before)
            .unwrap()
            .expect("unchanged artifacts must be incomplete");
        assert!(incomplete.contains("STATE.md"));
        assert!(incomplete.contains("RUN_LOG.md"));
        assert!(incomplete.contains("Markdown"));
        assert!(incomplete.contains("HTML"));

        fs::write(spec.dir.join("STATE.md"), "# State\n\nStatus: complete\n").unwrap();
        OpenOptions::new()
            .append(true)
            .open(spec.dir.join("RUN_LOG.md"))
            .unwrap()
            .write_all(b"- finished: verified\n")
            .unwrap();
        fs::write(spec.dir.join("reports/result.md"), "# Verified\n").unwrap();
        fs::write(
            spec.dir.join("reports/result.html"),
            "<!doctype html><title>Verified</title>\n",
        )
        .unwrap();

        assert_eq!(
            loop_completion_contract_error(&spec, &before).unwrap(),
            None
        );
    }

    #[test]
    fn scheduled_writes_are_limited_to_loop_state_log_and_reports() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace_text = workspace.path().to_str().unwrap();
        let spec = crate::tui::loop_engineering::init_loop(workspace_text, "daily-triage").unwrap();
        for allowed in [
            ".a3s/loops/daily/STATE.md",
            ".a3s/loops/daily/RUN_LOG.md",
            ".a3s/loops/daily/reports/report.md",
        ] {
            let allowed = allowed.replace("/daily/", "/daily-triage/");
            assert!(is_scheduled_loop_artifact(
                workspace.path(),
                "daily-triage",
                &serde_json::json!({"file_path": allowed})
            ));
        }
        assert!(is_scheduled_loop_artifact(
            workspace.path(),
            "daily-triage",
            &serde_json::json!({"file_path": spec.dir.join("reports/absolute.md")})
        ));
        for denied in [
            "src/main.rs",
            ".a3s/loops/daily-triage/loop.toml",
            ".a3s/loops/daily-triage/skills/triage.md",
            ".a3s/loops/daily-triage/reports",
            ".a3s/loops/daily-triage/reports/../../loop.toml",
            ".a3s/loops/other/reports/report.md",
        ] {
            assert!(!is_scheduled_loop_artifact(
                workspace.path(),
                "daily-triage",
                &serde_json::json!({"file_path": denied})
            ));
        }
    }

    #[test]
    fn schedule_lifecycle_is_durable_at_most_once_and_notifies_after_display() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace_text = workspace.path().to_str().unwrap();
        let spec = crate::tui::loop_engineering::init_loop(workspace_text, "daily-triage").unwrap();
        let canonical = canonical_command_path(workspace.path()).unwrap();

        let enabled = enable_loop_schedule(
            workspace.path(),
            "daily-triage",
            Some("1h"),
            Some("deepseek/deepseek-v4-flash".to_string()),
        )
        .unwrap();
        assert!(enabled.enabled);
        assert_eq!(enabled.cadence_seconds, 3_600);
        assert_eq!(enabled.workspace, canonical);
        assert_eq!(
            list_loop_schedules(workspace.path()).unwrap(),
            vec![enabled]
        );

        let queued = queue_loop_schedule_run(workspace.path(), "daily-triage").unwrap();
        let due = queued.pending_run_at_ms.unwrap();
        let schedule = schedule_path(&spec.dir);
        let claim = claim_due_run(&canonical, &schedule, due).unwrap().unwrap();
        assert_eq!(claim.state.loop_id, "daily-triage");
        assert!(claim.state.pending_run_at_ms.is_none());
        assert!(claim.state.active_run.is_some());
        assert!(claim_due_run(&canonical, &schedule, due).unwrap().is_none());

        recover_interrupted_runs(&canonical).unwrap();
        let recovered = read_schedule(&schedule).unwrap();
        assert!(recovered.active_run.is_none());
        assert!(recovered.pending_run_at_ms.is_none());
        assert_eq!(
            recovered.last_run.as_ref().map(|run| run.outcome),
            Some(ScheduleRunOutcome::Interrupted)
        );

        let first = list_pending_schedule_notifications(&canonical, 16).unwrap();
        let second = list_pending_schedule_notifications(&canonical, 16).unwrap();
        assert_eq!(first, second, "peeking must not consume notifications");
        assert_eq!(first.len(), 1);
        acknowledge_schedule_notifications(&canonical, &first).unwrap();
        assert!(list_pending_schedule_notifications(&canonical, 16)
            .unwrap()
            .is_empty());
        reconcile_schedule_notifications(&canonical).unwrap();
        assert!(list_pending_schedule_notifications(&canonical, 16)
            .unwrap()
            .is_empty());

        let disabled = disable_loop_schedule(workspace.path(), "daily-triage").unwrap();
        assert!(!disabled.enabled);
        assert!(disabled.next_run_at_ms.is_none());
        assert!(disabled.pending_run_at_ms.is_none());
    }

    #[test]
    fn schedule_identity_tampering_fails_closed() {
        let workspace = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let workspace_text = workspace.path().to_str().unwrap();
        let spec = crate::tui::loop_engineering::init_loop(workspace_text, "daily-triage").unwrap();
        let mut state =
            enable_loop_schedule(workspace.path(), "daily-triage", Some("1h"), None).unwrap();
        state.workspace = canonical_command_path(other.path()).unwrap();
        state.updated_at_ms = state.updated_at_ms.saturating_add(1);
        state.revision = state.revision.saturating_add(1);
        write_schedule(&schedule_path(&spec.dir), &state).unwrap();

        let error = list_loop_schedules(workspace.path()).unwrap_err();
        assert!(
            error.to_string().contains("another workspace"),
            "unexpected error: {error:#}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_loop_directory_escape_still_fails_closed() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let spec = crate::tui::loop_engineering::init_loop(
            workspace.path().to_str().unwrap(),
            "daily-triage",
        )
        .unwrap();
        let escaped = outside.path().join("daily-triage");
        fs::rename(&spec.dir, &escaped).unwrap();
        symlink(&escaped, &spec.dir).unwrap();

        assert!(validate_loop_directory(workspace.path(), &spec.dir, "daily-triage").is_err());
    }

    #[test]
    fn denylist_validation_accepts_only_bounded_suffix_patterns() {
        validate_denylist(&[
            ".env*".to_string(),
            "secrets/**".to_string(),
            "infra/prod".to_string(),
        ])
        .unwrap();
        for invalid in [
            "../secret",
            "/etc/**",
            "C:/secret/**",
            "secret/*/token",
            "secret**tail",
        ] {
            assert!(
                validate_denylist(&[invalid.to_string()]).is_err(),
                "{invalid}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn scheduled_report_write_rejects_symlinked_report_targets() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let workspace_text = workspace.path().to_str().unwrap();
        let spec = crate::tui::loop_engineering::init_loop(workspace_text, "daily-triage").unwrap();
        symlink(
            outside.path().join("missing.md"),
            spec.dir.join("reports/escape.md"),
        )
        .unwrap();
        assert!(!is_scheduled_loop_artifact(
            workspace.path(),
            "daily-triage",
            &serde_json::json!({"file_path": ".a3s/loops/daily-triage/reports/escape.md"})
        ));
    }

    #[test]
    fn code_exec_result_extracts_session_and_summary() {
        let (session, summary) = parse_code_exec_result(
            br#"{"schemaVersion":1,"ok":true,"data":{"sessionId":"session-1","text":"done"}}"#,
        );
        assert_eq!(session.as_deref(), Some("session-1"));
        assert_eq!(summary.as_deref(), Some("done"));
    }
}
