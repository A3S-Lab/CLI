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
    ensure_request_valid(&record.request)?;
    validate_capability_evidence(&record.capability_state)?;
    validate_plan_value(&record.plan, &record.plan_digest)?;
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
        || result.data.get("capabilityBefore") != Some(&capability_before)
        || result.data.get("capabilityAfter") != Some(&capability_after)
        || !result.data.get("operations").is_some_and(Value::is_array)
        || !result.data.get("resumed").is_some_and(Value::is_boolean)
        || result.data.get("replayed").and_then(Value::as_bool) != Some(false)
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
