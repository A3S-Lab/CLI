use std::fmt::Write as _;
use std::path::Path;

use rand::RngCore;
use serde_json::Value;

use super::super::super::capability::{PluginCapabilityEvidence, PluginCapabilityEvidenceStatus};
use super::super::super::process::{
    normalize_plan_request, PluginLifecycleAction, PluginPlanRequest,
};
use super::super::super::{PluginManagerError, PluginManagerResult};
use super::{
    invalid_store, StoredApplyIntent, StoredOperationResult, StoredPluginPlan,
    OPERATION_RECORD_SCHEMA, PLAN_LIFETIME_MS,
};

pub(super) fn validate_plan_record(record: &StoredPluginPlan) -> PluginManagerResult<()> {
    if record.schema != OPERATION_RECORD_SCHEMA {
        return Err(invalid_store("reviewed plan schema is unsupported"));
    }
    validate_operation_id(&record.operation_id)?;
    validate_digest(&record.plan_digest)?;
    validate_digest(record.upstream_plan_digest())?;
    ensure_request_valid(&record.request)?;
    validate_capability_evidence(&record.capability_state)?;
    validate_plan_value(&record.plan, &record.plan_digest)?;
    if record.registry_source_revision != registry_source_revision(&record.plan)? {
        return Err(invalid_store(
            "reviewed plan Registry source revision does not match its payload",
        ));
    }
    validate_plugin_operation_plan(record)?;
    if record.created_at_ms == 0
        || record.expires_at_ms <= record.created_at_ms
        || record.expires_at_ms - record.created_at_ms > PLAN_LIFETIME_MS
    {
        return Err(invalid_store("reviewed plan lifetime is invalid"));
    }
    Ok(())
}

pub(super) fn validate_intent(
    intent: &StoredApplyIntent,
    plan: &StoredPluginPlan,
) -> PluginManagerResult<()> {
    if intent.schema != OPERATION_RECORD_SCHEMA
        || intent.operation_id != plan.operation_id
        || intent.plan_digest != plan.plan_digest
        || intent.started_at_ms < plan.created_at_ms
        || intent.started_at_ms > plan.expires_at_ms
    {
        return Err(invalid_store("plugin apply intent is invalid"));
    }
    match &plan.plugin_operation_plan {
        Some(operation_plan) => operation_plan
            .verify_confirmed_apply(
                &plan.operation_id,
                &operation_plan.plan_digest,
                intent.confirmation.as_ref(),
                intent.started_at_ms,
            )
            .map_err(|error| {
                invalid_store(format!("plugin apply intent authority is invalid: {error}"))
            })?,
        None if intent.confirmation.is_some() => {
            return Err(invalid_store(
                "an umbrella-only plugin apply intent contains unrelated confirmation",
            ));
        }
        None => {}
    }
    match (&plan.managed_plan_request, &intent.managed_apply_request) {
        (Some(plan_request), Some(apply_request)) => {
            plan_request.validate().map_err(|error| {
                invalid_store(format!("managed plan request is invalid: {error}"))
            })?;
            apply_request.validate().map_err(|error| {
                invalid_store(format!("managed apply request is invalid: {error}"))
            })?;
            if apply_request.assignment_generation != plan_request.assignment_generation
                || apply_request.capabilities_digest != plan_request.capabilities_digest
                || apply_request.scope != plan_request.scope
                || apply_request.package_id != plan_request.package_id
                || apply_request.operation_id != plan.operation_id
                || apply_request.plan_digest
                    != plan
                        .plugin_operation_plan
                        .as_ref()
                        .map(|envelope| envelope.plan_digest.as_str())
                        .unwrap_or_default()
                || apply_request.confirmation != intent.confirmation
            {
                return Err(invalid_store(
                    "managed apply intent does not bind its exact reviewed plan",
                ));
            }
        }
        (None, None) => {}
        _ => {
            return Err(invalid_store(
                "managed plan and apply intent ownership disagree",
            ))
        }
    }
    Ok(())
}

