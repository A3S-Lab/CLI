//! Shared planning-only authorization for reviewed package enablement.

use a3s_use::cognitive_package::{
    CognitivePackageAuthorizationEvidence, CognitivePackageAuthorizationProvider,
};
use a3s_use_core::{
    PlanActor, PlanAuthority, PlanPolicyDecision, PlanScope, PlanScopeKind, PlannedWorkspaceImpact,
    PluginOperationAction, PluginOperationPlan, PluginOperationPlanBinding,
    PluginOperationPlanDraft, PluginOperationPlanEnvelope, PluginWorkspaceGrantChangeSet, UseError,
    UseResult,
};
use async_trait::async_trait;

use super::{PluginAuthorizationPolicy, PluginManagerError};

#[derive(Clone)]
pub(super) struct EnablementPlanningAuthorization {
    scope: PlanScope,
    actor: PlanActor,
    policy: PluginAuthorizationPolicy,
}

impl EnablementPlanningAuthorization {
    pub(super) fn new(
        scope: PlanScope,
        actor: PlanActor,
        policy: PluginAuthorizationPolicy,
    ) -> Self {
        Self {
            scope,
            actor,
            policy,
        }
    }

    fn preview_plan(&self, draft: &PluginOperationPlanDraft) -> UseResult<PluginOperationPlan> {
        let mut preview = draft.clone();
        if self.scope.kind == PlanScopeKind::Workspace {
            let (enabled_before, enabled_after) = enablement_transition(preview.action)?;
            preview.workspace_impacts.push(PlannedWorkspaceImpact {
                scope_id: self.scope.id.clone(),
                grant_before_digest: None,
                grant_after_digest: None,
                enabled_before,
                enabled_after,
            });
        }
        let policy_digest = self
            .policy
            .descriptor_digest()
            .map_err(policy_unavailable)?;
        preview
            .bind(PluginOperationPlanBinding {
                operation_id: "enablement:policy-preview".to_string(),
                created_at_ms: 1,
                expires_at_ms: 2,
                scope: self.scope.clone(),
                authority: PlanAuthority {
                    actor: self.actor,
                    decision: PlanPolicyDecision::Ask,
                    policy_digest,
                    confirmation_required: true,
                },
            })
            .map_err(|_| policy_plan_invalid())
    }
}

#[async_trait]
impl CognitivePackageAuthorizationProvider for EnablementPlanningAuthorization {
    fn name(&self) -> &'static str {
        match self.actor {
            PlanActor::Agent => "a3s-cli-managed-enablement-policy",
            PlanActor::User => "a3s-cli-user-enablement-policy",
        }
    }

    fn bind_authority(&self, draft: &PluginOperationPlanDraft) -> UseResult<PlanAuthority> {
        let plan = self.preview_plan(draft)?;
        self.policy
            .evaluate_plan(&plan)
            .map(|evaluation| evaluation.authority())
            .map_err(policy_unavailable)
    }

    fn bind_operation(
        &self,
        draft: &PluginOperationPlanDraft,
        default_binding: PluginOperationPlanBinding,
    ) -> UseResult<PluginOperationPlanBinding> {
        draft.validate()?;
        if default_binding.scope != self.scope {
            return Err(policy_plan_invalid());
        }
        Ok(default_binding)
    }

    fn verify_authority(&self, plan: &PluginOperationPlan) -> UseResult<()> {
        if plan.scope != self.scope || plan.authority.actor != self.actor {
            return Err(policy_plan_invalid());
        }
        self.policy
            .verify_plan_authority(plan)
            .map(drop)
            .map_err(policy_unavailable)
    }

    async fn authorize(
        &self,
        _envelope: &PluginOperationPlanEnvelope,
        _changes: Option<&PluginWorkspaceGrantChangeSet>,
        _now_ms: u64,
    ) -> UseResult<CognitivePackageAuthorizationEvidence> {
        let (code, message) = match self.actor {
            PlanActor::Agent => (
                "use.plugin.host_enablement_authorization_required",
                "The planning-only managed-host policy cannot authorize a package mutation.",
            ),
            PlanActor::User => (
                "use.plugin.enablement_authorization_required",
                "The planning-only user policy cannot authorize a package mutation.",
            ),
        };
        Err(UseError::new(code, message))
    }
}

fn enablement_transition(action: PluginOperationAction) -> UseResult<(bool, bool)> {
    match action {
        PluginOperationAction::Enable => Ok((false, true)),
        PluginOperationAction::Disable => Ok((true, false)),
        PluginOperationAction::Install
        | PluginOperationAction::Upgrade
        | PluginOperationAction::Uninstall => Err(policy_plan_invalid()),
    }
}

fn policy_unavailable(_error: PluginManagerError) -> UseError {
    UseError::new(
        "use.plugin.host_policy_invalid",
        "The host plugin policy could not authorize the reviewed enablement plan.",
    )
}

fn policy_plan_invalid() -> UseError {
    UseError::new(
        "use.plugin.host_policy_invalid",
        "The enablement plan cannot be evaluated by the host plugin policy.",
    )
}
