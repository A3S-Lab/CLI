use a3s_use_core::{
    PlanActor, PlanAuthority, PlanPackageRole, PlanPolicyDecision, PlanScopeKind,
    PluginOperationAction, PluginOperationPlan, PluginOperationPlanBinding,
    PluginOperationPlanDraft, PluginOperationPlanEnvelope, PluginReleaseChannel,
};
use serde_json::Value;

use super::store::PluginPlanIdentity;
use crate::plugin_manager::capability::{PluginCapabilityEvidence, PluginCapabilityEvidenceStatus};
use crate::plugin_manager::process::{
    normalize_plan_digest, PluginLifecycleAction, PluginPlanRequest,
};
use crate::plugin_manager::{PluginAuthorizationPolicy, PluginManagerError, PluginManagerResult};

const OPERATION_PLAN_FIELD: &str = "pluginOperationPlan";
const OPERATION_PLAN_DIGEST_FIELD: &str = "pluginOperationPlanDigest";

pub(super) struct PreparedPlanArtifact {
    pub plan: Value,
    pub plan_digest: String,
    pub upstream_plan_digest: Option<String>,
    pub plugin_operation_plan: Option<PluginOperationPlanEnvelope>,
}

pub(super) fn prepare(
    authorization: &PluginAuthorizationPolicy,
    request: &PluginPlanRequest,
    actor: PlanActor,
    capability_state: &PluginCapabilityEvidence,
    identity: &PluginPlanIdentity,
    upstream_plan_digest: String,
    mut raw_plan: Value,
) -> PluginManagerResult<PreparedPlanArtifact> {
    let object = raw_plan.as_object_mut().ok_or_else(|| {
        PluginManagerError::Upstream("a3s plugin plan response must be an object".to_string())
    })?;
    let Some(draft_value) = object.remove(OPERATION_PLAN_FIELD) else {
        if object.contains_key(OPERATION_PLAN_DIGEST_FIELD) {
            return Err(upstream_error(
                "pluginOperationPlanDigest is present without pluginOperationPlan",
            ));
        }
        return Ok(PreparedPlanArtifact {
            plan: raw_plan,
            plan_digest: upstream_plan_digest,
            upstream_plan_digest: None,
            plugin_operation_plan: None,
        });
    };
    if object.remove(OPERATION_PLAN_DIGEST_FIELD).is_some() {
        return Err(upstream_error(
            "the delegated planner must not authorize or digest its draft",
        ));
    }

    let draft: PluginOperationPlanDraft = serde_json::from_value(draft_value)
        .map_err(|error| upstream_error(format!("pluginOperationPlan is invalid: {error}")))?;
    let mut plan = draft
        .bind(host_binding(actor, identity, authorization)?)
        .map_err(|error| {
            upstream_error(format!("pluginOperationPlan cannot be host-bound: {error}"))
        })?;
    validate_resolved_request(&plan, request, capability_state)?;
    let evaluation = authorization.evaluate_plan(&plan)?;
    plan.authority = evaluation.authority();
    let envelope = PluginOperationPlanEnvelope::new(plan).map_err(|error| {
        upstream_error(format!(
            "authorized pluginOperationPlan is invalid: {error}"
        ))
    })?;
    let plan_digest = normalize_plan_digest(&envelope.plan_digest)?;

    object.insert("planDigest".to_string(), Value::String(plan_digest.clone()));
    object.insert(
        OPERATION_PLAN_FIELD.to_string(),
        serde_json::to_value(&envelope.plan).map_err(json_error)?,
    );
    object.insert(
        OPERATION_PLAN_DIGEST_FIELD.to_string(),
        Value::String(envelope.plan_digest.clone()),
    );
    object.remove("canonicalPlanDigest");

    Ok(PreparedPlanArtifact {
        plan: raw_plan,
        plan_digest,
        upstream_plan_digest: Some(upstream_plan_digest),
        plugin_operation_plan: Some(envelope),
    })
}

fn host_binding(
    actor: PlanActor,
    identity: &PluginPlanIdentity,
    authorization: &PluginAuthorizationPolicy,
) -> PluginManagerResult<PluginOperationPlanBinding> {
    Ok(PluginOperationPlanBinding {
        operation_id: identity.operation_id.clone(),
        created_at_ms: identity.created_at_ms,
        expires_at_ms: identity.expires_at_ms,
        scope: a3s_use_core::PlanScope {
            kind: PlanScopeKind::User,
            id: "current".to_string(),
        },
        authority: PlanAuthority {
            actor,
            decision: PlanPolicyDecision::Ask,
            policy_digest: authorization.descriptor_digest()?,
            confirmation_required: true,
        },
    })
}

