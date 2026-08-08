use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_use::cognitive_package::{
    CognitivePackageEnablementPlanStatus, CognitivePackageEnablementPreparation,
    CognitivePackageEnablementRequest, CognitivePackageEnablementResult,
};
use a3s_use_core::{
    PlanActor, PluginHostApplyRequest, PluginHostApplyResult, PluginHostCapabilities,
    PluginHostEnablementPlanRequest, PluginHostEnablementPlanResult,
    PluginHostEnablementPlanStatus, UseError, UseResult, PLUGIN_HOST_APPLY_RESULT_SCHEMA,
    PLUGIN_HOST_ENABLEMENT_PLAN_RESULT_SCHEMA,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::components::{
    apply_reviewed_cognitive_enablement, code_cognitive_package_manager_with_authorization,
};
use crate::plugin_manager::enablement_authorization::EnablementPlanningAuthorization;
use crate::plugin_manager::operation::store::io::{
    ensure_real_directory, read_optional_record, write_new_record, WriteDisposition,
};
use crate::plugin_manager::runtime_host::ReviewedEnablementRuntimeEvidence;
use crate::plugin_manager::{PluginManager, PluginManagerError, PluginManagerResult};

const PLAN_RECORD_SCHEMA: &str = "a3s.cli.reviewed-enablement-plan.v2";
const APPLY_INTENT_SCHEMA: &str = "a3s.cli.reviewed-enablement-apply-intent.v1";
const APPLY_RESULT_SCHEMA: &str = "a3s.cli.reviewed-enablement-apply-result.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredReviewedEnablementPlan {
    schema: String,
    request: PluginHostEnablementPlanRequest,
    result: PluginHostEnablementPlanResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime: Option<ReviewedEnablementRuntimeEvidence>,
}