fn validate_plugin_operation_plan(record: &StoredPluginPlan) -> PluginManagerResult<()> {
    let Some(envelope) = &record.plugin_operation_plan else {
        if record.lifecycle_required {
            return Err(invalid_store(
                "reviewed plan requires lifecycle evidence but omits its canonical operation plan",
            ));
        }
        if record.plan.get("pluginOperationPlan").is_some()
            || record.plan.get("pluginOperationPlanDigest").is_some()
            || !record.planning_bundles.is_empty()
            || record.grant_snapshot.is_some()
            || record.managed_plan_request.is_some()
            || record.scope != super::super::super::default_plan_scope()
        {
            return Err(invalid_store(
                "reviewed plan payload contains an unbound plugin operation plan",
            ));
        }
        return Ok(());
    };
    envelope
        .validate()
        .map_err(|error| invalid_store(format!("plugin operation plan is invalid: {error}")))?;
    validate_cognitive_planning_evidence(record, envelope)?;
    let envelope_digest = envelope
        .plan_digest
        .strip_prefix("sha256:")
        .unwrap_or(&envelope.plan_digest);
    if validate_digest(envelope_digest).is_err() {
        return Err(invalid_store(
            "plugin operation plan digest is not canonical",
        ));
    }
    let expected_action = match record.request.action {
        PluginLifecycleAction::Install => a3s_use_core::PluginOperationAction::Install,
        PluginLifecycleAction::Upgrade => a3s_use_core::PluginOperationAction::Upgrade,
        PluginLifecycleAction::Uninstall => a3s_use_core::PluginOperationAction::Uninstall,
    };
    let source_revision_required = envelope.package_lock.is_some()
        && matches!(
            expected_action,
            a3s_use_core::PluginOperationAction::Install
                | a3s_use_core::PluginOperationAction::Upgrade
        );
    if source_revision_required != record.registry_source_revision.is_some() {
        return Err(invalid_store(
            "reviewed cognitive-package plan has inconsistent Registry source revision evidence",
        ));
    }
    let expected_package_id = record
        .request
        .component_id
        .strip_prefix("use/")
        .unwrap_or_default();
    let serialized_plan = serde_json::to_value(&envelope.plan).map_err(|error| {
        invalid_store(format!(
            "plugin operation plan could not be encoded: {error}"
        ))
    })?;
    if record.upstream_plan_digest.is_none()
        || envelope_digest != record.plan_digest
        || envelope.plan.operation_id != record.operation_id
        || envelope.plan.created_at_ms != record.created_at_ms
        || envelope.plan.expires_at_ms != record.expires_at_ms
        || envelope.plan.action != expected_action
        || envelope.plan.package_id != expected_package_id
        || envelope.plan.authority.actor != record.actor
        || envelope.plan.scope != record.scope
        || record.capability_state.status != PluginCapabilityEvidenceStatus::Verified
        || envelope.plan.state.capability_generation
            != record.capability_state.generation.unwrap_or(0)
        || record.plan.get("pluginOperationPlan") != Some(&serialized_plan)
        || record
            .plan
            .get("pluginOperationPlanDigest")
            .and_then(Value::as_str)
            != Some(envelope.plan_digest.as_str())
    {
        return Err(invalid_store(
            "plugin operation plan does not match its reviewed Manager record",
        ));
    }
    match &record.managed_plan_request {
        Some(request) => {
            request.validate().map_err(|error| {
                invalid_store(format!("managed plan request is invalid: {error}"))
            })?;
            if request.scope.plan_scope() != record.scope
                || request.package_id.as_str() != expected_package_id
                || request.action != expected_action
                || request.package_lock != envelope.package_lock
            {
                return Err(invalid_store(
                    "managed plan request does not bind its Manager record",
                ));
            }
        }
        None if record.scope == super::super::super::default_plan_scope() => {}
        None => {
            return Err(invalid_store(
                "a workspace-scoped Manager record has no managed-scope request",
            ))
        }
    }
    Ok(())
}

