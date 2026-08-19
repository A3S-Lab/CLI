//! Workspace-scoped durable execution for exact A3S Use Flow generations.
// The library test target compiles this process adapter without CLI/TUI
// consumers so it can run registry contract tests in isolation.
#![cfg_attr(test, allow(dead_code))]

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions as StdOpenOptions};
use std::path::PathBuf;
use std::sync::Arc;

use a3s_flow::{
    FlowEngine, FlowError, HookSnapshot, LocalFileEventStore, NativeTsRuntime,
    NativeTsRuntimeConfig, StepSnapshot, WaitSnapshot, WorkflowRunSnapshot, WorkflowRunStatus,
    WorkflowSpec,
};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

use super::flow::{
    InstalledFlowReference, ParsedFlowDesign, ResolvedUseFlowIdentity, UseFlowCatalog,
};

const RUNTIME_DIRECTORY: &str = ".a3s/flow-runtime";
const EVENT_DIRECTORY: &str = "events";
const BINDING_DIRECTORY: &str = "bindings";
const SOURCE_DIRECTORY: &str = "sources";
const CACHE_DIRECTORY: &str = "native-ts";
const LOCK_FILE: &str = "runtime.lock";
const BINDING_SCHEMA: &str = "a3s.code.installed-flow-run.v1";
const PUBLIC_RUN_SCHEMA_VERSION: u32 = 1;
const MAX_RUN_ID_BYTES: usize = 128;
const MAX_BINDING_BYTES: u64 = 1024 * 1024;
pub(crate) const DEFAULT_RUN_LIST_LIMIT: usize = 100;
pub(crate) const MAX_RUN_LIST_LIMIT: usize = 200;

#[derive(Debug, Error)]
pub(crate) enum InstalledFlowRuntimeError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Execution(String),
    #[error("{0}")]
    State(String),
}

type RuntimeResult<T> = Result<T, InstalledFlowRuntimeError>;

/// Path-free public projection of one durable installed Flow run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledFlowRun {
    pub(crate) schema_version: u32,
    pub(crate) run_id: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) flow: ResolvedUseFlowIdentity,
    pub(crate) status: WorkflowRunStatus,
    pub(crate) input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    pub(crate) last_sequence: u64,
    pub(crate) event_count: usize,
    pub(crate) steps: BTreeMap<String, StepSnapshot>,
    pub(crate) waits: BTreeMap<String, WaitSnapshot>,
    pub(crate) hooks: BTreeMap<String, HookSnapshot>,
}

impl InstalledFlowRun {
    pub(crate) fn status_label(&self) -> &'static str {
        match self.status {
            WorkflowRunStatus::Pending => "pending",
            WorkflowRunStatus::Running => "running",
            WorkflowRunStatus::Suspended => "suspended",
            WorkflowRunStatus::Completed => "completed",
            WorkflowRunStatus::Failed => "failed",
            WorkflowRunStatus::Cancelled => "cancelled",
        }
    }
}

/// Path-free public projection of one event envelope. The `run_created`
/// payload omits the engine's internal staged entrypoint.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledFlowRunEvent {
    pub(crate) run_id: String,
    pub(crate) sequence: u64,
    pub(crate) event_id: String,
    pub(crate) timestamp: DateTime<Utc>,
    pub(crate) key: String,
    pub(crate) event: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredRunBinding {
    schema: String,
    run_id: String,
    created_at: DateTime<Utc>,
    flow: ResolvedUseFlowIdentity,
}

/// One host-owned local runtime. Every CLI and TUI entrypoint creates
/// this adapter from the same workspace and therefore shares one event store.
#[derive(Debug, Clone)]
pub(crate) struct InstalledFlowRuntime {
    workspace: PathBuf,
    root: PathBuf,
    compiler_binary: PathBuf,
}

