//! Presentation-neutral coding-agent heartbeat and native island bridge.
//!
//! The TUI owns exact lifecycle projection and periodically publishes it. The
//! OS-level `a3s-webview --agent-island` process owns all island rendering and
//! reads only the private, sanitized shared snapshot.

use super::*;
use crate::system_agents::{
    epoch_ms, sanitize_display_text, AgentActivityState, AgentChildPresence, AgentPresence,
    AgentPresencePublisher, SystemAgentRefreshResult,
};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::oneshot;

pub(super) const AGENT_PRESENCE_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const TERMINAL_STATE_RETENTION: Duration = Duration::from_secs(8);
const AGENT_ISLAND_ENV: &str = "A3S_AGENT_ISLAND";
const AGENT_ISLAND_BIN_ENV: &str = "A3S_AGENT_ISLAND_BIN";
const MAX_AGENT_ISLAND_STDERR_BYTES: u64 = 8 * 1024;
const MAX_AGENT_ISLAND_PROBE_OUTPUT_BYTES: u64 = 8 * 1024;
const AGENT_ISLAND_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const AGENT_ISLAND_SINGLETON_EXIT_MAX: Duration = Duration::from_secs(5);
const AGENT_ISLAND_CONTENTION_RECHECK: Duration = Duration::from_secs(30);
const AGENT_ISLAND_STABLE_RUNTIME: Duration = Duration::from_secs(30);
const AGENT_ISLAND_MAX_CONSECUTIVE_FAILURES: u8 = 4;

#[derive(Debug)]
struct RecentTerminalState {
    session_id: String,
    state: AgentActivityState,
    task: Option<String>,
    started_at_ms: Option<u64>,
    recorded_at: Instant,
}

impl RecentTerminalState {
    fn is_visible_at(&self, session_id: &str, now: Instant) -> bool {
        self.session_id == session_id
            && now.saturating_duration_since(self.recorded_at) <= TERMINAL_STATE_RETENTION
    }
}

#[derive(Debug)]
pub(super) struct AgentPresenceRuntime {
    pub(super) publisher: AgentPresencePublisher,
    pub(super) refreshing: bool,
    terminal: Option<RecentTerminalState>,
    island: AgentIslandSupervisor,
    last_warnings: Vec<String>,
}

impl AgentPresenceRuntime {
    pub(super) fn new() -> Self {
        Self {
            publisher: AgentPresencePublisher::from_environment(),
            refreshing: false,
            terminal: None,
            island: AgentIslandSupervisor::default(),
            last_warnings: Vec::new(),
        }
    }

    fn recent_terminal(&self, session_id: &str) -> Option<&RecentTerminalState> {
        let now = Instant::now();
        self.terminal
            .as_ref()
            .filter(|terminal| terminal.is_visible_at(session_id, now))
    }

    pub(super) fn record_terminal(
        &mut self,
        session_id: String,
        state: AgentActivityState,
        task: Option<String>,
        started_at_ms: Option<u64>,
    ) {
        self.terminal = Some(RecentTerminalState {
            session_id,
            state,
            task,
            started_at_ms,
            recorded_at: Instant::now(),
        });
    }

    fn apply_refresh(
        &mut self,
        result: SystemAgentRefreshResult,
    ) -> Option<AgentIslandLaunchRequest> {
        self.refreshing = false;
        if result.warnings != self.last_warnings {
            for warning in &result.warnings {
                tracing::warn!(%warning, "system-agent presence refresh degraded");
            }
            self.last_warnings = result.warnings;
        }
        let (Some(snapshot_path), Some(lock_path)) = (result.snapshot_path, result.lock_path)
        else {
            return None;
        };
        self.island.observe_snapshot(AgentIslandLaunchRequest {
            snapshot_path,
            lock_path,
        })
    }

    fn poll_island(&mut self, now: Instant) -> Option<AgentIslandLaunchRequest> {
        self.island.poll(now)
    }

    fn apply_island_launch_result(
        &mut self,
        result: Result<AgentIslandLaunchOutcome, String>,
        now: Instant,
    ) {
        self.island.apply_launch_result(result, now);
    }
}