fn validate_resolved_request(
    plan: &PluginOperationPlan,
    request: &PluginPlanRequest,
    capability_state: &PluginCapabilityEvidence,
) -> PluginManagerResult<()> {
    let expected_action = match request.action {
        PluginLifecycleAction::Install => PluginOperationAction::Install,
        PluginLifecycleAction::Upgrade => PluginOperationAction::Upgrade,
        PluginLifecycleAction::Uninstall => PluginOperationAction::Uninstall,
    };
    let package_id = request
        .component_id
        .strip_prefix("use/")
        .unwrap_or_default();
    if plan.action != expected_action
        || plan.package_id != package_id
        || plan.scope.kind != PlanScopeKind::User
        || plan.scope.id != "current"
    {
        return Err(upstream_error(
            "pluginOperationPlan action, package, or scope does not match the host request",
        ));
    }
    if capability_state.status != PluginCapabilityEvidenceStatus::Verified
        || capability_state.generation != Some(plan.state.capability_generation)
    {
        return Err(upstream_error(
            "pluginOperationPlan does not match the verified capability generation",
        ));
    }
    let root = plan
        .packages
        .iter()
        .find(|package| package.role == PlanPackageRole::Root)
        .ok_or_else(|| upstream_error("pluginOperationPlan has no root package"))?;
    let release = match request.action {
        PluginLifecycleAction::Install | PluginLifecycleAction::Upgrade => {
            root.after.as_ref().map(|state| &state.release)
        }
        PluginLifecycleAction::Uninstall => root.before.as_ref().map(|state| &state.release),
    }
    .ok_or_else(|| upstream_error("pluginOperationPlan root release is missing"))?;
    if request
        .version
        .as_deref()
        .is_some_and(|version| release.version != version)
        || request
            .channel
            .as_deref()
            .is_some_and(|channel| release_channel_name(release.channel) != channel)
    {
        return Err(upstream_error(
            "pluginOperationPlan release does not match the requested version or channel",
        ));
    }
    Ok(())
}

fn release_channel_name(channel: PluginReleaseChannel) -> &'static str {
    match channel {
        PluginReleaseChannel::Stable => "stable",
        PluginReleaseChannel::Beta => "beta",
        PluginReleaseChannel::Nightly => "nightly",
    }
}

fn upstream_error(message: impl Into<String>) -> PluginManagerError {
    PluginManagerError::Upstream(format!(
        "delegated plugin plan contract is invalid: {}",
        message.into()
    ))
}

fn json_error(error: serde_json::Error) -> PluginManagerError {
    PluginManagerError::Infrastructure(format!(
        "failed to encode authorized plugin operation plan: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_manager::policy::tests::install_plan;

    fn install_draft() -> PluginOperationPlanDraft {
        let mut plan = install_plan();
        plan.workspace_impacts.clear();
        PluginOperationPlanDraft::new(
            plan.action,
            plan.package_id,
            plan.component_id,
            plan.packages,
            plan.providers,
            plan.workspace_impacts,
            plan.impact,
            plan.state,
        )
        .unwrap()
    }

    #[test]
    fn complete_draft_is_host_bound_authorized_and_redigested() {
        let draft = install_draft();
        let capability = PluginCapabilityEvidence {
            status: PluginCapabilityEvidenceStatus::Verified,
            observed_at_ms: 1,
            generation: Some(draft.state.capability_generation),
            revision: Some("a".repeat(64)),
            error: None,
        };
        let identity = PluginPlanIdentity {
            operation_id: "plugin-install-host".to_string(),
            created_at_ms: 10,
            expires_at_ms: 20,
        };
        let upstream_digest = "b".repeat(64);
        let prepared = prepare(
            &PluginAuthorizationPolicy::default(),
            &PluginPlanRequest {
                action: PluginLifecycleAction::Install,
                component_id: "use/acme/research".to_string(),
                version: Some("2.0.0".to_string()),
                channel: Some("stable".to_string()),
            },
            PlanActor::User,
            &capability,
            &identity,
            upstream_digest.clone(),
            serde_json::json!({
                "dryRun": true,
                "planDigest": upstream_digest,
                "pluginOperationPlan": draft,
            }),
        )
        .unwrap();
        let envelope = prepared.plugin_operation_plan.as_ref().unwrap();

        assert_eq!(
            prepared.upstream_plan_digest.as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
        assert_eq!(envelope.plan.operation_id, identity.operation_id);
        assert_eq!(envelope.plan.authority.actor, PlanActor::User);
        assert_eq!(envelope.plan.authority.decision, PlanPolicyDecision::Ask);
        assert_eq!(
            prepared.plan["pluginOperationPlanDigest"],
            envelope.plan_digest
        );
        assert_eq!(prepared.plan["planDigest"], prepared.plan_digest);
        assert_ne!(prepared.plan_digest, upstream_digest);
    }

    #[test]
    fn draft_cannot_change_the_requested_package_or_capability_generation() {
        let draft = install_draft();
        let capability = PluginCapabilityEvidence {
            status: PluginCapabilityEvidenceStatus::Verified,
            observed_at_ms: 1,
            generation: Some(draft.state.capability_generation + 1),
            revision: Some("a".repeat(64)),
            error: None,
        };
        let result = prepare(
            &PluginAuthorizationPolicy::default(),
            &PluginPlanRequest {
                action: PluginLifecycleAction::Install,
                component_id: "use/acme/research".to_string(),
                version: Some("2.0.0".to_string()),
                channel: Some("stable".to_string()),
            },
            PlanActor::User,
            &capability,
            &PluginPlanIdentity {
                operation_id: "plugin-install-host".to_string(),
                created_at_ms: 10,
                expires_at_ms: 20,
            },
            "b".repeat(64),
            serde_json::json!({
                "dryRun": true,
                "planDigest": "b".repeat(64),
                "pluginOperationPlan": draft,
            }),
        );

        assert!(matches!(result, Err(PluginManagerError::Upstream(_))));
    }
}
