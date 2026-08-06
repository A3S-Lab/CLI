use serde::Serialize;
use serde_json::{Map, Value};

use a3s_use_core::{
    PlanPackageRole, PlanScope, PluginDesiredState, PluginHostApplyRequest, PluginHostApplyResult,
    PluginHostCapabilities, PluginHostPackageState, PluginHostPlanRequest, PluginHostPlanResult,
    PluginObservedState, PLUGIN_HOST_APPLY_RESULT_SCHEMA, PLUGIN_HOST_PLAN_RESULT_SCHEMA,
};
use olpc_cjson::CanonicalFormatter;
use sha2::{Digest, Sha256};

use super::capability::{
    installation_snapshot, observe, PluginCapabilityEvidence, PluginCapabilityEvidenceStatus,
};
use super::process::{
    normalize_plan_digest, normalize_plan_request, PluginApplyRequest, PluginLifecycleAction,
    PluginPlanRequest,
};
use super::{PluginManager, PluginManagerError, PluginManagerResult};

mod enablement;
pub(in crate::plugin_manager) mod lock;
mod plan_artifact;
mod planner;
pub(super) mod store;

use store::{
    NewPluginPlan, PluginOperationStore, StoredOperationResult, StoredPluginLifecycle,
    StoredPluginPlan, OPERATION_RECORD_SCHEMA,
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
    let scope = super::default_plan_scope();
    let stored = plan_record_locked(manager, request, actor, &scope, None).await?;
    reviewed_plan_output(&stored)
}

pub(in crate::plugin_manager) async fn plan_managed(
    manager: &PluginManager,
    request: &PluginHostPlanRequest,
    capabilities: &PluginHostCapabilities,
) -> PluginManagerResult<(PluginHostPlanResult, bool)> {
    let _mutation_guard = manager.operation_store.acquire_mutation_lock().await?;
    if let Some(stored) = manager.operation_store.find_managed_plan(request).await? {
        let result = managed_plan_result(&stored, true)?;
        result
            .validate_for(request, capabilities)
            .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))?;
        return Ok((result, true));
    }
    let manager_request = managed_manager_request(request)?;
    let scope = request.scope.plan_scope();
    let stored = plan_record_locked(
        manager,
        &manager_request,
        a3s_use_core::PlanActor::Agent,
        &scope,
        Some((request, capabilities)),
    )
    .await?;
    let result = managed_plan_result(&stored, false)?;
    result
        .validate_for(request, capabilities)
        .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))?;
    Ok((result, false))
}

async fn plan_record_locked(
    manager: &PluginManager,
    request: &PluginPlanRequest,
    actor: a3s_use_core::PlanActor,
    scope: &PlanScope,
    managed: Option<(&PluginHostPlanRequest, &PluginHostCapabilities)>,
) -> PluginManagerResult<StoredPluginPlan> {
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
        plan_artifact::HostPlanContext {
            authorization: manager.authorization_policy(),
            actor,
            scope,
            observed: plan_artifact::ObservedPlanState {
                capability: &capability_state,
                state_revision,
            },
            identity: &identity,
        },
        &request,
        upstream_plan_digest,
        raw_plan,
    )?;
    if let Some((managed_request, capabilities)) = managed {
        let envelope = prepared.plugin_operation_plan.as_ref().ok_or_else(|| {
            PluginManagerError::Upstream(
                "managed plugin planning did not produce a canonical operation plan".to_string(),
            )
        })?;
        let candidate = PluginHostPlanResult {
            schema: PLUGIN_HOST_PLAN_RESULT_SCHEMA.to_string(),
            request_id: managed_request.request_id.clone(),
            assignment_generation: managed_request.assignment_generation,
            capabilities_digest: managed_request.capabilities_digest.clone(),
            scope: managed_request.scope.clone(),
            package_id: managed_request.package_id.clone(),
            plan: envelope.clone(),
            replayed: false,
        };
        candidate
            .validate_for(managed_request, capabilities)
            .map_err(|error| {
                PluginManagerError::Upstream(format!(
                    "delegated managed plugin plan is invalid: {error}"
                ))
            })?;
    }
    let stored = manager
        .operation_store
        .create_plan_for_actor(NewPluginPlan {
            identity,
            request,
            actor,
            scope: scope.clone(),
            plan_digest: prepared.plan_digest,
            upstream_plan_digest: prepared.upstream_plan_digest,
            capability_state,
            plan: prepared.plan,
            plugin_operation_plan: prepared.plugin_operation_plan,
            managed_plan_request: managed.map(|(request, _)| request.clone()),
        })
        .await?;
    Ok(stored)
}

