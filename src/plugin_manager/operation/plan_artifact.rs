use std::collections::BTreeMap;

use a3s_use::cognitive_package::{
    bind_cognitive_package_provider_plan, plan_cognitive_package_provider_generations,
};
use a3s_use_core::{
    PlanActor, PlanAuthority, PlanPackageChangeKind, PlanPackageRole, PlanPolicyDecision,
    PlanScope, PlanScopeKind, PlannedPackageState, PlannedWorkspaceImpact, PluginOperationAction,
    PluginOperationPlan, PluginOperationPlanBinding, PluginOperationPlanDraft,
    PluginOperationPlanEnvelope, PluginPackageLock, PluginPlanningBundle, PluginReleaseChannel,
    PluginWorkspaceGrantSnapshot, UseError,
};
use serde_json::Value;

use super::store::PluginPlanIdentity;
use crate::plugin_manager::capability::{PluginCapabilityEvidence, PluginCapabilityEvidenceStatus};
use crate::plugin_manager::process::{
    normalize_plan_digest, PluginLifecycleAction, PluginPlanRequest,
};
use crate::plugin_manager::PluginRuntimeHost;
use crate::plugin_manager::{PluginAuthorizationPolicy, PluginManagerError, PluginManagerResult};

const OPERATION_PLAN_FIELD: &str = "pluginOperationPlan";
const OPERATION_PLAN_DIGEST_FIELD: &str = "pluginOperationPlanDigest";

pub(super) struct PreparedPlanArtifact {
    pub plan: Value,
    pub plan_digest: String,
    pub upstream_plan_digest: Option<String>,
    pub plugin_operation_plan: Option<PluginOperationPlanEnvelope>,
    pub planning_bundles: BTreeMap<String, PluginPlanningBundle>,
}

pub(super) struct ObservedPlanState<'a> {
    pub capability: &'a PluginCapabilityEvidence,
    pub state_revision: u64,
}

pub(super) struct HostPlanContext<'a> {
    pub authorization: &'a PluginAuthorizationPolicy,
    pub actor: PlanActor,
    pub scope: &'a PlanScope,
    pub observed: ObservedPlanState<'a>,
    pub identity: &'a PluginPlanIdentity,
    pub grant_snapshot: Option<&'a PluginWorkspaceGrantSnapshot>,
    pub installed_generations: &'a BTreeMap<String, u64>,
    pub runtime_host: &'a PluginRuntimeHost,
}

pub(super) async fn prepare(
    context: HostPlanContext<'_>,
    request: &PluginPlanRequest,
    upstream_plan_digest: String,
    installed_package_lock: Option<PluginPackageLock>,
    mut raw_plan: Value,
) -> PluginManagerResult<PreparedPlanArtifact> {
    let package_locks = reviewed_package_locks(&raw_plan, request, installed_package_lock)?;
    let planning_bundles = match (&package_locks, request.action) {
        (Some(locks), PluginLifecycleAction::Install | PluginLifecycleAction::Upgrade) => {
            let component_plan = super::planner::single_component_plan(&raw_plan, request)?;
            super::planner::verified_planning_bundles(component_plan, &locks.package_lock)?
        }
        (Some(_), PluginLifecycleAction::Uninstall) | (None, _) => BTreeMap::new(),
    };
    let object = raw_plan.as_object_mut().ok_or_else(|| {
        PluginManagerError::Upstream("a3s plugin plan response must be an object".to_string())
    })?;
    let Some(draft_value) = object.remove(OPERATION_PLAN_FIELD) else {
        if package_locks.is_some() {
            return Err(upstream_error(
                "a reviewed cognitive-package lock requires a complete pluginOperationPlan",
            ));
        }
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
            planning_bundles,
        });
    };
    if object.remove(OPERATION_PLAN_DIGEST_FIELD).is_some() {
        return Err(upstream_error(
            "the delegated planner must not authorize or digest its draft",
        ));
    }

    let draft_bytes = serde_json::to_vec(&draft_value).map_err(json_error)?;
    let draft = if package_locks.is_some() {
        PluginOperationPlanDraft::from_unbound_json(&draft_bytes)
    } else {
        PluginOperationPlanDraft::from_json(&draft_bytes)
    }
    .map_err(|error| upstream_error(format!("pluginOperationPlan is invalid: {error}")))?;
    let plan = bind_authorized_plan(
        &context,
        request,
        draft,
        package_locks.as_ref().map(|locks| &locks.package_lock),
        &planning_bundles,
    )
    .await?;
    let envelope = match package_locks {
        None => PluginOperationPlanEnvelope::new(plan),
        Some(ReviewedPackageLocks {
            package_lock,
            prior_package_lock: None,
        }) => {
            validate_prebound_lock_digest(&plan, &package_lock)?;
            PluginOperationPlanEnvelope::new_with_package_lock(plan, package_lock)
        }
        Some(ReviewedPackageLocks {
            package_lock,
            prior_package_lock: Some(prior_package_lock),
        }) => {
            validate_prebound_lock_digest(&plan, &package_lock)?;
            PluginOperationPlanEnvelope::new_with_upgrade_package_locks(
                plan,
                prior_package_lock,
                package_lock,
            )
        }
    }
    .map_err(|error| {
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
        planning_bundles,
    })
}

