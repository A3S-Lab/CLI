use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use a3s_use_core::{
    CatalogAvailability, CatalogPlanningTarget, CatalogSurface, ExecutablePlanningSurface,
    PlanActor, PlanEnforcementProfile, PlanPackageRole, PlanQualifiedSurfaceRef,
    PlannedOperationImpact, PlannedProviderEvidence, PlannedStateEvidence, PlanningArtifactRef,
    PlanningSurfaceActivation, PluginCatalogRecord, PluginHostApplyRequest,
    PluginHostEnablementPlanRequest, PluginHostEnablementPlanStatus, PluginHostManager,
    PluginHostObservationRequest, PluginHostObservationStatus, PluginManagedScope,
    PluginOperationAction, PluginOperationPlanDraft, PluginPackageId, PluginPackageLock,
    PluginPlanningBundle, PluginReleaseChannel, PluginSurfaceKind, PluginSurfaceRef,
    ToolReleaseDescriptor, ToolWorkloadClass, PLUGIN_CATALOG_SCHEMA_V3,
    PLUGIN_HOST_APPLY_REQUEST_SCHEMA, PLUGIN_HOST_ENABLEMENT_PLAN_REQUEST_SCHEMA,
    PLUGIN_HOST_OBSERVATION_REQUEST_SCHEMA, PLUGIN_MANAGED_SCOPE_SCHEMA,
    PLUGIN_PLANNING_BUNDLE_SCHEMA, PLUGIN_WORKSPACE_GRANT_SNAPSHOT_SCHEMA,
};
use a3s_use_extension::{StoredWorkspaceGrant, WorkspaceGrantStore};
use sha2::{Digest, Sha256};

use super::*;
use crate::plugin_manager::capability::{PluginCapabilityEvidence, PluginCapabilityEvidenceStatus};
use crate::plugin_manager::operation::store::{NewPluginPlan, PluginPlanIdentity};
use crate::plugin_manager::process::{PluginLifecycleAction, PluginPlanRequest};
use crate::plugin_manager::{
    ManagedPluginHostManager, PluginAuthorizationPolicy, PluginManagerPolicy,
};
use crate::tuf_test_support::{
    host_target, package_directory_archive, TestRepository, TestServer, TestTarget, FUTURE,
};

const TOOL_TASK_RELEASE: &[u8] = br#"{"artifact":{"digest":"sha256:7777777777777777777777777777777777777777777777777777777777777777","mediaType":"application/vnd.oci.image.manifest.v1+json","sizeBytes":1048576},"compatibility":[{"component":"a3s-runtime","versionRequirement":">=0.2.0, <0.3.0"},{"component":"a3s-use","versionRequirement":">=0.3.0, <0.4.0"}],"dependencies":[],"kind":"tool","name":"a3s/example-task-tool","provenance":{"buildOperationId":"github-actions:29694368226","builderId":"github-actions:A3S-Lab/Use","commitSha":"1234567890abcdef1234567890abcdef12345678","manifestDigest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","sourceRepository":"https://github.com/A3S-Lab/Use.git"},"schema":"a3s.use.tool-release.v1","version":"1.0.0","workload":{"class":"task","entrypoint":["/usr/local/bin/example-tool"],"interactive":false,"interface":"cli","maxStderrBytes":1048576,"maxStdoutBytes":4194304,"successExitCodes":[0],"timeoutMs":120000}}"#;

