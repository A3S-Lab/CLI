use serde_json::{Map, Value};

use super::capability::{
    installation_snapshot, observe, PluginCapabilityEvidence, PluginCapabilityEvidenceStatus,
};
use super::process::{
    normalize_plan_digest, normalize_plan_request, PluginApplyRequest, PluginLifecycleAction,
    PluginPackageToggleRequest, PluginPlanRequest,
};
use super::{PluginManager, PluginManagerError, PluginManagerResult};

mod lock;
mod plan_artifact;
mod planner;
pub(super) mod store;

use store::{
    PluginOperationStore, StoredOperationResult, StoredPluginLifecycle, StoredPluginPlan,
    OPERATION_RECORD_SCHEMA,
};

pub(super) fn store(state_root: &std::path::Path) -> PluginOperationStore {
    PluginOperationStore::new(state_root.join("plugin-manager/operations"))
}

pub(super) async fn plan(
    manager: &PluginManager,
    request: &PluginPlanRequest,
    actor: a3s_use_core::PlanActor,
) -> PluginManagerResult<Value> {
    let _mutation_guard = manager.operation_store.acquire_mutation_lock().await?;
    let request = normalize_plan_request(request)?;
    let installation_state = installation_snapshot(manager).await;
    let capability_state = installation_state.evidence();
    let installed_planning_evidence = if request.action != PluginLifecycleAction::Install
        && installation_state.items.iter().any(|item| {
            item.component_id == request.component_id && item.planner_evidence.is_some()
        }) {
        Some(
            manager
                .process
                .installed_planning_evidence(&request.component_id)
                .await?,
        )
    } else {
        None
    };
    let raw_plan = manager.process.plan(&request).await?;
    let upstream_plan_digest = plan_digest_from_value(&raw_plan)?;
    let state_revision = manager.operation_store.planner_state_revision().await?;
    let raw_plan = planner::attach_draft(
        &request,
        &installation_state,
        installed_planning_evidence.as_ref(),
        state_revision,
        raw_plan,
    )?;
    let identity = manager
        .operation_store
        .allocate_plan_identity(request.action)
        .await?;
    let prepared = plan_artifact::prepare(
        manager.authorization_policy(),
        &request,
        actor,
        plan_artifact::ObservedPlanState {
            capability: &capability_state,
            state_revision,
        },
        &identity,
        upstream_plan_digest,
        raw_plan,
    )?;
    let stored = manager
        .operation_store
        .create_plan_for_actor(
            identity,
            request,
            actor,
            prepared.plan_digest,
            prepared.upstream_plan_digest,
            capability_state,
            prepared.plan,
            prepared.plugin_operation_plan,
        )
        .await?;
    reviewed_plan_output(&stored)
}

