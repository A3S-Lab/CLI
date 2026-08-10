use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use a3s_runtime::contract::{
    IsolationLevel, NetworkMode, ResourceControl, RuntimeActionRequest, RuntimeApplyRequest,
    RuntimeCapabilities, RuntimeEvidence, RuntimeExecRequest, RuntimeExecResult, RuntimeFeature,
    RuntimeInspection, RuntimeLogChunk, RuntimeLogQuery, RuntimeLogStream, RuntimeObservation,
    RuntimeRemoval, RuntimeUnitClass, RuntimeUnitState,
};
use a3s_runtime::{
    ProviderId, RuntimeClient, RuntimeClientRegistry, RuntimeError, RuntimeProviderFactory,
    RuntimeResult,
};
use a3s_use::plugin_runtime::{
    RuntimeBindingStore, RuntimeProviderAssignment, RuntimeTaskDispatchRequest,
    RuntimeTaskInvocation,
};
use a3s_use_core::{
    CatalogAvailability, CatalogPlanningTarget, CatalogSurface, ExecutablePlanningSurface,
    PlanActor, PlanPackageRole, PlanQualifiedSurfaceRef, PlannedOperationImpact,
    PlannedStateEvidence, PlanningArtifactRef, PlanningSurfaceActivation, PluginCatalogRecord,
    PluginOperationAction, PluginOperationPlanDraft, PluginPlanningBundle, PluginReleaseChannel,
    PluginSurfaceKind, PluginSurfaceRef, ToolReleaseDescriptor, ToolWorkloadClass,
    PLUGIN_CATALOG_SCHEMA_V3, PLUGIN_PLANNING_BUNDLE_SCHEMA,
    PLUGIN_WORKSPACE_GRANT_SNAPSHOT_SCHEMA,
};
use a3s_use_extension::{
    ExtensionLifecycleIdentity, ExtensionPaths, ExtensionRegistry, StoredWorkspaceGrant,
    WorkspaceGrantStore,
};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

use super::*;
use crate::plugin_manager::capability::PluginCapabilityEvidenceStatus;
use crate::plugin_manager::operation::store::{NewPluginPlan, PluginPlanIdentity};
use crate::plugin_manager::process::{PluginLifecycleAction, PluginPlanRequest};
use crate::plugin_manager::{PluginAuthorizationPolicy, PluginManagerPolicy};
use crate::tuf_test_support::{
    host_target, package_directory_archive, TestRepository, TestServer, TestTarget, FUTURE,
};

