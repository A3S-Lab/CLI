use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use a3s_box_core::{ExecutionManagerError, ExecutionManagerResult};
use a3s_box_runtime::{is_process_alive_with_identity, pid_start_time};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use super::{QualificationServiceKind, MAX_PROCESSES};

const STATE_SCHEMA: &str = "a3s.cli.runtime-qualification-processes.v1";
const MAX_STATE_BYTES: u64 = 64 * 1024;
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in super::super) struct QualificationProcessRecord {
    pub(super) execution_id: String,
    pub(super) execution_generation: u64,
    pub(super) runtime_generation: u64,
    pub(super) unit_id: String,
    pub(super) kind: QualificationServiceKind,
    pub(super) container_port: u16,
    pub(super) host_port: u16,
    pub(super) pid: u32,
    pub(super) pid_start_time: u64,
    pub(super) started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct QualificationProcessState {
    schema: String,
    pub(super) records: BTreeMap<String, QualificationProcessRecord>,
}

impl Default for QualificationProcessState {
    fn default() -> Self {
        Self {
            schema: STATE_SCHEMA.to_string(),
            records: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(in super::super) struct QualificationProcessStore {
    path: PathBuf,
    pub(super) lock: Arc<Mutex<()>>,
}

impl QualificationProcessStore {
    pub(in super::super) fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    pub(super) async fn load_unlocked(&self) -> ExecutionManagerResult<QualificationProcessState> {
        let metadata = match tokio::fs::metadata(&self.path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(QualificationProcessState::default())
            }
            Err(error) => return Err(state_error("inspect", &self.path, error)),
        };
        if metadata.len() > MAX_STATE_BYTES {
            return Err(ExecutionManagerError::Internal(format!(
                "qualification process state exceeds {MAX_STATE_BYTES} bytes"
            )));
        }
        let bytes = tokio::fs::read(&self.path)
            .await
            .map_err(|error| state_error("read", &self.path, error))?;
        let state: QualificationProcessState = serde_json::from_slice(&bytes).map_err(|error| {
            ExecutionManagerError::Internal(format!(
                "qualification process state is invalid JSON: {error}"
            ))
        })?;
        validate_state(&state)?;
        Ok(state)
    }

    pub(super) async fn save_unlocked(
        &self,
        state: &QualificationProcessState,
    ) -> ExecutionManagerResult<()> {
        validate_state(state)?;
        let bytes = serde_json::to_vec(state).map_err(|error| {
            ExecutionManagerError::Internal(format!(
                "could not encode qualification process state: {error}"
            ))
        })?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(ExecutionManagerError::Internal(format!(
                "qualification process state exceeds {MAX_STATE_BYTES} bytes"
            )));
        }
        let parent = self.path.parent().ok_or_else(|| {
            ExecutionManagerError::Internal(
                "qualification process state path has no parent".to_string(),
            )
        })?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| state_error("create parent for", &self.path, error))?;
        let temporary = self.path.with_extension("json.tmp");
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .await
            .map_err(|error| state_error("open temporary", &temporary, error))?;
        file.write_all(&bytes)
            .await
            .map_err(|error| state_error("write temporary", &temporary, error))?;
        file.sync_all()
            .await
            .map_err(|error| state_error("sync temporary", &temporary, error))?;
        drop(file);
        tokio::fs::rename(&temporary, &self.path)
            .await
            .map_err(|error| state_error("replace", &self.path, error))?;
        Ok(())
    }

    pub(in super::super) async fn active_records(
        &self,
    ) -> ExecutionManagerResult<Vec<QualificationProcessRecord>> {
        let _guard = self.lock.lock().await;
        Ok(self.load_unlocked().await?.records.into_values().collect())
    }
}

fn validate_state(state: &QualificationProcessState) -> ExecutionManagerResult<()> {
    if state.schema != STATE_SCHEMA || state.records.len() > MAX_PROCESSES {
        return Err(ExecutionManagerError::Internal(
            "qualification process state has an unsupported schema or cardinality".to_string(),
        ));
    }
    let mut pids = BTreeSet::new();
    let mut ports = BTreeSet::new();
    for (key, record) in &state.records {
        let valid = key == &record.execution_id
            && !record.execution_id.is_empty()
            && !record.unit_id.is_empty()
            && record.unit_id.len() <= 255
            && record.execution_generation > 0
            && record.runtime_generation > 0
            && record.container_port > 0
            && record.host_port > 0
            && record.pid > 1
            && record.pid_start_time > 0
            && pids.insert(record.pid)
            && ports.insert(record.host_port);
        if !valid {
            return Err(ExecutionManagerError::Internal(
                "qualification process state contains invalid or duplicate identity evidence"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

pub(in super::super) struct QualificationProcessCleanup {
    state_path: PathBuf,
}

impl QualificationProcessCleanup {
    pub(in super::super) fn new(store: &QualificationProcessStore) -> Self {
        Self {
            state_path: store.path().to_path_buf(),
        }
    }
}

impl Drop for QualificationProcessCleanup {
    fn drop(&mut self) {
        let Ok(bytes) = std::fs::read(&self.state_path) else {
            return;
        };
        let Ok(state) = serde_json::from_slice::<QualificationProcessState>(&bytes) else {
            return;
        };
        for process in state.records.values() {
            if process.pid <= 1 || process.pid == std::process::id() {
                continue;
            }
            if pid_start_time(process.pid) != Some(process.pid_start_time) {
                continue;
            }
            if let Ok(pid) = i32::try_from(process.pid) {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                    let mut status = 0;
                    libc::waitpid(pid, &mut status, 0);
                }
            }
        }
    }
}

pub(super) async fn terminate_process(pid: u32, start_time: u64) -> ExecutionManagerResult<()> {
    if !process_is_running(pid, start_time) {
        reap_child(pid);
        return Ok(());
    }
    signal_process(pid, libc::SIGTERM)?;
    let deadline = tokio::time::Instant::now() + STOP_TIMEOUT;
    while process_is_running(pid, start_time) {
        if tokio::time::Instant::now() >= deadline {
            signal_process(pid, libc::SIGKILL)?;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let kill_deadline = tokio::time::Instant::now() + STOP_TIMEOUT;
    while process_is_running(pid, start_time) {
        if tokio::time::Instant::now() >= kill_deadline {
            return Err(ExecutionManagerError::Unavailable(format!(
                "qualification process {pid} did not terminate"
            )));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    reap_child(pid);
    Ok(())
}

fn signal_process(pid: u32, signal: i32) -> ExecutionManagerResult<()> {
    let pid = i32::try_from(pid).map_err(|_| {
        ExecutionManagerError::Internal("qualification PID exceeded i32".to_string())
    })?;
    let result = unsafe { libc::kill(pid, signal) };
    if result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(ExecutionManagerError::Unavailable(format!(
            "could not signal qualification process {pid}: {}",
            std::io::Error::last_os_error()
        )))
    }
}

pub(super) fn process_is_running(pid: u32, expected_start_time: u64) -> bool {
    if !is_process_alive_with_identity(pid, Some(expected_start_time)) {
        return false;
    }
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(end) = stat.rfind(')') else {
        return false;
    };
    !matches!(
        stat[end + 1..].trim_start().chars().next(),
        Some('Z' | 'X') | None
    )
}

fn reap_child(pid: u32) {
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };
    let mut status = 0;
    unsafe {
        libc::waitpid(pid, &mut status, libc::WNOHANG);
    }
}

pub(super) fn state_error(
    action: &str,
    path: &Path,
    error: std::io::Error,
) -> ExecutionManagerError {
    ExecutionManagerError::Internal(format!(
        "could not {action} qualification process state {}: {error}",
        path.display()
    ))
}