impl App {
    pub(super) fn local_agent_presence(&self) -> AgentPresence {
        let now_ms = epoch_ms();
        let now = Instant::now();
        let children = self
            .runtime
            .subagent_ids()
            .into_iter()
            .zip(self.runtime.subagents())
            .filter_map(|(id, child)| child_presence(id, child, now, now_ms))
            .collect();

        let recent_terminal = self.agent_presence.recent_terminal(&self.session_id);
        let state =
            match self.state {
                State::Awaiting => AgentActivityState::WaitingApproval,
                State::Rebuilding => AgentActivityState::Working,
                State::Streaming
                    if self.plan.tasks().iter().any(|task| {
                        task.status == a3s_code_core::planning::TaskStatus::InProgress
                    }) =>
                {
                    AgentActivityState::Planning
                }
                State::Streaming => AgentActivityState::Working,
                State::Idle => recent_terminal
                    .map(|terminal| terminal.state)
                    .unwrap_or(AgentActivityState::Idle),
            };
        let task = self
            .running_task
            .clone()
            .or_else(|| recent_terminal.and_then(|terminal| terminal.task.clone()));
        let started_at_ms = self
            .stream_started
            .map(|started| {
                now_ms
                    .saturating_sub(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
            })
            .or_else(|| recent_terminal.and_then(|terminal| terminal.started_at_ms))
            .unwrap_or_else(|| self.agent_presence.publisher.started_at_ms());

        AgentPresence::new(
            self.agent_presence.publisher.instance_id(),
            std::process::id(),
            &self.cwd,
            task,
            state,
            children,
            started_at_ms,
        )
    }

    pub(super) fn refresh_agent_presence(&mut self) -> Cmd<Msg> {
        self.agent_presence.refreshing = true;
        let publisher = self.agent_presence.publisher.clone();
        let local = self.local_agent_presence();
        cmd::cmd(move || async move {
            Msg::AgentPresenceRefreshed(publisher.publish_collect_and_export(local).await)
        })
    }

    pub(super) fn apply_agent_presence_refresh(
        &mut self,
        result: SystemAgentRefreshResult,
    ) -> Option<Cmd<Msg>> {
        self.agent_presence
            .apply_refresh(result)
            .map(launch_agent_island)
    }

    pub(super) fn poll_agent_island(&mut self) -> Option<Cmd<Msg>> {
        self.agent_presence
            .poll_island(Instant::now())
            .map(launch_agent_island)
    }

    pub(super) fn apply_agent_island_launch_result(
        &mut self,
        result: Result<AgentIslandLaunchOutcome, String>,
    ) {
        self.agent_presence
            .apply_island_launch_result(result, Instant::now());
    }

    pub(super) fn record_local_agent_terminal(&mut self, state: AgentActivityState) {
        let started_at_ms = self.stream_started.map(|started| {
            epoch_ms()
                .saturating_sub(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
        });
        self.agent_presence.record_terminal(
            self.session_id.clone(),
            state,
            self.running_task.clone(),
            started_at_ms,
        );
    }
}

fn child_presence(
    id: String,
    child: &runtime_projection::SubagentRun,
    now: Instant,
    now_ms: u64,
) -> Option<AgentChildPresence> {
    if child
        .ended
        .is_some_and(|ended| now.saturating_duration_since(ended) > TERMINAL_STATE_RETENTION)
    {
        return None;
    }

    let state = match child.outcome {
        Some(runtime_projection::SubagentOutcome::Succeeded) => AgentActivityState::Completed,
        Some(runtime_projection::SubagentOutcome::Failed) => AgentActivityState::Failed,
        Some(runtime_projection::SubagentOutcome::Cancelled) => AgentActivityState::Cancelled,
        Some(runtime_projection::SubagentOutcome::TrackingLost) => AgentActivityState::Unknown,
        None => AgentActivityState::Working,
    };
    Some(AgentChildPresence {
        id,
        agent: child.display_agent(),
        task: nonempty_presence_text(&child.description),
        state,
        started_at_ms: Some(
            now_ms.saturating_sub(
                now.saturating_duration_since(child.started)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
            ),
        ),
    })
}

pub(super) fn agent_presence_tick() -> Cmd<Msg> {
    cmd::tick(AGENT_PRESENCE_REFRESH_INTERVAL, Msg::AgentPresenceTick)
}

fn nonempty_presence_text(value: &str) -> Option<String> {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!value.is_empty()).then_some(value)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AgentIslandLaunchRequest {
    snapshot_path: PathBuf,
    lock_path: PathBuf,
}

#[derive(Debug)]
pub(super) enum AgentIslandLaunchOutcome {
    Spawned(AgentIslandMonitor),
    Skipped(&'static str),
    Unsupported(String),
}

#[derive(Debug)]
enum AgentIslandLifecycle {
    AwaitingSnapshot,
    Launching,
    Running(AgentIslandMonitor),
    Backoff { retry_at: Instant },
    Stopped,
}

#[derive(Debug)]
struct AgentIslandSupervisor {
    lifecycle: AgentIslandLifecycle,
    request: Option<AgentIslandLaunchRequest>,
    consecutive_failures: u8,
}

impl Default for AgentIslandSupervisor {
    fn default() -> Self {
        Self {
            lifecycle: AgentIslandLifecycle::AwaitingSnapshot,
            request: None,
            consecutive_failures: 0,
        }
    }
}

impl AgentIslandSupervisor {
    fn observe_snapshot(
        &mut self,
        request: AgentIslandLaunchRequest,
    ) -> Option<AgentIslandLaunchRequest> {
        self.request = Some(request);
        if !matches!(self.lifecycle, AgentIslandLifecycle::AwaitingSnapshot) {
            return None;
        }
        self.lifecycle = AgentIslandLifecycle::Launching;
        self.request.clone()
    }

    fn poll(&mut self, now: Instant) -> Option<AgentIslandLaunchRequest> {
        let exit = match &mut self.lifecycle {
            AgentIslandLifecycle::Running(monitor) => monitor.try_take_exit(),
            _ => None,
        };
        if let Some(exit) = exit {
            self.apply_exit(exit, now);
        }

        let retry_due = matches!(
            self.lifecycle,
            AgentIslandLifecycle::Backoff { retry_at } if now >= retry_at
        );
        if retry_due {
            self.lifecycle = AgentIslandLifecycle::Launching;
            return self.request.clone();
        }
        None
    }

    fn apply_launch_result(
        &mut self,
        result: Result<AgentIslandLaunchOutcome, String>,
        now: Instant,
    ) {
        if !matches!(self.lifecycle, AgentIslandLifecycle::Launching) {
            tracing::debug!("ignoring stale native system-agent island launch result");
            return;
        }
        match result {
            Ok(AgentIslandLaunchOutcome::Spawned(monitor)) => {
                tracing::debug!("native system-agent island helper launched");
                self.lifecycle = AgentIslandLifecycle::Running(monitor);
            }
            Ok(AgentIslandLaunchOutcome::Skipped(reason)) => {
                tracing::debug!(reason, "native system-agent island helper skipped");
                self.lifecycle = AgentIslandLifecycle::Stopped;
            }
            Ok(AgentIslandLaunchOutcome::Unsupported(error)) => {
                tracing::warn!(%error, "native system-agent island helper is incompatible");
                self.lifecycle = AgentIslandLifecycle::Stopped;
            }
            Err(error) => {
                tracing::warn!(%error, "native system-agent island helper failed to launch");
                self.schedule_retry(now, Duration::ZERO, &error);
            }
        }
    }

    fn apply_exit(&mut self, exit: AgentIslandExit, now: Instant) {
        if exit.success && exit.ran_for <= AGENT_ISLAND_SINGLETON_EXIT_MAX {
            // Lock contention is a successful, immediate exit: another TUI's
            // helper already owns the per-user island. Recheck infrequently so
            // a surviving TUI can take over after that owner disappears without
            // creating a process loop while it remains healthy.
            tracing::debug!(
                status = %exit.status,
                retry_after_ms = AGENT_ISLAND_CONTENTION_RECHECK.as_millis(),
                "native system-agent island helper observed singleton contention"
            );
            self.consecutive_failures = 0;
            self.lifecycle = AgentIslandLifecycle::Backoff {
                retry_at: now + AGENT_ISLAND_CONTENTION_RECHECK,
            };
            return;
        }

        let reason = if exit.success {
            // A helper that survived startup and later exited cleanly most
            // likely hit its stale-snapshot watchdog. Relaunch it with the same
            // bounded policy used for crashes so a transient exporter outage
            // does not permanently remove the island.
            if exit.ran_for >= AGENT_ISLAND_STABLE_RUNTIME {
                self.consecutive_failures = 0;
            }
            format!("{} after watchdog-length runtime", exit.status)
        } else if exit.detail.is_empty() {
            exit.status
        } else {
            format!("{}: {}", exit.status, exit.detail)
        };
        tracing::warn!(
            runtime_ms = exit.ran_for.as_millis(),
            %reason,
            "native system-agent island helper exited unexpectedly"
        );
        self.schedule_retry(
            now,
            if exit.success {
                Duration::ZERO
            } else {
                exit.ran_for
            },
            &reason,
        );
    }

    fn schedule_retry(&mut self, now: Instant, ran_for: Duration, reason: &str) {
        if ran_for >= AGENT_ISLAND_STABLE_RUNTIME {
            self.consecutive_failures = 0;
        }
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= AGENT_ISLAND_MAX_CONSECUTIVE_FAILURES {
            tracing::warn!(
                failures = self.consecutive_failures,
                %reason,
                "native system-agent island helper retries exhausted"
            );
            self.lifecycle = AgentIslandLifecycle::Stopped;
            return;
        }
        let delay = agent_island_retry_delay(self.consecutive_failures);
        tracing::debug!(
            failures = self.consecutive_failures,
            retry_after_ms = delay.as_millis(),
            "native system-agent island helper restart scheduled"
        );
        self.lifecycle = AgentIslandLifecycle::Backoff {
            retry_at: now + delay,
        };
    }
}

fn agent_island_retry_delay(consecutive_failures: u8) -> Duration {
    match consecutive_failures {
        0 | 1 => AGENT_PRESENCE_REFRESH_INTERVAL,
        2 => AGENT_PRESENCE_REFRESH_INTERVAL * 2,
        _ => AGENT_PRESENCE_REFRESH_INTERVAL * 4,
    }
}

#[derive(Debug)]
struct AgentIslandExit {
    success: bool,
    status: String,
    detail: String,
    ran_for: Duration,
}

#[derive(Debug)]
pub(super) struct AgentIslandMonitor {
    exit: oneshot::Receiver<AgentIslandExit>,
    started_at: Instant,
}

impl AgentIslandMonitor {
    fn try_take_exit(&mut self) -> Option<AgentIslandExit> {
        match self.exit.try_recv() {
            Ok(exit) => Some(exit),
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => Some(AgentIslandExit {
                success: false,
                status: "helper monitor stopped before reporting an exit".to_string(),
                detail: String::new(),
                ran_for: self.started_at.elapsed(),
            }),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AgentIslandEnvironment {
    setting: Option<OsString>,
    binary_override: Option<PathBuf>,
    ssh: bool,
    display_available: bool,
    linux: bool,
}

impl AgentIslandEnvironment {
    fn current() -> Self {
        Self {
            setting: std::env::var_os(AGENT_ISLAND_ENV),
            binary_override: std::env::var_os(AGENT_ISLAND_BIN_ENV)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            ssh: ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
                .into_iter()
                .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty())),
            display_available: ["WAYLAND_DISPLAY", "DISPLAY"]
                .into_iter()
                .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty())),
            linux: cfg!(target_os = "linux"),
        }
    }

    fn explicitly_enabled(&self) -> bool {
        self.setting.as_deref().is_some_and(env_truthy)
    }

    fn disabled(&self) -> bool {
        self.setting.as_deref().is_some_and(env_falsey)
    }

    fn skip_reason(&self) -> Option<&'static str> {
        if self.disabled() {
            return Some("disabled by A3S_AGENT_ISLAND");
        }
        if self.explicitly_enabled() {
            return None;
        }
        if self.ssh {
            return Some("SSH session without explicit island enablement");
        }
        if self.linux && !self.display_available {
            return Some("headless Linux session without a display server");
        }
        None
    }
}

fn env_truthy(value: &OsStr) -> bool {
    value
        .to_str()
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "on"))
}