pub(super) fn requires_grant_snapshot(
    raw_plan: &Value,
    request: &PluginPlanRequest,
    installed_package_lock: Option<&PluginPackageLock>,
) -> PluginManagerResult<bool> {
    reviewed_package_locks(raw_plan, request, installed_package_lock.cloned())
        .map(|locks| locks.is_some())
}

struct ReviewedPackageLocks {
    package_lock: PluginPackageLock,
    prior_package_lock: Option<PluginPackageLock>,
}

fn reviewed_package_locks(
    raw_plan: &Value,
    request: &PluginPlanRequest,
    installed_package_lock: Option<PluginPackageLock>,
) -> PluginManagerResult<Option<ReviewedPackageLocks>> {
    let candidate = reviewed_package_lock(raw_plan, request)?;
    match request.action {
        PluginLifecycleAction::Install => match (candidate, installed_package_lock) {
            (Some(package_lock), None) => Ok(Some(ReviewedPackageLocks {
                package_lock,
                prior_package_lock: None,
            })),
            (None, None) => Ok(None),
            _ => Err(upstream_error(
                "an install plan has inconsistent current and candidate dependency locks",
            )),
        },
        PluginLifecycleAction::Upgrade => match (candidate, installed_package_lock) {
            (Some(package_lock), Some(prior_package_lock)) => Ok(Some(ReviewedPackageLocks {
                package_lock,
                prior_package_lock: Some(prior_package_lock),
            })),
            (None, None) => Ok(None),
            _ => Err(upstream_error(
                "an upgrade plan requires exact prior and candidate dependency locks",
            )),
        },
        PluginLifecycleAction::Uninstall => match (candidate, installed_package_lock) {
            (None, Some(package_lock)) => Ok(Some(ReviewedPackageLocks {
                package_lock,
                prior_package_lock: None,
            })),
            (None, None) => Ok(None),
            _ => Err(upstream_error(
                "an uninstall plan must bind only its exact installed dependency lock",
            )),
        },
    }
}

fn validate_prebound_lock_digest(
    plan: &PluginOperationPlan,
    package_lock: &PluginPackageLock,
) -> PluginManagerResult<()> {
    let actual = package_lock
        .descriptor_digest()
        .map_err(|error| upstream_error(error.to_string()))?;
    if plan
        .package_lock_digest
        .as_ref()
        .is_some_and(|expected| expected != &actual)
        || plan.prior_package_lock_digest.is_some()
    {
        return Err(upstream_error(
            "pluginOperationPlan does not match its reviewed cognitive-package locks",
        ));
    }
    Ok(())
}

