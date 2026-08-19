use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use a3s_runtime::RuntimeClientRegistry;
use a3s_use::cognitive_package::{
    bind_cognitive_package_grants, bind_cognitive_package_provider_plan,
    plan_cognitive_package_provider_generations, plan_cognitive_package_providers,
    reconstruct_cognitive_package_grants, CognitivePackageEnablementDraft,
    CognitivePackageEnablementPlanResult,
};
use a3s_use::plugin_lifecycle::PluginRuntimeServiceReadinessHost;
use a3s_use::plugin_runtime::{
    RuntimeBindingStore, RuntimeProviderAssignment, RuntimeProviderSelection,
    RuntimeTaskDispatchRequest, RuntimeTaskDispatcher, RuntimeTaskExecution,
};
use a3s_use_core::{
    ExecutablePlanningSurface, PlanAuthority, PlanQualifiedSurfaceRef, PluginOperationAction,
    PluginOperationPlan, PluginOperationPlanBinding, PluginPlanningBundle,
    PluginWorkspaceGrantSnapshot, UseError, UseResult,
};
use a3s_use_extension::{ExtensionPaths, ExtensionRegistry};
use serde::{Deserialize, Serialize};

use crate::components::{
    CodeCognitivePackageLifecycleFactory, ComponentPaths, UnavailableRuntimeServiceHost,
};