fn env_falsey(value: &OsStr) -> bool {
    value
        .to_str()
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "off"))
}

fn agent_island_args(request: &AgentIslandLaunchRequest) -> Vec<OsString> {
    vec![
        OsString::from("--agent-island"),
        OsString::from("--snapshot"),
        request.snapshot_path.as_os_str().to_os_string(),
        OsString::from("--lock-file"),
        request.lock_path.as_os_str().to_os_string(),
    ]
}

fn resolve_agent_island_binary(environment: &AgentIslandEnvironment) -> std::io::Result<PathBuf> {
    environment
        .binary_override
        .clone()
        .or_else(remote_ui::webview_helper_path)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "a3s-webview is missing; install it or set A3S_AGENT_ISLAND_BIN",
            )
        })
}

fn launch_agent_island(request: AgentIslandLaunchRequest) -> Cmd<Msg> {
    cmd::cmd(move || async move {
        let result =
            launch_agent_island_with_environment(request, AgentIslandEnvironment::current())
                .await
                .map_err(|error| error.to_string());
        Msg::AgentIslandLaunchFinished(result)
    })
}

async fn launch_agent_island_with_environment(
    request: AgentIslandLaunchRequest,
    environment: AgentIslandEnvironment,
) -> std::io::Result<AgentIslandLaunchOutcome> {
    if let Some(reason) = environment.skip_reason() {
        return Ok(AgentIslandLaunchOutcome::Skipped(reason));
    }
    let binary = resolve_agent_island_binary(&environment)?;
    if !probe_agent_island_capability(&binary).await? {
        return Ok(AgentIslandLaunchOutcome::Unsupported(format!(
            "{} does not support --agent-island; update a3s-webview to a compatible release",
            binary.display()
        )));
    }
    let child = tokio::process::Command::new(&binary)
        .args(agent_island_args(&request))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    Ok(AgentIslandLaunchOutcome::Spawned(
        monitor_agent_island_child(child),
    ))
}