async fn bind_authorized_plan(
    context: &HostPlanContext<'_>,
    request: &PluginPlanRequest,
    draft: PluginOperationPlanDraft,
    package_lock: Option<&PluginPackageLock>,
    planning_bundles: &BTreeMap<String, PluginPlanningBundle>,
) -> PluginManagerResult<PluginOperationPlan> {
    let provisional_binding = host_binding(
        context.actor,
        context.scope,
        context.identity,
        context.authorization,
    )?;
    if let Some(package_lock) = package_lock {
        let snapshot = context.grant_snapshot.ok_or_else(|| {
            upstream_error("a reviewed cognitive-package plan omitted its durable Grant snapshot")
        })?;
        let generations = plan_cognitive_package_provider_generations(
            draft.action,
            &draft.packages,
            context.observed.state_revision,
            Some(package_lock),
            planning_bundles,
            context.installed_generations,
        )
        .map_err(|error| {
            upstream_error(format!(
                "cognitive-package provider generations cannot be derived: {error}"
            ))
        })?;
        let assignments = context
            .runtime_host
            .assignments_for(planning_bundles)
            .map_err(|error| upstream_error(error.to_string()))?;
        let bound = bind_cognitive_package_provider_plan(
            draft,
            provisional_binding,
            snapshot,
            planning_bundles,
            &generations,
            assignments,
            context.runtime_host.registry(),
            |candidate| {
                validate_resolved_request(
                    candidate,
                    request,
                    context.scope,
                    ObservedPlanState {
                        capability: context.observed.capability,
                        state_revision: context.observed.state_revision,
                    },
                )
                .map_err(host_policy_error)?;
                context
                    .authorization
                    .evaluate_plan(candidate)
                    .map(|evaluation| evaluation.authority())
                    .map_err(host_policy_error)
            },
        )
        .await
        .map_err(|error| {
            upstream_error(format!(
                "cognitive-package providers cannot be host-bound: {error}"
            ))
        })?;
        let (plan, _, _) = bound.into_parts();
        validate_resolved_request(
            &plan,
            request,
            context.scope,
            ObservedPlanState {
                capability: context.observed.capability,
                state_revision: context.observed.state_revision,
            },
        )?;
        return Ok(plan);
    }

    let mut provisional_draft = draft.clone();
    bind_workspace_activation_impact(&mut provisional_draft, context.scope)?;
    let provisional_plan = provisional_draft
        .clone()
        .bind(provisional_binding.clone())
        .map_err(|error| {
            upstream_error(format!("pluginOperationPlan cannot be host-bound: {error}"))
        })?;
    validate_resolved_request(
        &provisional_plan,
        request,
        context.scope,
        ObservedPlanState {
            capability: context.observed.capability,
            state_revision: context.observed.state_revision,
        },
    )?;
    let provisional_evaluation = context.authorization.evaluate_plan(&provisional_plan)?;
    let binding = PluginOperationPlanBinding {
        authority: provisional_evaluation.authority(),
        ..provisional_binding
    };
    let plan = provisional_draft.bind(binding).map_err(|error| {
        upstream_error(format!(
            "authorized pluginOperationPlan is invalid: {error}"
        ))
    })?;
    validate_resolved_request(
        &plan,
        request,
        context.scope,
        ObservedPlanState {
            capability: context.observed.capability,
            state_revision: context.observed.state_revision,
        },
    )?;
    let final_evaluation = context.authorization.evaluate_plan(&plan)?;
    if final_evaluation.authority() != provisional_evaluation.authority() {
        return Err(upstream_error(
            "final plugin planning changed the host policy authority",
        ));
    }
    Ok(plan)
}

fn host_policy_error(error: PluginManagerError) -> UseError {
    UseError::new(
        "use.plugin.host_policy_invalid",
        format!("The A3S host rejected provider planning evidence: {error}"),
    )
}

fn bind_workspace_activation_impact(
    draft: &mut PluginOperationPlanDraft,
    scope: &PlanScope,
) -> PluginManagerResult<()> {
    if scope.kind != PlanScopeKind::Workspace {
        return Ok(());
    }
    let (enabled_before, enabled_after) = match draft.action {
        PluginOperationAction::Install => (false, true),
        PluginOperationAction::Upgrade => (true, true),
        PluginOperationAction::Uninstall => (true, false),
        PluginOperationAction::Enable | PluginOperationAction::Disable => {
            return Err(upstream_error(
                "enablement plans must be created through the reviewed enablement planner",
            ));
        }
    };
    let grant_change_required = draft.packages.iter().any(|package| {
        (enabled_before
            && matches!(
                package.change,
                PlanPackageChangeKind::Remove | PlanPackageChangeKind::Replace
            )
            && package
                .before
                .as_ref()
                .is_some_and(has_workspace_permissions))
            || (enabled_after
                && matches!(
                    package.change,
                    PlanPackageChangeKind::Add | PlanPackageChangeKind::Replace
                )
                && package
                    .after
                    .as_ref()
                    .is_some_and(has_workspace_permissions))
    });
    if grant_change_required {
        return Err(upstream_error(
            "managed Workspace permission changes require the exact host Grant planner",
        ));
    }
    bind_provisional_workspace_activation_impact(draft, scope)
}