impl InstalledFlowRuntime {
    pub(crate) fn new(workspace: impl Into<PathBuf>) -> Self {
        let workspace = workspace.into();
        let root = workspace.join(RUNTIME_DIRECTORY);
        Self {
            workspace,
            root,
            compiler_binary: native_ts_compiler(),
        }
    }

    #[cfg(test)]
    fn with_compiler(workspace: impl Into<PathBuf>, compiler_binary: impl Into<PathBuf>) -> Self {
        let workspace = workspace.into();
        let root = workspace.join(RUNTIME_DIRECTORY);
        Self {
            workspace,
            root,
            compiler_binary: compiler_binary.into(),
        }
    }

    /// Re-verify, stage, preflight, and start one exact installed Flow. No run
    /// event is created until source verification and native compilation pass.
    pub(crate) async fn run(
        &self,
        catalog: &UseFlowCatalog,
        design: &ParsedFlowDesign,
        input: Value,
        requested_run_id: Option<String>,
    ) -> RuntimeResult<InstalledFlowRun> {
        if design.installed_flow.is_none() {
            return Err(InstalledFlowRuntimeError::InvalidRequest(
                "workflow design has no installedFlow identity; bind an exact A3S Use Flow before run"
                    .to_string(),
            ));
        }
        if !catalog.is_available() {
            return Err(InstalledFlowRuntimeError::Conflict(
                "A3S Use is not installed or ready".to_string(),
            ));
        }
        let run_id = requested_run_id.unwrap_or_else(generate_run_id);
        validate_run_id(&run_id)?;

        let _lock = self.acquire_lock(LockMode::Exclusive).await?;
        let resolved = catalog
            .resolve_design(design)
            .map_err(|error| InstalledFlowRuntimeError::Conflict(error.to_string()))?;
        let execution = catalog.resolve_execution(design).await.map_err(|_| {
            InstalledFlowRuntimeError::Conflict(
                "installed Flow source verification failed; refresh or reinstall the exact package generation"
                    .to_string(),
            )
        })?;
        debug_assert_eq!(resolved, execution.identity);

        let source_path = self
            .stage_verified_source(&resolved.source_sha256, &execution.source)
            .await?;
        let entrypoint = source_path.strip_prefix(&self.workspace).map_err(|_| {
            InstalledFlowRuntimeError::State(
                "local Flow source staging entry escaped the workspace".to_string(),
            )
        })?;
        let entrypoint = entrypoint.to_str().ok_or_else(|| {
            InstalledFlowRuntimeError::State(
                "local Flow runtime path is not valid Unicode".to_string(),
            )
        })?;
        let spec = WorkflowSpec::native_ts(
            format!("{}:{}", resolved.package_id, resolved.flow_id),
            resolved.version.clone(),
            entrypoint,
            resolved.export_name.clone(),
        );
        let runtime = Arc::new(NativeTsRuntime::new(self.runtime_config()));
        runtime.preflight(&spec).await.map_err(|_| {
            InstalledFlowRuntimeError::Execution(
                "installed Flow native TypeScript preflight failed".to_string(),
            )
        })?;

        let binding = self.ensure_binding(&run_id, resolved).await?;
        let engine = self.engine(runtime);
        engine
            .start_with_id(run_id.clone(), spec, input)
            .await
            .map_err(map_engine_start_error)?;
        let snapshot = engine
            .snapshot(&run_id)
            .await
            .map_err(map_engine_state_error)?;
        let event_count = engine
            .history(&run_id)
            .await
            .map_err(map_engine_state_error)?
            .len();
        Ok(project_run(binding, snapshot, event_count))
    }

    pub(crate) async fn list(&self, limit: Option<usize>) -> RuntimeResult<Vec<InstalledFlowRun>> {
        let limit = limit.unwrap_or(DEFAULT_RUN_LIST_LIMIT);
        if limit == 0 || limit > MAX_RUN_LIST_LIMIT {
            return Err(InstalledFlowRuntimeError::InvalidRequest(format!(
                "Flow run list limit must be between 1 and {MAX_RUN_LIST_LIMIT}"
            )));
        }
        let _lock = self.acquire_lock(LockMode::Shared).await?;
        let mut runs = self.list_locked().await?;
        runs.truncate(limit);
        Ok(runs)
    }