fn managed_manager_request(
    request: &PluginHostPlanRequest,
) -> PluginManagerResult<PluginPlanRequest> {
    let (version, channel) = request
        .candidate
        .as_ref()
        .map(|candidate| {
            (
                Some(candidate.record.version.clone()),
                Some(match candidate.record.channel {
                    a3s_use_core::PluginReleaseChannel::Stable => "stable".to_string(),
                    a3s_use_core::PluginReleaseChannel::Beta => "beta".to_string(),
                    a3s_use_core::PluginReleaseChannel::Nightly => "nightly".to_string(),
                }),
            )
        })
        .unwrap_or((None, None));
    let (version, channel) = if request.action == a3s_use_core::PluginOperationAction::Install {
        (version, channel)
    } else {
        (None, None)
    };
    normalize_plan_request(&PluginPlanRequest {
        action: match request.action {
            a3s_use_core::PluginOperationAction::Install => PluginLifecycleAction::Install,
            a3s_use_core::PluginOperationAction::Upgrade => PluginLifecycleAction::Upgrade,
            a3s_use_core::PluginOperationAction::Uninstall => PluginLifecycleAction::Uninstall,
        },
        component_id: request.package_id.component_id(),
        version,
        channel,
    })
}

fn managed_plan_result(
    stored: &StoredPluginPlan,
    replayed: bool,
) -> PluginManagerResult<PluginHostPlanResult> {
    let request = stored.managed_plan_request.as_ref().ok_or_else(|| {
        PluginManagerError::Infrastructure(
            "managed plugin plan omitted its durable host request".to_string(),
        )
    })?;
    let plan = stored.plugin_operation_plan.clone().ok_or_else(|| {
        PluginManagerError::Infrastructure(
            "managed plugin plan omitted its canonical operation envelope".to_string(),
        )
    })?;
    Ok(PluginHostPlanResult {
        schema: PLUGIN_HOST_PLAN_RESULT_SCHEMA.to_string(),
        request_id: request.request_id.clone(),
        assignment_generation: request.assignment_generation,
        capabilities_digest: request.capabilities_digest.clone(),
        scope: request.scope.clone(),
        package_id: request.package_id.clone(),
        plan,
        replayed,
    })
}

pub(super) async fn apply(
    manager: &PluginManager,
    request: &PluginApplyRequest,
    confirmed: bool,
) -> PluginManagerResult<Value> {
    let plan_digest = normalize_plan_digest(&request.plan_digest)?;
    let (operation_id, legacy_request) = apply_identity(request)?;
    let (_, result, replayed) = apply_record(
        manager,
        operation_id,
        legacy_request,
        plan_digest,
        ApplyAuthority::Local { confirmed },
    )
    .await?;
    rendered_result(result, replayed)
}

