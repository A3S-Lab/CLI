use std::collections::BTreeMap;
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use a3s_box_core::{
    ExecutionGeneration, ExecutionId, ExecutionManagerError, ExecutionManagerResult,
    ExecutionPortConnector, ExecutionPortStream, ExecutionState, KillOutcome,
};
use a3s_box_runtime::{
    pid_start_time, BoxRecord, LocalExecutionBackend, LocalExecutionHandle,
    LocalExecutionObservation,
};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::process::Command;

use self::process_state::{
    process_is_running, state_error, terminate_process, QualificationProcessRecord,
};
pub(super) use self::process_state::{QualificationProcessCleanup, QualificationProcessStore};

mod process_state;

pub(super) const CHILD_MARKER_ENV: &str = "A3S_CLI_RUNTIME_QUALIFICATION_CHILD";
pub(super) const CHILD_KIND_ENV: &str = "A3S_CLI_RUNTIME_QUALIFICATION_KIND";
pub(super) const CHILD_PORT_ENV: &str = "A3S_CLI_RUNTIME_QUALIFICATION_PORT";
pub(super) const CHILD_GENERATION_ENV: &str = "A3S_CLI_RUNTIME_QUALIFICATION_GENERATION";
pub(super) const CHILD_UNIT_ID_ENV: &str = "A3S_CLI_RUNTIME_QUALIFICATION_UNIT_ID";
pub(super) const CHILD_TEST_NAME: &str =
    "plugin_manager::gateway_readiness::tests::real_process::child::runtime_service_child";

const MAX_PROCESSES: usize = 16;
const START_TIMEOUT: Duration = Duration::from_secs(5);
const RUNTIME_UNIT_LABEL: &str = "a3s.runtime.unit-id";
const RUNTIME_GENERATION_LABEL: &str = "a3s.runtime.generation";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum QualificationServiceKind {
    Tool,
    Mcp,
}

impl QualificationServiceKind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Mcp => "mcp",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ExpectedService {
    pub(super) kind: QualificationServiceKind,
    pub(super) runtime_generation: u64,
    pub(super) container_port: NonZeroU16,
}

#[derive(Clone)]
pub(super) struct QualificationProcessBackend {
    store: QualificationProcessStore,
    expected: Arc<BTreeMap<String, ExpectedService>>,
    test_binary: PathBuf,
}

impl QualificationProcessBackend {
    pub(super) fn new(
        store: QualificationProcessStore,
        expected: BTreeMap<String, ExpectedService>,
    ) -> ExecutionManagerResult<Self> {
        if expected.is_empty() || expected.len() > MAX_PROCESSES {
            return Err(ExecutionManagerError::InvalidRequest(
                "qualification backend requires a bounded non-empty expected service set"
                    .to_string(),
            ));
        }
        let test_binary = std::env::current_exe().map_err(|error| {
            ExecutionManagerError::Internal(format!(
                "could not locate the Runtime qualification test binary: {error}"
            ))
        })?;
        Ok(Self {
            store,
            expected: Arc::new(expected),
            test_binary,
        })
    }

    pub(super) fn connector(&self) -> QualificationProcessConnector {
        QualificationProcessConnector {
            store: self.store.clone(),
        }
    }

