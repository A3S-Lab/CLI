use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use a3s_runtime::RuntimeClientRegistry;
use a3s_use::plugin_lifecycle::PluginRuntimeServiceReadinessHost;
use a3s_use::plugin_runtime::{RuntimeProviderAssignment, RuntimeProviderSelection};
use a3s_use_core::{
    ExecutablePlanningSurface, PlanQualifiedSurfaceRef, PluginPlanningBundle, UseError, UseResult,
};

use crate::components::{CodeCognitivePackageLifecycleFactory, UnavailableRuntimeServiceHost};

/// Host-owned Runtime providers, assignments, and Gateway readiness adapter.
///
/// Package metadata supplies provider-neutral workload templates only. This
/// composition is selected at the trusted A3S process boundary and is reused
/// by planning and reviewed apply. The default contains no providers or
/// assignments, so release-backed Runtime surfaces fail closed.
pub struct PluginRuntimeHost {
    registry: RuntimeClientRegistry,
    assignments: BTreeMap<PlanQualifiedSurfaceRef, RuntimeProviderAssignment>,
    readiness: Arc<dyn PluginRuntimeServiceReadinessHost>,
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
            registry: RuntimeClientRegistry::new(),
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
            registry,
            assignments: indexed,
            readiness,
        })
    }

    pub(crate) fn registry(&self) -> &RuntimeClientRegistry {
        &self.registry
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
    ) -> UseResult<CodeCognitivePackageLifecycleFactory> {
        CodeCognitivePackageLifecycleFactory::managed(selection, self.readiness.clone())
    }
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
