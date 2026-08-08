use std::sync::{Arc, Mutex};

use a3s_runtime::contract::{
    IsolationLevel, NetworkMode, ResourceControl, RuntimeActionRequest, RuntimeApplyRequest,
    RuntimeCapabilities, RuntimeExecRequest, RuntimeExecResult, RuntimeFeature, RuntimeInspection,
    RuntimeLogChunk, RuntimeLogQuery, RuntimeObservation, RuntimeRemoval, RuntimeUnitClass,
};
use a3s_runtime::{
    ProviderId, RuntimeClient, RuntimeClientRegistry, RuntimeError, RuntimeProviderFactory,
    RuntimeResult,
};
use a3s_use::plugin_runtime::RuntimeProviderAssignment;
use a3s_use_core::{
    CatalogAvailability, CatalogPlanningTarget, CatalogSurface, ExecutablePlanningSurface,
    PlanActor, PlanPackageRole, PlanQualifiedSurfaceRef, PlannedOperationImpact,
    PlannedStateEvidence, PlanningArtifactRef, PlanningSurfaceActivation, PluginCatalogRecord,
    PluginOperationAction, PluginOperationPlanDraft, PluginPlanningBundle, PluginReleaseChannel,
    PluginSurfaceKind, PluginSurfaceRef, ToolReleaseDescriptor, ToolWorkloadClass,
    PLUGIN_CATALOG_SCHEMA_V3, PLUGIN_PLANNING_BUNDLE_SCHEMA,
    PLUGIN_WORKSPACE_GRANT_SNAPSHOT_SCHEMA,
};
use a3s_use_extension::{StoredWorkspaceGrant, WorkspaceGrantStore};
use async_trait::async_trait;
use sha2::{Digest, Sha256};

use super::*;
use crate::plugin_manager::capability::{PluginCapabilityEvidence, PluginCapabilityEvidenceStatus};
use crate::plugin_manager::operation::store::{NewPluginPlan, PluginPlanIdentity};
use crate::plugin_manager::process::{PluginLifecycleAction, PluginPlanRequest};
use crate::plugin_manager::{PluginAuthorizationPolicy, PluginManagerPolicy};
use crate::tuf_test_support::{
    host_target, package_directory_archive, TestRepository, TestServer, TestTarget, FUTURE,
};

