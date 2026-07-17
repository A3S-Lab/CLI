//! Cross-process coding-agent presence used by the Code TUI and system status
//! surfaces.
//!
//! Cooperating `a3s code` TUI processes publish an exact, short-lived heartbeat
//! in the per-user A3S state directory. Other coding agents are discovered
//! through the same process collector as `a3s top`; those rows are deliberately
//! marked as inferred because a live process does not prove that a task is
//! executing.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

#[cfg(test)]
use crate::top::AgentKind;
use crate::top::{collect_processes, ProcessRow};

const PRESENCE_SCHEMA: &str = "a3s.agent_presence.v1";
const SYSTEM_SNAPSHOT_SCHEMA: &str = "a3s.system_agent_snapshot.v1";
const SYSTEM_SNAPSHOT_FILE: &str = "system-snapshot.json";
const ISLAND_LOCK_FILE: &str = "island.lock";
const PRESENCE_TTL_MS: u64 = 10_000;
const PRESENCE_FUTURE_SKEW_MS: u64 = 5_000;
const MAX_REGISTRY_ENTRIES: usize = 4_096;
const MAX_PRESENCE_FILES: usize = 256;
const MAX_PRESENCE_BYTES: u64 = 64 * 1024;
const MAX_TASK_CHARS: usize = 240;
const MAX_AGENT_CHARS: usize = 64;
const MAX_CHILD_ID_CHARS: usize = 64;
const MAX_WORKSPACE_CHARS: usize = 128;
const MAX_CHILDREN: usize = 64;
const MAX_SYSTEM_ACTIVITIES: usize = 256;
const MAX_SYSTEM_SNAPSHOT_BYTES: u64 = 1024 * 1024;
const MAX_ACTIVITY_ID_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentActivityState {
    Planning,
    Working,
    WaitingApproval,
    WaitingInput,
    Idle,
    Completed,
    Failed,
    Cancelled,
    /// Only a process was observed; no lifecycle event or heartbeat proves
    /// whether the agent is currently executing a task.
    Unknown,
}

