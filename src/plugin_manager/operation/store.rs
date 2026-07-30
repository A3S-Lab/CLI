use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use a3s_use_core::PlanActor;

use super::super::capability::PluginCapabilityEvidence;
use super::super::process::{normalize_plan_request, PluginPlanRequest};
use super::super::{PluginManagerError, PluginManagerResult};
use super::lock::PluginMutationLock;

mod io;
mod record;

use io::{
    ensure_store_directories, read_directory_records, read_optional_record, read_required_record,
    remove_file_if_present, write_new_record, WriteDisposition,
};
use record::{
    ensure_request_valid, new_operation_id, now_ms, record_file_name, validate_capability_evidence,
    validate_digest, validate_intent, validate_operation_id, validate_plan_record,
    validate_plan_value, validate_record_path, validate_result,
};

pub(super) const OPERATION_RECORD_SCHEMA: &str = "a3s.cli.plugin-operation-record.v1";
const PLAN_LIFETIME_MS: u64 = 60 * 60 * 1_000;
const COMPLETED_RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_OPERATION_RECORD_BYTES: u64 = 5 * 1024 * 1024;
const MAX_OPERATION_RECORDS: usize = 256;
const MAX_OPERATION_DIRECTORY_ENTRIES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StoredPluginPlan {
    pub schema: String,
    pub operation_id: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub request: PluginPlanRequest,
    #[serde(default = "default_plan_actor")]
    pub actor: PlanActor,
    pub plan_digest: String,
    pub capability_state: PluginCapabilityEvidence,
    pub plan: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredApplyIntent {
    schema: String,
    operation_id: String,
    plan_digest: String,
    started_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StoredOperationResult {
    pub schema: String,
    pub operation_id: String,
    pub plan_digest: String,
    pub completed_at_ms: u64,
    pub capability_before: PluginCapabilityEvidence,
    pub capability_after: PluginCapabilityEvidence,
    pub data: Value,
}

#[derive(Debug, Clone)]
pub(in crate::plugin_manager) struct PluginOperationStore {
    root: PathBuf,
}

impl PluginOperationStore {
    pub(super) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(super) async fn acquire_mutation_lock(&self) -> PluginManagerResult<PluginMutationLock> {
        PluginMutationLock::acquire(self.root.join("mutation.lock")).await
    }

    #[cfg(test)]
    pub(super) async fn create_plan(
        &self,
        request: PluginPlanRequest,
        plan_digest: String,
        capability_state: PluginCapabilityEvidence,
        plan: Value,
    ) -> PluginManagerResult<StoredPluginPlan> {
        self.create_plan_for_actor(
            request,
            PlanActor::User,
            plan_digest,
            capability_state,
            plan,
        )
        .await
    }

    pub(super) async fn create_plan_for_actor(
        &self,
        request: PluginPlanRequest,
        actor: PlanActor,
        plan_digest: String,
        capability_state: PluginCapabilityEvidence,
        plan: Value,
    ) -> PluginManagerResult<StoredPluginPlan> {
        let store = self.clone();
        run_blocking("create reviewed plugin plan", move || {
            store.create_plan_sync(request, actor, plan_digest, capability_state, plan)
        })
        .await
    }

    pub(super) async fn resolve_plan(
        &self,
        operation_id: Option<String>,
        legacy_request: Option<PluginPlanRequest>,
        plan_digest: String,
    ) -> PluginManagerResult<StoredPluginPlan> {
        let store = self.clone();
        run_blocking("resolve reviewed plugin plan", move || {
            store.resolve_plan_sync(operation_id, legacy_request, &plan_digest)
        })
        .await
    }

    pub(super) async fn has_intent(&self, plan: &StoredPluginPlan) -> PluginManagerResult<bool> {
        let store = self.clone();
        let plan = plan.clone();
        run_blocking("inspect plugin apply intent", move || {
            store.has_intent_sync(&plan)
        })
        .await
    }

    pub(super) async fn persist_intent(
        &self,
        plan: &StoredPluginPlan,
    ) -> PluginManagerResult<bool> {
        let store = self.clone();
        let plan = plan.clone();
        run_blocking("persist plugin apply intent", move || {
            store.persist_intent_sync(&plan)
        })
        .await
    }

    pub(super) async fn result(
        &self,
        plan: &StoredPluginPlan,
    ) -> PluginManagerResult<Option<StoredOperationResult>> {
        let store = self.clone();
        let plan = plan.clone();
        run_blocking("read durable plugin operation result", move || {
            store.result_sync(&plan)
        })
        .await
    }

    pub(super) async fn persist_result(
        &self,
        result: StoredOperationResult,
    ) -> PluginManagerResult<(StoredOperationResult, bool)> {
        let store = self.clone();
        run_blocking("persist durable plugin operation result", move || {
            store.persist_result_sync(result)
        })
        .await
    }

    fn create_plan_sync(
        &self,
        request: PluginPlanRequest,
        actor: PlanActor,
        plan_digest: String,
        capability_state: PluginCapabilityEvidence,
        plan: Value,
    ) -> PluginManagerResult<StoredPluginPlan> {
        ensure_request_valid(&request)?;
        validate_digest(&plan_digest)?;
        validate_capability_evidence(&capability_state)?;
        validate_plan_value(&plan, &plan_digest)?;
        let now = now_ms()?;
        self.prune_sync(now)?;
        for _ in 0..4 {
            let operation_id = new_operation_id(request.action)?;
            let record = StoredPluginPlan {
                schema: OPERATION_RECORD_SCHEMA.to_string(),
                operation_id,
                created_at_ms: now,
                expires_at_ms: now.saturating_add(PLAN_LIFETIME_MS),
                request: request.clone(),
                actor,
                plan_digest: plan_digest.clone(),
                capability_state: capability_state.clone(),
                plan: plan.clone(),
            };
            validate_plan_record(&record)?;
            let path = self.plan_path(&record.operation_id);
            match write_new_record(&path, &record)? {
                WriteDisposition::Created => return Ok(record),
                WriteDisposition::AlreadyExists => continue,
            }
        }
        Err(PluginManagerError::Infrastructure(
            "could not allocate a unique plugin operation ID".to_string(),
        ))
    }

    fn resolve_plan_sync(
        &self,
        operation_id: Option<String>,
        legacy_request: Option<PluginPlanRequest>,
        plan_digest: &str,
    ) -> PluginManagerResult<StoredPluginPlan> {
        validate_digest(plan_digest)?;
        let plan = match operation_id {
            Some(operation_id) => {
                validate_operation_id(&operation_id)?;
                let plan =
                    read_required_record::<StoredPluginPlan>(&self.plan_path(&operation_id))?;
                if plan.operation_id != operation_id {
                    return Err(invalid_store(
                        "reviewed plan filename does not match its operation ID",
                    ));
                }
                plan
            }
            None => self.find_legacy_plan_sync(
                legacy_request.as_ref().ok_or_else(|| {
                    PluginManagerError::InvalidRequest(
                        "operationId or the legacy plugin operation fields are required"
                            .to_string(),
                    )
                })?,
                plan_digest,
            )?,
        };
        validate_plan_record(&plan)?;
        if plan.plan_digest != plan_digest {
            return Err(PluginManagerError::InvalidRequest(
                "operationId and planDigest do not identify the same reviewed plan".to_string(),
            ));
        }
        Ok(plan)
    }

    fn find_legacy_plan_sync(
        &self,
        request: &PluginPlanRequest,
        plan_digest: &str,
    ) -> PluginManagerResult<StoredPluginPlan> {
        let request = normalize_plan_request(request)?;
        let directory = self.plans_root();
        let records = read_directory_records::<StoredPluginPlan>(&directory)?;
        let mut matching = Vec::new();
        for (path, record) in records {
            validate_plan_record(&record)?;
            validate_record_path(&path, &record.operation_id)?;
            if record.request == request && record.plan_digest == plan_digest {
                matching.push(record);
            }
        }
        matching
            .into_iter()
            .max_by(|left, right| {
                left.created_at_ms
                    .cmp(&right.created_at_ms)
                    .then_with(|| left.operation_id.cmp(&right.operation_id))
            })
            .ok_or_else(|| {
                PluginManagerError::InvalidRequest(
                    "the reviewed plugin plan is not present in the durable operation store"
                        .to_string(),
                )
            })
    }

    fn has_intent_sync(&self, plan: &StoredPluginPlan) -> PluginManagerResult<bool> {
        validate_plan_record(plan)?;
        let Some(intent) =
            read_optional_record::<StoredApplyIntent>(&self.intent_path(&plan.operation_id))?
        else {
            return Ok(false);
        };
        validate_intent(&intent, plan)?;
        Ok(true)
    }

    fn persist_intent_sync(&self, plan: &StoredPluginPlan) -> PluginManagerResult<bool> {
        validate_plan_record(plan)?;
        let path = self.intent_path(&plan.operation_id);
        if let Some(intent) = read_optional_record::<StoredApplyIntent>(&path)? {
            validate_intent(&intent, plan)?;
            return Ok(true);
        }
        let started_at_ms = now_ms()?;
        if started_at_ms > plan.expires_at_ms {
            return Err(PluginManagerError::OperationFailed(
                "the reviewed plugin plan expired; create and review a new plan".to_string(),
            ));
        }
        let intent = StoredApplyIntent {
            schema: OPERATION_RECORD_SCHEMA.to_string(),
            operation_id: plan.operation_id.clone(),
            plan_digest: plan.plan_digest.clone(),
            started_at_ms,
        };
        match write_new_record(&path, &intent)? {
            WriteDisposition::Created => Ok(false),
            WriteDisposition::AlreadyExists => {
                let intent = read_required_record::<StoredApplyIntent>(&path)?;
                validate_intent(&intent, plan)?;
                Ok(true)
            }
        }
    }

    fn result_sync(
        &self,
        plan: &StoredPluginPlan,
    ) -> PluginManagerResult<Option<StoredOperationResult>> {
        validate_plan_record(plan)?;
        let Some(result) =
            read_optional_record::<StoredOperationResult>(&self.result_path(&plan.operation_id))?
        else {
            return Ok(None);
        };
        let intent =
            read_required_record::<StoredApplyIntent>(&self.intent_path(&plan.operation_id))?;
        validate_intent(&intent, plan)?;
        validate_result(&result, plan, &intent)?;
        Ok(Some(result))
    }

    fn persist_result_sync(
        &self,
        result: StoredOperationResult,
    ) -> PluginManagerResult<(StoredOperationResult, bool)> {
        let plan = read_required_record::<StoredPluginPlan>(&self.plan_path(&result.operation_id))?;
        validate_plan_record(&plan)?;
        let intent =
            read_required_record::<StoredApplyIntent>(&self.intent_path(&result.operation_id))?;
        validate_intent(&intent, &plan)?;
        validate_result(&result, &plan, &intent)?;
        let path = self.result_path(&result.operation_id);
        if let Some(existing) = read_optional_record::<StoredOperationResult>(&path)? {
            validate_result(&existing, &plan, &intent)?;
            return Ok((existing, false));
        }
        match write_new_record(&path, &result)? {
            WriteDisposition::Created => Ok((result, true)),
            WriteDisposition::AlreadyExists => {
                let existing = read_required_record::<StoredOperationResult>(&path)?;
                validate_result(&existing, &plan, &intent)?;
                Ok((existing, false))
            }
        }
    }

    fn prune_sync(&self, now: u64) -> PluginManagerResult<()> {
        ensure_store_directories(self)?;
        let plans = read_directory_records::<StoredPluginPlan>(&self.plans_root())?;
        let mut retained = 0usize;
        for (path, plan) in plans {
            validate_plan_record(&plan)?;
            validate_record_path(&path, &plan.operation_id)?;
            let intent_path = self.intent_path(&plan.operation_id);
            let result_path = self.result_path(&plan.operation_id);
            let intent = read_optional_record::<StoredApplyIntent>(&intent_path)?;
            if let Some(intent) = &intent {
                validate_intent(intent, &plan)?;
            }
            let result = read_optional_record::<StoredOperationResult>(&result_path)?;
            if let Some(result) = &result {
                let intent = intent.as_ref().ok_or_else(|| {
                    invalid_store("durable plugin operation result has no apply intent")
                })?;
                validate_result(result, &plan, intent)?;
            }
            let expired = now > plan.expires_at_ms;
            let completed_retention_elapsed = result.as_ref().is_some_and(|result| {
                now > result
                    .completed_at_ms
                    .saturating_add(COMPLETED_RETENTION_MS)
            });
            if expired && completed_retention_elapsed {
                remove_file_if_present(&result_path)?;
                remove_file_if_present(&intent_path)?;
                remove_file_if_present(&self.plan_path(&plan.operation_id))?;
                continue;
            }
            if expired && result.is_none() && intent.is_none() {
                remove_file_if_present(&self.plan_path(&plan.operation_id))?;
                continue;
            }
            retained += 1;
        }
        if retained >= MAX_OPERATION_RECORDS {
            return Err(PluginManagerError::Infrastructure(format!(
                "plugin operation store reached its {MAX_OPERATION_RECORDS}-record limit"
            )));
        }
        Ok(())
    }

    fn plans_root(&self) -> PathBuf {
        self.root.join("plans")
    }

    fn intents_root(&self) -> PathBuf {
        self.root.join("intents")
    }

    fn results_root(&self) -> PathBuf {
        self.root.join("results")
    }

    fn plan_path(&self, operation_id: &str) -> PathBuf {
        self.plans_root().join(record_file_name(operation_id))
    }

    fn intent_path(&self, operation_id: &str) -> PathBuf {
        self.intents_root().join(record_file_name(operation_id))
    }

    fn result_path(&self, operation_id: &str) -> PathBuf {
        self.results_root().join(record_file_name(operation_id))
    }
}

fn default_plan_actor() -> PlanActor {
    PlanActor::User
}

async fn run_blocking<T: Send + 'static>(
    label: &'static str,
    operation: impl FnOnce() -> PluginManagerResult<T> + Send + 'static,
) -> PluginManagerResult<T> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| {
            PluginManagerError::Infrastructure(format!("{label} task failed: {error}"))
        })?
}

fn invalid_store(message: impl Into<String>) -> PluginManagerError {
    PluginManagerError::Infrastructure(format!(
        "durable plugin operation store is invalid: {}",
        message.into()
    ))
}

#[cfg(test)]
mod tests;
