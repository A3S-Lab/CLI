//! Local reviewed enablement planning and apply.
//!
//! CLI and Web use this application service as the sole enablement mutation
//! path. A3S Use remains the only planner and lifecycle owner.

mod store;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use a3s_use::cognitive_package::{
    CognitivePackageEnablementPlanStatus, CognitivePackageEnablementPreparation,
    CognitivePackageEnablementRequest, CognitivePackageEnablementResult,
};
use a3s_use_core::{
    PlanActor, PlanPolicyDecision, PluginDesiredState, PluginHostPackageState,
    PluginOperationAction, PluginOperationConfirmation, PluginOperationPlan,
    PluginOperationPlanEnvelope, UseError, PLUGIN_OPERATION_CONFIRMATION_SCHEMA,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::components::{
    apply_reviewed_cognitive_enablement, code_cognitive_package_manager_with_authorization,
};
use crate::plugin_manager::enablement_authorization::EnablementPlanningAuthorization;
use crate::plugin_manager::process::{normalize_component_id, normalize_plan_digest};
use crate::plugin_manager::{PluginManager, PluginManagerError, PluginManagerResult};

const COGNITIVE_PACKAGE_RECEIPT_SCHEMA_VERSION: u32 = 3;
const PLAN_RESULT_SCHEMA: &str = "a3s.cli.plugin-enablement-plan-result.v1";
static ENABLEMENT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginEnablementPlanRequest {
    pub component_id: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_package_generation: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginEnablementApplyRequest {
    pub operation_id: String,
    pub plan_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum PluginEnablementPlanStatus {
    NoChange,
    Planned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PluginEnablementPlanResult {
    schema: String,
    component_id: String,
    package_id: String,
    expected_package_generation: u64,
    enabled: bool,
    planned_at_ms: u64,
    status: PluginEnablementPlanStatus,
    state: PluginHostPackageState,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    canonical_plan_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<PluginOperationPlanEnvelope>,
}

impl PluginEnablementPlanRequest {
    fn normalize(&self) -> PluginManagerResult<Self> {
        if self.expected_package_generation == Some(0) {
            return Err(PluginManagerError::InvalidRequest(
                "expectedPackageGeneration must be greater than zero".to_string(),
            ));
        }
        Ok(Self {
            component_id: normalize_component_id(&self.component_id)?,
            enabled: self.enabled,
            expected_package_generation: self.expected_package_generation,
        })
    }
}

impl PluginEnablementApplyRequest {
    fn normalize(&self) -> PluginManagerResult<Self> {
        PluginOperationPlan::validate_operation_id(&self.operation_id).map_err(|error| {
            PluginManagerError::InvalidRequest(format!(
                "invalid reviewed enablement operation ID: {error}"
            ))
        })?;
        Ok(Self {
            operation_id: self.operation_id.clone(),
            plan_digest: format!("sha256:{}", normalize_plan_digest(&self.plan_digest)?),
        })
    }
}

impl PluginEnablementPlanResult {
    fn no_change(
        request: &PluginEnablementPlanRequest,
        package_id: &str,
        planned_at_ms: u64,
        state: PluginHostPackageState,
    ) -> PluginManagerResult<Self> {
        Self::new(request, package_id, planned_at_ms, state, None)
    }

    fn planned(
        request: &PluginEnablementPlanRequest,
        package_id: &str,
        planned_at_ms: u64,
        state: PluginHostPackageState,
        plan: PluginOperationPlanEnvelope,
    ) -> PluginManagerResult<Self> {
        Self::new(request, package_id, planned_at_ms, state, Some(plan))
    }

    fn new(
        request: &PluginEnablementPlanRequest,
        package_id: &str,
        planned_at_ms: u64,
        state: PluginHostPackageState,
        plan: Option<PluginOperationPlanEnvelope>,
    ) -> PluginManagerResult<Self> {
        let (status, operation_id, canonical_plan_digest) = match &plan {
            Some(envelope) => (
                PluginEnablementPlanStatus::Planned,
                Some(envelope.plan.operation_id.clone()),
                Some(envelope.plan_digest.clone()),
            ),
            None => (PluginEnablementPlanStatus::NoChange, None, None),
        };
        let result = Self {
            schema: PLAN_RESULT_SCHEMA.to_string(),
            component_id: request.component_id.clone(),
            package_id: package_id.to_string(),
            expected_package_generation: request
                .expected_package_generation
                .ok_or_else(plan_invalid)?,
            enabled: request.enabled,
            planned_at_ms,
            status,
            state,
            operation_id,
            canonical_plan_digest,
            plan,
        };
        result.validate()?;
        Ok(result)
    }

    fn validate(&self) -> PluginManagerResult<()> {
        let target = if self.enabled {
            PluginDesiredState::Enabled
        } else {
            PluginDesiredState::InstalledDisabled
        };
        self.state.validate().map_err(|_| plan_invalid())?;
        if self.schema != PLAN_RESULT_SCHEMA
            || normalize_component_id(&self.component_id)? != self.component_id
            || self.component_id != format!("use/{}", self.package_id)
            || self.expected_package_generation == 0
            || self.planned_at_ms == 0
            || self.state.package_generation != Some(self.expected_package_generation)
        {
            return Err(plan_invalid());
        }
        match self.status {
            PluginEnablementPlanStatus::NoChange
                if self.state.desired == target
                    && self.operation_id.is_none()
                    && self.canonical_plan_digest.is_none()
                    && self.plan.is_none() =>
            {
                Ok(())
            }
            PluginEnablementPlanStatus::Planned if self.state.desired != target => {
                let plan = self.plan.as_ref().ok_or_else(plan_invalid)?;
                plan.validate().map_err(|_| plan_invalid())?;
                let action = if self.enabled {
                    PluginOperationAction::Enable
                } else {
                    PluginOperationAction::Disable
                };
                if plan.plan.package_id != self.package_id
                    || plan.plan.action != action
                    || plan.plan.created_at_ms != self.planned_at_ms
                    || self.operation_id.as_deref() != Some(plan.plan.operation_id.as_str())
                    || self.canonical_plan_digest.as_deref() != Some(plan.plan_digest.as_str())
                    || plan.plan.state.receipt_digest != self.state.receipt_digest
                    || plan.plan.state.capability_generation != self.state.capability_generation
                {
                    return Err(plan_invalid());
                }
                Ok(())
            }
            _ => Err(plan_invalid()),
        }
    }
}

pub(in crate::plugin_manager) async fn plan(
    manager: &PluginManager,
    request: &PluginEnablementPlanRequest,
) -> PluginManagerResult<Value> {
    let _mutation_guard = manager.operation_store.acquire_mutation_lock().await?;
    let mut request = request.normalize()?;
    let package_id = package_id(&request.component_id)?;
    let scope = super::super::default_plan_scope();
    let authorization = Arc::new(EnablementPlanningAuthorization::new(
        scope.clone(),
        PlanActor::User,
        manager.authorization_policy().clone(),
    ));
    let package_manager = code_cognitive_package_manager_with_authorization(
        &manager.component_paths,
        scope,
        authorization.clone(),
    )
    .map_err(use_infrastructure_error)?;
    let extension = package_manager
        .registry()
        .get(&package_id)
        .await
        .map_err(use_infrastructure_error)?
        .ok_or_else(|| not_installed(&package_id))?;
    if extension.receipt.schema_version != COGNITIVE_PACKAGE_RECEIPT_SCHEMA_VERSION {
        return Err(PluginManagerError::InvalidRequest(format!(
            "plugin '{}' uses unsupported receipt schema v{}; only schema v3 is accepted",
            package_id, extension.receipt.schema_version
        )));
    }
    let observed = package_manager
        .observe_package(&package_id)
        .await
        .map_err(use_operation_error)?;
    if observed.desired == PluginDesiredState::Absent {
        return Err(not_installed(&package_id));
    }
    let observed_generation = observed.package_generation.ok_or_else(|| {
        PluginManagerError::Infrastructure(
            "current package observation omitted its Use-owned generation".to_string(),
        )
    })?;
    let expected_generation = request
        .expected_package_generation
        .unwrap_or(observed_generation);
    request.expected_package_generation = Some(expected_generation);
    let cognitive_request = CognitivePackageEnablementRequest::new(
        generated_operation_id(&request.component_id, request.enabled, expected_generation),
        package_id.clone(),
        expected_generation,
        request.enabled,
    )
    .map_err(use_request_error)?;
    let (planned, runtime) = match package_manager
        .prepare_enablement(&cognitive_request)
        .await
        .map_err(use_operation_error)?
    {
        CognitivePackageEnablementPreparation::Outcome(planned) => (*planned, None),
        CognitivePackageEnablementPreparation::Draft(draft) => {
            let provisional = authorization
                .provisional_authority()
                .map_err(use_operation_error)?;
            let (planned, runtime) = manager
                .runtime_host
                .bind_enablement_plan(*draft, provisional, |plan| {
                    authorization.evaluate_plan(plan)
                })
                .await
                .map_err(use_operation_error)?;
            (planned, Some(runtime))
        }
    };
    let result = match planned.status {
        CognitivePackageEnablementPlanStatus::NoChange => PluginEnablementPlanResult::no_change(
            &request,
            &package_id,
            planned.planned_at_ms,
            planned.state,
        )?,
        CognitivePackageEnablementPlanStatus::Planned => {
            let result = PluginEnablementPlanResult::planned(
                &request,
                &package_id,
                planned.planned_at_ms,
                planned.state,
                planned.plan.ok_or_else(plan_invalid)?,
            )?;
            let runtime = runtime.ok_or_else(plan_invalid)?;
            store::ReviewedEnablementStore::from_state_root(&manager.component_paths.state_root)
                .persist_plan(store::StoredEnablementPlan::new(result.clone(), runtime)?)
                .await?;
            result
        }
        CognitivePackageEnablementPlanStatus::Completed => {
            return Err(PluginManagerError::Infrastructure(
                "new reviewed enablement operation unexpectedly matched a completed Use operation"
                    .to_string(),
            ));
        }
    };
    serde_json::to_value(result).map_err(json_error)
}

pub(in crate::plugin_manager) async fn apply(
    manager: &PluginManager,
    request: &PluginEnablementApplyRequest,
    confirmed: bool,
) -> PluginManagerResult<Value> {
    let _mutation_guard = manager.operation_store.acquire_mutation_lock().await?;
    apply_locked(manager, request, confirmed, false)
        .await?
        .ok_or_else(|| {
            PluginManagerError::InvalidRequest(
                "reviewed plugin enablement plan was not found".to_string(),
            )
        })
}

pub(in crate::plugin_manager) async fn apply_if_present(
    manager: &PluginManager,
    request: &PluginEnablementApplyRequest,
    confirmed: bool,
) -> PluginManagerResult<Option<Value>> {
    let _mutation_guard = manager.operation_store.acquire_mutation_lock().await?;
    apply_locked(manager, request, confirmed, true).await
}

async fn apply_locked(
    manager: &PluginManager,
    request: &PluginEnablementApplyRequest,
    confirmed: bool,
    allow_absent: bool,
) -> PluginManagerResult<Option<Value>> {
    let request = request.normalize()?;
    let store =
        store::ReviewedEnablementStore::from_state_root(&manager.component_paths.state_root);
    let Some(plan_record) = store.plan(&request.operation_id).await? else {
        return if allow_absent {
            Ok(None)
        } else {
            Err(PluginManagerError::InvalidRequest(
                "reviewed plugin enablement plan was not found".to_string(),
            ))
        };
    };
    plan_record.validate()?;
    let plan = &plan_record.result;
    validate_apply_request(&request, plan)?;

    if let Some(result) = store.result(&request.operation_id, plan).await? {
        let intent = store
            .intent(&request.operation_id, plan)
            .await?
            .ok_or_else(store_invalid)?;
        verify_existing_confirmation(plan, confirmed, intent.confirmation.as_ref())?;
        if intent.request != request {
            return Err(operation_conflict());
        }
        return render_apply_result(plan, result.cognitive_result, true).map(Some);
    }

    let (intent, resumed) = match store.intent(&request.operation_id, plan).await? {
        Some(intent) => {
            verify_existing_confirmation(plan, confirmed, intent.confirmation.as_ref())?;
            if intent.request != request {
                return Err(operation_conflict());
            }
            (intent, true)
        }
        None => {
            let confirmation = confirmation_for_new_intent(manager, plan, confirmed)?;
            let intent = store::StoredEnablementApplyIntent::new(request.clone(), confirmation)?;
            store.persist_intent(intent.clone(), plan).await?;
            (intent, false)
        }
    };

    let envelope = plan.plan.as_ref().ok_or_else(plan_invalid)?;
    let selection = manager
        .runtime_host
        .reconstruct_enablement_selection(&envelope.plan, &plan_record.runtime)
        .await
        .map_err(use_operation_error)?;
    let lifecycle = manager
        .runtime_host
        .lifecycle_factory(selection)
        .map_err(use_operation_error)?;
    let cognitive = apply_reviewed_cognitive_enablement(
        envelope,
        intent.confirmation.as_ref(),
        plan.expected_package_generation,
        &manager.component_paths,
        lifecycle,
    )
    .await
    .map_err(|error| PluginManagerError::OperationFailed(error.to_string()))?;
    let cognitive_replayed = cognitive.replayed;
    let record = store::StoredEnablementApplyResult::new(request, cognitive.clone(), plan)?;
    let created = store.persist_result(record, plan).await?;
    render_apply_result(plan, cognitive, resumed || cognitive_replayed || !created).map(Some)
}

fn validate_apply_request(
    request: &PluginEnablementApplyRequest,
    plan: &PluginEnablementPlanResult,
) -> PluginManagerResult<()> {
    plan.validate()?;
    if plan.status != PluginEnablementPlanStatus::Planned
        || plan.operation_id.as_deref() != Some(request.operation_id.as_str())
        || plan.canonical_plan_digest.as_deref() != Some(request.plan_digest.as_str())
    {
        return Err(PluginManagerError::InvalidRequest(
            "reviewed enablement operation ID or plan digest does not match the stored plan"
                .to_string(),
        ));
    }
    Ok(())
}

fn confirmation_for_new_intent(
    manager: &PluginManager,
    plan: &PluginEnablementPlanResult,
    confirmed: bool,
) -> PluginManagerResult<Option<PluginOperationConfirmation>> {
    let envelope = plan.plan.as_ref().ok_or_else(plan_invalid)?;
    let evaluation = manager.verify_plan_authority(&envelope.plan)?;
    let applied_at_ms = now_ms()?;
    let confirmation = match evaluation.decision {
        PlanPolicyDecision::Ask if confirmed => Some(PluginOperationConfirmation {
            schema: PLUGIN_OPERATION_CONFIRMATION_SCHEMA.to_string(),
            operation_id: envelope.plan.operation_id.clone(),
            plan_digest: envelope.plan_digest.clone(),
            confirmed_by: PlanActor::User,
            confirmed_at_ms: applied_at_ms,
        }),
        PlanPolicyDecision::Allow => None,
        PlanPolicyDecision::Ask | PlanPolicyDecision::Deny => None,
    };
    envelope
        .verify_confirmed_apply(
            &envelope.plan.operation_id,
            &envelope.plan_digest,
            confirmation.as_ref(),
            applied_at_ms,
        )
        .map_err(|error| PluginManagerError::OperationFailed(error.to_string()))?;
    Ok(confirmation)
}

fn verify_existing_confirmation(
    plan: &PluginEnablementPlanResult,
    confirmed: bool,
    confirmation: Option<&PluginOperationConfirmation>,
) -> PluginManagerResult<()> {
    let envelope = plan.plan.as_ref().ok_or_else(plan_invalid)?;
    match (envelope.plan.authority.decision, confirmed, confirmation) {
        (PlanPolicyDecision::Ask, true, Some(confirmation)) => envelope
            .verify_confirmed_apply(
                &envelope.plan.operation_id,
                &envelope.plan_digest,
                Some(confirmation),
                confirmation.confirmed_at_ms,
            )
            .map_err(|error| PluginManagerError::Infrastructure(error.to_string())),
        (PlanPolicyDecision::Allow, _, None) => envelope
            .verify_confirmed_apply(
                &envelope.plan.operation_id,
                &envelope.plan_digest,
                None,
                envelope.plan.created_at_ms,
            )
            .map_err(|error| PluginManagerError::Infrastructure(error.to_string())),
        (PlanPolicyDecision::Ask, false, _) => Err(PluginManagerError::OperationFailed(
            "the reviewed plugin enablement plan requires exact user confirmation".to_string(),
        )),
        _ => Err(store_invalid()),
    }
}

fn render_apply_result(
    plan: &PluginEnablementPlanResult,
    mut result: CognitivePackageEnablementResult,
    replayed: bool,
) -> PluginManagerResult<Value> {
    result.replayed = replayed;
    let mut value = serde_json::to_value(result).map_err(json_error)?;
    let object = result_object(&mut value)?;
    object.insert(
        "componentId".to_string(),
        Value::String(plan.component_id.clone()),
    );
    object.insert(
        "canonicalPlanDigest".to_string(),
        Value::String(
            plan.canonical_plan_digest
                .clone()
                .ok_or_else(plan_invalid)?,
        ),
    );
    object.insert("durableEnablement".to_string(), Value::Bool(true));
    Ok(value)
}

fn cognitive_request(
    plan: &PluginEnablementPlanResult,
) -> PluginManagerResult<CognitivePackageEnablementRequest> {
    let envelope = plan.plan.as_ref().ok_or_else(plan_invalid)?;
    CognitivePackageEnablementRequest::new(
        envelope.plan.operation_id.clone(),
        plan.package_id.clone(),
        plan.expected_package_generation,
        plan.enabled,
    )
    .map_err(use_request_error)
}

fn generated_operation_id(component_id: &str, enabled: bool, generation: u64) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = ENABLEMENT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let identity = format!(
        "a3s-code-reviewed-enablement-v1\n{component_id}\n{enabled}\n{generation}\n{}\n{timestamp}\n{sequence}",
        std::process::id()
    );
    format!(
        "plugin-enablement:{:x}",
        Sha256::digest(identity.as_bytes())
    )
}

fn package_id(component_id: &str) -> PluginManagerResult<String> {
    component_id
        .strip_prefix("use/")
        .map(str::to_string)
        .ok_or_else(|| PluginManagerError::InvalidRequest("invalid Use package ID".to_string()))
}

fn result_object(value: &mut Value) -> PluginManagerResult<&mut Map<String, Value>> {
    value.as_object_mut().ok_or_else(|| {
        PluginManagerError::Infrastructure(
            "A3S Use enablement response must be a JSON object".to_string(),
        )
    })
}

fn now_ms() -> PluginManagerResult<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))
        .and_then(|duration| {
            u64::try_from(duration.as_millis()).map_err(|_| {
                PluginManagerError::Infrastructure("system clock is out of range".to_string())
            })
        })
}