#[tokio::test]
async fn reviewed_permission_graph_persists_exact_grant_without_child_mutation() {
    let temporary = tempfile::tempdir().unwrap();
    let package_root = temporary.path().join("package");
    std::fs::create_dir_all(package_root.join("tools/convert/bin")).unwrap();
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
    executable = "tools/convert/bin/convert"
    command = "acme-worker-convert"
    json_output = true
    interactive = false
    timeout_ms = 120000
    activation = "lazy"
    optional = false
  }
}
"#;
    std::fs::write(package_root.join("a3s-use-extension.acl"), manifest).unwrap();
    std::fs::write(
        package_root.join("README.md"),
        "# Worker\n\nPermission-bearing host Grant fixture.\n",
    )
    .unwrap();
    let executable = package_root.join("tools/convert/bin/convert");
    std::fs::write(
        &executable,
        "#!/bin/sh\nset -eu\nprintf '{\"status\":\"ok\"}\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();

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

    let descriptor = ToolReleaseDescriptor::from_json(TOOL_TASK_RELEASE).unwrap();
    let planning_target = format!("extensions/acme/worker/1.0.0/stable/{target}/planning-v1.json");
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
            descriptor: descriptor.clone(),
            artifact: PlanningArtifactRef {
                uri: format!(
                    "oci://registry.example/acme/worker@{}",
                    descriptor.artifact.digest
                ),
                digest: descriptor.artifact.digest.clone(),
                media_type: descriptor.artifact.media_type.clone(),
            },
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
                target_name,
                custom: Some(serde_json::to_value(&catalog).unwrap()),
            },
            TestTarget {
                archive: planning_bytes,
                target_name: planning_target,
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
    let surface = PluginSurfaceRef {
        kind: PluginSurfaceKind::Tool,
        id: "convert".to_string(),
    };
    let transition = verified_catalog
        .install_transition(PlanPackageRole::Root, std::slice::from_ref(&surface))
        .unwrap();
    let permission = verified_catalog
        .record
        .permission_ceiling
        .surfaces
        .iter()
        .find(|permission| permission.surface == surface)
        .unwrap();
    let provider = native_provider(
        &package_lock,
        &package_digest,
        &surface,
        permission.native_execution,
    );
    let mut draft = PluginOperationPlanDraft::new(
        PluginOperationAction::Install,
        "acme/worker",
        "use/acme/worker",
        vec![transition],
        vec![provider],
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
    draft.validate().unwrap();

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
    let upstream_digest = "a".repeat(64);
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
        },
        &request,
        upstream_digest.clone(),
        None,
        serde_json::json!({
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
        }),
    )
    .unwrap();
    let envelope = prepared.plugin_operation_plan.clone().unwrap();
    assert!(envelope.plan.workspace_impacts[0]
        .grant_after_digest
        .is_some());

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
    let manager = PluginManager::new_with_policy(
        config_path,
        workspace,
        component_paths.clone(),
        registry_store,
        PluginManagerPolicy {
            offline: false,
            authorization: policy,
        },
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
            managed_plan_request: None,
        })
        .await
        .unwrap();
    let apply_request = PluginApplyRequest {
        operation_id: stored.operation_id.clone(),
        plan_digest: format!("sha256:{}", stored.plan_digest),
    };

    let applied = manager
        .apply_confirmed_operation(&apply_request)
        .await
        .unwrap();

    assert_eq!(applied["replayed"], false);
    assert!(!child_mutation_log.exists());
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

    let manager = Arc::new(manager);
    let host = ManagedPluginHostManager::new(
        manager,
        "host:permission-test",
        "cli:0.11.1:permission-test",
    )
    .unwrap();
    let managed_scope = PluginManagedScope {
        schema: PLUGIN_MANAGED_SCOPE_SCHEMA.to_string(),
        host_id: "host:permission-test".to_string(),
        scope_id: "current".to_string(),
        authority_id: "cloud:permission-test".to_string(),
        fence_generation: 1,
        fence_digest: format!("sha256:{}", "d".repeat(64)),
    };
    host.fence_store()
        .initialize(managed_scope.clone())
        .await
        .unwrap();
    let capabilities_digest = host
        .capabilities()
        .await
        .unwrap()
        .descriptor_digest()
        .unwrap();
    let package_id = PluginPackageId::parse("acme/worker").unwrap();
    let observation = host
        .observe(PluginHostObservationRequest {
            schema: PLUGIN_HOST_OBSERVATION_REQUEST_SCHEMA.to_string(),
            request_id: "request:observe:permission-worker".to_string(),
            assignment_generation: 1,
            capabilities_digest: capabilities_digest.clone(),
            scope: managed_scope.clone(),
            package_id: package_id.clone(),
        })
        .await
        .unwrap();
    let PluginHostObservationStatus::Available { state: before } = observation.status else {
        panic!("expected the installed permission-bearing package");
    };
    let plan = host
        .plan_enablement(PluginHostEnablementPlanRequest {
            schema: PLUGIN_HOST_ENABLEMENT_PLAN_REQUEST_SCHEMA.to_string(),
            request_id: "request:disable:permission-worker".to_string(),
            assignment_generation: 1,
            capabilities_digest: capabilities_digest.clone(),
            scope: managed_scope.clone(),
            package_id: package_id.clone(),
            expected_package_generation: before.package_generation.unwrap(),
            enabled: false,
        })
        .await
        .unwrap();
    assert_eq!(plan.status, PluginHostEnablementPlanStatus::Planned);
    let envelope = plan.plan.as_ref().unwrap();
    assert!(envelope.plan.workspace_impacts[0]
        .grant_before_digest
        .is_some());
    let unconfirmed = PluginHostApplyRequest {
        schema: PLUGIN_HOST_APPLY_REQUEST_SCHEMA.to_string(),
        request_id: "request:apply-disable:permission-worker".to_string(),
        assignment_generation: 1,
        capabilities_digest,
        scope: managed_scope,
        package_id,
        operation_id: envelope.plan.operation_id.clone(),
        plan_digest: envelope.plan_digest.clone(),
        confirmation: None,
    };
    assert_eq!(
        host.apply(unconfirmed).await.unwrap_err().code,
        "use.plugin.plan_confirmation_mismatch"
    );
    let grant_after_rejection = WorkspaceGrantStore::new(component_paths.state_root.join("use"))
        .observe("current", "acme/worker", &package_digest)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        grant_after_rejection,
        StoredWorkspaceGrant::Granted(_)
    ));
    assert!(!component_paths
        .state_root
        .join("plugin-manager/managed-host/reviewed-enablement/apply-intents")
        .join(format!(
            "{:x}.json",
            Sha256::digest(envelope.plan.operation_id.as_bytes())
        ))
        .exists());
}

fn native_provider(
    package_lock: &PluginPackageLock,
    package_digest: &str,
    surface: &PluginSurfaceRef,
    native_execution: bool,
) -> PlannedProviderEvidence {
    let target = &package_lock.host.target;
    let use_version = &package_lock.host.use_version;
    PlannedProviderEvidence {
        surface: PlanQualifiedSurfaceRef {
            package_id: "acme/worker".to_string(),
            surface: surface.clone(),
        },
        provider_id: "a3s-use-native-launcher".to_string(),
        provider_build_id: format!("a3s-use:{use_version}:{target}"),
        capability_digest: prefixed_digest(
            format!("a3s-use-native-launcher-v1\n{use_version}\n{target}").as_bytes(),
        ),
        semantics_profile_digest: prefixed_digest(
            format!(
                "a3s-use-static-surface-v1\nacme/worker\n{:?}\n{}\n{package_digest}",
                surface.kind, surface.id
            )
            .as_bytes(),
        ),
        enforcement: if native_execution {
            PlanEnforcementProfile::NativeUnconfined
        } else {
            PlanEnforcementProfile::Container
        },
    }
}

fn prefixed_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