pub(super) async fn apply(
    manager: &PluginManager,
    request: &PluginApplyRequest,
    confirmed: bool,
) -> PluginManagerResult<Value> {
    let plan_digest = normalize_plan_digest(&request.plan_digest)?;
    let (operation_id, legacy_request) = apply_identity(request)?;
    let _mutation_guard = manager.operation_store.acquire_mutation_lock().await?;
    let plan = manager
        .operation_store
        .resolve_plan(operation_id, legacy_request, plan_digest.clone())
        .await?;

    if let Some(result) = manager.operation_store.result(&plan).await? {
        return replayed_result(result);
    }

    let intent_exists = manager.operation_store.has_intent(&plan).await?;
    let apply_started_at_ms = unix_time_millis()?;
    if !intent_exists && apply_started_at_ms > plan.expires_at_ms {
        return Err(PluginManagerError::OperationFailed(
            "the reviewed plugin plan expired; create and review a new plan".to_string(),
        ));
    }

    let capability_before = observe(manager).await;
    ensure_capability_precondition(&plan.capability_state, &capability_before, intent_exists)?;
    manager
        .operation_store
        .verify_planner_state(&plan, intent_exists)
        .await?;
    let confirmation = if intent_exists {
        None
    } else {
        verify_new_apply_authority(manager, &plan, confirmed, apply_started_at_ms)?
    };
    let intent = manager
        .operation_store
        .persist_intent_with_confirmation(&plan, confirmation)
        .await?;
    manager
        .operation_store
        .begin_lifecycle(&plan, intent.started_at_ms)
        .await?;

    let raw_result = manager
        .process
        .apply(&plan.request, plan.upstream_plan_digest())
        .await?;
    validate_apply_result(&raw_result, plan.upstream_plan_digest())?;
    let capability_after = observe(manager).await;
    manager
        .operation_store
        .verify_lifecycle_observation(&plan, &capability_after)
        .await?;
    let state_revision_after = manager.operation_store.advance_planner_state(&plan).await?;
    let observed_completed_at_ms = unix_time_millis()?;
    let lifecycle = manager
        .operation_store
        .complete_lifecycle(
            &plan,
            &capability_after,
            state_revision_after,
            observed_completed_at_ms,
        )
        .await?;
    let completed_at_ms = lifecycle_completed_at_ms(lifecycle.as_ref(), observed_completed_at_ms)?;
    let data = applied_output(
        raw_result,
        &plan,
        AppliedStateEvidence {
            capability_before: &capability_before,
            capability_after: &capability_after,
            state_revision_after,
            lifecycle: lifecycle.as_ref(),
        },
        completed_at_ms,
        intent.resumed,
        false,
    )?;
    let result = StoredOperationResult {
        schema: OPERATION_RECORD_SCHEMA.to_string(),
        operation_id: plan.operation_id.clone(),
        plan_digest,
        completed_at_ms,
        capability_before,
        capability_after,
        data,
    };
    let (durable, created) = manager.operation_store.persist_result(result).await?;
    if created {
        Ok(durable.data)
    } else {
        replayed_result(durable)
    }
}

fn verify_new_apply_authority(
    manager: &PluginManager,
    plan: &StoredPluginPlan,
    confirmed: bool,
    now_ms: u64,
) -> PluginManagerResult<Option<a3s_use_core::PluginOperationConfirmation>> {
    let Some(operation_plan) = &plan.plugin_operation_plan else {
        return Ok(None);
    };
    let evaluation = manager.verify_plan_authority(&operation_plan.plan)?;
    let confirmation = match evaluation.decision {
        a3s_use_core::PlanPolicyDecision::Allow => None,
        a3s_use_core::PlanPolicyDecision::Ask if confirmed => {
            Some(a3s_use_core::PluginOperationConfirmation {
                schema: a3s_use_core::PLUGIN_OPERATION_CONFIRMATION_SCHEMA.to_string(),
                operation_id: plan.operation_id.clone(),
                plan_digest: operation_plan.plan_digest.clone(),
                confirmed_by: a3s_use_core::PlanActor::User,
                confirmed_at_ms: now_ms,
            })
        }
        a3s_use_core::PlanPolicyDecision::Ask | a3s_use_core::PlanPolicyDecision::Deny => None,
    };
    operation_plan
        .verify_confirmed_apply(
            &plan.operation_id,
            &operation_plan.plan_digest,
            confirmation.as_ref(),
            now_ms,
        )
        .map_err(|error| PluginManagerError::OperationFailed(error.to_string()))?;
    Ok(confirmation)
}

pub(super) async fn set_enabled(
    manager: &PluginManager,
    request: &PluginPackageToggleRequest,
) -> PluginManagerResult<Value> {
    let _mutation_guard = manager.operation_store.acquire_mutation_lock().await?;
    let capability_before = observe(manager).await;
    let mut result = manager.process.set_enabled(request).await?;
    let capability_after = observe(manager).await;
    let object = result_object(&mut result)?;
    insert_manager_field(
        object,
        "capabilityBefore",
        serde_json::to_value(capability_before).map_err(json_error)?,
    );
    insert_manager_field(
        object,
        "capabilityAfter",
        serde_json::to_value(capability_after).map_err(json_error)?,
    );
    insert_manager_field(object, "replayed", Value::Bool(false));
    Ok(result)
}