fn validate_cognitive_planning_evidence(
    record: &StoredPluginPlan,
    envelope: &a3s_use_core::PluginOperationPlanEnvelope,
) -> PluginManagerResult<()> {
    let Some(lock) = envelope.package_lock.as_ref() else {
        if !record.planning_bundles.is_empty() || record.grant_snapshot.is_some() {
            return Err(invalid_store(
                "a non-cognitive plan contains cognitive planning evidence",
            ));
        }
        return Ok(());
    };
    let snapshot = record
        .grant_snapshot
        .as_ref()
        .ok_or_else(|| invalid_store("a cognitive plan omitted its durable Grant snapshot"))?;
    snapshot
        .validate()
        .map_err(|error| invalid_store(format!("Grant snapshot is invalid: {error}")))?;
    if snapshot.scope_id != envelope.plan.scope.id
        || snapshot.state_revision != envelope.plan.state.state_revision
    {
        return Err(invalid_store(
            "the durable Grant snapshot does not bind its cognitive plan",
        ));
    }
    a3s_use::cognitive_package::reconstruct_cognitive_package_grants(&envelope.plan, snapshot)
        .map_err(|error| {
            invalid_store(format!(
                "the durable Grant snapshot cannot reconstruct its reviewed plan: {error}"
            ))
        })?;

    if envelope.plan.action == a3s_use_core::PluginOperationAction::Uninstall {
        if !record.planning_bundles.is_empty() {
            return Err(invalid_store(
                "an uninstall plan contains candidate executable planning bundles",
            ));
        }
        return Ok(());
    }
    let expected = lock
        .packages
        .iter()
        .filter(|package| package.catalog.record.planning.is_some())
        .map(|package| package.package_id())
        .collect::<std::collections::BTreeSet<_>>();
    let actual = record
        .planning_bundles
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    if actual != expected {
        return Err(invalid_store(
            "durable planning bundles do not cover the exact executable package lock",
        ));
    }
    for (package_id, bundle) in &record.planning_bundles {
        let package = lock.package(package_id).ok_or_else(|| {
            invalid_store("a durable planning bundle is outside its package lock")
        })?;
        bundle
            .validate_catalog_binding(&package.catalog)
            .map_err(|error| invalid_store(format!("planning bundle is invalid: {error}")))?;
        let target = package.catalog.record.planning.as_ref().ok_or_else(|| {
            invalid_store("a durable planning bundle has no signed catalog target")
        })?;
        let bytes = bundle
            .canonical_bytes()
            .map_err(|error| invalid_store(format!("planning bundle is invalid: {error}")))?;
        let digest = bundle
            .descriptor_digest()
            .map_err(|error| invalid_store(format!("planning bundle is invalid: {error}")))?;
        if bytes.len() as u64 != target.length || digest != target.sha256 {
            return Err(invalid_store(
                "a durable planning bundle changed from its signed catalog target",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_result(
    result: &StoredOperationResult,
    plan: &StoredPluginPlan,
    intent: &StoredApplyIntent,
) -> PluginManagerResult<()> {
    validate_capability_evidence(&result.capability_before)?;
    validate_capability_evidence(&result.capability_after)?;
    let capability_before = serde_json::to_value(&result.capability_before).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to validate plugin capability evidence: {error}"
        ))
    })?;
    let capability_after = serde_json::to_value(&result.capability_after).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to validate plugin capability evidence: {error}"
        ))
    })?;
    let operation_binding_valid = match &plan.plugin_operation_plan {
        Some(operation_plan) => {
            let authority =
                serde_json::to_value(&operation_plan.plan.authority).map_err(|error| {
                    PluginManagerError::Infrastructure(format!(
                        "failed to validate plugin plan authority: {error}"
                    ))
                })?;
            result
                .data
                .get("pluginOperationPlanDigest")
                .and_then(Value::as_str)
                == Some(operation_plan.plan_digest.as_str())
                && result.data.get("authority") == Some(&authority)
        }
        None => {
            result.data.get("pluginOperationPlanDigest").is_none()
                && result.data.get("authority").is_none()
        }
    };
    let canonical_plan_digest = format!("sha256:{}", result.plan_digest);
    if result.schema != OPERATION_RECORD_SCHEMA
        || result.operation_id != plan.operation_id
        || result.plan_digest != plan.plan_digest
        || result.completed_at_ms < intent.started_at_ms
        || !result.data.is_object()
        || result.data.get("operationId").and_then(Value::as_str)
            != Some(result.operation_id.as_str())
        || result.data.get("planDigest").and_then(Value::as_str)
            != Some(result.plan_digest.as_str())
        || result
            .data
            .get("canonicalPlanDigest")
            .and_then(Value::as_str)
            != Some(canonical_plan_digest.as_str())
        || result.data.get("completedAtMs").and_then(Value::as_u64) != Some(result.completed_at_ms)
        || result
            .data
            .get("stateRevisionAfter")
            .is_some_and(|value| value.as_u64().is_none_or(|revision| revision == 0))
        || result.data.get("capabilityBefore") != Some(&capability_before)
        || result.data.get("capabilityAfter") != Some(&capability_after)
        || !result.data.get("operations").is_some_and(Value::is_array)
        || !result.data.get("resumed").is_some_and(Value::is_boolean)
        || result.data.get("replayed").and_then(Value::as_bool) != Some(false)
        || !operation_binding_valid
    {
        return Err(invalid_store("durable plugin operation result is invalid"));
    }
    Ok(())
}