    pub(crate) async fn get(&self, run_id: &str) -> RuntimeResult<InstalledFlowRun> {
        validate_run_id(run_id)?;
        let _lock = self.acquire_lock(LockMode::Shared).await?;
        self.get_locked(run_id).await
    }

    pub(crate) async fn events(&self, run_id: &str) -> RuntimeResult<Vec<InstalledFlowRunEvent>> {
        validate_run_id(run_id)?;
        let _lock = self.acquire_lock(LockMode::Shared).await?;
        let binding = self.read_binding(run_id).await?;
        let engine = self.read_engine();
        let events = engine
            .history(run_id)
            .await
            .map_err(map_engine_state_error)?;
        Ok(events
            .into_iter()
            .map(|envelope| {
                let key = envelope.event.event_key().to_string();
                let mut event = serde_json::to_value(&envelope.event).unwrap_or(Value::Null);
                sanitize_public_event(&mut event, &binding.flow.source_sha256);
                InstalledFlowRunEvent {
                    run_id: envelope.run_id,
                    sequence: envelope.sequence,
                    event_id: envelope.event_id.to_string(),
                    timestamp: envelope.timestamp,
                    key,
                    event,
                }
            })
            .collect())
    }

    /// Find the newest durable run bound to the exact identity persisted in a
    /// design. This intentionally does not consult the live catalog, so run
    /// history remains inspectable after upgrade, disable, or uninstall.
    pub(crate) async fn latest_for_design(
        &self,
        design: &ParsedFlowDesign,
    ) -> RuntimeResult<InstalledFlowRun> {
        let reference = design.installed_flow.as_ref().ok_or_else(|| {
            InstalledFlowRuntimeError::InvalidRequest(
                "workflow design has no installedFlow identity".to_string(),
            )
        })?;
        let _lock = self.acquire_lock(LockMode::Shared).await?;
        self.list_locked()
            .await?
            .into_iter()
            .find(|run| flow_matches_reference(&run.flow, reference))
            .ok_or_else(|| {
                InstalledFlowRuntimeError::NotFound(format!(
                    "no durable local run was found for installedFlow '{}:{}' generation {}",
                    reference.package_id, reference.flow_id, reference.lifecycle_generation
                ))
            })
    }