fn bind_provisional_workspace_activation_impact(
    draft: &mut PluginOperationPlanDraft,
    scope: &PlanScope,
) -> PluginManagerResult<()> {
    if scope.kind != PlanScopeKind::Workspace {
        return Ok(());
    }
    if !draft.workspace_impacts.is_empty() {
        return Err(upstream_error(
            "a delegated draft cannot select its managed Workspace impact",
        ));
    }
    let (enabled_before, enabled_after) = match draft.action {
        PluginOperationAction::Install => (false, true),
        PluginOperationAction::Upgrade => (true, true),
        PluginOperationAction::Uninstall => (true, false),
        PluginOperationAction::Enable | PluginOperationAction::Disable => {
            return Err(upstream_error(
                "enablement plans must be created through the reviewed enablement planner",
            ));
        }
    };
    draft.workspace_impacts.push(PlannedWorkspaceImpact {
        scope_id: scope.id.clone(),
        grant_before_digest: None,
        grant_after_digest: None,
        enabled_before,
        enabled_after,
    });
    draft.validate().map_err(|error| {
        upstream_error(format!(
            "managed Workspace activation impact is invalid: {error}"
        ))
    })
}

fn has_workspace_permissions(state: &PlannedPackageState) -> bool {
    !state.permissions.surfaces.is_empty()
}

fn reviewed_package_lock(
    raw_plan: &Value,
    request: &PluginPlanRequest,
) -> PluginManagerResult<Option<PluginPackageLock>> {
    let Some(plans) = raw_plan.get("plans").and_then(Value::as_array) else {
        return Ok(None);
    };
    if plans.len() != 1 {
        return Ok(None);
    }
    let Some(locks) = plans[0]
        .get("cognitivePackageLocks")
        .and_then(Value::as_object)
    else {
        return Ok(None);
    };
    if locks.len() != 1 {
        return Err(upstream_error(
            "reviewed component plan must carry exactly one cognitive-package lock",
        ));
    }
    let value = locks.get(&request.component_id).ok_or_else(|| {
        upstream_error("reviewed cognitive-package lock does not match the requested component")
    })?;
    let package_lock = serde_json::from_value::<PluginPackageLock>(value.clone())
        .map_err(|error| upstream_error(format!("reviewed package lock is invalid: {error}")))?;
    package_lock
        .validate()
        .map_err(|error| upstream_error(error.to_string()))?;
    Ok(Some(package_lock))
}