fn not_installed(package_id: &str) -> PluginManagerError {
    PluginManagerError::OperationFailed(format!(
        "cognitive package '{package_id}' is not installed"
    ))
}

fn plan_invalid() -> PluginManagerError {
    PluginManagerError::Infrastructure(
        "durable reviewed plugin enablement plan is invalid".to_string(),
    )
}

fn store_invalid() -> PluginManagerError {
    PluginManagerError::Infrastructure(
        "durable reviewed plugin enablement store is invalid".to_string(),
    )
}

fn operation_conflict() -> PluginManagerError {
    PluginManagerError::InvalidRequest(
        "reviewed plugin enablement operation is bound to different durable evidence".to_string(),
    )
}

fn use_request_error(error: UseError) -> PluginManagerError {
    PluginManagerError::InvalidRequest(format!("{}: {}", error.code, error.message))
}

fn use_operation_error(error: UseError) -> PluginManagerError {
    PluginManagerError::OperationFailed(format!("{}: {}", error.code, error.message))
}

fn use_infrastructure_error(error: UseError) -> PluginManagerError {
    PluginManagerError::Infrastructure(format!("{}: {}", error.code, error.message))
}

fn json_error(error: serde_json::Error) -> PluginManagerError {
    PluginManagerError::Infrastructure(format!(
        "failed to encode reviewed cognitive-package enablement evidence: {error}"
    ))
}