#[tokio::test]
#[cfg_attr(
    windows,
    ignore = "requires the real A3S_USE_E2E_BIN supplied by the host integration gate"
)]
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
    let registry_store = RegistryStore::for_test(temporary.path());
    registry_store
        .add_test_source("fixture", server.base_url(), &repository.root_sha256)
        .await
        .unwrap();

    let mut component_paths = ComponentPaths::for_test(temporary.path());
    let _use_home = RealUseHomeOverride::for_component_paths(&component_paths);
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
    let policy = PluginAuthorizationPolicy::default();
    let observation_manager = PluginManager::new_with_policy(
        config_path.clone(),
        workspace.clone(),
        component_paths.clone(),
        registry_store.clone(),
        PluginManagerPolicy {
            offline: false,
            authorization: policy.clone(),
        },
    );
    let capability = crate::plugin_manager::capability::observe(&observation_manager).await;
    assert_eq!(capability.status, PluginCapabilityEvidenceStatus::Verified);
    assert_eq!(capability.generation, Some(0));
    drop(observation_manager);

    let resolved = registry_store
        .resolve_package(Some("fixture"), "acme/worker", Some("1.0.0"), "stable")
        .await
        .unwrap();
    assert_eq!(resolved.planning_bundle.as_ref(), Some(&planning));
    let verified_catalog = resolved.verified_catalog.clone();
    let package_lock = registry_store
        .resolve_cognitive_package_lock(&resolved)
        .await
        .unwrap();
    let planning_bundles = registry_store
        .resolve_cognitive_package_planning_bundles(&resolved, &package_lock)
        .await
        .unwrap();
    let registry_source_revision = resolved.registry_source_revision.clone();
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
        registry_name: Some("fixture".to_string()),
    };
    let identity = PluginPlanIdentity {
        operation_id: "plugin-install-reviewed-worker".to_string(),
        created_at_ms: u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap(),
        expires_at_ms: u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap() + 60_000,
    };
    let scope = crate::plugin_manager::default_plan_scope();
    let grant_snapshot = a3s_use_core::PluginWorkspaceGrantSnapshot {
        schema: PLUGIN_WORKSPACE_GRANT_SNAPSHOT_SCHEMA.to_string(),
        scope_id: scope.id.clone(),
        state_revision: 1,
        grants: Vec::new(),
    };
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
            "registrySourceRevision": registry_source_revision,
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
    let runtime_host = managed_runtime_host(runtime.clone(), true);
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

    let manager = PluginManager::new_with_policy_and_runtime(
        config_path.clone(),
        workspace.clone(),
        component_paths.clone(),
        registry_store.clone(),
        PluginManagerPolicy {
            offline: false,
            authorization: policy.clone(),
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
            registry_source_revision: store::registry_source_revision(&prepared.plan).unwrap(),
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
    assert!(
        drift
            .to_string()
            .contains("reviewed Runtime provider evidence cannot be reconstructed"),
        "unexpected pre-mutation rejection: {drift}"
    );
    assert!(!installed_record.exists());
    assert!(!child_mutation_log.exists());
    assert!(!server
        .requests()
        .iter()
        .any(|path| path == &format!("/targets/{target_name}")));

    runtime.set_provider_build("runtime-build-1");
    let applied = match manager.apply_confirmed_operation(&apply_request).await {
        Ok(applied) => applied,
        Err(error) => {
            let observed = manager.installation_snapshot().await;
            panic!("managed apply failed: {error}; post-mutation capability: {observed:?}");
        }
    };

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
    let default_scope = crate::plugin_manager::default_plan_scope();
    let grant = WorkspaceGrantStore::new(component_paths.state_root.join("use"))
        .observe(&default_scope.id, "acme/worker", &package_digest)
        .await
        .unwrap()
        .unwrap();
    let StoredWorkspaceGrant::Granted(receipt) = grant else {
        panic!("expected an active host-reviewed Grant receipt");
    };
    assert_eq!(receipt.grant.scope_id, default_scope.id);
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

    let installed = crate::components::code_cognitive_package_manager(
        &component_paths,
        crate::plugin_manager::default_plan_scope(),
    )
    .unwrap()
    .registry()
    .get("acme/worker")
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        installed.plan_ready_planning_bundle().unwrap(),
        Some(&planning)
    );

    server.clear_requests();
    server.replace_routes(Default::default());
    drop(manager);

    let manager_for = |runtime_host| {
        PluginManager::new_with_policy_and_runtime(
            config_path.clone(),
            workspace.clone(),
            component_paths.clone(),
            registry_store.clone(),
            PluginManagerPolicy {
                offline: false,
                authorization: policy.clone(),
            },
            runtime_host,
        )
    };

    let retirement_planner = manager_for(managed_runtime_host(runtime.clone(), false));
    let enabled_state = crate::components::code_cognitive_package_manager(
        &component_paths,
        crate::plugin_manager::default_plan_scope(),
    )
    .unwrap()
    .observe_package("acme/worker")
    .await
    .unwrap();
    let disable_plan = retirement_planner
        .plan_package_enablement(&PluginEnablementPlanRequest {
            component_id: "use/acme/worker".to_string(),
            enabled: false,
            expected_package_generation: enabled_state.package_generation,
        })
        .await
        .unwrap();
    assert_eq!(disable_plan["status"], "planned");
    assert_eq!(
        disable_plan["plan"]["plan"]["providers"],
        serde_json::json!([])
    );
    let disable_request = PluginEnablementApplyRequest {
        operation_id: disable_plan["operationId"].as_str().unwrap().to_string(),
        plan_digest: disable_plan["canonicalPlanDigest"]
            .as_str()
            .unwrap()
            .to_string(),
    };
    drop(retirement_planner);

    let retirement_applier = manager_for(managed_runtime_host(runtime.clone(), false));
    let disabled = retirement_applier
        .apply_confirmed_package_enablement(&disable_request)
        .await
        .unwrap();
    assert_eq!(disabled["state"]["desired"], "installed-disabled");
    assert_eq!(disabled["replayed"], false);
    let disabled_generation = disabled["state"]["packageGeneration"].as_u64().unwrap();
    let binding_store = RuntimeBindingStore::new(component_paths.state_root.join("use"));
    let stopped_binding = binding_store
        .get_generation(
            &crate::plugin_manager::default_plan_scope(),
            &PlanQualifiedSurfaceRef {
                package_id: "acme/worker".to_string(),
                surface: surface.clone(),
            },
            installed.receipt.lifecycle_generation.unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    drop(retirement_applier);

    let activation_planner = manager_for(managed_runtime_host(runtime.clone(), true));
    let enable_plan = activation_planner
        .plan_package_enablement(&PluginEnablementPlanRequest {
            component_id: "use/acme/worker".to_string(),
            enabled: true,
            expected_package_generation: Some(disabled_generation),
        })
        .await
        .unwrap();
    assert_eq!(enable_plan["status"], "planned");
    assert_eq!(
        enable_plan["plan"]["plan"]["providers"][0]["providerId"],
        "test-runtime"
    );
    let enable_request = PluginEnablementApplyRequest {
        operation_id: enable_plan["operationId"].as_str().unwrap().to_string(),
        plan_digest: enable_plan["canonicalPlanDigest"]
            .as_str()
            .unwrap()
            .to_string(),
    };
    drop(activation_planner);

    runtime.set_provider_build("runtime-build-2");
    let drifted_applier = manager_for(managed_runtime_host(runtime.clone(), true));
    let drift = drifted_applier
        .apply_confirmed_package_enablement(&enable_request)
        .await
        .unwrap_err();
    assert!(drift
        .to_string()
        .contains("Runtime provider evidence changed"));
    drop(drifted_applier);

    runtime.set_provider_build("runtime-build-1");
    let activation_applier = manager_for(managed_runtime_host(runtime.clone(), true));
    let enabled = activation_applier
        .apply_confirmed_package_enablement(&enable_request)
        .await
        .unwrap();
    assert_eq!(enabled["state"]["desired"], "enabled");
    assert_eq!(enabled["replayed"], true);
    let rebound = binding_store
        .get_generation(
            &crate::plugin_manager::default_plan_scope(),
            &PlanQualifiedSurfaceRef {
                package_id: "acme/worker".to_string(),
                surface,
            },
            installed.receipt.lifecycle_generation.unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        rebound.semantics_profile_digest(),
        stopped_binding.semantics_profile_digest()
    );

    let lifecycle_identity = ExtensionLifecycleIdentity::new(
        "acme/worker",
        package_digest,
        format!("sha256:{}", installed.receipt.manifest_sha256),
        installed.receipt.lifecycle_generation.unwrap(),
    )
    .unwrap();
    Box::pin(exercise_runtime_task_dispatch(
        activation_applier,
        config_path,
        workspace,
        component_paths,
        registry_store,
        policy,
        runtime,
        lifecycle_identity,
    ))
    .await;
    assert!(server.requests().is_empty());
    assert!(!child_mutation_log.exists());
}

#[allow(clippy::too_many_arguments)]
async fn exercise_runtime_task_dispatch(
    activation_manager: PluginManager,
    config_path: std::path::PathBuf,
    workspace: std::path::PathBuf,
    component_paths: ComponentPaths,
    registry_store: RegistryStore,
    policy: PluginAuthorizationPolicy,
    runtime: Arc<MutableRuntime>,
    lifecycle_identity: ExtensionLifecycleIdentity,
) {
    let dispatch_request = |invocation_id: &str, request_id: &str| {
        RuntimeTaskDispatchRequest::new(
            lifecycle_identity.clone(),
            crate::plugin_manager::default_plan_scope(),
            "convert",
            RuntimeTaskInvocation::new(
                invocation_id,
                vec!["--format".to_string(), "json".to_string()],
            )
            .unwrap(),
            request_id,
            None,
        )
        .unwrap()
    };

    let first_execution = activation_manager
        .invoke_runtime_task(dispatch_request("invoke-first", "request-first"))
        .await
        .unwrap();
    assert_eq!(first_execution.stdout, "{\"ok\":true}\n");
    assert_eq!(first_execution.stderr, "");
    assert_eq!(first_execution.exit_code, 0);
    assert_eq!(
        *runtime.last_args.lock().unwrap(),
        ["--format".to_string(), "json".to_string()]
    );

    // A fresh Code manager has no operation-plan memory. Exact Registry and
    // Runtime binding receipts are sufficient to reconnect the reviewed host
    // provider without selecting a newer assignment.
    drop(activation_manager);
    let restarted = Arc::new(PluginManager::new_with_policy_and_runtime(
        config_path,
        workspace,
        component_paths.clone(),
        registry_store,
        PluginManagerPolicy {
            offline: false,
            authorization: policy,
        },
        managed_runtime_host(runtime.clone(), true),
    ));
    restarted
        .invoke_runtime_task(dispatch_request("invoke-restarted", "request-restarted"))
        .await
        .unwrap();

    // Accepted calls keep the exact package generation leased until output
    // capture and Runtime cleanup complete. Hide rejects later calls while
    // drain remains blocked on the already accepted invocation.
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    runtime.set_apply_gate(started.clone(), release.clone());
    let active_manager = restarted.clone();
    let active_request = dispatch_request("invoke-active", "request-active");
    let active =
        tokio::spawn(async move { active_manager.invoke_runtime_task(active_request).await });
    tokio::time::timeout(Duration::from_secs(1), started.notified())
        .await
        .unwrap();

    let extension_paths = ExtensionPaths::new(
        component_paths.data_root.join("use"),
        component_paths.state_root.join("use"),
    );
    let extension_registry = ExtensionRegistry::new(extension_paths);
    extension_registry
        .hide_lifecycle_package(&lifecycle_identity)
        .await
        .unwrap();
    let drain_error = extension_registry
        .drain_lifecycle_package(&lifecycle_identity, Duration::from_millis(10))
        .await
        .unwrap_err();
    assert_eq!(drain_error.code, "use.extension.drain_timeout");

    let rejected = restarted
        .invoke_runtime_task(dispatch_request("invoke-late", "request-late"))
        .await
        .unwrap_err();
    assert!(matches!(rejected, PluginManagerError::OperationFailed(_)));
    assert!(rejected
        .to_string()
        .contains("use.plugin.runtime.generation_unavailable"));

    release.notify_one();
    active.await.unwrap().unwrap();
    extension_registry
        .drain_lifecycle_package(&lifecycle_identity, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(runtime.apply_count.load(Ordering::SeqCst), 3);
    assert_eq!(runtime.remove_count.load(Ordering::SeqCst), 3);
}

struct RealUseHomeOverride {
    previous_use_home: Option<std::ffi::OsString>,
    previous_data_home: Option<std::ffi::OsString>,
    previous_state_home: Option<std::ffi::OsString>,
}

impl RealUseHomeOverride {
    fn for_component_paths(component_paths: &ComponentPaths) -> Option<Self> {
        std::env::var_os("A3S_USE_E2E_BIN")?;
        let previous_use_home = std::env::var_os("A3S_USE_HOME");
        let previous_data_home = std::env::var_os("A3S_DATA_HOME");
        let previous_state_home = std::env::var_os("A3S_STATE_HOME");
        std::env::remove_var("A3S_USE_HOME");
        std::env::set_var("A3S_DATA_HOME", &component_paths.data_root);
        std::env::set_var("A3S_STATE_HOME", &component_paths.state_root);
        Some(Self {
            previous_use_home,
            previous_data_home,
            previous_state_home,
        })
    }
}

impl Drop for RealUseHomeOverride {
    fn drop(&mut self) {
        restore_environment("A3S_USE_HOME", self.previous_use_home.take());
        restore_environment("A3S_DATA_HOME", self.previous_data_home.take());
        restore_environment("A3S_STATE_HOME", self.previous_state_home.take());
    }
}

fn restore_environment(name: &str, previous: Option<std::ffi::OsString>) {
    match previous {
        Some(previous) => std::env::set_var(name, previous),
        None => std::env::remove_var(name),
    }
}

fn prefixed_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

struct StaticRuntimeFactory {
    provider_id: ProviderId,
    client: Arc<MutableRuntime>,
}

fn managed_runtime_host(
    runtime: Arc<MutableRuntime>,
    assigned: bool,
) -> crate::plugin_manager::PluginRuntimeHost {
    let mut registry = RuntimeClientRegistry::new();
    registry
        .register(Arc::new(StaticRuntimeFactory {
            provider_id: ProviderId::parse("test-runtime").unwrap(),
            client: runtime,
        }))
        .unwrap();
    let assignments = assigned
        .then(|| {
            RuntimeProviderAssignment::new(
                PlanQualifiedSurfaceRef {
                    package_id: "acme/worker".to_string(),
                    surface: PluginSurfaceRef {
                        kind: PluginSurfaceKind::Tool,
                        id: "convert".to_string(),
                    },
                },
                "test-runtime",
            )
            .unwrap()
        })
        .into_iter()
        .collect();
    crate::plugin_manager::PluginRuntimeHost::new(
        registry,
        assignments,
        Arc::new(crate::components::UnavailableRuntimeServiceHost),
    )
    .unwrap()
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
    observation: Mutex<Option<RuntimeObservation>>,
    apply_gate: Mutex<Option<(Arc<Notify>, Arc<Notify>)>>,
    apply_count: AtomicUsize,
    remove_count: AtomicUsize,
    last_args: Mutex<Vec<String>>,
}

impl MutableRuntime {
    fn new(capabilities: RuntimeCapabilities) -> Self {
        Self {
            capabilities: Mutex::new(capabilities),
            observation: Mutex::new(None),
            apply_gate: Mutex::new(None),
            apply_count: AtomicUsize::new(0),
            remove_count: AtomicUsize::new(0),
            last_args: Mutex::new(Vec::new()),
        }
    }

    fn set_provider_build(&self, provider_build: &str) {
        self.capabilities.lock().unwrap().provider_build = provider_build.to_string();
    }

    fn set_apply_gate(&self, started: Arc<Notify>, release: Arc<Notify>) {
        *self.apply_gate.lock().unwrap() = Some((started, release));
    }
}

#[async_trait]
impl RuntimeClient for MutableRuntime {
    async fn capabilities(&self) -> RuntimeResult<RuntimeCapabilities> {
        Ok(self.capabilities.lock().unwrap().clone())
    }

    async fn apply(&self, request: &RuntimeApplyRequest) -> RuntimeResult<RuntimeObservation> {
        self.apply_count.fetch_add(1, Ordering::SeqCst);
        *self.last_args.lock().unwrap() = request.spec.process.args.clone();
        let gate = self.apply_gate.lock().unwrap().take();
        if let Some((started, release)) = gate {
            started.notify_one();
            release.notified().await;
        }
        let capabilities = self.capabilities.lock().unwrap().clone();
        let spec_digest = request.spec.digest().map_err(RuntimeError::Protocol)?;
        let observation = RuntimeObservation {
            schema: RuntimeObservation::SCHEMA.to_string(),
            unit_id: request.spec.unit_id.clone(),
            generation: request.spec.generation,
            spec_digest: spec_digest.clone(),
            class: request.spec.class,
            state: RuntimeUnitState::Succeeded,
            provider_resource_id: Some("resource-01".to_string()),
            provider_build: Some(capabilities.provider_build.clone()),
            observed_at_ms: 1_000,
            started_at_ms: Some(900),
            finished_at_ms: Some(1_000),
            health: None,
            outputs: Vec::new(),
            usage: None,
            evidence: Some(RuntimeEvidence {
                provider_build: capabilities.provider_build,
                spec_digest,
                semantics_profile_digest: request.spec.semantics_profile_digest.clone(),
                claims: std::collections::BTreeMap::new(),
            }),
            provider_attestation: None,
            failure: None,
        };
        *self.observation.lock().unwrap() = Some(observation.clone());
        Ok(observation)
    }

    async fn inspect(&self, unit_id: &str) -> RuntimeResult<RuntimeInspection> {
        Ok(match self.observation.lock().unwrap().clone() {
            Some(observation) if observation.unit_id == unit_id => RuntimeInspection::Found {
                schema: RuntimeInspection::SCHEMA.to_string(),
                observation: Box::new(observation),
            },
            _ => RuntimeInspection::NotFound {
                schema: RuntimeInspection::SCHEMA.to_string(),
                unit_id: unit_id.to_string(),
                last_generation: None,
            },
        })
    }

    async fn stop(&self, _request: &RuntimeActionRequest) -> RuntimeResult<RuntimeInspection> {
        Err(unexpected_runtime_operation("stop"))
    }

    async fn remove(&self, request: &RuntimeActionRequest) -> RuntimeResult<RuntimeRemoval> {
        self.remove_count.fetch_add(1, Ordering::SeqCst);
        let already_absent = self.observation.lock().unwrap().take().is_none();
        Ok(RuntimeRemoval {
            schema: RuntimeRemoval::SCHEMA.to_string(),
            request_id: request.request_id.clone(),
            unit_id: request.unit_id.clone(),
            generation: request.generation,
            removed_at_ms: 1_200,
            already_absent,
        })
    }

    async fn logs(&self, query: &RuntimeLogQuery) -> RuntimeResult<Vec<RuntimeLogChunk>> {
        if query.cursor.is_some() || query.stream != Some(RuntimeLogStream::Stdout) {
            return Ok(Vec::new());
        }
        Ok(vec![RuntimeLogChunk {
            schema: RuntimeLogChunk::SCHEMA.to_string(),
            cursor: "stdout-1".to_string(),
            sequence: 1,
            observed_at_ms: 1_000,
            stream: RuntimeLogStream::Stdout,
            data: "{\"ok\":true}\n".to_string(),
        }])
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