fn reviewed_plan_output(plan: &StoredPluginPlan) -> PluginManagerResult<Value> {
    let mut output = plan.plan.clone();
    let object = result_object(&mut output)?;
    insert_manager_field(
        object,
        "operationId",
        Value::String(plan.operation_id.clone()),
    );
    insert_manager_field(
        object,
        "canonicalPlanDigest",
        Value::String(format!("sha256:{}", plan.plan_digest)),
    );
    insert_manager_field(
        object,
        "createdAtMs",
        Value::Number(plan.created_at_ms.into()),
    );
    insert_manager_field(
        object,
        "expiresAtMs",
        Value::Number(plan.expires_at_ms.into()),
    );
    insert_manager_field(
        object,
        "capabilityState",
        serde_json::to_value(&plan.capability_state).map_err(json_error)?,
    );
    insert_manager_field(
        object,
        "actor",
        serde_json::to_value(plan.actor).map_err(json_error)?,
    );
    insert_manager_field(
        object,
        "scope",
        serde_json::json!({
            "kind": "user",
            "id": "current",
        }),
    );
    Ok(output)
}

struct AppliedStateEvidence<'a> {
    capability_before: &'a PluginCapabilityEvidence,
    capability_after: &'a PluginCapabilityEvidence,
    state_revision_after: u64,
    lifecycle: Option<&'a StoredPluginLifecycle>,
}

fn applied_output(
    mut output: Value,
    plan: &StoredPluginPlan,
    state: AppliedStateEvidence<'_>,
    completed_at_ms: u64,
    resumed: bool,
    replayed: bool,
) -> PluginManagerResult<Value> {
    let object = result_object(&mut output)?;
    insert_manager_field(
        object,
        "planDigest",
        Value::String(plan.plan_digest.clone()),
    );
    insert_manager_field(
        object,
        "operationId",
        Value::String(plan.operation_id.clone()),
    );
    insert_manager_field(
        object,
        "canonicalPlanDigest",
        Value::String(format!("sha256:{}", plan.plan_digest)),
    );
    insert_manager_field(
        object,
        "capabilityBefore",
        serde_json::to_value(state.capability_before).map_err(json_error)?,
    );
    insert_manager_field(
        object,
        "capabilityAfter",
        serde_json::to_value(state.capability_after).map_err(json_error)?,
    );
    insert_manager_field(
        object,
        "completedAtMs",
        Value::Number(completed_at_ms.into()),
    );
    insert_manager_field(object, "resumed", Value::Bool(resumed));
    insert_manager_field(object, "replayed", Value::Bool(replayed));
    insert_manager_field(
        object,
        "stateRevisionAfter",
        Value::Number(state.state_revision_after.into()),
    );
    if let Some(lifecycle) = state.lifecycle {
        let cutover = lifecycle.cutover.as_ref().ok_or_else(|| {
            PluginManagerError::Infrastructure(
                "completed plugin lifecycle evidence has no capability cutover".to_string(),
            )
        })?;
        insert_manager_field(
            object,
            "lifecycleBindingDigest",
            Value::String(lifecycle.binding.binding_digest().to_string()),
        );
        insert_manager_field(
            object,
            "lifecycleCutoverDigest",
            Value::String(cutover.cutover_digest().to_string()),
        );
        insert_manager_field(
            object,
            "capabilitySnapshotDigest",
            Value::String(cutover.capability_snapshot_digest().to_string()),
        );
    }
    object.remove("pluginOperationPlanDigest");
    object.remove("authority");
    if let Some(operation_plan) = &plan.plugin_operation_plan {
        insert_manager_field(
            object,
            "pluginOperationPlanDigest",
            Value::String(operation_plan.plan_digest.clone()),
        );
        insert_manager_field(
            object,
            "authority",
            serde_json::to_value(&operation_plan.plan.authority).map_err(json_error)?,
        );
    }
    Ok(output)
}

fn lifecycle_completed_at_ms(
    lifecycle: Option<&StoredPluginLifecycle>,
    observed_completed_at_ms: u64,
) -> PluginManagerResult<u64> {
    lifecycle
        .map(|record| {
            record
                .cutover
                .as_ref()
                .map(|cutover| cutover.committed_at_ms())
                .ok_or_else(|| {
                    PluginManagerError::Infrastructure(
                        "completed plugin lifecycle evidence has no capability cutover".to_string(),
                    )
                })
        })
        .transpose()
        .map(|completed_at_ms| completed_at_ms.unwrap_or(observed_completed_at_ms))
}