fn monitor_agent_island_child(mut child: tokio::process::Child) -> AgentIslandMonitor {
    let stderr = child.stderr.take();
    let started_at = Instant::now();
    let (exit_tx, exit) = oneshot::channel();
    tokio::spawn(async move {
        let read_stderr = read_bounded(stderr, MAX_AGENT_ISLAND_STDERR_BYTES);
        let (status, stderr) = tokio::join!(child.wait(), read_stderr);
        let ran_for = started_at.elapsed();
        let detail = sanitize_display_text(&String::from_utf8_lossy(&stderr), 512);
        let exit = match status {
            Ok(status) => AgentIslandExit {
                success: status.success(),
                status: status.to_string(),
                detail,
                ran_for,
            },
            Err(error) => AgentIslandExit {
                success: false,
                status: format!("wait failed: {error}"),
                detail,
                ran_for,
            },
        };
        let _ = exit_tx.send(exit);
    });
    AgentIslandMonitor { exit, started_at }
}

async fn probe_agent_island_capability(binary: &Path) -> std::io::Result<bool> {
    let mut command = tokio::process::Command::new(binary);
    command
        .args(["--agent-island", "--help"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout = tokio::spawn(read_bounded(stdout, MAX_AGENT_ISLAND_PROBE_OUTPUT_BYTES));
    let stderr = tokio::spawn(read_bounded(stderr, MAX_AGENT_ISLAND_PROBE_OUTPUT_BYTES));

    match tokio::time::timeout(AGENT_ISLAND_PROBE_TIMEOUT, child.wait()).await {
        Ok(status) => {
            status?;
        }
        Err(_) => {
            // `kill` also waits for the child, so the timed-out probe cannot
            // leave a zombie behind. The pipe tasks finish when it closes.
            let _ = child.kill().await;
            let _ = stdout.await;
            let _ = stderr.await;
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "timed out probing {} for --agent-island support",
                    binary.display()
                ),
            ));
        }
    }
    let stdout = stdout.await.unwrap_or_default();
    let stderr = stderr.await.unwrap_or_default();
    Ok(crate::update::webview_supports_agent_island_output(
        &stdout, &stderr,
    ))
}

async fn read_bounded<R>(reader: Option<R>, limit: u64) -> Vec<u8>
where
    R: AsyncRead + Unpin,
{
    let Some(reader) = reader else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    let _ = reader.take(limit).read_to_end(&mut bytes).await;
    bytes
}

#[cfg(test)]
#[path = "agent_presence_tests.rs"]
mod tests;