pub(super) fn validate_plan_value(value: &Value, plan_digest: &str) -> PluginManagerResult<()> {
    if !value.is_object()
        || value.get("dryRun").and_then(Value::as_bool) != Some(true)
        || value.get("planDigest").and_then(Value::as_str) != Some(plan_digest)
    {
        return Err(invalid_store(
            "reviewed plugin plan payload does not match its digest",
        ));
    }
    Ok(())
}

pub(in crate::plugin_manager::operation) fn registry_source_revision(
    value: &Value,
) -> PluginManagerResult<Option<String>> {
    let Some(plans) = value.get("plans").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut revision = None;
    for plan in plans {
        let Some(candidate) = plan.get("registrySourceRevision") else {
            continue;
        };
        let candidate = candidate.as_str().ok_or_else(|| {
            invalid_store("reviewed plan Registry source revision is not a digest")
        })?;
        validate_digest(candidate)
            .map_err(|_| invalid_store("reviewed plan Registry source revision is invalid"))?;
        if revision
            .as_deref()
            .is_some_and(|existing| existing != candidate)
        {
            return Err(invalid_store(
                "reviewed plan contains conflicting Registry source revisions",
            ));
        }
        revision = Some(candidate.to_string());
    }
    Ok(revision)
}

pub(super) fn validate_capability_evidence(
    evidence: &PluginCapabilityEvidence,
) -> PluginManagerResult<()> {
    if evidence.observed_at_ms == 0 {
        return Err(invalid_store("capability observation time is invalid"));
    }
    match evidence.status {
        PluginCapabilityEvidenceStatus::Verified
            if evidence.generation.is_some()
                && evidence.revision.as_deref().is_some_and(valid_digest)
                && evidence.error.is_none() =>
        {
            Ok(())
        }
        PluginCapabilityEvidenceStatus::Unavailable
            if evidence.generation.is_none()
                && evidence.revision.is_none()
                && evidence
                    .error
                    .as_deref()
                    .is_some_and(|error| !error.is_empty() && error.chars().count() <= 500) =>
        {
            Ok(())
        }
        PluginCapabilityEvidenceStatus::Verified | PluginCapabilityEvidenceStatus::Unavailable => {
            Err(invalid_store("capability state evidence is invalid"))
        }
    }
}

pub(super) fn ensure_request_valid(request: &PluginPlanRequest) -> PluginManagerResult<()> {
    if &normalize_plan_request(request)? != request {
        return Err(invalid_store("reviewed plugin request is not canonical"));
    }
    Ok(())
}

pub(super) fn validate_operation_id(value: &str) -> PluginManagerResult<()> {
    let mut characters = value.chars();
    let valid = value.len() <= 256
        && matches!(characters.next(), Some(first) if first.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '.' | '_' | ':' | '/' | '@' | '-')
        });
    if valid {
        Ok(())
    } else {
        Err(PluginManagerError::InvalidRequest(
            "operationId is invalid".to_string(),
        ))
    }
}

pub(super) fn validate_digest(value: &str) -> PluginManagerResult<()> {
    if valid_digest(value) {
        Ok(())
    } else {
        Err(PluginManagerError::InvalidRequest(
            "planDigest must contain 64 lowercase hexadecimal characters".to_string(),
        ))
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn new_operation_id(action: PluginLifecycleAction) -> PluginManagerResult<String> {
    let mut random = [0u8; 16];
    rand::rngs::OsRng
        .try_fill_bytes(&mut random)
        .map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "failed to generate plugin operation ID: {error}"
            ))
        })?;
    let mut id = format!("plugin-{}-", action.as_str());
    for byte in random {
        write!(&mut id, "{byte:02x}").map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "failed to format plugin operation ID: {error}"
            ))
        })?;
    }
    Ok(id)
}

pub(super) fn record_file_name(operation_id: &str) -> String {
    use sha2::{Digest, Sha256};

    format!("{:x}.json", Sha256::digest(operation_id.as_bytes()))
}

pub(super) fn validate_record_path(path: &Path, operation_id: &str) -> PluginManagerResult<()> {
    let expected = record_file_name(operation_id);
    if path.file_name().and_then(|name| name.to_str()) != Some(expected.as_str()) {
        return Err(invalid_store(
            "reviewed plan filename does not match its operation ID",
        ));
    }
    Ok(())
}

pub(super) fn now_ms() -> PluginManagerResult<u64> {
    u64::try_from(chrono::Utc::now().timestamp_millis()).map_err(|error| {
        PluginManagerError::Infrastructure(format!("system time is before the Unix epoch: {error}"))
    })
}