    async fn list_locked(&self) -> RuntimeResult<Vec<InstalledFlowRun>> {
        let engine = self.read_engine();
        let run_ids = engine
            .list_run_ids()
            .await
            .map_err(map_engine_state_error)?;
        let mut runs = Vec::with_capacity(run_ids.len());
        for run_id in run_ids {
            runs.push(self.get_locked(&run_id).await?);
        }
        runs.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.run_id.cmp(&left.run_id))
        });
        Ok(runs)
    }

    async fn get_locked(&self, run_id: &str) -> RuntimeResult<InstalledFlowRun> {
        let binding = self.read_binding(run_id).await?;
        let engine = self.read_engine();
        let snapshot = engine.snapshot(run_id).await.map_err(|error| match error {
            FlowError::RunNotFound(_) => InstalledFlowRuntimeError::NotFound(format!(
                "durable local Flow run '{run_id}' was not found"
            )),
            other => map_engine_state_error(other),
        })?;
        let event_count = engine
            .history(run_id)
            .await
            .map_err(map_engine_state_error)?
            .len();
        Ok(project_run(binding, snapshot, event_count))
    }

    fn runtime_config(&self) -> NativeTsRuntimeConfig {
        NativeTsRuntimeConfig::new(
            self.compiler_binary.clone(),
            self.root.join(CACHE_DIRECTORY),
            self.workspace.clone(),
        )
    }

    fn engine(&self, runtime: Arc<NativeTsRuntime>) -> FlowEngine {
        FlowEngine::new(
            Arc::new(LocalFileEventStore::new(self.root.join(EVENT_DIRECTORY))),
            runtime,
        )
    }

    fn read_engine(&self) -> FlowEngine {
        self.engine(Arc::new(NativeTsRuntime::new(self.runtime_config())))
    }

    async fn stage_verified_source(
        &self,
        source_sha256: &str,
        source: &[u8],
    ) -> RuntimeResult<PathBuf> {
        if format!("{:x}", Sha256::digest(source)) != source_sha256 {
            return Err(InstalledFlowRuntimeError::Conflict(
                "installed Flow source changed during execution resolution".to_string(),
            ));
        }
        let directory = self.root.join(SOURCE_DIRECTORY);
        tokio::fs::create_dir_all(&directory).await.map_err(|_| {
            InstalledFlowRuntimeError::State(
                "local Flow source staging directory could not be created".to_string(),
            )
        })?;
        let path = directory.join(format!("{source_sha256}.ts"));
        match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(InstalledFlowRuntimeError::State(
                        "local Flow source staging entry is not a regular file".to_string(),
                    ));
                }
                let staged = tokio::fs::read(&path).await.map_err(|_| {
                    InstalledFlowRuntimeError::State(
                        "local Flow source staging entry could not be read".to_string(),
                    )
                })?;
                if format!("{:x}", Sha256::digest(&staged)) != source_sha256 {
                    return Err(InstalledFlowRuntimeError::State(
                        "local Flow source staging digest is invalid".to_string(),
                    ));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(InstalledFlowRuntimeError::State(
                    "local Flow source staging entry could not be inspected".to_string(),
                ))
            }
        }

        let temporary = directory.join(format!(".{source_sha256}.{}.tmp", random_hex(8)));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .await
            .map_err(|_| {
                InstalledFlowRuntimeError::State(
                    "local Flow source staging entry could not be created".to_string(),
                )
            })?;
        file.write_all(source).await.map_err(|_| {
            InstalledFlowRuntimeError::State(
                "local Flow source staging entry could not be written".to_string(),
            )
        })?;
        file.sync_all().await.map_err(|_| {
            InstalledFlowRuntimeError::State(
                "local Flow source staging entry could not be committed".to_string(),
            )
        })?;
        drop(file);
        tokio::fs::rename(&temporary, &path).await.map_err(|_| {
            InstalledFlowRuntimeError::State(
                "local Flow source staging entry could not be installed".to_string(),
            )
        })?;
        Ok(path)
    }

    async fn ensure_binding(
        &self,
        run_id: &str,
        flow: ResolvedUseFlowIdentity,
    ) -> RuntimeResult<StoredRunBinding> {
        match self.read_binding(run_id).await {
            Ok(existing) => {
                if !same_flow_generation(&existing.flow, &flow) {
                    return Err(InstalledFlowRuntimeError::Conflict(format!(
                        "durable local Flow run '{run_id}' is bound to a different installed Flow generation"
                    )));
                }
                return Ok(existing);
            }
            Err(InstalledFlowRuntimeError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }

        let binding = StoredRunBinding {
            schema: BINDING_SCHEMA.to_string(),
            run_id: run_id.to_string(),
            created_at: Utc::now(),
            flow,
        };
        let directory = self.root.join(BINDING_DIRECTORY);
        tokio::fs::create_dir_all(&directory).await.map_err(|_| {
            InstalledFlowRuntimeError::State(
                "local Flow run binding directory could not be created".to_string(),
            )
        })?;
        let bytes = serde_json::to_vec_pretty(&binding).map_err(|_| {
            InstalledFlowRuntimeError::State(
                "local Flow run binding could not be encoded".to_string(),
            )
        })?;
        let temporary = directory.join(format!(".{run_id}.{}.tmp", random_hex(8)));
        let final_path = self.binding_path(run_id);
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .await
            .map_err(|_| {
                InstalledFlowRuntimeError::State(
                    "local Flow run binding could not be created".to_string(),
                )
            })?;
        file.write_all(&bytes).await.map_err(|_| {
            InstalledFlowRuntimeError::State(
                "local Flow run binding could not be written".to_string(),
            )
        })?;
        file.sync_all().await.map_err(|_| {
            InstalledFlowRuntimeError::State(
                "local Flow run binding could not be committed".to_string(),
            )
        })?;
        drop(file);
        tokio::fs::rename(&temporary, &final_path)
            .await
            .map_err(|_| {
                InstalledFlowRuntimeError::State(
                    "local Flow run binding could not be installed".to_string(),
                )
            })?;
        Ok(binding)
    }

    async fn read_binding(&self, run_id: &str) -> RuntimeResult<StoredRunBinding> {
        let path = self.binding_path(run_id);
        let metadata = tokio::fs::symlink_metadata(&path).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                InstalledFlowRuntimeError::NotFound(format!(
                    "durable local Flow run '{run_id}' was not found"
                ))
            } else {
                InstalledFlowRuntimeError::State(
                    "local Flow run binding could not be inspected".to_string(),
                )
            }
        })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > MAX_BINDING_BYTES
        {
            return Err(InstalledFlowRuntimeError::State(
                "local Flow run binding is not a bounded regular file".to_string(),
            ));
        }
        let bytes = tokio::fs::read(&path).await.map_err(|_| {
            InstalledFlowRuntimeError::State("local Flow run binding could not be read".to_string())
        })?;
        let binding: StoredRunBinding = serde_json::from_slice(&bytes).map_err(|_| {
            InstalledFlowRuntimeError::State("local Flow run binding is invalid".to_string())
        })?;
        validate_binding(&binding, run_id)?;
        Ok(binding)
    }

    fn binding_path(&self, run_id: &str) -> PathBuf {
        self.root
            .join(BINDING_DIRECTORY)
            .join(format!("{run_id}.json"))
    }

    async fn acquire_lock(&self, mode: LockMode) -> RuntimeResult<WorkspaceRuntimeLock> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&root).map_err(|_| {
                InstalledFlowRuntimeError::State(
                    "local Flow runtime directory could not be created".to_string(),
                )
            })?;
            let lock_path = root.join(LOCK_FILE);
            if let Ok(metadata) = std::fs::symlink_metadata(&lock_path) {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(InstalledFlowRuntimeError::State(
                        "local Flow runtime lock is not a regular file".to_string(),
                    ));
                }
            }
            let file = StdOpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(lock_path)
                .map_err(|_| {
                    InstalledFlowRuntimeError::State(
                        "local Flow runtime lock could not be opened".to_string(),
                    )
                })?;
            match mode {
                LockMode::Shared => FileExt::lock_shared(&file),
                LockMode::Exclusive => FileExt::lock_exclusive(&file),
            }
            .map_err(|_| {
                InstalledFlowRuntimeError::State(
                    "local Flow runtime lock could not be acquired".to_string(),
                )
            })?;
            Ok(WorkspaceRuntimeLock { file })
        })
        .await
        .map_err(|_| {
            InstalledFlowRuntimeError::State("local Flow runtime lock task failed".to_string())
        })?
    }
}