#[tokio::test]
async fn reviewed_managed_runtime_graph_rejects_drift_and_persists_exact_grant() {
    let temporary = tempfile::tempdir().unwrap();
    let package_root = temporary.path().join("package");
    std::fs::create_dir_all(package_root.join("releases")).unwrap();
    let manifest = r#"extension "acme/worker" {
  schema_version = 3
  version = "1.0.0"
  route = "worker"
  requires_use = ">=0.3.0, <0.4.0"
  actions = ["read", "execute"]

  repository {
    url = "https://github.com/acme/worker"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }

  tool "convert" {
    workload = "task"
    interface = "cli"
    release = "releases/convert-tool-v1.json"
    command = "acme-worker-convert"
    json_output = true
    interactive = false
    timeout_ms = 120000
    activation = "lazy"
    optional = false
  }
}
"#;
    let tool_release = r#"{"artifact":{"digest":"sha256:7777777777777777777777777777777777777777777777777777777777777777","mediaType":"application/vnd.oci.image.manifest.v1+json","sizeBytes":1048576},"compatibility":[{"component":"a3s-runtime","versionRequirement":">=0.2.0, <0.3.0"},{"component":"a3s-use","versionRequirement":">=0.3.0, <0.4.0"}],"dependencies":[],"kind":"tool","name":"acme/worker-convert","provenance":{"buildOperationId":"test:worker-convert","builderId":"test:a3s-cli","commitSha":"1234567890abcdef1234567890abcdef12345678","manifestDigest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","sourceRepository":"https://github.com/acme/worker.git"},"schema":"a3s.use.tool-release.v1","version":"1.0.0","workload":{"class":"task","entrypoint":["/usr/local/bin/acme-worker-convert"],"interactive":false,"interface":"cli","maxStderrBytes":1048576,"maxStdoutBytes":4194304,"successExitCodes":[0],"timeoutMs":120000}}"#;
    std::fs::write(package_root.join("a3s-use-extension.acl"), manifest).unwrap();
    std::fs::write(
        package_root.join("README.md"),
        "# Worker\n\nPermission-bearing host Grant fixture.\n",
    )
    .unwrap();
    std::fs::write(
        package_root.join("releases/convert-tool-v1.json"),
        tool_release,
    )
    .unwrap();

    let archive = package_directory_archive(&package_root);
    let (package_sha256, file_count, expanded_bytes) = package_fingerprint(&package_root);
    let package_digest = format!("sha256:{package_sha256}");
    let target = host_target();
    let target_name =
        format!("extensions/acme/worker/1.0.0/stable/{target}/worker-1.0.0-{target}.tar.gz");
    let mut catalog = PluginCatalogRecord::from_json(include_bytes!(
        "../../fixtures/complete-catalog-record-v3.json"
    ))
    .unwrap();
    catalog.schema = PLUGIN_CATALOG_SCHEMA_V3.to_string();
    catalog.package_id = "acme/worker".to_string();
    catalog.display_name = "Worker".to_string();
    catalog.description = "Permission-bearing host Grant fixture.".to_string();
    catalog.publisher = "acme".to_string();
    catalog.keywords = vec!["worker".to_string()];
    catalog.categories = vec!["development".to_string()];
    catalog.version = "1.0.0".to_string();
    catalog.channel = PluginReleaseChannel::Stable;
    catalog.requires_use = ">=0.3.0, <0.4.0".to_string();
    catalog.dependencies.clear();
    catalog.target = target.to_string();
    catalog.surfaces = vec![CatalogSurface {
        kind: PluginSurfaceKind::Tool,
        id: "convert".to_string(),
        optional: false,
        workload: Some(ToolWorkloadClass::Task),
        mcp_transport: None,
        mcp_tool_count: None,
        okf_bundle: None,
        requires: Vec::new(),
    }];
    catalog
        .permission_ceiling
        .surfaces
        .retain(|permission| permission.surface.id == "convert");
    let permission = catalog.permission_ceiling.surfaces.first_mut().unwrap();
    permission.native_execution = false;
    permission.filesystem.clear();
    permission.network_egress.clear();
    permission.secrets.clear();
    catalog.permission_ceiling_digest = catalog.permission_ceiling.descriptor_digest().unwrap();
    catalog.archive.target_name = target_name.clone();
    catalog.archive.length = archive.len() as u64;
    catalog.archive.sha256 = prefixed_digest(&archive);
    catalog.package.expanded_bytes = expanded_bytes;
    catalog.package.file_count = file_count;
    catalog.package.sha256 = Some(package_digest.clone());
    catalog.package.manifest_sha256 = Some(prefixed_digest(manifest.as_bytes()));
    catalog.license = "MIT".to_string();
    catalog.repository = "https://github.com/acme/worker".to_string();
    catalog.availability = CatalogAvailability::Available;

    let planning_target = format!("extensions/acme/worker/1.0.0/stable/{target}/planning-v1.json");
    let descriptor = ToolReleaseDescriptor::from_json(tool_release.as_bytes()).unwrap();
    let planning = PluginPlanningBundle {
        schema: PLUGIN_PLANNING_BUNDLE_SCHEMA.to_string(),
        package_id: catalog.package_id.clone(),
        version: catalog.version.clone(),
        channel: catalog.channel,
        target: catalog.target.clone(),
        archive_sha256: catalog.archive.sha256.clone(),
        package_sha256: package_digest.clone(),
        manifest_sha256: catalog.package.manifest_sha256.clone().unwrap(),
        permission_ceiling_digest: catalog.permission_ceiling_digest.clone(),
        surfaces: vec![ExecutablePlanningSurface::ToolTask {
            id: "convert".to_string(),
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
        }],
    };
    let planning_bytes = planning.canonical_bytes().unwrap();
    catalog.planning = Some(CatalogPlanningTarget {
        target_name: planning_target.clone(),
        length: planning_bytes.len() as u64,
        sha256: prefixed_digest(&planning_bytes),
    });
    catalog.validate().unwrap();

    let repository = TestRepository::with_targets(
        vec![
            TestTarget {
                archive,
                target_name: target_name.clone(),
                custom: Some(serde_json::to_value(&catalog).unwrap()),
            },
            TestTarget {
                archive: planning_bytes,
                target_name: planning_target.clone(),
                custom: None,
            },
        ],
        97,
        FUTURE,
    );
    let server = TestServer::start(repository.routes.clone());
    let registry_store = RegistryStore::new(temporary.path().join("registries"));
    std::fs::create_dir_all(registry_store.root()).unwrap();
    std::fs::write(
        registry_store.root().join("fixture.acl"),
        format!(
            "registry \"fixture\" {{\n  url = \"{}\"\n  trust_root = \"sha256:{}\"\n  enabled = true\n  managed_root = false\n}}\n",
            server.base_url(),
            repository.root_sha256
        ),
    )
    .unwrap();

    let mut component_paths = ComponentPaths::for_test(temporary.path());
    let resolved = registry_store
        .resolve_package(
            &component_paths.state_root,
            "acme/worker",
            Some("1.0.0"),
            "stable",
        )
        .await
        .unwrap();
    assert_eq!(resolved.planning_bundle.as_ref(), Some(&planning));
    let verified_catalog = resolved.verified_catalog.clone();
    let package_lock = registry_store
        .resolve_cognitive_package_lock(&component_paths.state_root, &resolved)
        .await
        .unwrap();
    let planning_bundles = registry_store
        .resolve_cognitive_package_planning_bundles(&component_paths.state_root, &package_lock)
        .await
        .unwrap();
    assert_eq!(planning_bundles.get("acme/worker"), Some(&planning));
    let surface = PluginSurfaceRef {
        kind: PluginSurfaceKind::Tool,
        id: "convert".to_string(),
    };
    let transition = verified_catalog
        .install_transition(PlanPackageRole::Root, std::slice::from_ref(&surface))
        .unwrap();
    let mut draft = PluginOperationPlanDraft::new_unbound(
        PluginOperationAction::Install,
        "acme/worker",
        "use/acme/worker",
        vec![transition],
        Vec::new(),
        PlannedOperationImpact {
            download_bytes: verified_catalog.record.archive.length,
            installed_bytes_after: verified_catalog.record.package.expanded_bytes,
            reclaimed_bytes: 0,
            drain_required: false,
            retained_data: false,
            okf_changes: Vec::new(),
        },
        PlannedStateEvidence {
            state_revision: 1,
            capability_generation: 0,
            receipt_digest: None,
        },
    )
    .unwrap();
    draft.package_lock_digest = Some(package_lock.descriptor_digest().unwrap());
    draft.validate_unbound().unwrap();

    let request = PluginPlanRequest {
        action: PluginLifecycleAction::Install,
        component_id: "use/acme/worker".to_string(),
        version: Some("1.0.0".to_string()),
        channel: Some("stable".to_string()),
    };
    let identity = PluginPlanIdentity {
        operation_id: "plugin-install-reviewed-worker".to_string(),
        created_at_ms: u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap(),
        expires_at_ms: u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap() + 60_000,
    };
    let capability = PluginCapabilityEvidence {
        status: PluginCapabilityEvidenceStatus::Verified,
        observed_at_ms: 1,
        generation: Some(0),
        revision: Some("f".repeat(64)),
        error: None,
    };
    let scope = crate::plugin_manager::default_plan_scope();
    let grant_snapshot = a3s_use_core::PluginWorkspaceGrantSnapshot {
        schema: PLUGIN_WORKSPACE_GRANT_SNAPSHOT_SCHEMA.to_string(),
        scope_id: scope.id.clone(),
        state_revision: 1,
        grants: Vec::new(),
    };
    let policy = PluginAuthorizationPolicy::default();
    let installed_generations = std::collections::BTreeMap::new();
    let upstream_digest = "a".repeat(64);
    let raw_plan = serde_json::json!({
        "dryRun": true,
        "planDigest": upstream_digest,
        "pluginOperationPlan": draft,
        "plans": [{
            "component": "use/acme/worker",
            "action": "install",
            "mutates": true,
            "resolvedRegistryPackages": {"use/acme/worker": resolved.package},
            "verifiedPluginCatalogRecords": {"use/acme/worker": verified_catalog},
            "verifiedPluginPlanningBundles": {"use/acme/worker": planning},
            "cognitivePackageLocks": {"use/acme/worker": package_lock},
        }],
    });
    let missing_runtime_host = crate::plugin_manager::PluginRuntimeHost::default();
    let missing_assignment = match plan_artifact::prepare(
        plan_artifact::HostPlanContext {
            authorization: &policy,
            actor: PlanActor::User,
            scope: &scope,
            observed: plan_artifact::ObservedPlanState {
                capability: &capability,
                state_revision: 1,
            },
            identity: &identity,
            grant_snapshot: Some(&grant_snapshot),
            installed_generations: &installed_generations,
            runtime_host: &missing_runtime_host,
        },
        &request,
        upstream_digest.clone(),
        None,
        raw_plan.clone(),
    )
    .await
    {
        Ok(_) => panic!("managed planning must reject a missing Runtime assignment"),
        Err(error) => error,
    };
    assert!(missing_assignment
        .to_string()
        .contains("no explicit host Runtime assignment"));
    assert!(!server
        .requests()
        .iter()
        .any(|path| path == &format!("/targets/{target_name}")));

    let runtime = Arc::new(MutableRuntime::new(task_runtime_capabilities(
        "runtime-build-1",
        "application/vnd.oci.image.manifest.v1+json",
    )));
    let mut runtime_registry = RuntimeClientRegistry::new();
    runtime_registry
        .register(Arc::new(StaticRuntimeFactory {
            provider_id: ProviderId::parse("test-runtime").unwrap(),
            client: runtime.clone(),
        }))
        .unwrap();
    let runtime_host = crate::plugin_manager::PluginRuntimeHost::new(
        runtime_registry,
        vec![RuntimeProviderAssignment::new(
            PlanQualifiedSurfaceRef {
                package_id: "acme/worker".to_string(),
                surface: surface.clone(),
            },
            "test-runtime",
        )
        .unwrap()],
        Arc::new(crate::components::UnavailableRuntimeServiceHost),
    )
    .unwrap();
    let prepared = plan_artifact::prepare(
        plan_artifact::HostPlanContext {
            authorization: &policy,
            actor: PlanActor::User,
            scope: &scope,
            observed: plan_artifact::ObservedPlanState {
                capability: &capability,
                state_revision: 1,
            },
            identity: &identity,
            grant_snapshot: Some(&grant_snapshot),
            installed_generations: &installed_generations,
            runtime_host: &runtime_host,
        },
        &request,
        upstream_digest.clone(),
        None,
        raw_plan,
    )
    .await
    .unwrap();
    let envelope = prepared.plugin_operation_plan.clone().unwrap();
    assert!(envelope.plan.workspace_impacts[0]
        .grant_after_digest
        .is_some());
    assert_eq!(envelope.plan.providers.len(), 1);
    assert_eq!(envelope.plan.providers[0].provider_id, "test-runtime");
    assert_eq!(
        envelope.plan.providers[0].provider_build_id,
        "runtime-build-1"
    );

    let installed_record = component_paths
        .state_root
        .join("use/extensions/acme/worker.json");
    let use_install = write_capability_use_fixture(temporary.path(), &installed_record);
    component_paths.set_install_override("A3S_USE_INSTALL_DIR", use_install);
    let child_mutation_log = temporary.path().join("forbidden-child-mutation.log");
    component_paths.current_exe = write_forbidden_a3s(temporary.path(), &child_mutation_log);
    let workspace = temporary.path().join("workspace");
    let config_path = temporary.path().join("config/a3s.acl");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let manager = PluginManager::new_with_policy_and_runtime(
        config_path,
        workspace,
        component_paths.clone(),
        registry_store,
        PluginManagerPolicy {
            offline: false,
            authorization: policy,
        },
        runtime_host,
    );
    let stored = manager
        .operation_store
        .create_plan_for_actor(NewPluginPlan {
            identity,
            request,
            actor: PlanActor::User,
            scope,
            plan_digest: prepared.plan_digest.clone(),
            upstream_plan_digest: prepared.upstream_plan_digest,
            capability_state: capability,
            plan: prepared.plan,
            plugin_operation_plan: prepared.plugin_operation_plan,
            planning_bundles: prepared.planning_bundles,
            grant_snapshot: Some(grant_snapshot),
            managed_plan_request: None,
        })
        .await
        .unwrap();
    let apply_request = PluginApplyRequest {
        operation_id: stored.operation_id.clone(),
        plan_digest: format!("sha256:{}", stored.plan_digest),
    };

    runtime.set_provider_build("runtime-build-2");
    let drift = manager
        .apply_confirmed_operation(&apply_request)
        .await
        .unwrap_err();
    assert!(drift
        .to_string()
        .contains("reviewed Runtime provider evidence cannot be reconstructed"));
    assert!(!installed_record.exists());
    assert!(!child_mutation_log.exists());
    assert!(!server
        .requests()
        .iter()
        .any(|path| path == &format!("/targets/{target_name}")));

    runtime.set_provider_build("runtime-build-1");
    let applied = manager
        .apply_confirmed_operation(&apply_request)
        .await
        .unwrap();

    assert_eq!(applied["replayed"], false);
    assert!(!child_mutation_log.exists());
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|path| *path == &format!("/targets/{target_name}"))
            .count(),
        1
    );
    let grant = WorkspaceGrantStore::new(component_paths.state_root.join("use"))
        .observe("current", "acme/worker", &package_digest)
        .await
        .unwrap()
        .unwrap();
    let StoredWorkspaceGrant::Granted(receipt) = grant else {
        panic!("expected an active host-reviewed Grant receipt");
    };
    assert_eq!(receipt.grant.scope_id, "current");
    assert_eq!(receipt.grant.package_id, "acme/worker");
    assert_eq!(receipt.grant.package_digest, package_digest);
    assert_eq!(receipt.grant.authority.actor, PlanActor::User);
    assert!(receipt.grant.authority.confirmation_digest.is_some());

    let replayed = manager
        .apply_confirmed_operation(&apply_request)
        .await
        .unwrap();
    assert_eq!(replayed["replayed"], true);
    assert_eq!(replayed["operations"], applied["operations"]);
    assert!(!child_mutation_log.exists());
}

