use serde_json::{Map, Value};

use super::capability::{observe, PluginCapabilityEvidence, PluginCapabilityEvidenceStatus};
use super::process::{
    normalize_plan_digest, normalize_plan_request, PluginApplyRequest, PluginPackageToggleRequest,
    PluginPlanRequest,
};
use super::{PluginManager, PluginManagerError, PluginManagerResult};

mod lock;
pub(super) mod store;

use store::{
    PluginOperationStore, StoredOperationResult, StoredPluginPlan, OPERATION_RECORD_SCHEMA,
};

pub(super) fn store(state_root: &std::path::Path) -> PluginOperationStore {
    PluginOperationStore::new(state_root.join("plugin-manager/operations"))
}

pub(super) async fn plan(
    manager: &PluginManager,
    request: &PluginPlanRequest,
) -> PluginManagerResult<Value> {
    let _mutation_guard = manager.operation_store.acquire_mutation_lock().await?;
    let request = normalize_plan_request(request)?;
    let capability_state = observe(manager).await;
    let raw_plan = manager.process.plan(&request).await?;
    let plan_digest = plan_digest_from_value(&raw_plan)?;
    let stored = manager
        .operation_store
        .create_plan(request, plan_digest.clone(), capability_state, raw_plan)
        .await?;
    reviewed_plan_output(&stored)
}

pub(super) async fn apply(
    manager: &PluginManager,
    request: &PluginApplyRequest,
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
    if !intent_exists && unix_time_millis()? > plan.expires_at_ms {
        return Err(PluginManagerError::OperationFailed(
            "the reviewed plugin plan expired; create and review a new plan".to_string(),
        ));
    }

    let capability_before = observe(manager).await;
    ensure_capability_precondition(&plan.capability_state, &capability_before, intent_exists)?;
    let resumed = manager.operation_store.persist_intent(&plan).await?;

    let raw_result = manager
        .process
        .apply(&plan.request, &plan.plan_digest)
        .await?;
    validate_apply_result(&raw_result, &plan.plan_digest)?;
    let capability_after = observe(manager).await;
    let completed_at_ms = unix_time_millis()?;
    let data = applied_output(
        raw_result,
        &plan,
        &capability_before,
        &capability_after,
        completed_at_ms,
        resumed,
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
    Ok(output)
}

fn applied_output(
    mut output: Value,
    plan: &StoredPluginPlan,
    capability_before: &PluginCapabilityEvidence,
    capability_after: &PluginCapabilityEvidence,
    completed_at_ms: u64,
    resumed: bool,
    replayed: bool,
) -> PluginManagerResult<Value> {
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
        "capabilityBefore",
        serde_json::to_value(capability_before).map_err(json_error)?,
    );
    insert_manager_field(
        object,
        "capabilityAfter",
        serde_json::to_value(capability_after).map_err(json_error)?,
    );
    insert_manager_field(
        object,
        "completedAtMs",
        Value::Number(completed_at_ms.into()),
    );
    insert_manager_field(object, "resumed", Value::Bool(resumed));
    insert_manager_field(object, "replayed", Value::Bool(replayed));
    Ok(output)
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
mod tests {
    use super::*;
    use crate::plugin_manager::process::PluginLifecycleAction;

    #[test]
    fn operation_id_apply_rejects_legacy_identity_fields() {
        let request = PluginApplyRequest {
            operation_id: Some("plugin-install-abc".to_string()),
            action: Some(PluginLifecycleAction::Install),
            component_id: None,
            version: None,
            channel: None,
            plan_digest: "a".repeat(64),
        };

        assert!(apply_identity(&request).is_err());
    }

    #[test]
    fn verified_capability_drift_fails_before_mutation() {
        let planned = PluginCapabilityEvidence {
            status: PluginCapabilityEvidenceStatus::Verified,
            observed_at_ms: 1,
            generation: Some(7),
            revision: Some("a".repeat(64)),
            error: None,
        };
        let current = PluginCapabilityEvidence {
            status: PluginCapabilityEvidenceStatus::Verified,
            observed_at_ms: 2,
            generation: Some(8),
            revision: Some("b".repeat(64)),
            error: None,
        };

        assert!(ensure_capability_state_unchanged(&planned, &current).is_err());
        assert!(ensure_capability_precondition(&planned, &current, true).is_ok());
    }

    #[test]
    fn newly_available_capability_state_requires_a_new_plan() {
        let planned = PluginCapabilityEvidence {
            status: PluginCapabilityEvidenceStatus::Unavailable,
            observed_at_ms: 1,
            generation: None,
            revision: None,
            error: Some("A3S Use is not ready".to_string()),
        };
        let current = PluginCapabilityEvidence {
            status: PluginCapabilityEvidenceStatus::Verified,
            observed_at_ms: 2,
            generation: Some(1),
            revision: Some("a".repeat(64)),
            error: None,
        };

        assert!(ensure_capability_state_unchanged(&planned, &current).is_err());
    }

    #[test]
    fn operation_id_apply_accepts_a_canonical_prefixed_digest() {
        let digest = "c".repeat(64);
        let request: PluginApplyRequest = serde_json::from_value(serde_json::json!({
            "operationId": "plugin-install-abc",
            "planDigest": format!("sha256:{digest}"),
        }))
        .unwrap();

        let (operation_id, legacy_request) = apply_identity(&request).unwrap();

        assert_eq!(operation_id.as_deref(), Some("plugin-install-abc"));
        assert!(legacy_request.is_none());
        assert_eq!(normalize_plan_digest(&request.plan_digest).unwrap(), digest);
    }
}