fn replayed_result(result: StoredOperationResult) -> PluginManagerResult<Value> {
    let mut data = result.data;
    let object = result_object(&mut data)?;
    object.insert("replayed".to_string(), Value::Bool(true));
    Ok(data)
}

fn apply_identity(
    request: &PluginApplyRequest,
) -> PluginManagerResult<(Option<String>, Option<PluginPlanRequest>)> {
    match request.operation_id.as_deref() {
        Some(operation_id) => {
            if request.action.is_some()
                || request.component_id.is_some()
                || request.version.is_some()
                || request.channel.is_some()
            {
                return Err(PluginManagerError::InvalidRequest(
                    "operationId apply requests cannot include legacy action, componentId, version, or channel fields".to_string(),
                ));
            }
            Ok((Some(operation_id.to_string()), None))
        }
        None => {
            let action = request.action.ok_or_else(|| {
                PluginManagerError::InvalidRequest(
                    "legacy plugin apply requires action".to_string(),
                )
            })?;
            let component_id = request.component_id.clone().ok_or_else(|| {
                PluginManagerError::InvalidRequest(
                    "legacy plugin apply requires componentId".to_string(),
                )
            })?;
            Ok((
                None,
                Some(PluginPlanRequest {
                    action,
                    component_id,
                    version: request.version.clone(),
                    channel: request.channel.clone(),
                }),
            ))
        }
    }
}

fn ensure_capability_state_unchanged(
    planned: &PluginCapabilityEvidence,
    current: &PluginCapabilityEvidence,
) -> PluginManagerResult<()> {
    let unchanged = match planned.status {
        PluginCapabilityEvidenceStatus::Verified => planned.same_registry_state(current),
        PluginCapabilityEvidenceStatus::Unavailable => {
            current.status == PluginCapabilityEvidenceStatus::Unavailable
        }
    };
    if !unchanged {
        return Err(PluginManagerError::OperationFailed(
            "the A3S Use capability availability, generation, or revision changed after review; create a new plugin plan"
                .to_string(),
        ));
    }
    Ok(())
}

fn ensure_capability_precondition(
    planned: &PluginCapabilityEvidence,
    current: &PluginCapabilityEvidence,
    intent_exists: bool,
) -> PluginManagerResult<()> {
    if intent_exists {
        return Ok(());
    }
    ensure_capability_state_unchanged(planned, current)
}

fn plan_digest_from_value(value: &Value) -> PluginManagerResult<String> {
    let digest = value
        .get("planDigest")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            PluginManagerError::Upstream("a3s plugin plan response has no planDigest".to_string())
        })?;
    normalize_plan_digest(digest)
}

fn validate_apply_result(value: &Value, plan_digest: &str) -> PluginManagerResult<()> {
    if !value.is_object()
        || value.get("planDigest").and_then(Value::as_str) != Some(plan_digest)
        || !value.get("operations").is_some_and(Value::is_array)
    {
        return Err(PluginManagerError::Upstream(
            "a3s plugin apply response does not match the reviewed plan".to_string(),
        ));
    }
    Ok(())
}

fn result_object(value: &mut Value) -> PluginManagerResult<&mut Map<String, Value>> {
    value.as_object_mut().ok_or_else(|| {
        PluginManagerError::Upstream("a3s plugin response must be a JSON object".to_string())
    })
}

fn insert_manager_field(object: &mut Map<String, Value>, key: &'static str, value: Value) {
    object.insert(key.to_string(), value);
}

fn unix_time_millis() -> PluginManagerResult<u64> {
    u64::try_from(chrono::Utc::now().timestamp_millis()).map_err(|error| {
        PluginManagerError::Infrastructure(format!("system time is before the Unix epoch: {error}"))
    })
}

fn json_error(error: serde_json::Error) -> PluginManagerError {
    PluginManagerError::Infrastructure(format!(
        "failed to encode plugin operation evidence: {error}"
    ))
}

#[cfg(test)]
mod tests;