fn prefixed_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

struct StaticRuntimeFactory {
    provider_id: ProviderId,
    client: Arc<MutableRuntime>,
}

#[async_trait]
impl RuntimeProviderFactory for StaticRuntimeFactory {
    fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    async fn create(&self) -> RuntimeResult<Arc<dyn RuntimeClient>> {
        Ok(self.client.clone())
    }
}

struct MutableRuntime {
    capabilities: Mutex<RuntimeCapabilities>,
}

impl MutableRuntime {
    fn new(capabilities: RuntimeCapabilities) -> Self {
        Self {
            capabilities: Mutex::new(capabilities),
        }
    }

    fn set_provider_build(&self, provider_build: &str) {
        self.capabilities.lock().unwrap().provider_build = provider_build.to_string();
    }
}

#[async_trait]
impl RuntimeClient for MutableRuntime {
    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
        Ok(self.capabilities.lock().unwrap().clone())
    }

    async fn apply(&self, _request: &RuntimeApplyRequest) -> RuntimeResult<RuntimeObservation> {
        Err(unexpected_runtime_operation("apply"))
    }

    async fn inspect(&self, _unit_id: &str) -> RuntimeResult<RuntimeInspection> {
        Err(unexpected_runtime_operation("inspect"))
    }

    async fn stop(&self, _request: &RuntimeActionRequest) -> RuntimeResult<RuntimeInspection> {
        Err(unexpected_runtime_operation("stop"))
    }

    async fn remove(&self, _request: &RuntimeActionRequest) -> RuntimeResult<RuntimeRemoval> {
        Err(unexpected_runtime_operation("remove"))
    }

    async fn logs(&self, _query: &RuntimeLogQuery) -> RuntimeResult<Vec<RuntimeLogChunk>> {
        Err(unexpected_runtime_operation("logs"))
    }

    async fn exec(&self, _request: &RuntimeExecRequest) -> RuntimeResult<RuntimeExecResult> {
        Err(unexpected_runtime_operation("exec"))
    }
}

fn unexpected_runtime_operation(operation: &str) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "test Runtime received unexpected {operation} operation"
    ))
}

fn task_runtime_capabilities(
    provider_build: &str,
    artifact_media_type: &str,
) -> RuntimeCapabilities {
    RuntimeCapabilities {
        schema: RuntimeCapabilities::SCHEMA.to_string(),
        provider_id: ProviderId::parse("test-runtime").unwrap(),
        provider_build: provider_build.to_string(),
        unit_classes: vec![RuntimeUnitClass::Task],
        artifact_media_types: vec![artifact_media_type.to_string()],
        isolation_levels: vec![IsolationLevel::Container],
        network_modes: vec![NetworkMode::None],
        mount_kinds: Vec::new(),
        health_check_kinds: Vec::new(),
        resource_controls: vec![
            ResourceControl::Cpu,
            ResourceControl::Memory,
            ResourceControl::Pids,
            ResourceControl::EphemeralStorage,
            ResourceControl::ExecutionTimeout,
        ],
        features: vec![
            RuntimeFeature::DurableIdentity,
            RuntimeFeature::Logs,
            RuntimeFeature::Stop,
            RuntimeFeature::Remove,
        ],
    }
}