impl StoredReviewedEnablementPlan {
    fn new(
        request: PluginHostEnablementPlanRequest,
        result: PluginHostEnablementPlanResult,
        runtime: Option<ReviewedEnablementRuntimeEvidence>,
    ) -> UseResult<Self> {
        let record = Self {
            schema: PLAN_RECORD_SCHEMA.to_string(),
            request,
            result,
            runtime,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> UseResult<()> {
        self.request.validate()?;
        self.result.validate()?;
        if self.schema != PLAN_RECORD_SCHEMA
            || self.result.replayed
            || self.result.request_id != self.request.request_id
            || self.result.assignment_generation != self.request.assignment_generation
            || self.result.capabilities_digest != self.request.capabilities_digest
            || self.result.scope != self.request.scope
            || self.result.package_id != self.request.package_id
            || self.result.expected_package_generation != self.request.expected_package_generation
            || self.result.enabled != self.request.enabled
        {
            return Err(store_invalid());
        }
        if let Some(plan) = &self.result.plan {
            if plan.plan.operation_id != operation_id(&self.request)? {
                return Err(store_invalid());
            }
            self.runtime
                .as_ref()
                .ok_or_else(store_invalid)?
                .validate_for(&plan.plan)?;
        } else if self.runtime.is_some() {
            return Err(store_invalid());
        }
        Ok(())
    }

    fn replayed_result(&self) -> PluginHostEnablementPlanResult {
        let mut result = self.result.clone();
        result.replayed = true;
        result
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredReviewedEnablementApplyIntent {
    schema: String,
    request: PluginHostApplyRequest,
}

impl StoredReviewedEnablementApplyIntent {
    fn new(request: PluginHostApplyRequest) -> UseResult<Self> {
        let intent = Self {
            schema: APPLY_INTENT_SCHEMA.to_string(),
            request,
        };
        intent.validate()?;
        Ok(intent)
    }

    fn validate(&self) -> UseResult<()> {
        self.request.validate()?;
        if self.schema != APPLY_INTENT_SCHEMA {
            return Err(store_invalid());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredReviewedEnablementApplyResult {
    schema: String,
    request: PluginHostApplyRequest,
    result: PluginHostApplyResult,
    cognitive_result: CognitivePackageEnablementResult,
}

impl StoredReviewedEnablementApplyResult {
    fn new(
        request: PluginHostApplyRequest,
        result: PluginHostApplyResult,
        mut cognitive_result: CognitivePackageEnablementResult,
        plan: &PluginHostEnablementPlanResult,
    ) -> UseResult<Self> {
        cognitive_result.replayed = false;
        let record = Self {
            schema: APPLY_RESULT_SCHEMA.to_string(),
            request,
            result,
            cognitive_result,
        };
        record.validate_for(plan)?;
        Ok(record)
    }

    fn validate_for(&self, plan: &PluginHostEnablementPlanResult) -> UseResult<()> {
        self.request.validate()?;
        self.result.validate()?;
        let cognitive_request = cognitive_request(plan)?;
        self.cognitive_result.validate_for(&cognitive_request)?;
        if self.schema != APPLY_RESULT_SCHEMA
            || self.result.replayed
            || self.cognitive_result.replayed
            || self.result.request_id != self.request.request_id
            || self.result.assignment_generation != self.request.assignment_generation
            || self.result.capabilities_digest != self.request.capabilities_digest
            || self.result.scope != self.request.scope
            || self.result.package_id != self.request.package_id
            || self.result.operation_id != self.request.operation_id
            || self.result.plan_digest != self.request.plan_digest
            || self.result.operation_id != self.cognitive_result.operation_id
            || self.result.package_id != self.cognitive_result.package_id
            || self.result.completed_at_ms != self.cognitive_result.completed_at_ms
            || self.result.operation_result_digest != self.cognitive_result.operation_result_digest
            || self.result.state != self.cognitive_result.state
        {
            return Err(store_invalid());
        }
        Ok(())
    }

    fn replayed_result(&self) -> PluginHostApplyResult {
        let mut result = self.result.clone();
        result.replayed = true;
        result
    }
}

#[derive(Debug, Clone)]
pub(super) struct ReviewedEnablementStore {
    root: PathBuf,
}

impl ReviewedEnablementStore {
    pub(super) fn from_state_root(state_root: &Path) -> Self {
        Self {
            root: state_root.join("plugin-manager/managed-host/reviewed-enablement"),
        }
    }

    async fn request_plan(
        &self,
        request_id: &str,
    ) -> UseResult<Option<StoredReviewedEnablementPlan>> {
        self.load(self.requests_root(), request_id).await
    }

    async fn operation_plan(
        &self,
        operation_id: &str,
    ) -> UseResult<Option<StoredReviewedEnablementPlan>> {
        self.load(self.operations_root(), operation_id).await
    }

    async fn persist_request_plan(&self, record: StoredReviewedEnablementPlan) -> UseResult<bool> {
        let key = record.request.request_id.clone();
        self.persist(self.requests_root(), &key, record).await
    }

    async fn persist_operation_plan(
        &self,
        record: StoredReviewedEnablementPlan,
    ) -> UseResult<bool> {
        let operation_id = record
            .result
            .plan
            .as_ref()
            .ok_or_else(store_invalid)?
            .plan
            .operation_id
            .clone();
        self.persist(self.operations_root(), &operation_id, record)
            .await
    }

    async fn apply_intent(
        &self,
        operation_id: &str,
    ) -> UseResult<Option<StoredReviewedEnablementApplyIntent>> {
        self.load(self.apply_intents_root(), operation_id).await
    }

    async fn persist_apply_intent(
        &self,
        intent: StoredReviewedEnablementApplyIntent,
    ) -> UseResult<bool> {
        let operation_id = intent.request.operation_id.clone();
        self.persist(self.apply_intents_root(), &operation_id, intent)
            .await
    }

    async fn apply_result(
        &self,
        operation_id: &str,
        plan: &PluginHostEnablementPlanResult,
    ) -> UseResult<Option<StoredReviewedEnablementApplyResult>> {
        let record: Option<StoredReviewedEnablementApplyResult> =
            self.load(self.apply_results_root(), operation_id).await?;
        if let Some(record) = &record {
            record.validate_for(plan)?;
        }
        Ok(record)
    }

    async fn persist_apply_result(
        &self,
        record: StoredReviewedEnablementApplyResult,
        plan: &PluginHostEnablementPlanResult,
    ) -> UseResult<bool> {
        record.validate_for(plan)?;
        let operation_id = record.request.operation_id.clone();
        self.persist(self.apply_results_root(), &operation_id, record)
            .await
    }

    async fn load<T>(&self, root: PathBuf, key: &str) -> UseResult<Option<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let path = record_path(&root, key);
        run_store(move || {
            ensure_real_directory(&root)?;
            read_optional_record(&path)
        })
        .await
        .map_err(store_unavailable)
    }

    async fn persist<T>(&self, root: PathBuf, key: &str, candidate: T) -> UseResult<bool>
    where
        T: Clone + PartialEq + Serialize + DeserializeOwned + Send + 'static,
    {
        let path = record_path(&root, key);
        let load_root = root.clone();
        let written = candidate.clone();
        let disposition = run_store(move || {
            ensure_real_directory(&root)?;
            write_new_record(&path, &written)
        })
        .await
        .map_err(store_unavailable)?;
        if disposition == WriteDisposition::Created {
            return Ok(true);
        }
        let current = self.load(load_root, key).await?;
        if current.as_ref() == Some(&candidate) {
            Ok(false)
        } else {
            Err(operation_conflict())
        }
    }

    fn requests_root(&self) -> PathBuf {
        self.root.join("requests")
    }

    fn operations_root(&self) -> PathBuf {
        self.root.join("operations")
    }

    fn apply_intents_root(&self) -> PathBuf {
        self.root.join("apply-intents")
    }

    fn apply_results_root(&self) -> PathBuf {
        self.root.join("apply-results")
    }
}

pub(super) async fn plan(
    manager: &PluginManager,
    store: &ReviewedEnablementStore,
    capabilities: &PluginHostCapabilities,
    request: &PluginHostEnablementPlanRequest,
) -> UseResult<PluginHostEnablementPlanResult> {
    if let Some(record) = store.request_plan(&request.request_id).await? {
        record.validate()?;
        if record.request != *request {
            return Err(operation_conflict());
        }
        if record.result.status == PluginHostEnablementPlanStatus::Planned {
            store.persist_operation_plan(record.clone()).await?;
        }
        let result = record.replayed_result();
        result.validate_for(request, capabilities)?;
        return Ok(result);
    }

    let operation_id = operation_id(request)?;
    if let Some(record) = store.operation_plan(&operation_id).await? {
        record.validate()?;
        if record.request != *request {
            return Err(operation_conflict());
        }
        store.persist_request_plan(record.clone()).await?;
        let result = record.replayed_result();
        result.validate_for(request, capabilities)?;
        return Ok(result);
    }

    let authorization = Arc::new(EnablementPlanningAuthorization::new(
        request.scope.plan_scope(),
        PlanActor::Agent,
        manager.authorization_policy().clone(),
    ));
    let package_manager = code_cognitive_package_manager_with_authorization(
        &manager.component_paths,
        request.scope.plan_scope(),
        authorization.clone(),
    )?;
    let cognitive_request = CognitivePackageEnablementRequest::new(
        operation_id,
        request.package_id.to_string(),
        request.expected_package_generation,
        request.enabled,
    )?;
    let (planned, runtime) = match package_manager
        .prepare_enablement(&cognitive_request)
        .await?
    {
        CognitivePackageEnablementPreparation::Outcome(planned) => (*planned, None),
        CognitivePackageEnablementPreparation::Draft(draft) => {
            let provisional = authorization.provisional_authority()?;
            let (planned, runtime) = manager
                .runtime_host
                .bind_enablement_plan(*draft, provisional, |plan| {
                    authorization.evaluate_plan(plan)
                })
                .await?;
            (planned, Some(runtime))
        }
    };
    let (status, plan) = match planned.status {
        CognitivePackageEnablementPlanStatus::NoChange => {
            (PluginHostEnablementPlanStatus::NoChange, None)
        }
        CognitivePackageEnablementPlanStatus::Planned => {
            (PluginHostEnablementPlanStatus::Planned, planned.plan)
        }
        CognitivePackageEnablementPlanStatus::Completed => {
            return Err(UseError::new(
                "use.plugin.host_enablement_plan_store_missing",
                "A completed Use enablement operation has no durable reviewed host plan.",
            ));
        }
    };
    let result = PluginHostEnablementPlanResult {
        schema: PLUGIN_HOST_ENABLEMENT_PLAN_RESULT_SCHEMA.to_string(),
        request_id: request.request_id.clone(),
        assignment_generation: request.assignment_generation,
        capabilities_digest: request.capabilities_digest.clone(),
        scope: request.scope.clone(),
        package_id: request.package_id.clone(),
        expected_package_generation: request.expected_package_generation,
        enabled: request.enabled,
        planned_at_ms: planned.planned_at_ms,
        status,
        state: planned.state,
        plan,
        replayed: false,
    };
    result.validate_for(request, capabilities)?;
    let record = StoredReviewedEnablementPlan::new(request.clone(), result.clone(), runtime)?;
    if status == PluginHostEnablementPlanStatus::Planned {
        store.persist_operation_plan(record.clone()).await?;
    }
    store.persist_request_plan(record).await?;
    Ok(result)
}

pub(super) async fn apply(
    manager: &PluginManager,
    store: &ReviewedEnablementStore,
    capabilities: &PluginHostCapabilities,
    request: &PluginHostApplyRequest,
) -> UseResult<Option<PluginHostApplyResult>> {
    let Some(plan_record) = store.operation_plan(&request.operation_id).await? else {
        return Ok(None);
    };
    plan_record.validate()?;
    let plan = &plan_record.result;
    request.validate_for_enablement_plan(plan, capabilities)?;

    if let Some(result) = store.apply_result(&request.operation_id, plan).await? {
        if result.request != *request {
            return Err(operation_conflict());
        }
        let intent = store
            .apply_intent(&request.operation_id)
            .await?
            .ok_or_else(store_invalid)?;
        intent.validate()?;
        if intent.request != *request {
            return Err(operation_conflict());
        }
        let replayed = result.replayed_result();
        replayed.validate_for(request, capabilities)?;
        return Ok(Some(replayed));
    }

    let resumed = match store.apply_intent(&request.operation_id).await? {
        Some(intent) => {
            intent.validate()?;
            if intent.request != *request {
                return Err(operation_conflict());
            }
            true
        }
        None => {
            let envelope = plan.plan.as_ref().ok_or_else(store_invalid)?;
            manager
                .verify_plan_authority(&envelope.plan)
                .map_err(policy_unavailable)?;
            request.verify_apply_for_enablement_plan(
                plan,
                capabilities,
                super::state::now_ms()?,
            )?;
            store
                .persist_apply_intent(StoredReviewedEnablementApplyIntent::new(request.clone())?)
                .await?;
            false
        }
    };

    let envelope = plan.plan.as_ref().ok_or_else(store_invalid)?;
    let runtime = plan_record.runtime.as_ref().ok_or_else(store_invalid)?;
    let selection = manager
        .runtime_host
        .reconstruct_enablement_selection(&envelope.plan, runtime)
        .await?;
    let lifecycle = manager.runtime_host.lifecycle_factory(selection)?;
    let cognitive = apply_reviewed_cognitive_enablement(
        envelope,
        request.confirmation.as_ref(),
        plan.expected_package_generation,
        &manager.component_paths,
        lifecycle,
    )
    .await
    .map_err(|_| {
        UseError::new(
            "use.plugin.host_operation_failed",
            "The reviewed cognitive-package enablement operation failed.",
        )
    })?;
    let mut result = PluginHostApplyResult {
        schema: PLUGIN_HOST_APPLY_RESULT_SCHEMA.to_string(),
        request_id: request.request_id.clone(),
        assignment_generation: request.assignment_generation,
        capabilities_digest: request.capabilities_digest.clone(),
        scope: request.scope.clone(),
        package_id: request.package_id.clone(),
        operation_id: request.operation_id.clone(),
        plan_digest: request.plan_digest.clone(),
        completed_at_ms: cognitive.completed_at_ms,
        operation_result_digest: cognitive.operation_result_digest.clone(),
        state: cognitive.state.clone(),
        replayed: false,
    };
    result.validate_for(request, capabilities)?;
    let cognitive_replayed = cognitive.replayed;
    let record =
        StoredReviewedEnablementApplyResult::new(request.clone(), result.clone(), cognitive, plan)?;
    let created = store.persist_apply_result(record, plan).await?;
    result.replayed = resumed || cognitive_replayed || !created;
    result.validate_for(request, capabilities)?;
    Ok(Some(result))
}

fn cognitive_request(
    plan: &PluginHostEnablementPlanResult,
) -> UseResult<CognitivePackageEnablementRequest> {
    let envelope = plan.plan.as_ref().ok_or_else(store_invalid)?;
    CognitivePackageEnablementRequest::new(
        envelope.plan.operation_id.clone(),
        plan.package_id.to_string(),
        plan.expected_package_generation,
        plan.enabled,
    )
}

fn operation_id(request: &PluginHostEnablementPlanRequest) -> UseResult<String> {
    let digest = request.descriptor_digest()?;
    Ok(format!(
        "enablement:{}",
        digest.strip_prefix("sha256:").unwrap_or(&digest)
    ))
}

fn record_path(root: &Path, key: &str) -> PathBuf {
    root.join(format!("{:x}.json", Sha256::digest(key.as_bytes())))
}

async fn run_store<T: Send + 'static>(
    operation: impl FnOnce() -> PluginManagerResult<T> + Send + 'static,
) -> PluginManagerResult<T> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "reviewed enablement store task failed: {error}"
            ))
        })?
}

fn operation_conflict() -> UseError {
    UseError::new(
        "use.plugin.host_enablement_operation_conflict",
        "The reviewed enablement identity is already bound to different durable evidence.",
    )
}

fn store_invalid() -> UseError {
    UseError::new(
        "use.plugin.host_enablement_store_invalid",
        "The durable reviewed enablement record is invalid.",
    )
}

fn store_unavailable(_error: PluginManagerError) -> UseError {
    UseError::new(
        "use.plugin.host_enablement_store_unavailable",
        "The durable reviewed enablement store is unavailable.",
    )
}

fn policy_unavailable(_error: PluginManagerError) -> UseError {
    UseError::new(
        "use.plugin.host_policy_invalid",
        "The host plugin policy could not authorize the reviewed enablement plan.",
    )
}
