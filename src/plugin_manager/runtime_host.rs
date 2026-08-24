use std::collections::BTreeMap;
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
    task_provider: Option<a3s_runtime::ProviderId>,
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
            .field("task_provider", &self.task_provider)
            .finish_non_exhaustive()
    }
}

impl Default for PluginRuntimeHost {
    fn default() -> Self {
        Self {
            registry: Arc::new(RuntimeClientRegistry::new()),
            assignments: BTreeMap::new(),
            task_provider: None,
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
            task_provider: None,
            readiness,
        })
    }

    /// Compose one explicitly selected provider for every release-backed Tool
    /// Task. Runtime Services remain unassigned until the same host also
    /// supplies a production Gateway readiness and drain adapter.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn new_task_provider(
        registry: RuntimeClientRegistry,
        provider_id: a3s_runtime::ProviderId,
        readiness: Arc<dyn PluginRuntimeServiceReadinessHost>,
    ) -> UseResult<Self> {
        if !registry.contains(&provider_id) {
            return Err(runtime_host_error(
                "The configured Tool Task Runtime provider is not registered.",
            ));
        }
        Ok(Self {
            registry: Arc::new(registry),
            assignments: BTreeMap::new(),
            task_provider: Some(provider_id),
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
            .map(|(surface, class)| {
                if let Some(assignment) = self.assignments.get(&surface) {
                    return Ok(assignment.clone());
                }
                if class == ManagedSurfaceClass::Task {
                    if let Some(provider_id) = self.task_provider.as_ref() {
                        return RuntimeProviderAssignment::new(
                            surface,
                            provider_id.as_str().to_string(),
                        );
                    }
                }
                Err(runtime_host_error(format!(
                    "Managed surface '{}/{:?}:{}' has no explicit host Runtime assignment.",
                    surface.package_id, surface.surface.kind, surface.surface.id
                )))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedSurfaceClass {
    Task,
    Service,
}

fn managed_surfaces(
    planning_bundles: &BTreeMap<String, PluginPlanningBundle>,
) -> BTreeMap<PlanQualifiedSurfaceRef, ManagedSurfaceClass> {
    planning_bundles
        .iter()
        .flat_map(|(package_id, bundle)| {
            bundle
                .surfaces
                .iter()
                .filter_map(|surface| {
                    let class = match surface {
                        ExecutablePlanningSurface::ToolTask { .. } => ManagedSurfaceClass::Task,
                        ExecutablePlanningSurface::ToolService { .. }
                        | ExecutablePlanningSurface::McpService { .. } => {
                            ManagedSurfaceClass::Service
                        }
                        ExecutablePlanningSurface::ToolTaskNative { .. }
                        | ExecutablePlanningSurface::McpStdio { .. } => return None,
                    };
                    Some((surface, class))
                })
                .map(move |(surface, class)| {
                    (
                        PlanQualifiedSurfaceRef {
                            package_id: package_id.clone(),
                            surface: surface.reference(),
                        },
                        class,
                    )
                })
        })
        .collect()
}

fn runtime_host_error(message: impl Into<String>) -> UseError {
    UseError::new("use.plugin.runtime.provider_assignment_invalid", message)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use a3s_runtime::{
        ProviderId, RuntimeClient, RuntimeClientRegistry, RuntimeError, RuntimeProviderFactory,
        RuntimeResult,
    };
    use a3s_use_core::{
        ExecutablePlanningSurface, McpReleaseDescriptor, PlanningArtifactRef,
        PlanningSurfaceActivation, PluginPlanningBundle, PluginReleaseChannel,
        ToolReleaseDescriptor, PLUGIN_PLANNING_BUNDLE_SCHEMA,
    };
    use async_trait::async_trait;

    use super::*;

    const TOOL_RELEASE: &str = r#"{"artifact":{"digest":"sha256:7777777777777777777777777777777777777777777777777777777777777777","mediaType":"application/vnd.oci.image.manifest.v1+json","sizeBytes":1048576},"compatibility":[{"component":"a3s-runtime","versionRequirement":">=0.3.0, <0.4.0"},{"component":"a3s-use","versionRequirement":">=0.3.0, <0.4.0"}],"dependencies":[],"kind":"tool","name":"acme/worker-convert","provenance":{"buildOperationId":"test:worker-convert","builderId":"test:a3s-cli","commitSha":"1234567890abcdef1234567890abcdef12345678","manifestDigest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","sourceRepository":"https://github.com/acme/worker.git"},"schema":"a3s.use.tool-release.v1","version":"1.0.0","workload":{"class":"task","entrypoint":["/usr/local/bin/acme-worker-convert"],"interactive":false,"interface":"cli","maxStderrBytes":1048576,"maxStdoutBytes":4194304,"successExitCodes":[0],"timeoutMs":120000}}"#;
    const TOOL_SERVICE_RELEASE: &str = r#"{"artifact":{"digest":"sha256:8888888888888888888888888888888888888888888888888888888888888888","mediaType":"application/vnd.oci.image.manifest.v1+json","sizeBytes":2097152},"compatibility":[{"component":"a3s-runtime","versionRequirement":">=0.3.0, <0.4.0"},{"component":"a3s-use","versionRequirement":">=0.3.0, <0.4.0"}],"dependencies":[],"kind":"tool","name":"acme/worker-service","provenance":{"buildOperationId":"test:worker-service","builderId":"test:a3s-cli","commitSha":"1234567890abcdef1234567890abcdef12345678","manifestDigest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","sourceRepository":"https://github.com/acme/worker.git"},"schema":"a3s.use.tool-release.v1","version":"1.0.0","workload":{"apiContractDigest":"sha256:9999999999999999999999999999999999999999999999999999999999999999","basePath":"/api","class":"service","health":{"failureThreshold":3,"intervalMs":10000,"path":"/healthz","successThreshold":1,"timeoutMs":2000},"interface":"http","network":"private","port":8080,"portName":"http","shutdownGraceMs":30000,"startupTimeoutMs":60000}}"#;
    const MCP_SERVICE_RELEASE: &str = r#"{"artifact":{"digest":"sha256:9999999999999999999999999999999999999999999999999999999999999999","mediaType":"application/vnd.oci.image.manifest.v1+json","sizeBytes":1048576},"compatibility":[{"component":"a3s-runtime","versionRequirement":">=0.3.0, <0.4.0"},{"component":"a3s-use","versionRequirement":">=0.3.0, <0.4.0"}],"dependencies":[],"kind":"mcp","name":"acme/worker-mcp","provenance":{"buildOperationId":"test:worker-mcp","builderId":"test:a3s-cli","commitSha":"1234567890abcdef1234567890abcdef12345678","manifestDigest":"sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","sourceRepository":"https://github.com/acme/worker.git"},"schema":"a3s.use.mcp-release.v1","service":{"endpointPath":"/mcp","health":{"failureThreshold":3,"intervalMs":10000,"path":"/healthz","successThreshold":1,"timeoutMs":2000},"port":8080,"portName":"mcp","protocolVersion":"2025-06-18","shutdownGraceMs":30000,"startupTimeoutMs":60000,"transport":"streamable-http"},"version":"1.0.0"}"#;

    struct UnavailableFactory {
        provider_id: ProviderId,
    }

    #[async_trait]
    impl RuntimeProviderFactory for UnavailableFactory {
        fn provider_id(&self) -> &ProviderId {
            &self.provider_id
        }

        async fn create(&self) -> RuntimeResult<Arc<dyn RuntimeClient>> {
            Err(RuntimeError::ProviderUnavailable(
                "the assignment test does not connect".to_string(),
            ))
        }
    }

    fn task_provider_host() -> PluginRuntimeHost {
        let provider_id = ProviderId::parse("a3s-box").unwrap();
        let mut registry = RuntimeClientRegistry::new();
        registry
            .register(Arc::new(UnavailableFactory {
                provider_id: provider_id.clone(),
            }))
            .unwrap();
        PluginRuntimeHost::new_task_provider(
            registry,
            provider_id,
            Arc::new(UnavailableRuntimeServiceHost),
        )
        .unwrap()
    }

    fn planning_bundle(surfaces: Vec<ExecutablePlanningSurface>) -> PluginPlanningBundle {
        PluginPlanningBundle {
            schema: PLUGIN_PLANNING_BUNDLE_SCHEMA.to_string(),
            package_id: "acme/worker".to_string(),
            version: "1.0.0".to_string(),
            channel: PluginReleaseChannel::Stable,
            target: "linux-x86_64".to_string(),
            archive_sha256:
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            package_sha256:
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
            manifest_sha256:
                "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    .to_string(),
            permission_ceiling_digest:
                "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                    .to_string(),
            surfaces,
        }
    }

    fn task_surface(id: &str) -> ExecutablePlanningSurface {
        let descriptor = ToolReleaseDescriptor::from_json(TOOL_RELEASE.as_bytes()).unwrap();
        ExecutablePlanningSurface::ToolTask {
            id: id.to_string(),
            activation: PlanningSurfaceActivation::Lazy,
            command: "acme-worker-convert".to_string(),
            json_output: true,
            timeout_ms: 120_000,
            artifact: PlanningArtifactRef {
                uri: format!(
                    "oci://registry.example.test/acme/worker-convert@{}",
                    descriptor.artifact.digest
                ),
                digest: descriptor.artifact.digest.clone(),
                media_type: descriptor.artifact.media_type.clone(),
            },
            descriptor,
        }
    }

    fn service_surface(id: &str) -> ExecutablePlanningSurface {
        let descriptor = ToolReleaseDescriptor::from_json(TOOL_SERVICE_RELEASE.as_bytes()).unwrap();
        ExecutablePlanningSurface::ToolService {
            id: id.to_string(),
            activation: PlanningSurfaceActivation::Eager,
            base_path: "/api".to_string(),
            artifact: PlanningArtifactRef {
                uri: format!(
                    "oci://registry.example.test/acme/worker-convert@{}",
                    descriptor.artifact.digest
                ),
                digest: descriptor.artifact.digest.clone(),
                media_type: descriptor.artifact.media_type.clone(),
            },
            descriptor,
        }
    }

    fn mcp_service_surface(id: &str) -> ExecutablePlanningSurface {
        let descriptor = McpReleaseDescriptor::from_json(MCP_SERVICE_RELEASE.as_bytes()).unwrap();
        ExecutablePlanningSurface::McpService {
            id: id.to_string(),
            activation: PlanningSurfaceActivation::Eager,
            artifact: PlanningArtifactRef {
                uri: format!(
                    "oci://registry.example.test/acme/worker-mcp@{}",
                    descriptor.artifact.digest
                ),
                digest: descriptor.artifact.digest.clone(),
                media_type: descriptor.artifact.media_type.clone(),
            },
            descriptor,
        }
    }

    #[test]
    fn configured_task_provider_derives_exact_task_assignments() {
        let bundles = BTreeMap::from([(
            "acme/worker".to_string(),
            planning_bundle(vec![task_surface("convert")]),
        )]);
        let assignments = task_provider_host().assignments_for(&bundles).unwrap();

        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].surface().package_id, "acme/worker");
        assert_eq!(assignments[0].surface().surface.id, "convert");
        assert_eq!(assignments[0].provider_id().as_str(), "a3s-box");
    }

    #[test]
    fn configured_task_provider_does_not_claim_runtime_services() {
        for (service, id) in [
            (service_surface("serve"), "serve"),
            (mcp_service_surface("library"), "library"),
        ] {
            let bundles = BTreeMap::from([(
                "acme/worker".to_string(),
                planning_bundle(vec![task_surface("convert"), service]),
            )]);
            let error = task_provider_host().assignments_for(&bundles).unwrap_err();

            assert_eq!(error.code, "use.plugin.runtime.provider_assignment_invalid");
            assert!(error.message.contains(id));
        }
    }
}