pub(in crate::plugin_manager) async fn apply_managed(
    manager: &PluginManager,
    request: &PluginHostApplyRequest,
    capabilities: &PluginHostCapabilities,
) -> PluginManagerResult<PluginHostApplyResult> {
    let plan_digest = normalize_plan_digest(&request.plan_digest)?;
    let (plan, result, replayed) = apply_record(
        manager,
        Some(request.operation_id.clone()),
        None,
        plan_digest,
        ApplyAuthority::Managed {
            request,
            capabilities,
        },
    )
    .await?;
    let plan_result = managed_plan_result(&plan, false)?;
    request
        .validate_for_plan(&plan_result, capabilities)
        .map_err(|error| PluginManagerError::InvalidRequest(error.to_string()))?;
    let outcome = managed_apply_result(request, &plan, &result, replayed)?;
    outcome
        .validate_for(request, capabilities)
        .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))?;
    Ok(outcome)
}

#[derive(Clone, Copy)]
enum ApplyAuthority<'a> {
    Local {
        confirmed: bool,
    },
    Managed {
        request: &'a PluginHostApplyRequest,
        capabilities: &'a PluginHostCapabilities,
    },
}

async fn apply_record(
    manager: &PluginManager,
    operation_id: Option<String>,
    legacy_request: Option<PluginPlanRequest>,
    plan_digest: String,
    authority: ApplyAuthority<'_>,
) -> PluginManagerResult<(StoredPluginPlan, StoredOperationResult, bool)> {
    let _mutation_guard = manager.operation_store.acquire_mutation_lock().await?;
    let plan = manager
        .operation_store
        .resolve_plan(operation_id, legacy_request, plan_digest.clone())
        .await?;
    validate_apply_authority_context(&plan, authority)?;
    if let ApplyAuthority::Managed {
        request,
        capabilities,
    } = authority
    {
        let plan_result = managed_plan_result(&plan, false)?;
        request
            .validate_for_plan(&plan_result, capabilities)
            .map_err(|error| PluginManagerError::InvalidRequest(error.to_string()))?;
    }

    if let Some(result) = manager.operation_store.result(&plan).await? {
        if let ApplyAuthority::Managed { request, .. } = authority {
            manager
                .operation_store
                .persist_managed_intent(&plan, request.clone())
                .await?;
        }
        return Ok((plan, result, true));
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
    let intent = match authority {
        ApplyAuthority::Local { confirmed } => {
            let confirmation = if intent_exists {
                None
            } else {
                verify_new_apply_authority(manager, &plan, confirmed, apply_started_at_ms)?
            };
            manager
                .operation_store
                .persist_intent_with_confirmation(&plan, confirmation)
                .await?
        }
        ApplyAuthority::Managed {
            request,
            capabilities,
        } => {
            if !intent_exists {
                verify_managed_apply_authority(
                    manager,
                    &plan,
                    request,
                    capabilities,
                    apply_started_at_ms,
                )?;
            }
            manager
                .operation_store
                .persist_managed_intent(&plan, request.clone())
                .await?
        }
    };
    manager
        .operation_store
        .begin_lifecycle(&plan, intent.started_at_ms)
        .await?;

    let raw_result = if let Some(operation_plan) = plan.plugin_operation_plan.as_ref() {
        let operation = crate::components::apply_reviewed_cognitive_package(
            operation_plan,
            intent.confirmation.as_ref(),
            &manager.component_paths,
            &manager.registry_store,
        )
        .await
        .map_err(|error| {
            PluginManagerError::OperationFailed(bounded_operation_error(&error.to_string()))
        })?;
        serde_json::json!({
            "planDigest": plan.upstream_plan_digest(),
            "operations": [operation],
        })
    } else {
        manager
            .process
            .apply(&plan.request, plan.upstream_plan_digest())
            .await?
    };
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
    Ok((plan, durable, !created))
}

fn validate_apply_authority_context(
    plan: &StoredPluginPlan,
    authority: ApplyAuthority<'_>,
) -> PluginManagerResult<()> {
    match (&plan.managed_plan_request, authority) {
        (None, ApplyAuthority::Local { .. }) if plan.scope == super::default_plan_scope() => Ok(()),
        (Some(stored), ApplyAuthority::Managed { request, .. })
            if stored.assignment_generation == request.assignment_generation
                && stored.capabilities_digest == request.capabilities_digest
                && stored.scope == request.scope
                && stored.package_id == request.package_id =>
        {
            Ok(())
        }
        (Some(_), ApplyAuthority::Local { .. }) => Err(PluginManagerError::InvalidRequest(
            "managed workspace plans can be applied only through the fenced PluginHostManager"
                .to_string(),
        )),
        (None, ApplyAuthority::Managed { .. }) => Err(PluginManagerError::InvalidRequest(
            "the managed apply request does not identify a managed workspace plan".to_string(),
        )),
        _ => Err(PluginManagerError::InvalidRequest(
            "the managed apply request scope or assignment does not match its plan".to_string(),
        )),
    }
}

fn rendered_result(result: StoredOperationResult, replayed: bool) -> PluginManagerResult<Value> {
    if replayed {
        replayed_result(result)
    } else {
        Ok(result.data)
    }
}

fn managed_apply_result(
    request: &PluginHostApplyRequest,
    plan: &StoredPluginPlan,
    result: &StoredOperationResult,
    replayed: bool,
) -> PluginManagerResult<PluginHostApplyResult> {
    let operation_plan = plan.plugin_operation_plan.as_ref().ok_or_else(|| {
        PluginManagerError::Infrastructure(
            "managed apply result omitted its canonical operation plan".to_string(),
        )
    })?;
    let state = managed_package_state(operation_plan, result)?;
    Ok(PluginHostApplyResult {
        schema: PLUGIN_HOST_APPLY_RESULT_SCHEMA.to_string(),
        request_id: request.request_id.clone(),
        assignment_generation: request.assignment_generation,
        capabilities_digest: request.capabilities_digest.clone(),
        scope: request.scope.clone(),
        package_id: request.package_id.clone(),
        operation_id: request.operation_id.clone(),
        plan_digest: request.plan_digest.clone(),
        completed_at_ms: result.completed_at_ms,
        operation_result_digest: canonical_value_digest(&result.data)?,
        state,
        replayed,
    })
}

fn managed_package_state(
    plan: &a3s_use_core::PluginOperationPlanEnvelope,
    result: &StoredOperationResult,
) -> PluginManagerResult<PluginHostPackageState> {
    let capability_generation = result.capability_after.generation.ok_or_else(|| {
        PluginManagerError::Infrastructure(
            "managed apply completed without a verified capability generation".to_string(),
        )
    })?;
    let capability_revision =
        prefixed_digest(result.capability_after.revision.as_deref().ok_or_else(|| {
            PluginManagerError::Infrastructure(
                "managed apply completed without a verified capability revision".to_string(),
            )
        })?)?;
    if plan.plan.action == a3s_use_core::PluginOperationAction::Uninstall {
        let state = PluginHostPackageState {
            version: None,
            package_generation: None,
            package_digest: None,
            manifest_digest: None,
            receipt_digest: None,
            capability_generation,
            capability_revision,
            desired: PluginDesiredState::Absent,
            observed: PluginObservedState::Removed,
            selected_surfaces: Vec::new(),
        };
        state
            .validate()
            .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))?;
        return Ok(state);
    }

    let root = plan
        .plan
        .packages
        .iter()
        .find(|package| package.role == PlanPackageRole::Root)
        .and_then(|package| package.after.as_ref())
        .ok_or_else(|| {
            PluginManagerError::Infrastructure(
                "managed apply plan omitted its installed root state".to_string(),
            )
        })?;
    let receipt_value = result
        .data
        .pointer("/operations/0/packageGraph/root/receipt")
        .cloned()
        .ok_or_else(|| {
            PluginManagerError::Infrastructure(
                "managed apply result omitted its installed root receipt".to_string(),
            )
        })?;
    let receipt: a3s_use_extension::ExtensionReceipt = serde_json::from_value(receipt_value)
        .map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "managed apply root receipt is invalid: {error}"
            ))
        })?;
    let package_digest = prefixed_digest(receipt.package_sha256.as_deref().ok_or_else(|| {
        PluginManagerError::Infrastructure(
            "managed apply root receipt omitted its package digest".to_string(),
        )
    })?)?;
    let manifest_digest = prefixed_digest(&receipt.manifest_sha256)?;
    let receipt_digest = receipt
        .descriptor_digest()
        .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))?;
    if receipt.package_id != plan.plan.package_id
        || receipt.version != root.release.version
        || package_digest != root.release.package_sha256
        || manifest_digest != root.release.manifest_sha256
    {
        return Err(PluginManagerError::Infrastructure(
            "managed apply receipt does not match its reviewed root transition".to_string(),
        ));
    }
    let mut selected_surfaces = root
        .release
        .surfaces
        .iter()
        .map(a3s_use_core::CatalogSurface::reference)
        .collect::<Vec<_>>();
    selected_surfaces.sort();
    selected_surfaces.dedup();
    let desired = if receipt.enabled {
        PluginDesiredState::Enabled
    } else {
        PluginDesiredState::InstalledDisabled
    };
    let state = PluginHostPackageState {
        version: Some(receipt.version),
        package_generation: receipt.lifecycle_generation,
        package_digest: Some(package_digest),
        manifest_digest: Some(manifest_digest),
        receipt_digest: Some(receipt_digest),
        capability_generation,
        capability_revision,
        desired,
        observed: if desired == PluginDesiredState::Enabled {
            PluginObservedState::Ready
        } else {
            PluginObservedState::Installed
        },
        selected_surfaces,
    };
    state
        .validate()
        .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))?;
    Ok(state)
}