    fn expected_for(
        &self,
        record: &BoxRecord,
    ) -> ExecutionManagerResult<(String, ExpectedService, u64)> {
        let managed = record.managed_execution.as_ref().ok_or_else(|| {
            ExecutionManagerError::InvalidRequest(
                "qualification backend requires managed execution metadata".to_string(),
            )
        })?;
        let unit_id = record
            .labels
            .get(RUNTIME_UNIT_LABEL)
            .cloned()
            .ok_or_else(|| {
                ExecutionManagerError::InvalidRequest(
                    "qualification backend requires the Runtime unit label".to_string(),
                )
            })?;
        let label_generation = record
            .labels
            .get(RUNTIME_GENERATION_LABEL)
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| {
                ExecutionManagerError::InvalidRequest(
                    "qualification backend requires the Runtime generation label".to_string(),
                )
            })?;
        let expected = self.expected.get(&unit_id).cloned().ok_or_else(|| {
            ExecutionManagerError::InvalidRequest(format!(
                "qualification backend rejected unexpected Runtime unit {unit_id:?}"
            ))
        })?;
        if expected.runtime_generation != label_generation {
            return Err(ExecutionManagerError::Conflict {
                execution_id: ExecutionId::new(record.id.clone())?,
                message: "qualification Runtime generation evidence changed".to_string(),
            });
        }
        Ok((unit_id, expected, managed.generation.get()))
    }

    fn handle(record: &BoxRecord, process: &QualificationProcessRecord) -> LocalExecutionHandle {
        LocalExecutionHandle {
            started_at: process.started_at,
            pid: Some(process.pid),
            pid_start_time: Some(process.pid_start_time),
            exec_socket_path: record.box_dir.join("sockets/exec.sock"),
            console_log: record.box_dir.join("logs/console.log"),
            anonymous_volumes: Vec::new(),
            oci_runtime: None,
        }
    }

    async fn spawn_process(
        &self,
        record: &BoxRecord,
        unit_id: &str,
        expected: &ExpectedService,
        execution_generation: u64,
    ) -> ExecutionManagerResult<QualificationProcessRecord> {
        let console_log = record.box_dir.join("logs/console.log");
        tokio::fs::create_dir_all(console_log.parent().ok_or_else(|| {
            ExecutionManagerError::Internal(
                "qualification console log path has no parent".to_string(),
            )
        })?)
        .await
        .map_err(|error| state_error("create", &record.box_dir, error))?;
        tokio::fs::write(&console_log, b"")
            .await
            .map_err(|error| state_error("create", &console_log, error))?;
        let reservation = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| {
                ExecutionManagerError::Unavailable(format!(
                    "could not reserve qualification service port: {error}"
                ))
            })?;
        let host_port = reservation.local_addr().map_err(|error| {
            ExecutionManagerError::Unavailable(format!(
                "could not inspect qualification service port: {error}"
            ))
        })?;
        drop(reservation);

        let mut command = Command::new(&self.test_binary);
        command
            .arg(CHILD_TEST_NAME)
            .arg("--exact")
            .arg("--nocapture")
            .arg("--test-threads=1")
            .env(CHILD_MARKER_ENV, "1")
            .env(CHILD_KIND_ENV, expected.kind.as_str())
            .env(CHILD_PORT_ENV, host_port.port().to_string())
            .env(
                CHILD_GENERATION_ENV,
                expected.runtime_generation.to_string(),
            )
            .env(CHILD_UNIT_ID_ENV, unit_id)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        let mut child = command.spawn().map_err(|error| {
            ExecutionManagerError::Unavailable(format!(
                "could not spawn qualification service process: {error}"
            ))
        })?;
        let pid = child.id().ok_or_else(|| {
            ExecutionManagerError::Unavailable(
                "qualification service process did not expose a PID".to_string(),
            )
        })?;
        let Some(pid_start_time) = pid_start_time(pid) else {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(ExecutionManagerError::Unavailable(
                "qualification service process did not expose a Linux start-time identity"
                    .to_string(),
            ));
        };
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        loop {
            if let Some(status) = child.try_wait().map_err(|error| {
                ExecutionManagerError::Unavailable(format!(
                    "could not inspect qualification service startup: {error}"
                ))
            })? {
                return Err(ExecutionManagerError::Unavailable(format!(
                    "qualification service exited during startup with {status}"
                )));
            }
            if matches!(
                tokio::time::timeout(Duration::from_millis(50), TcpStream::connect(host_port))
                    .await,
                Ok(Ok(_))
            ) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(ExecutionManagerError::Unavailable(
                    "qualification service did not become reachable before its startup deadline"
                        .to_string(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        Ok(QualificationProcessRecord {
            execution_id: record.id.clone(),
            execution_generation,
            runtime_generation: expected.runtime_generation,
            unit_id: unit_id.to_string(),
            kind: expected.kind,
            container_port: expected.container_port.get(),
            host_port: host_port.port(),
            pid,
            pid_start_time,
            started_at: Utc::now(),
        })
    }
}

#[async_trait]
impl LocalExecutionBackend for QualificationProcessBackend {
    async fn preflight(&self, record: &BoxRecord) -> ExecutionManagerResult<()> {
        self.expected_for(record).map(|_| ())
    }

    async fn start(&self, record: &BoxRecord) -> ExecutionManagerResult<LocalExecutionHandle> {
        let (unit_id, expected, execution_generation) = self.expected_for(record)?;
        let _guard = self.store.lock.lock().await;
        let mut state = self.store.load_unlocked().await?;
        if let Some(existing) = state.records.get(&record.id) {
            if existing.unit_id == unit_id
                && existing.execution_generation == execution_generation
                && existing.runtime_generation == expected.runtime_generation
                && process_is_running(existing.pid, existing.pid_start_time)
            {
                return Ok(Self::handle(record, existing));
            }
            state.records.remove(&record.id);
            self.store.save_unlocked(&state).await?;
        }
        if state.records.len() >= MAX_PROCESSES {
            return Err(ExecutionManagerError::Unavailable(
                "qualification process limit reached".to_string(),
            ));
        }
        let process = self
            .spawn_process(record, &unit_id, &expected, execution_generation)
            .await?;
        state.records.insert(record.id.clone(), process.clone());
        if let Err(error) = self.store.save_unlocked(&state).await {
            let _ = terminate_process(process.pid, process.pid_start_time).await;
            return Err(error);
        }
        Ok(Self::handle(record, &process))
    }

    async fn inspect(
        &self,
        record: &BoxRecord,
    ) -> ExecutionManagerResult<LocalExecutionObservation> {
        let (unit_id, expected, execution_generation) = self.expected_for(record)?;
        let _guard = self.store.lock.lock().await;
        let mut state = self.store.load_unlocked().await?;
        let Some(process) = state.records.get(&record.id).cloned() else {
            return Ok(LocalExecutionObservation {
                state: ExecutionState::Stopped,
                handle: None,
                exit_code: None,
            });
        };
        if process.unit_id != unit_id
            || process.execution_generation != execution_generation
            || process.runtime_generation != expected.runtime_generation
        {
            return Err(ExecutionManagerError::Conflict {
                execution_id: ExecutionId::new(record.id.clone())?,
                message: "qualification process identity changed".to_string(),
            });
        }
        if process_is_running(process.pid, process.pid_start_time) {
            return Ok(LocalExecutionObservation {
                state: ExecutionState::Running,
                handle: Some(Self::handle(record, &process)),
                exit_code: None,
            });
        }
        state.records.remove(&record.id);
        self.store.save_unlocked(&state).await?;
        Ok(LocalExecutionObservation {
            state: ExecutionState::Stopped,
            handle: None,
            exit_code: None,
        })
    }

    async fn pause(
        &self,
        _record: &BoxRecord,
        _keep_memory: bool,
    ) -> ExecutionManagerResult<LocalExecutionHandle> {
        Err(ExecutionManagerError::Unavailable(
            "qualification services do not support pause".to_string(),
        ))
    }

    async fn resume(&self, _record: &BoxRecord) -> ExecutionManagerResult<LocalExecutionHandle> {
        Err(ExecutionManagerError::Unavailable(
            "qualification services do not support resume".to_string(),
        ))
    }

    async fn kill(&self, record: &BoxRecord) -> ExecutionManagerResult<KillOutcome> {
        let (unit_id, expected, execution_generation) = self.expected_for(record)?;
        let _guard = self.store.lock.lock().await;
        let mut state = self.store.load_unlocked().await?;
        let Some(process) = state.records.get(&record.id).cloned() else {
            return Ok(KillOutcome::AlreadyStopped);
        };
        if process.unit_id != unit_id
            || process.execution_generation != execution_generation
            || process.runtime_generation != expected.runtime_generation
        {
            return Err(ExecutionManagerError::Conflict {
                execution_id: ExecutionId::new(record.id.clone())?,
                message: "qualification process identity changed before termination".to_string(),
            });
        }
        terminate_process(process.pid, process.pid_start_time).await?;
        state.records.remove(&record.id);
        self.store.save_unlocked(&state).await?;
        Ok(KillOutcome::Killed)
    }
}

#[derive(Clone)]
pub(super) struct QualificationProcessConnector {
    store: QualificationProcessStore,
}

#[async_trait]
impl ExecutionPortConnector for QualificationProcessConnector {
    async fn connect_port(
        &self,
        execution_id: &ExecutionId,
        generation: ExecutionGeneration,
        port: NonZeroU16,
        timeout: Duration,
    ) -> ExecutionManagerResult<ExecutionPortStream> {
        if timeout.is_zero() {
            return Err(ExecutionManagerError::InvalidRequest(
                "qualification port timeout must be non-zero".to_string(),
            ));
        }
        let process = {
            let _guard = self.store.lock.lock().await;
            self.store
                .load_unlocked()
                .await?
                .records
                .get(execution_id.as_str())
                .cloned()
                .ok_or_else(|| ExecutionManagerError::NotFound(execution_id.clone()))?
        };
        if process.execution_generation != generation.get() || process.container_port != port.get()
        {
            return Err(ExecutionManagerError::Conflict {
                execution_id: execution_id.clone(),
                message: "qualification port request did not match the exact generation"
                    .to_string(),
            });
        }
        if !process_is_running(process.pid, process.pid_start_time) {
            return Err(ExecutionManagerError::NotFound(execution_id.clone()));
        }
        let address = (std::net::Ipv4Addr::LOCALHOST, process.host_port);
        let stream = tokio::time::timeout(timeout, TcpStream::connect(address))
            .await
            .map_err(|_| {
                ExecutionManagerError::Unavailable(
                    "qualification port connection timed out".to_string(),
                )
            })?
            .map_err(|error| {
                ExecutionManagerError::Unavailable(format!(
                    "qualification port connection failed: {error}"
                ))
            })?;
        stream.set_nodelay(true).map_err(|error| {
            ExecutionManagerError::Unavailable(format!(
                "qualification port could not enable TCP_NODELAY: {error}"
            ))
        })?;
        let unchanged = {
            let _guard = self.store.lock.lock().await;
            self.store
                .load_unlocked()
                .await?
                .records
                .get(execution_id.as_str())
                == Some(&process)
        };
        if !unchanged || !process_is_running(process.pid, process.pid_start_time) {
            return Err(ExecutionManagerError::Conflict {
                execution_id: execution_id.clone(),
                message: "qualification process generation changed during connection".to_string(),
            });
        }
        Ok(Box::pin(stream))
    }
}