#[derive(Clone, Copy)]
enum LockMode {
    Shared,
    Exclusive,
}

struct WorkspaceRuntimeLock {
    file: File,
}

impl Drop for WorkspaceRuntimeLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn project_run(
    binding: StoredRunBinding,
    snapshot: WorkflowRunSnapshot,
    event_count: usize,
) -> InstalledFlowRun {
    InstalledFlowRun {
        schema_version: PUBLIC_RUN_SCHEMA_VERSION,
        run_id: snapshot.run_id,
        created_at: binding.created_at,
        flow: binding.flow,
        status: snapshot.status,
        input: snapshot.input,
        output: snapshot.output,
        error: snapshot.error,
        last_sequence: snapshot.last_sequence,
        event_count,
        steps: snapshot.steps,
        waits: snapshot.waits,
        hooks: snapshot.hooks,
    }
}

fn sanitize_public_event(event: &mut Value, source_sha256: &str) {
    let Some(runtime) = event.pointer_mut("/spec/runtime") else {
        return;
    };
    let Some(runtime) = runtime.as_object_mut() else {
        return;
    };
    runtime.remove("entrypoint");
    runtime.insert(
        "sourceSha256".to_string(),
        Value::String(source_sha256.to_string()),
    );
}

fn validate_binding(binding: &StoredRunBinding, expected_run_id: &str) -> RuntimeResult<()> {
    if binding.schema != BINDING_SCHEMA || binding.run_id != expected_run_id {
        return Err(InstalledFlowRuntimeError::State(
            "local Flow run binding identity is invalid".to_string(),
        ));
    }
    validate_run_id(&binding.run_id)?;
    let canonical_version = semver::Version::parse(&binding.flow.version)
        .ok()
        .is_some_and(|version| version.to_string() == binding.flow.version);
    if binding.flow.schema != "a3s.use.resolved-flow.v1"
        || binding.flow.catalog_generation == 0
        || binding.flow.lifecycle_generation == 0
        || !is_lower_sha256(&binding.flow.catalog_revision)
        || !is_lower_sha256(&binding.flow.source_sha256)
        || !canonical_version
    {
        return Err(InstalledFlowRuntimeError::State(
            "local Flow run binding package identity is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_run_id(run_id: &str) -> RuntimeResult<()> {
    if run_id.is_empty()
        || run_id.len() > MAX_RUN_ID_BYTES
        || !run_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err(InstalledFlowRuntimeError::InvalidRequest(
            "Flow run ID must contain 1-128 ASCII letters, digits, '-' or '_'".to_string(),
        ));
    }
    Ok(())
}

fn same_flow_generation(left: &ResolvedUseFlowIdentity, right: &ResolvedUseFlowIdentity) -> bool {
    left.key == right.key
        && left.package_id == right.package_id
        && left.route == right.route
        && left.flow_id == right.flow_id
        && left.version == right.version
        && left.lifecycle_generation == right.lifecycle_generation
        && left.engine == right.engine
        && left.runtime == right.runtime
        && left.export_name == right.export_name
        && left.source_sha256 == right.source_sha256
}

fn flow_matches_reference(
    flow: &ResolvedUseFlowIdentity,
    reference: &InstalledFlowReference,
) -> bool {
    flow.package_id == reference.package_id
        && flow.flow_id == reference.flow_id
        && flow.version == reference.version
        && flow.lifecycle_generation == reference.lifecycle_generation
        && flow.source_sha256 == reference.source_sha256
}

fn map_engine_start_error(error: FlowError) -> InstalledFlowRuntimeError {
    match error {
        FlowError::InvalidRunId(_) => {
            InstalledFlowRuntimeError::InvalidRequest("Flow run ID is invalid".to_string())
        }
        FlowError::RunConflict { run_id, reason } => InstalledFlowRuntimeError::Conflict(format!(
            "durable local Flow run '{run_id}' conflicts with the existing run: {reason}"
        )),
        FlowError::Runtime(_) => InstalledFlowRuntimeError::Execution(
            "installed Flow native TypeScript execution failed".to_string(),
        ),
        other => map_engine_state_error(other),
    }
}

fn map_engine_state_error(error: FlowError) -> InstalledFlowRuntimeError {
    match error {
        FlowError::RunNotFound(run_id) => InstalledFlowRuntimeError::NotFound(format!(
            "durable local Flow run '{run_id}' was not found"
        )),
        _ => InstalledFlowRuntimeError::State(
            "durable local Flow event history is unavailable or invalid".to_string(),
        ),
    }
}

fn native_ts_compiler() -> PathBuf {
    std::env::var_os("A3S_FLOW_NATIVE_TS_COMPILER")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("a3s-flow-native-compiler"))
}

fn generate_run_id() -> String {
    format!("run-{}", random_hex(16))
}

fn random_hex(bytes: usize) -> String {
    let mut value = vec![0_u8; bytes];
    rand::thread_rng().fill_bytes(&mut value);
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::use_registry::flow::{UseFlowCatalogItem, UseFlowEngine, UseFlowRuntime};

    fn source_digest(source: &[u8]) -> String {
        format!("{:x}", Sha256::digest(source))
    }

    fn test_catalog(package_root: &Path, source_path: &Path, digest: &str) -> UseFlowCatalog {
        UseFlowCatalog {
            schema_version: 1,
            generation: 4,
            revision: "4".repeat(64),
            items: vec![UseFlowCatalogItem {
                key: "report:review".to_string(),
                package_id: "use/acme/report".to_string(),
                route: "report".to_string(),
                version: "1.0.0".to_string(),
                lifecycle_generation: 9,
                id: "review".to_string(),
                engine: UseFlowEngine::A3sFlow,
                runtime: UseFlowRuntime::NativeTs,
                package_root: package_root.to_path_buf(),
                source_path: source_path.to_path_buf(),
                export_name: "run".to_string(),
                sha256: digest.to_string(),
                media_type: "text/typescript".to_string(),
                requires_tools: vec!["collect".to_string()],
                requires_mcp: vec!["papers".to_string()],
                requires_okf: vec!["domain".to_string()],
            }],
        }
    }

    fn test_design(digest: &str) -> ParsedFlowDesign {
        let value = json!({
            "version": "a3s.workflow.design.v1",
            "name": "Review report",
            "installedFlow": {
                "schema": "a3s.use.installed-flow.v1",
                "packageId": "use/acme/report",
                "flowId": "review",
                "version": "1.0.0",
                "lifecycleGeneration": 9,
                "sourceSha256": digest,
            },
            "nodes": [],
            "edges": [],
        });
        crate::use_registry::flow::parse_flow_design(&value.to_string()).unwrap()
    }

    fn fixture() -> (TempDir, PathBuf, PathBuf, Vec<u8>, String) {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let package_root = temp.path().join("managed-package");
        let source_path = package_root.join("flows/review.ts");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        let source = b"export async function run() { return {}; }\n".to_vec();
        fs::write(&source_path, &source).unwrap();
        let digest = source_digest(&source);
        (temp, workspace, source_path, source, digest)
    }

    #[tokio::test]
    async fn source_drift_is_rejected_before_any_run_event_or_binding() {
        let (_temp, workspace, source_path, _source, digest) = fixture();
        let package_root = source_path.parent().unwrap().parent().unwrap();
        let catalog = test_catalog(package_root, &source_path, &digest);
        let design = test_design(&digest);
        fs::write(&source_path, "export const replaced = true;\n").unwrap();
        let runtime = InstalledFlowRuntime::with_compiler(&workspace, "missing-compiler");

        let error = runtime
            .run(
                &catalog,
                &design,
                json!({}),
                Some("drifted-run".to_string()),
            )
            .await
            .expect_err("drifted managed source must fail closed");
        assert!(matches!(error, InstalledFlowRuntimeError::Conflict(_)));
        assert!(!runtime.root.join(EVENT_DIRECTORY).exists());
        assert!(!runtime.binding_path("drifted-run").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_substitution_is_rejected_before_any_run_event() {
        use std::os::unix::fs::symlink;

        let (temp, workspace, source_path, source, digest) = fixture();
        let package_root = source_path.parent().unwrap().parent().unwrap();
        let catalog = test_catalog(package_root, &source_path, &digest);
        let design = test_design(&digest);
        let replacement = temp.path().join("replacement.ts");
        fs::write(&replacement, source).unwrap();
        fs::remove_file(&source_path).unwrap();
        symlink(&replacement, &source_path).unwrap();
        let runtime = InstalledFlowRuntime::with_compiler(&workspace, "missing-compiler");

        let error = runtime
            .run(
                &catalog,
                &design,
                json!({}),
                Some("symlink-run".to_string()),
            )
            .await
            .expect_err("symlinked managed source must fail closed");
        assert!(matches!(error, InstalledFlowRuntimeError::Conflict(_)));
        assert!(!runtime.root.join(EVENT_DIRECTORY).exists());
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, body).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn durable_run_is_idempotent_path_free_and_survives_package_removal() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let package_root = temp.path().join("managed-package");
        let source_path = package_root.join("flows/review.ts");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        let source = br#"#!/bin/sh
set -eu
cat >/dev/null
printf '%s\n' '{"protocol":"a3s.flow.native_ts.v1","kind":"workflow","ok":true,"output":{"type":"complete","output":{"marker":"durable"}}}'
"#;
        fs::write(&source_path, source).unwrap();
        let digest = source_digest(source);
        let catalog = test_catalog(&package_root, &source_path, &digest);
        let design = test_design(&digest);
        let compiler = temp.path().join("fake-compiler");
        write_executable(
            &compiler,
            "#!/bin/sh\nset -eu\n[ \"$1\" = compile ]\n[ \"$3\" = -o ]\ncp \"$2\" \"$4\"\nchmod +x \"$4\"\n",
        );

        let runtime = InstalledFlowRuntime::with_compiler(&workspace, &compiler);
        let first = runtime
            .run(
                &catalog,
                &design,
                json!({"request": 7}),
                Some("stable-run".to_string()),
            )
            .await
            .unwrap();
        assert_eq!(first.status, WorkflowRunStatus::Completed);
        assert_eq!(first.output, Some(json!({"marker": "durable"})));
        assert_eq!(first.event_count, 3);

        let recreated = InstalledFlowRuntime::with_compiler(&workspace, &compiler);
        let restored = recreated.get("stable-run").await.unwrap();
        assert_eq!(restored.status, WorkflowRunStatus::Completed);
        assert_eq!(restored.event_count, 3);
        let repeated = recreated
            .run(
                &catalog,
                &design,
                json!({"request": 7}),
                Some("stable-run".to_string()),
            )
            .await
            .unwrap();
        assert_eq!(repeated.event_count, 3, "idempotent start appended events");

        let conflict = recreated
            .run(
                &catalog,
                &design,
                json!({"request": 8}),
                Some("stable-run".to_string()),
            )
            .await
            .expect_err("same run ID with new input must conflict");
        assert!(matches!(conflict, InstalledFlowRuntimeError::Conflict(_)));

        fs::remove_dir_all(&package_root).unwrap();
        let historical = recreated.latest_for_design(&design).await.unwrap();
        assert_eq!(historical.run_id, "stable-run");
        let events = recreated.events("stable-run").await.unwrap();
        assert_eq!(events.len(), 3);
        assert!(events
            .iter()
            .all(|event| { event.event.pointer("/spec/runtime/entrypoint").is_none() }));
        let public_json = serde_json::to_string(&(historical, events)).unwrap();
        assert!(!public_json.contains(&package_root.display().to_string()));
        assert!(!public_json.contains(&workspace.display().to_string()));
        assert!(!public_json.contains("entrypoint"));
        let raw_events = fs::read_to_string(
            recreated
                .root
                .join(EVENT_DIRECTORY)
                .join("stable-run.jsonl"),
        )
        .unwrap();
        assert!(raw_events.contains(".a3s/flow-runtime/sources/"));
        assert!(!raw_events.contains(&workspace.display().to_string()));
    }
}