fn prefixed_digest(value: &str) -> PluginManagerResult<String> {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(PluginManagerError::Infrastructure(
            "managed plugin evidence contains an invalid SHA-256 digest".to_string(),
        ));
    }
    Ok(format!("sha256:{value}"))
}

fn canonical_value_digest(value: &Value) -> PluginManagerResult<String> {
    let mut bytes = Vec::new();
    let mut serializer =
        serde_json::Serializer::with_formatter(&mut bytes, CanonicalFormatter::new());
    value.serialize(&mut serializer).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to canonicalize managed operation result: {error}"
        ))
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
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

fn verify_managed_apply_authority(
    manager: &PluginManager,
    plan: &StoredPluginPlan,
    request: &PluginHostApplyRequest,
    capabilities: &PluginHostCapabilities,
    now_ms: u64,
) -> PluginManagerResult<()> {
    let operation_plan = plan.plugin_operation_plan.as_ref().ok_or_else(|| {
        PluginManagerError::InvalidRequest(
            "managed apply requires a canonical plugin operation plan".to_string(),
        )
    })?;
    manager.verify_plan_authority(&operation_plan.plan)?;
    let plan_result = managed_plan_result(plan, false)?;
    request
        .verify_apply_for_plan(&plan_result, capabilities, now_ms)
        .map_err(|error| PluginManagerError::OperationFailed(error.to_string()))
}

pub(super) use enablement::set_enabled;

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
        serde_json::to_value(&plan.scope).map_err(json_error)?,
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

fn bounded_operation_error(value: &str) -> String {
    const MAX_ERROR_CHARACTERS: usize = 1_000;

    let value = value.trim().replace(['\n', '\r'], " ");
    let count = value.chars().count();
    let mut bounded = value
        .chars()
        .take(if count > MAX_ERROR_CHARACTERS {
            MAX_ERROR_CHARACTERS - 1
        } else {
            MAX_ERROR_CHARACTERS
        })
        .collect::<String>();
    if count > MAX_ERROR_CHARACTERS {
        bounded.push('…');
    }
    if bounded.is_empty() {
        "reviewed cognitive-package operation failed".to_string()
    } else {
        bounded
    }
}

#[cfg(test)]
mod tests;