fn host_binding(
    actor: PlanActor,
    scope: &PlanScope,
    identity: &PluginPlanIdentity,
    authorization: &PluginAuthorizationPolicy,
) -> PluginManagerResult<PluginOperationPlanBinding> {
    Ok(PluginOperationPlanBinding {
        operation_id: identity.operation_id.clone(),
        created_at_ms: identity.created_at_ms,
        expires_at_ms: identity.expires_at_ms,
        scope: scope.clone(),
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
    scope: &PlanScope,
    observed: ObservedPlanState<'_>,
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
    if plan.action != expected_action || plan.package_id != package_id || &plan.scope != scope {
        return Err(upstream_error(
            "pluginOperationPlan action, package, or scope does not match the host request",
        ));
    }
    if observed.capability.status != PluginCapabilityEvidenceStatus::Verified
        || observed.capability.generation != Some(plan.state.capability_generation)
        || observed.state_revision == 0
        || plan.state.state_revision != observed.state_revision
    {
        return Err(upstream_error(
            "pluginOperationPlan does not match verified capability or durable state evidence",
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

    fn permission_free_install_draft() -> PluginOperationPlanDraft {
        let mut plan = install_plan();
        plan.providers.clear();
        plan.workspace_impacts.clear();
        for package in &mut plan.packages {
            for state in [&mut package.before, &mut package.after]
                .into_iter()
                .flatten()
            {
                state
                    .release
                    .surfaces
                    .retain(|surface| surface.kind == a3s_use_core::PluginSurfaceKind::Skill);
                state.permissions.surfaces.clear();
                state.release.permission_ceiling_digest =
                    state.permissions.descriptor_digest().unwrap();
            }
            package
                .surfaces
                .retain(|change| change.surface.kind == a3s_use_core::PluginSurfaceKind::Skill);
        }
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
    fn permission_free_workspace_draft_binds_only_its_enablement_transition() {
        let mut draft = permission_free_install_draft();
        let scope = PlanScope {
            kind: PlanScopeKind::Workspace,
            id: "workspace:research".to_string(),
        };

        bind_workspace_activation_impact(&mut draft, &scope).unwrap();

        assert_eq!(draft.workspace_impacts.len(), 1);
        let impact = &draft.workspace_impacts[0];
        assert_eq!(impact.scope_id, scope.id);
        assert!(impact.grant_before_digest.is_none());
        assert!(impact.grant_after_digest.is_none());
        assert!(!impact.enabled_before);
        assert!(impact.enabled_after);
    }

    #[test]
    fn permission_bearing_workspace_draft_requires_the_exact_grant_planner() {
        let mut draft = install_draft();
        let error = bind_workspace_activation_impact(
            &mut draft,
            &PlanScope {
                kind: PlanScopeKind::Workspace,
                id: "workspace:research".to_string(),
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("Grant planner"));
    }

    #[tokio::test]
    async fn complete_draft_is_host_bound_authorized_and_redigested() {
        let draft = install_draft();
        let state_revision = draft.state.state_revision;
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
        let installed_generations = BTreeMap::new();
        let runtime_host = PluginRuntimeHost::default();
        let prepared = prepare(
            HostPlanContext {
                authorization: &PluginAuthorizationPolicy::default(),
                actor: PlanActor::User,
                scope: &crate::plugin_manager::default_plan_scope(),
                observed: ObservedPlanState {
                    capability: &capability,
                    state_revision,
                },
                identity: &identity,
                grant_snapshot: None,
                installed_generations: &installed_generations,
                runtime_host: &runtime_host,
            },
            &PluginPlanRequest {
                action: PluginLifecycleAction::Install,
                component_id: "use/acme/research".to_string(),
                version: Some("2.0.0".to_string()),
                channel: Some("stable".to_string()),
            },
            upstream_digest.clone(),
            None,
            serde_json::json!({
                "dryRun": true,
                "planDigest": upstream_digest,
                "pluginOperationPlan": draft,
            }),
        )
        .await
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

    #[tokio::test]
    async fn draft_cannot_change_the_requested_package_or_capability_generation() {
        let draft = install_draft();
        let state_revision = draft.state.state_revision;
        let capability = PluginCapabilityEvidence {
            status: PluginCapabilityEvidenceStatus::Verified,
            observed_at_ms: 1,
            generation: Some(draft.state.capability_generation + 1),
            revision: Some("a".repeat(64)),
            error: None,
        };
        let installed_generations = BTreeMap::new();
        let runtime_host = PluginRuntimeHost::default();
        let result = prepare(
            HostPlanContext {
                authorization: &PluginAuthorizationPolicy::default(),
                actor: PlanActor::User,
                scope: &crate::plugin_manager::default_plan_scope(),
                observed: ObservedPlanState {
                    capability: &capability,
                    state_revision,
                },
                identity: &PluginPlanIdentity {
                    operation_id: "plugin-install-host".to_string(),
                    created_at_ms: 10,
                    expires_at_ms: 20,
                },
                grant_snapshot: None,
                installed_generations: &installed_generations,
                runtime_host: &runtime_host,
            },
            &PluginPlanRequest {
                action: PluginLifecycleAction::Install,
                component_id: "use/acme/research".to_string(),
                version: Some("2.0.0".to_string()),
                channel: Some("stable".to_string()),
            },
            "b".repeat(64),
            None,
            serde_json::json!({
                "dryRun": true,
                "planDigest": "b".repeat(64),
                "pluginOperationPlan": draft,
            }),
        )
        .await;

        assert!(matches!(result, Err(PluginManagerError::Upstream(_))));
    }

    #[tokio::test]
    async fn draft_cannot_change_the_durable_state_revision() {
        let draft = install_draft();
        let capability = PluginCapabilityEvidence {
            status: PluginCapabilityEvidenceStatus::Verified,
            observed_at_ms: 1,
            generation: Some(draft.state.capability_generation),
            revision: Some("a".repeat(64)),
            error: None,
        };
        let installed_generations = BTreeMap::new();
        let runtime_host = PluginRuntimeHost::default();
        let result = prepare(
            HostPlanContext {
                authorization: &PluginAuthorizationPolicy::default(),
                actor: PlanActor::User,
                scope: &crate::plugin_manager::default_plan_scope(),
                observed: ObservedPlanState {
                    capability: &capability,
                    state_revision: draft.state.state_revision + 1,
                },
                identity: &PluginPlanIdentity {
                    operation_id: "plugin-install-host".to_string(),
                    created_at_ms: 10,
                    expires_at_ms: 20,
                },
                grant_snapshot: None,
                installed_generations: &installed_generations,
                runtime_host: &runtime_host,
            },
            &PluginPlanRequest {
                action: PluginLifecycleAction::Install,
                component_id: "use/acme/research".to_string(),
                version: Some("2.0.0".to_string()),
                channel: Some("stable".to_string()),
            },
            "b".repeat(64),
            None,
            serde_json::json!({
                "dryRun": true,
                "planDigest": "b".repeat(64),
                "pluginOperationPlan": draft,
            }),
        )
        .await;

        assert!(matches!(result, Err(PluginManagerError::Upstream(_))));
    }
}