/// Host-owned Runtime providers, assignments, and Gateway readiness adapter.
///
/// Package metadata supplies provider-neutral workload templates only. This
/// composition is selected at the trusted A3S process boundary and is reused
/// by planning and reviewed apply. The default contains no providers or
/// assignments, so release-backed Runtime surfaces fail closed.
pub struct PluginRuntimeHost {
    registry: Arc<RuntimeClientRegistry>,
    assignments: BTreeMap<PlanQualifiedSurfaceRef, RuntimeProviderAssignment>,
    readiness: Arc<dyn PluginRuntimeServiceReadinessHost>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReviewedEnablementRuntimeEvidence {
    pub planning_bundles: BTreeMap<String, PluginPlanningBundle>,
    pub grant_snapshot: PluginWorkspaceGrantSnapshot,
    pub provider_generations: BTreeMap<String, u64>,
}

impl ReviewedEnablementRuntimeEvidence {
    pub(crate) fn validate_for(&self, plan: &PluginOperationPlan) -> UseResult<()> {
        self.grant_snapshot.validate()?;
        if self.grant_snapshot.scope_id != plan.scope.id
            || self.grant_snapshot.state_revision != plan.state.state_revision
            || self
                .provider_generations
                .values()
                .any(|generation| *generation == 0)
            || (plan.action == PluginOperationAction::Disable
                && (!self.planning_bundles.is_empty() || !self.provider_generations.is_empty()))
            || !matches!(
                plan.action,
                PluginOperationAction::Enable | PluginOperationAction::Disable
            )
        {
            return Err(runtime_host_error(
                "Reviewed enablement Runtime evidence does not match its immutable plan.",
            ));
        }
        Ok(())
    }
}

impl std::fmt::Debug for PluginRuntimeHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginRuntimeHost")
            .field("assignments", &self.assignments.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Default for PluginRuntimeHost {
    fn default() -> Self {
        Self {
            registry: Arc::new(RuntimeClientRegistry::new()),
            assignments: BTreeMap::new(),
            readiness: Arc::new(UnavailableRuntimeServiceHost),
        }
    }
}

impl PluginRuntimeHost {
    pub fn new(
        registry: RuntimeClientRegistry,
        assignments: Vec<RuntimeProviderAssignment>,
        readiness: Arc<dyn PluginRuntimeServiceReadinessHost>,
    ) -> UseResult<Self> {
        let mut indexed = BTreeMap::new();
        for assignment in assignments {
            let surface = assignment.surface().clone();
            if indexed.insert(surface, assignment).is_some() {
                return Err(runtime_host_error(
                    "A Runtime surface has more than one host assignment.",
                ));
            }
        }
        Ok(Self {
            registry: Arc::new(registry),
            assignments: indexed,
            readiness,
        })
    }

    pub(crate) fn registry(&self) -> &RuntimeClientRegistry {
        self.registry.as_ref()
    }

    pub(crate) fn has_provider(&self, provider_id: &str) -> bool {
        a3s_runtime::ProviderId::parse(provider_id)
            .is_ok_and(|provider_id| self.registry.contains(&provider_id))
    }

    /// Invoke one exact published managed Tool Task through the provider that
    /// was reviewed and retained in its durable Runtime binding receipt.
    ///
    /// The dispatcher owns the package-generation lease for the complete
    /// invocation, log capture, and cleanup sequence. It deliberately does
    /// not consult current surface assignments, so an upgrade can never
    /// redirect a stale request to a different generation.
    pub(crate) async fn invoke_runtime_task(
        &self,
        paths: &ComponentPaths,
        request: RuntimeTaskDispatchRequest,
    ) -> UseResult<RuntimeTaskExecution> {
        let extension_paths =
            ExtensionPaths::new(paths.data_root.join("use"), paths.state_root.join("use"));
        RuntimeTaskDispatcher::new(
            ExtensionRegistry::new(extension_paths.clone()),
            RuntimeBindingStore::from_extension_paths(&extension_paths),
            self.registry.clone(),
        )
        .invoke(request)
        .await
    }

    pub(crate) fn assignments_for(
        &self,
        planning_bundles: &BTreeMap<String, PluginPlanningBundle>,
    ) -> UseResult<Vec<RuntimeProviderAssignment>> {
        let required = managed_surfaces(planning_bundles);
        required
            .into_iter()
            .map(|surface| {
                self.assignments.get(&surface).cloned().ok_or_else(|| {
                    runtime_host_error(format!(
                        "Managed surface '{}/{:?}:{}' has no explicit host Runtime assignment.",
                        surface.package_id, surface.surface.kind, surface.surface.id
                    ))
                })
            })
            .collect()
    }

    pub(crate) fn lifecycle_factory(
        &self,
        selection: RuntimeProviderSelection,
        paths: &ComponentPaths,
    ) -> UseResult<CodeCognitivePackageLifecycleFactory> {
        CodeCognitivePackageLifecycleFactory::managed(
            selection,
            self.registry.clone(),
            self.readiness.clone(),
            paths,
        )
    }

    pub(crate) async fn bind_enablement_plan<F>(
        &self,
        prepared: CognitivePackageEnablementDraft,
        provisional_authority: PlanAuthority,
        evaluate_authority: F,
    ) -> UseResult<(
        CognitivePackageEnablementPlanResult,
        ReviewedEnablementRuntimeEvidence,
    )>
    where
        F: Fn(&PluginOperationPlan) -> UseResult<PlanAuthority>,
    {
        let provisional_binding = prepared.provisional_binding(provisional_authority);
        let provider_generations = plan_cognitive_package_provider_generations(
            prepared.draft.action,
            &prepared.draft.packages,
            prepared.draft.state.state_revision,
            None,
            &prepared.planning_bundles,
            &prepared.installed_generations,
        )?;
        let plan = match prepared.draft.action {
            PluginOperationAction::Enable => {
                let assignments = self.assignments_for(&prepared.planning_bundles)?;
                let bound = bind_cognitive_package_provider_plan(
                    prepared.draft.clone(),
                    provisional_binding,
                    &prepared.grant_snapshot,
                    &prepared.planning_bundles,
                    &provider_generations,
                    assignments,
                    self.registry(),
                    &evaluate_authority,
                )
                .await?;
                bound.into_parts().0
            }
            PluginOperationAction::Disable => bind_retiring_enablement_plan(
                prepared.draft.clone(),
                provisional_binding,
                &prepared.grant_snapshot,
                &evaluate_authority,
            )?,
            _ => {
                return Err(runtime_host_error(
                    "Runtime enablement binding received a non-enablement draft.",
                ))
            }
        };
        let evidence = ReviewedEnablementRuntimeEvidence {
            planning_bundles: prepared.planning_bundles.clone(),
            grant_snapshot: prepared.grant_snapshot.clone(),
            provider_generations,
        };
        evidence.validate_for(&plan)?;
        let result = prepared.bind(plan)?;
        Ok((result, evidence))
    }

    pub(crate) async fn reconstruct_enablement_selection(
        &self,
        plan: &PluginOperationPlan,
        evidence: &ReviewedEnablementRuntimeEvidence,
    ) -> UseResult<RuntimeProviderSelection> {
        evidence.validate_for(plan)?;
        let grants = reconstruct_cognitive_package_grants(plan, &evidence.grant_snapshot)?;
        if plan.action == PluginOperationAction::Disable {
            return Ok(RuntimeProviderSelection::default());
        }
        let assignments = self.assignments_for(&evidence.planning_bundles)?;
        let providers = plan_cognitive_package_providers(
            &plan.packages,
            &evidence.planning_bundles,
            grants.proposals(),
            &plan.scope,
            &evidence.provider_generations,
            assignments,
            self.registry(),
        )
        .await?;
        providers.verify_reviewed_evidence(&plan.providers)?;
        Ok(providers.into_parts().1)
    }
}

fn bind_retiring_enablement_plan<F>(
    draft: a3s_use_core::PluginOperationPlanDraft,
    provisional_binding: PluginOperationPlanBinding,
    snapshot: &PluginWorkspaceGrantSnapshot,
    evaluate_authority: F,
) -> UseResult<PluginOperationPlan>
where
    F: Fn(&PluginOperationPlan) -> UseResult<PlanAuthority>,
{
    draft.validate_unbound()?;
    let mut preflight = draft.clone();
    bind_cognitive_package_grants(&mut preflight, &provisional_binding, snapshot)?;
    let preflight = preflight.bind(provisional_binding.clone())?;
    let authority = evaluate_authority(&preflight)?;
    let final_binding = PluginOperationPlanBinding {
        authority: authority.clone(),
        ..provisional_binding
    };
    let mut final_draft = draft;
    bind_cognitive_package_grants(&mut final_draft, &final_binding, snapshot)?;
    let plan = final_draft.bind(final_binding)?;
    if evaluate_authority(&plan)? != authority {
        return Err(runtime_host_error(
            "Final Grant retirement evidence changed the host authorization decision.",
        ));
    }
    Ok(plan)
}

fn managed_surfaces(
    planning_bundles: &BTreeMap<String, PluginPlanningBundle>,
) -> BTreeSet<PlanQualifiedSurfaceRef> {
    planning_bundles
        .iter()
        .flat_map(|(package_id, bundle)| {
            bundle
                .surfaces
                .iter()
                .filter(|&surface| {
                    matches!(
                        surface,
                        ExecutablePlanningSurface::ToolTask { .. }
                            | ExecutablePlanningSurface::ToolService { .. }
                            | ExecutablePlanningSurface::McpService { .. }
                    )
                })
                .map(move |surface| PlanQualifiedSurfaceRef {
                    package_id: package_id.clone(),
                    surface: surface.reference(),
                })
        })
        .collect()
}

fn runtime_host_error(message: impl Into<String>) -> UseError {
    UseError::new("use.plugin.runtime.provider_assignment_invalid", message)
}