impl AgentActivityState {
    pub(crate) fn attention_rank(self) -> u8 {
        match self {
            Self::Failed => 0,
            Self::WaitingApproval | Self::WaitingInput => 1,
            Self::Planning | Self::Working => 2,
            Self::Cancelled => 3,
            Self::Unknown => 4,
            Self::Idle => 5,
            Self::Completed => 6,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentActivityConfidence {
    /// A cooperating agent emitted a fresh lifecycle heartbeat.
    Exact,
    /// Only a matching host process was observed.
    Process,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SystemAgentActivity {
    pub(crate) id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) agent: String,
    pub(crate) workspace: Option<String>,
    pub(crate) task: Option<String>,
    pub(crate) state: AgentActivityState,
    pub(crate) confidence: AgentActivityConfidence,
    pub(crate) started_at_ms: Option<u64>,
    pub(crate) local: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SystemAgentSnapshot {
    pub(crate) activities: Vec<SystemAgentActivity>,
    pub(crate) warnings: Vec<String>,
}

/// Outcome of one heartbeat, whole-system collection, and shared snapshot
/// export. The TUI does not retain the collected rows; the native island is the
/// presentation owner and reads only the sanitized file named by
/// `snapshot_path`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SystemAgentRefreshResult {
    pub(crate) snapshot_path: Option<PathBuf>,
    pub(crate) lock_path: Option<PathBuf>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SystemAgentProtocolSnapshot {
    schema: String,
    updated_at_ms: u64,
    degraded: bool,
    activities: Vec<SystemAgentProtocolActivity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SystemAgentProtocolActivity {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<String>,
    agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<String>,
    state: AgentActivityState,
    confidence: AgentActivityConfidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at_ms: Option<u64>,
}

#[derive(Debug, Default)]
struct PresenceScan {
    presences: Vec<AgentPresence>,
    truncated: bool,
}

impl SystemAgentProtocolSnapshot {
    fn from_collected(snapshot: &SystemAgentSnapshot, updated_at_ms: u64) -> Self {
        let activities = snapshot
            .activities
            .iter()
            .take(MAX_SYSTEM_ACTIVITIES)
            .map(|activity| SystemAgentProtocolActivity {
                id: sanitize_nonempty(&activity.id, MAX_ACTIVITY_ID_CHARS, "agent"),
                parent_id: sanitize_optional(activity.parent_id.as_deref(), MAX_ACTIVITY_ID_CHARS),
                agent: sanitize_nonempty(&activity.agent, MAX_AGENT_CHARS, "agent"),
                workspace: sanitize_optional(activity.workspace.as_deref(), MAX_WORKSPACE_CHARS),
                task: sanitize_optional(activity.task.as_deref(), MAX_TASK_CHARS),
                state: activity.state,
                confidence: activity.confidence,
                started_at_ms: activity.started_at_ms,
            })
            .collect();
        Self {
            schema: SYSTEM_SNAPSHOT_SCHEMA.to_string(),
            updated_at_ms,
            degraded: !snapshot.warnings.is_empty()
                || snapshot.activities.len() > MAX_SYSTEM_ACTIVITIES,
            activities,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentChildPresence {
    pub(crate) id: String,
    pub(crate) agent: String,
    pub(crate) task: Option<String>,
    pub(crate) state: AgentActivityState,
    pub(crate) started_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentPresence {
    schema: String,
    pub(crate) instance_id: String,
    pub(crate) pid: u32,
    pub(crate) workspace: String,
    pub(crate) task: Option<String>,
    pub(crate) state: AgentActivityState,
    pub(crate) children: Vec<AgentChildPresence>,
    pub(crate) started_at_ms: u64,
    pub(crate) updated_at_ms: u64,
}

impl AgentPresence {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        instance_id: impl Into<String>,
        pid: u32,
        workspace: impl Into<String>,
        task: Option<String>,
        state: AgentActivityState,
        children: Vec<AgentChildPresence>,
        started_at_ms: u64,
    ) -> Self {
        let children = children
            .into_iter()
            .map(|mut child| {
                child.id = sanitize_nonempty(&child.id, MAX_CHILD_ID_CHARS, "child");
                child.agent = sanitize_nonempty(&child.agent, MAX_AGENT_CHARS, "agent");
                child
            })
            .collect();
        Self {
            schema: PRESENCE_SCHEMA.to_string(),
            instance_id: instance_id.into(),
            pid,
            workspace: workspace.into(),
            // Task text remains local and unredacted. `protocol_copy` decides
            // whether a sanitized label may cross the process boundary.
            task,
            state,
            children,
            started_at_ms,
            updated_at_ms: epoch_ms(),
        }
    }

    fn protocol_copy(&self, share_tasks: bool) -> Self {
        Self {
            schema: PRESENCE_SCHEMA.to_string(),
            instance_id: self.instance_id.clone(),
            pid: self.pid,
            workspace: workspace_basename(&self.workspace),
            task: share_tasks
                .then(|| sanitize_optional(self.task.as_deref(), MAX_TASK_CHARS))
                .flatten(),
            state: self.state,
            children: self
                .children
                .iter()
                .take(MAX_CHILDREN)
                .map(|child| AgentChildPresence {
                    id: sanitize_nonempty(&child.id, MAX_CHILD_ID_CHARS, "child"),
                    agent: sanitize_nonempty(&child.agent, MAX_AGENT_CHARS, "agent"),
                    task: share_tasks
                        .then(|| sanitize_optional(child.task.as_deref(), MAX_TASK_CHARS))
                        .flatten(),
                    state: child.state,
                    started_at_ms: child.started_at_ms,
                })
                .collect(),
            started_at_ms: self.started_at_ms,
            updated_at_ms: self.updated_at_ms,
        }
    }

    fn sanitize_received(&mut self) {
        self.workspace = workspace_basename(&self.workspace);
        self.task = sanitize_optional(self.task.as_deref(), MAX_TASK_CHARS);
        self.children.truncate(MAX_CHILDREN);
        for child in &mut self.children {
            child.id = sanitize_nonempty(&child.id, MAX_CHILD_ID_CHARS, "child");
            child.agent = sanitize_nonempty(&child.agent, MAX_AGENT_CHARS, "agent");
            child.task = sanitize_optional(child.task.as_deref(), MAX_TASK_CHARS);
        }
    }

    fn valid_at(&self, now_ms: u64) -> bool {
        self.schema == PRESENCE_SCHEMA
            && !self.instance_id.trim().is_empty()
            && self.updated_at_ms <= now_ms.saturating_add(PRESENCE_FUTURE_SKEW_MS)
            && now_ms.saturating_sub(self.updated_at_ms) <= PRESENCE_TTL_MS
    }
}

/// Per-process publisher identity and heartbeat file location.
#[derive(Clone, Debug)]
pub(crate) struct AgentPresencePublisher {
    instance_id: String,
    directory: Option<PathBuf>,
    path: Option<PathBuf>,
    started_at_ms: u64,
    share_tasks: bool,
    closed: Arc<AtomicBool>,
    write_lock: Arc<Mutex<()>>,
}

impl AgentPresencePublisher {
    pub(crate) fn from_environment() -> Self {
        let directory = std::env::var_os("A3S_AGENT_STATUS_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                a3s::components::ComponentPaths::from_env()
                    .ok()
                    .map(|paths| paths.state_root.join("code").join("agent-presence"))
            })
            .and_then(|directory| {
                absolute_status_directory(directory, std::env::current_dir().ok().as_deref())
            });
        let share_tasks = std::env::var_os("A3S_AGENT_STATUS_SHARE_TASKS")
            .is_some_and(|value| value == OsStr::new("1"));
        Self::new(directory, share_tasks)
    }

    #[cfg(test)]
    pub(crate) fn for_directory(directory: PathBuf) -> Self {
        Self::new(Some(directory), false)
    }

    #[cfg(test)]
    fn for_directory_with_task_sharing(directory: PathBuf) -> Self {
        Self::new(Some(directory), true)
    }

    fn new(directory: Option<PathBuf>, share_tasks: bool) -> Self {
        let pid = std::process::id();
        let instance_id = format!("{pid}-{:016x}", rand::random::<u64>());
        let path = directory
            .as_ref()
            .map(|directory| directory.join(format!("{instance_id}.json")));
        Self {
            instance_id,
            directory,
            path,
            started_at_ms: epoch_ms(),
            share_tasks,
            closed: Arc::new(AtomicBool::new(false)),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub(crate) fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }

    /// Publish the local exact state, collect whole-system evidence, and export
    /// the native island's sanitized shared snapshot. File and process failures
    /// are partial: successfully collected rows are still exported with
    /// `degraded: true` whenever the private snapshot file can be replaced.
    pub(crate) async fn publish_collect_and_export(
        &self,
        mut local: AgentPresence,
    ) -> SystemAgentRefreshResult {
        local.updated_at_ms = epoch_ms();
        let publish = self.write_presence(&local);
        let processes = collect_processes();
        let (publish_result, process_result) = tokio::join!(publish, processes);

        let now_ms = epoch_ms();
        let mut warnings = Vec::new();
        if let Err(error) = publish_result {
            warnings.push(format!("heartbeat: {error}"));
        }

        let scan = match self.scan_presences(now_ms).await {
            Ok(scan) => scan,
            Err(error) => {
                warnings.push(format!("presence: {error}"));
                PresenceScan::default()
            }
        };
        if scan.truncated {
            warnings.push(format!(
                "presence: live registry exceeds the {MAX_PRESENCE_FILES}-publisher evidence limit"
            ));
        }
        let mut presences = scan.presences;
        // The current process remains exact even when its heartbeat directory
        // is unavailable or an atomic replace was briefly invisible to this
        // scan. It must cross the same privacy boundary as remote heartbeats:
        // never aggregate the TUI's full path or raw in-memory task directly.
        presences.retain(|presence| presence.instance_id != local.instance_id);
        presences.push(local.protocol_copy(self.share_tasks));

        let processes = match process_result {
            Ok(processes) => processes,
            Err(error) => {
                warnings.push(format!("processes: {error}"));
                Vec::new()
            }
        };

        let snapshot = SystemAgentSnapshot {
            activities: aggregate_activities(&presences, &processes, &self.instance_id, now_ms),
            warnings,
        };
        let mut result = SystemAgentRefreshResult {
            snapshot_path: None,
            lock_path: self
                .directory
                .as_ref()
                .map(|directory| directory.join(ISLAND_LOCK_FILE)),
            warnings: snapshot.warnings.clone(),
        };
        match self.write_system_snapshot(&snapshot).await {
            Ok(path) => result.snapshot_path = Some(path),
            Err(error) => result.warnings.push(format!("snapshot: {error}")),
        }
        result
    }

    pub(crate) async fn remove(&self) {
        self.closed.store(true, Ordering::Release);
        let _write = self.write_lock.lock().await;
        if let Some(path) = &self.path {
            let _ = tokio::fs::remove_file(path).await;
        }
    }

    async fn write_presence(&self, presence: &AgentPresence) -> anyhow::Result<()> {
        let _write = self.write_lock.lock().await;
        if self.closed.load(Ordering::Acquire) {
            anyhow::bail!("publisher is closed");
        }
        let (Some(directory), Some(path)) = (&self.directory, &self.path) else {
            anyhow::bail!("no per-user A3S state directory is available");
        };
        if presence.instance_id != self.instance_id || presence.pid != std::process::id() {
            anyhow::bail!("heartbeat identity does not match its publisher");
        }
        ensure_private_directory(directory)
            .await
            .with_context(|| "secure the agent-presence directory")?;

        let presence = presence.protocol_copy(self.share_tasks);
        let bytes = serde_json::to_vec(&presence)?;
        if bytes.len() as u64 > MAX_PRESENCE_BYTES {
            anyhow::bail!("heartbeat exceeds the protocol size limit");
        }
        let temporary = directory.join(format!(
            ".{}-{:016x}.tmp",
            self.instance_id,
            rand::random::<u64>()
        ));
        let result = async {
            let mut options = tokio::fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options
                .open(&temporary)
                .await
                .with_context(|| "create a private heartbeat temporary file")?;
            file.write_all(&bytes)
                .await
                .with_context(|| "write the heartbeat temporary file")?;
            file.flush()
                .await
                .with_context(|| "flush the heartbeat temporary file")?;
            drop(file);

            if self.closed.load(Ordering::Acquire) {
                anyhow::bail!("publisher closed during heartbeat write");
            }
            replace_presence_file(&temporary, path)
                .await
                .with_context(|| "replace the published heartbeat")
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
        }
        result
    }

    async fn write_system_snapshot(
        &self,
        snapshot: &SystemAgentSnapshot,
    ) -> anyhow::Result<PathBuf> {
        let _write = self.write_lock.lock().await;
        if self.closed.load(Ordering::Acquire) {
            anyhow::bail!("publisher is closed");
        }
        let Some(directory) = &self.directory else {
            anyhow::bail!("no per-user A3S state directory is available");
        };
        ensure_private_directory(directory)
            .await
            .with_context(|| "secure the agent-presence directory")?;

        let snapshot = SystemAgentProtocolSnapshot::from_collected(snapshot, epoch_ms());
        let bytes = serde_json::to_vec(&snapshot)?;
        if bytes.len() as u64 > MAX_SYSTEM_SNAPSHOT_BYTES {
            anyhow::bail!("system-agent snapshot exceeds the protocol size limit");
        }

        let path = directory.join(SYSTEM_SNAPSHOT_FILE);
        let temporary = directory.join(format!(
            ".system-snapshot-{:016x}.tmp",
            rand::random::<u64>()
        ));
        let result = async {
            let mut options = tokio::fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options
                .open(&temporary)
                .await
                .with_context(|| "create a private snapshot temporary file")?;
            file.write_all(&bytes)
                .await
                .with_context(|| "write the snapshot temporary file")?;
            file.flush()
                .await
                .with_context(|| "flush the snapshot temporary file")?;
            drop(file);
            if self.closed.load(Ordering::Acquire) {
                anyhow::bail!("publisher closed during snapshot write");
            }
            replace_presence_file(&temporary, &path)
                .await
                .with_context(|| "replace the shared system-agent snapshot")
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
        }
        result.map(|()| path)
    }

    #[cfg(test)]
    async fn read_presences(&self, now_ms: u64) -> anyhow::Result<Vec<AgentPresence>> {
        Ok(self.scan_presences(now_ms).await?.presences)
    }

    async fn scan_presences(&self, now_ms: u64) -> anyhow::Result<PresenceScan> {
        let Some(directory) = &self.directory else {
            return Ok(PresenceScan::default());
        };
        let mut entries = match tokio::fs::read_dir(directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PresenceScan::default())
            }
            Err(error) => return Err(error.into()),
        };
        let mut presences = Vec::new();
        let mut truncated = false;
        let mut entries_seen = 0usize;
        while let Some(entry) = entries.next_entry().await? {
            if entries_seen == MAX_REGISTRY_ENTRIES {
                anyhow::bail!("agent-presence registry exceeds the directory entry limit");
            }
            entries_seen += 1;

            let path = entry.path();
            let Some(identity) = parse_presence_file_name(&entry.file_name()) else {
                continue;
            };
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let Ok(metadata) = entry.metadata().await else {
                continue;
            };
            if !metadata.is_file() || metadata.len() > MAX_PRESENCE_BYTES {
                continue;
            }
            let Some(mut presence) = read_presence_file(&path).await else {
                continue;
            };
            if presence.instance_id != identity.instance_id
                || presence.pid != identity.pid
                || !presence.valid_at(now_ms)
            {
                continue;
            }
            presence.sanitize_received();
            if presences.len() == MAX_PRESENCE_FILES {
                // Continue validating and counting after the evidence cap so a
                // live publisher flood cannot look complete. The caller keeps
                // the bounded rows and marks the exported snapshot degraded.
                truncated = true;
            } else {
                presences.push(presence);
            }
        }
        Ok(PresenceScan {
            presences,
            truncated,
        })
    }
}

fn absolute_status_directory(directory: PathBuf, current_dir: Option<&Path>) -> Option<PathBuf> {
    if directory.is_absolute() {
        Some(directory)
    } else {
        current_dir.map(|current_dir| current_dir.join(directory))
    }
}

fn aggregate_activities(
    presences: &[AgentPresence],
    processes: &[ProcessRow],
    local_instance_id: &str,
    now_ms: u64,
) -> Vec<SystemAgentActivity> {
    let mut activities = Vec::new();
    let mut exact_pids = HashSet::new();

    for presence in presences
        .iter()
        .filter(|presence| presence.valid_at(now_ms))
    {
        exact_pids.insert(presence.pid);
        let local = presence.instance_id == local_instance_id;
        activities.extend(activities_for_presence(presence, local));
    }

    for process in root_agent_processes(processes) {
        if exact_pids.contains(&process.pid) {
            continue;
        }
        let Some(agent) = process.agent else {
            continue;
        };
        activities.push(SystemAgentActivity {
            id: format!("process:{}", process.pid),
            parent_id: None,
            agent: agent.label().to_string(),
            workspace: process
                .cwd
                .as_deref()
                .map(workspace_basename)
                .filter(|workspace| !workspace.is_empty()),
            // Never expose command arguments: non-interactive agents often
            // carry the full user prompt on their command line.
            task: Some("active process".to_string()),
            state: AgentActivityState::Unknown,
            confidence: AgentActivityConfidence::Process,
            started_at_ms: None,
            local: false,
        });
    }

    sort_activities(&mut activities);
    activities
}

pub(crate) fn activities_for_presence(
    presence: &AgentPresence,
    local: bool,
) -> Vec<SystemAgentActivity> {
    let mut activities = vec![SystemAgentActivity {
        id: presence.instance_id.clone(),
        parent_id: None,
        agent: "a3s-code".to_string(),
        workspace: nonempty(presence.workspace.clone()),
        task: presence.task.clone(),
        state: presence.state,
        confidence: AgentActivityConfidence::Exact,
        started_at_ms: Some(presence.started_at_ms),
        local,
    }];
    activities.extend(presence.children.iter().map(|child| SystemAgentActivity {
        id: format!("{}:{}", presence.instance_id, child.id),
        parent_id: Some(presence.instance_id.clone()),
        agent: child.agent.clone(),
        workspace: nonempty(presence.workspace.clone()),
        task: child.task.clone(),
        state: child.state,
        confidence: AgentActivityConfidence::Exact,
        started_at_ms: child.started_at_ms,
        local,
    }));
    activities
}

pub(crate) fn sort_activities(activities: &mut [SystemAgentActivity]) {
    activities.sort_by(|left, right| {
        left.state
            .attention_rank()
            .cmp(&right.state.attention_rank())
            .then_with(|| right.local.cmp(&left.local))
            .then_with(|| left.parent_id.is_some().cmp(&right.parent_id.is_some()))
            .then_with(|| left.agent.cmp(&right.agent))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn root_agent_processes(processes: &[ProcessRow]) -> Vec<&ProcessRow> {
    let by_pid = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect::<HashMap<_, _>>();
    processes
        .iter()
        .filter(|process| process.agent.is_some())
        .filter(|process| {
            let agent = process.agent;
            let mut parent = process.ppid;
            let mut visited = HashSet::new();
            while parent != 0 && visited.insert(parent) {
                let Some(candidate) = by_pid.get(&parent) else {
                    break;
                };
                if candidate.agent == agent {
                    return false;
                }
                parent = candidate.ppid;
            }
            true
        })
        .collect()
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

#[derive(Debug, PartialEq, Eq)]
struct PresenceFileIdentity {
    instance_id: String,
    pid: u32,
}

fn parse_presence_file_name(file_name: &OsStr) -> Option<PresenceFileIdentity> {
    let file_name = file_name.to_str()?;
    let instance_id = file_name.strip_suffix(".json")?;
    let (pid, nonce) = instance_id.split_once('-')?;
    if pid.is_empty() || nonce.len() != 16 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let parsed_pid = pid.parse::<u32>().ok()?;
    if parsed_pid == 0 || pid != parsed_pid.to_string() {
        return None;
    }
    Some(PresenceFileIdentity {
        instance_id: instance_id.to_string(),
        pid: parsed_pid,
    })
}

async fn read_presence_file(path: &Path) -> Option<AgentPresence> {
    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .open(path)
        .await
        .ok()?;
    let metadata = file.metadata().await.ok()?;
    if !metadata.is_file() || metadata.len() > MAX_PRESENCE_BYTES {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_PRESENCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .ok()?;
    if bytes.len() as u64 > MAX_PRESENCE_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn workspace_basename(workspace: &str) -> String {
    let basename = Path::new(workspace)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("workspace");
    sanitize_nonempty(basename, MAX_WORKSPACE_CHARS, "workspace")
}

fn sanitize_optional(value: Option<&str>, max_chars: usize) -> Option<String> {
    value
        .map(|value| sanitize_display_text(value, max_chars))
        .filter(|value| !value.is_empty())
}

fn sanitize_nonempty(value: &str, max_chars: usize, fallback: &str) -> String {
    let value = sanitize_display_text(value, max_chars);
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

/// Convert untrusted terminal-facing text into a bounded, single-line label.
///
/// This strips complete ANSI/ECMA-48 control strings, C0/C1 controls, and
/// bidirectional formatting controls while preserving ordinary Unicode text.
pub(crate) fn sanitize_display_text(value: &str, max_chars: usize) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Text,
        Escape,
        EscapeIntermediate,
        Csi,
        ControlString,
        ControlStringEscape,
    }

    let mut state = State::Text;
    let mut output = String::new();
    let mut output_chars = 0usize;
    let mut pending_space = false;

    for character in value.chars() {
        match state {
            State::Escape => {
                state = match character {
                    '[' => State::Csi,
                    ']' | 'P' | 'X' | '^' | '_' => State::ControlString,
                    '\u{20}'..='\u{2f}' => State::EscapeIntermediate,
                    _ => State::Text,
                };
                continue;
            }
            State::EscapeIntermediate => {
                if ('\u{30}'..='\u{7e}').contains(&character) {
                    state = State::Text;
                }
                continue;
            }
            State::Csi => {
                if ('\u{40}'..='\u{7e}').contains(&character) {
                    state = State::Text;
                }
                continue;
            }
            State::ControlString => {
                state = match character {
                    '\u{7}' | '\u{9c}' => State::Text,
                    '\u{1b}' => State::ControlStringEscape,
                    _ => State::ControlString,
                };
                continue;
            }
            State::ControlStringEscape => {
                state = if character == '\\' {
                    State::Text
                } else if character == '\u{1b}' {
                    State::ControlStringEscape
                } else {
                    State::ControlString
                };
                continue;
            }
            State::Text => {}
        }

        state = match character {
            '\u{1b}' => State::Escape,
            '\u{9b}' => State::Csi,
            '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => State::ControlString,
            _ => State::Text,
        };
        if !matches!(state, State::Text) {
            continue;
        }

        let bidi_control = matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{206f}'
        );
        if character.is_control() || bidi_control || character.is_whitespace() {
            pending_space |= !output.is_empty();
            continue;
        }
        if output_chars >= max_chars {
            break;
        }
        if pending_space && output_chars + 1 < max_chars {
            output.push(' ');
            output_chars += 1;
        }
        pending_space = false;
        if output_chars < max_chars {
            output.push(character);
            output_chars += 1;
        }
    }
    output
}

pub(crate) fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

#[cfg(unix)]
async fn ensure_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut builder = tokio::fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(path).await?;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await
}

#[cfg(not(unix))]
async fn ensure_private_directory(path: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(path).await
}

#[cfg(not(windows))]
async fn replace_presence_file(temporary: &Path, path: &Path) -> std::io::Result<()> {
    tokio::fs::rename(temporary, path).await
}

#[cfg(windows)]
async fn replace_presence_file(temporary: &Path, path: &Path) -> std::io::Result<()> {
    let temporary = temporary.to_path_buf();
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || replace_file_windows(&temporary, &path))
        .await
        .map_err(|error| std::io::Error::other(format!("atomic replace task failed: {error}")))?
}

#[cfg(windows)]
fn replace_file_windows(temporary: &Path, path: &Path) -> std::io::Result<()> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temporary = temporary
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    // SAFETY: both pointers reference NUL-terminated UTF-16 buffers that stay
    // alive for the call. The files share a private directory/volume, and the
    // replace flags preserve the previous complete file until the move wins.
    let replaced = unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            path.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "system_agents_tests.rs"]
mod tests;
