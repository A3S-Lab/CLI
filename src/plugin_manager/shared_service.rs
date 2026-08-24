//! A3S Code composition for the Use-owned Plugin Manager service.

use std::sync::Arc;

use a3s_use::cognitive_package::{
    CognitivePackageAuthorizationEvidence, CognitivePackageAuthorizationProvider,
    CognitivePackageHostManager,
};
use a3s_use::plugin_manager::PluginManagerService;
use a3s_use::plugin_runtime::RuntimeProviderSelection;
use a3s_use_core::{
    PlanActor, PlanAuthority, PlanPolicyDecision, PlanScope, PlanScopeKind, PluginManagedScope,
    PluginOperationPlan, PluginOperationPlanBinding, PluginOperationPlanDraft,
    PluginOperationPlanEnvelope, PluginWorkspaceGrantChangeSet, UseError, UseResult,
    PLUGIN_MANAGED_SCOPE_SCHEMA_V2,
};
use a3s_use_extension::{ExtensionPaths, ExtensionRegistry};
use async_trait::async_trait;

use super::{PluginAuthorizationPolicy, PluginManager, PluginManagerError, PluginManagerResult};

const ASSIGNMENT_GENERATION: u64 = 1;
const FENCE_DIGEST: &str =
    "sha256:a78da178118aa5ec0e8eb90e5b3454b17fdb5ed295dcce4baceb057962e83994";

pub(super) fn compose(manager: &PluginManager) -> PluginManagerResult<PluginManagerService> {
    let scope = managed_scope();
    let lifecycle = manager
        .runtime_host
        .lifecycle_factory(
            RuntimeProviderSelection::default(),
            &manager.component_paths,
        )
        .map_err(composition_error)?;
    let registry = ExtensionRegistry::new(ExtensionPaths::new(
        manager.component_paths.data_root.join("use"),
        manager.component_paths.state_root.join("use"),
    ));
    let authorization = CodePluginManagerAuthorization {
        scope: scope.plan_scope(),
        policy: manager.policy.authorization.clone(),
    };
    let host = CognitivePackageHostManager::new(
        scope,
        format!("a3s-code-cli:{}", env!("CARGO_PKG_VERSION")),
        registry,
        Arc::new(lifecycle),
        Arc::new(authorization),
    )
    .map_err(composition_error)?;
    PluginManagerService::new(host, ASSIGNMENT_GENERATION).map_err(composition_error)
}

fn managed_scope() -> PluginManagedScope {
    PluginManagedScope {
        schema: PLUGIN_MANAGED_SCOPE_SCHEMA_V2.to_string(),
        host_id: "host:a3s-code".to_string(),
        scope_kind: PlanScopeKind::User,
        scope_id: a3s_use::COGNITIVE_PACKAGE_DEFAULT_SCOPE.to_string(),
        authority_id: "user:current".to_string(),
        fence_generation: ASSIGNMENT_GENERATION,
        fence_digest: FENCE_DIGEST.to_string(),
    }
}

/// Planning-only adapter from Code's trusted ACL policy to the Use-owned
/// immutable plan authority. Apply reopens the exact reviewed plan through
/// `CognitivePackageHostManager`, which injects confirmation separately.
#[derive(Clone)]
struct CodePluginManagerAuthorization {
    scope: PlanScope,
    policy: PluginAuthorizationPolicy,
}

impl CodePluginManagerAuthorization {
    fn preview(&self, draft: &PluginOperationPlanDraft) -> UseResult<PluginOperationPlan> {
        draft.validate()?;
        let policy_digest = self.policy.descriptor_digest().map_err(policy_error)?;
        draft.clone().bind(PluginOperationPlanBinding {
            operation_id: "manager:policy-preview".to_string(),
            created_at_ms: 1,
            expires_at_ms: 2,
            scope: self.scope.clone(),
            authority: PlanAuthority {
                actor: PlanActor::User,
                decision: PlanPolicyDecision::Ask,
                policy_digest,
                confirmation_required: true,
            },
        })
    }
}

#[async_trait]
impl CognitivePackageAuthorizationProvider for CodePluginManagerAuthorization {
    fn name(&self) -> &'static str {
        "a3s-code-plugin-manager-policy"
    }

    fn bind_authority(&self, draft: &PluginOperationPlanDraft) -> UseResult<PlanAuthority> {
        self.policy
            .evaluate_plan(&self.preview(draft)?)
            .map(|evaluation| evaluation.authority())
            .map_err(policy_error)
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
        if plan.scope != self.scope || plan.authority.actor != PlanActor::User {
            return Err(policy_plan_invalid());
        }
        self.policy
            .verify_plan_authority(plan)
            .map(drop)
            .map_err(policy_error)
    }

    async fn authorize(
        &self,
        _envelope: &PluginOperationPlanEnvelope,
        _changes: Option<&PluginWorkspaceGrantChangeSet>,
        _now_ms: u64,
    ) -> UseResult<CognitivePackageAuthorizationEvidence> {
        Err(UseError::new(
            "use.plugin.host_authorization_required",
            "The Code Plugin Manager planning policy cannot authorize mutation without exact reviewed confirmation.",
        ))
    }
}

fn composition_error(error: UseError) -> PluginManagerError {
    PluginManagerError::Infrastructure(format!(
        "the Use-owned Plugin Manager service could not be composed: {error}"
    ))
}

fn policy_error(_error: PluginManagerError) -> UseError {
    UseError::new(
        "use.plugin.host_policy_invalid",
        "The Code host plugin policy could not bind the immutable Plugin Manager plan.",
    )
}

fn policy_plan_invalid() -> UseError {
    UseError::new(
        "use.plugin.host_policy_invalid",
        "The Plugin Manager plan does not match the Code host policy scope.",
    )
}
