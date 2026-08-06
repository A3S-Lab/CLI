use std::sync::Arc;

use a3s_use_core::{
    PluginDesiredState, PluginHostEnablementRequest, PluginHostManager,
    PluginHostObservationRequest, PluginHostObservationStatus, PluginManagedScope,
    PluginObservedState, PluginPackageId, PLUGIN_HOST_ENABLEMENT_REQUEST_SCHEMA,
    PLUGIN_HOST_OBSERVATION_REQUEST_SCHEMA, PLUGIN_MANAGED_SCOPE_SCHEMA,
};

use super::*;
use crate::components::ComponentPaths;
use crate::plugin_manager::PluginManagerPolicy;
use crate::registry::RegistryStore;

fn managed_scope(generation: u64, digest: char) -> PluginManagedScope {
    PluginManagedScope {
        schema: PLUGIN_MANAGED_SCOPE_SCHEMA.to_string(),
        host_id: "host:node-01".to_string(),
        scope_id: "workspace:research".to_string(),
        authority_id: "cloud:organization-01".to_string(),
        fence_generation: generation,
        fence_digest: format!("sha256:{}", digest.to_string().repeat(64)),
    }
}

#[tokio::test]
async fn enablement_intent_binds_the_complete_request_before_lifecycle() {
    let temporary = tempfile::tempdir().unwrap();
    let new_host = || {
        ManagedPluginHostManager::new(manager(temporary.path()), "host:node-01", "cli:0.11.1:test")
            .unwrap()
    };
    let scope = managed_scope(7, 'a');
    let host = new_host();
    host.fence_store().initialize(scope.clone()).await.unwrap();
    let capabilities_digest = host
        .capabilities()
        .await
        .unwrap()
        .descriptor_digest()
        .unwrap();
    let request = PluginHostEnablementRequest {
        schema: PLUGIN_HOST_ENABLEMENT_REQUEST_SCHEMA.to_string(),
        request_id: "request:enable:missing-0001".to_string(),
        operation_id: "operation:enable:missing-0001".to_string(),
        assignment_generation: 4,
        capabilities_digest,
        scope: scope.clone(),
        package_id: PluginPackageId::parse("acme/missing").unwrap(),
        expected_package_generation: 1,
        enabled: true,
    };

    assert_eq!(
        host.set_enablement(request.clone()).await.unwrap_err().code,
        "use.extension.not_installed"
    );
    drop(host);

    let restarted = new_host();
    restarted.fence_store().initialize(scope).await.unwrap();
    let mut conflicting = request.clone();
    conflicting.request_id = "request:enable:missing-changed".to_string();
    assert_eq!(
        restarted
            .set_enablement(conflicting)
            .await
            .unwrap_err()
            .code,
        "use.plugin.host_enablement_operation_conflict"
    );
    assert_eq!(
        restarted.set_enablement(request).await.unwrap_err().code,
        "use.extension.not_installed"
    );
}

fn manager(root: &std::path::Path) -> Arc<PluginManager> {
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    Arc::new(PluginManager::new_with_policy(
        root.join("config.acl"),
        workspace,
        ComponentPaths::for_test(root),
        RegistryStore::new(root.join("registries")),
        PluginManagerPolicy::default(),
    ))
}

#[tokio::test]
async fn fence_is_explicit_exact_and_monotonic() {
    let temporary = tempfile::tempdir().unwrap();
    let host =
        ManagedPluginHostManager::new(manager(temporary.path()), "host:node-01", "cli:0.11.1:test")
            .unwrap();
    let first = managed_scope(7, 'a');
    host.fence_store().initialize(first.clone()).await.unwrap();
    host.fence_store().initialize(first.clone()).await.unwrap();
    assert_eq!(
        host.fence_store().current().await.unwrap(),
        Some(first.clone())
    );

    let conflicting = managed_scope(7, 'b');
    assert_eq!(
        host.fence_store()
            .initialize(conflicting)
            .await
            .unwrap_err()
            .code,
        "use.plugin.managed_scope_conflict"
    );

    let next = managed_scope(8, 'c');
    host.fence_store()
        .compare_and_advance(first.clone(), next.clone())
        .await
        .unwrap();
    assert_eq!(host.fence_store().current().await.unwrap(), Some(next));
    assert_eq!(
        host.fence_store()
            .compare_and_advance(first, managed_scope(9, 'd'))
            .await
            .unwrap_err()
            .code,
        "use.plugin.managed_scope_conflict"
    );
}

#[tokio::test]
async fn stale_scope_fails_before_observation_or_enablement() {
    let temporary = tempfile::tempdir().unwrap();
    let host =
        ManagedPluginHostManager::new(manager(temporary.path()), "host:node-01", "cli:0.11.1:test")
            .unwrap();
    let current = managed_scope(8, 'b');
    host.fence_store()
        .initialize(current.clone())
        .await
        .unwrap();
    let capabilities = host.capabilities().await.unwrap();
    let capabilities_digest = capabilities.descriptor_digest().unwrap();
    let package_id = PluginPackageId::parse("acme/research").unwrap();

    let stale_observation = PluginHostObservationRequest {
        schema: PLUGIN_HOST_OBSERVATION_REQUEST_SCHEMA.to_string(),
        request_id: "request:observe:0001".to_string(),
        assignment_generation: 4,
        capabilities_digest: capabilities_digest.clone(),
        scope: managed_scope(7, 'a'),
        package_id: package_id.clone(),
    };
    assert_eq!(
        host.observe(stale_observation).await.unwrap_err().code,
        "use.plugin.managed_scope_fence_mismatch"
    );

    let enablement = PluginHostEnablementRequest {
        schema: PLUGIN_HOST_ENABLEMENT_REQUEST_SCHEMA.to_string(),
        request_id: "request:enable:0001".to_string(),
        operation_id: "plugin-enable-managed-0001".to_string(),
        assignment_generation: 4,
        capabilities_digest,
        scope: managed_scope(7, 'a'),
        package_id,
        expected_package_generation: 7,
        enabled: true,
    };
    assert_eq!(
        host.set_enablement(enablement).await.unwrap_err().code,
        "use.plugin.managed_scope_fence_mismatch"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn signed_workspace_install_is_exact_fenced_and_replayable_after_restart() {
    use std::io::Read as _;
    use std::os::unix::fs::PermissionsExt;

    use a3s_use_core::{
        CatalogArchive, CatalogAvailability, CatalogPackage, CatalogSurface, PlanActor,
        PlanPackageChangeKind, PlanPackageRole, PlanPolicyDecision, PlanScopeKind,
        PluginCatalogRecord, PluginHostApplyRequest, PluginHostEnablementPlanRequest,
        PluginHostEnablementPlanStatus, PluginHostPlanRequest, PluginOperationAction,
        PluginOperationConfirmation, PluginPackageId, PluginPermissionCeiling,
        PluginReleaseChannel, PluginSurfaceKind, PluginSurfaceRef, PLUGIN_CATALOG_SCHEMA_V3,
        PLUGIN_HOST_APPLY_REQUEST_SCHEMA, PLUGIN_HOST_ENABLEMENT_PLAN_REQUEST_SCHEMA,
        PLUGIN_HOST_PLAN_REQUEST_SCHEMA, PLUGIN_OPERATION_CONFIRMATION_SCHEMA,
        PLUGIN_PERMISSION_SCHEMA,
    };
    use olpc_cjson::CanonicalFormatter;
    use serde::Serialize as _;
    use sha2::{Digest, Sha256};

    use crate::plugin_manager::{PluginApplyRequest, PluginManagerError};
    use crate::tuf_test_support::{
        host_target, package_directory_archive, TestRepository, TestServer, TestTarget, FUTURE,
    };

    let temporary = tempfile::tempdir().unwrap();
    let package_root = temporary.path().join("package");
    std::fs::create_dir_all(package_root.join("skills/main")).unwrap();
    let manifest = r#"extension "acme/guide" {
  schema_version = 3
  version = "1.0.0"
  route = "guide"
  requires_use = ">=0.3.0, <0.4.0"
  actions = ["read"]

  repository {
    url = "https://github.com/acme/guide"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }

  skill "main" {
    path = "skills/main/SKILL.md"
    requires_tool = []
    requires_mcp = []
    requires_okf = []
    optional = false
  }
}
"#;
    std::fs::write(package_root.join("a3s-use-extension.acl"), manifest).unwrap();
    std::fs::write(package_root.join("README.md"), "# Guide\n").unwrap();
    std::fs::write(
        package_root.join("skills/main/SKILL.md"),
        "---\nname: guide\ndescription: Managed guide fixture\n---\n# Guide\n",
    )
    .unwrap();
    let archive = package_directory_archive(&package_root);
    let (package_sha256, file_count, expanded_bytes) = package_fingerprint(&package_root);
    let permissions = PluginPermissionCeiling {
        schema: PLUGIN_PERMISSION_SCHEMA.to_string(),
        surfaces: Vec::new(),
    };
    let target = host_target();
    let target_name =
        format!("extensions/acme/guide/1.0.0/stable/{target}/guide-1.0.0-{target}.tar.gz");
    let catalog = PluginCatalogRecord {
        schema: PLUGIN_CATALOG_SCHEMA_V3.to_string(),
        package_id: "acme/guide".to_string(),
        display_name: "Guide".to_string(),
        description: "Managed workspace guide fixture.".to_string(),
        publisher: "acme".to_string(),
        keywords: vec!["guide".to_string()],
        categories: vec!["productivity".to_string()],
        version: "1.0.0".to_string(),
        channel: PluginReleaseChannel::Stable,
        requires_use: ">=0.3.0, <0.4.0".to_string(),
        dependencies: Vec::new(),
        target: target.to_string(),
        surfaces: vec![CatalogSurface {
            kind: PluginSurfaceKind::Skill,
            id: "main".to_string(),
            optional: false,
            workload: None,
            mcp_transport: None,
            mcp_tool_count: None,
            okf_bundle: None,
            requires: Vec::new(),
        }],
        permission_ceiling_digest: permissions.descriptor_digest().unwrap(),
        permission_ceiling: permissions,
        planning: None,
        archive: CatalogArchive {
            target_name: target_name.clone(),
            length: archive.len() as u64,
            sha256: format!("sha256:{:x}", Sha256::digest(&archive)),
        },
        package: CatalogPackage {
            expanded_bytes,
            file_count,
            sha256: Some(format!("sha256:{package_sha256}")),
            manifest_sha256: Some(format!("sha256:{:x}", Sha256::digest(manifest.as_bytes()))),
        },
        license: "MIT".to_string(),
        repository: "https://github.com/acme/guide".to_string(),
        availability: CatalogAvailability::Available,
    };
    catalog.validate().unwrap();
    let repository = TestRepository::with_targets(
        vec![TestTarget {
            archive,
            target_name,
            custom: Some(serde_json::to_value(catalog).unwrap()),
        }],
        91,
        FUTURE,
    );
    let server = TestServer::start(repository.routes.clone());
    let registry_store = RegistryStore::new(temporary.path().join("registries"));
    std::fs::create_dir_all(registry_store.root()).unwrap();
    std::fs::write(
        registry_store.root().join("fixture.acl"),
        format!(
            "registry \"fixture\" {{\n  url = \"{}\"\n  trust_root = \"sha256:{}\"\n}}\n",
            server.base_url(),
            repository.root_sha256
        ),
    )
    .unwrap();

    let mut component_paths = ComponentPaths::for_test(temporary.path());
    let resolved = registry_store
        .resolve_package(
            &component_paths.state_root,
            "acme/guide",
            Some("1.0.0"),
            "stable",
        )
        .await
        .unwrap();
    let verified_catalog = resolved.verified_catalog.clone().unwrap();
    let package_lock = registry_store
        .resolve_cognitive_package_lock(&component_paths.state_root, &resolved)
        .await
        .unwrap()
        .unwrap();
    let upstream_digest = "a".repeat(64);
    let raw_plan = serde_json::json!({
        "dryRun": true,
        "planDigest": upstream_digest,
        "plans": [{
            "component": "use/acme/guide",
            "action": "install",
            "mutates": true,
            "resolvedRegistryPackages": {"use/acme/guide": resolved.package},
            "verifiedPluginCatalogRecords": {"use/acme/guide": verified_catalog},
            "cognitivePackageLocks": {"use/acme/guide": package_lock},
        }],
    });
    let planner_calls = temporary.path().join("planner-calls.log");
    component_paths.current_exe = write_plan_fixture(
        temporary.path(),
        &planner_calls,
        &serde_json::json!({"ok": true, "data": raw_plan}),
    );
    let installed_record = component_paths
        .state_root
        .join("use/extensions/acme/guide.json");
    let use_install = write_capability_fixture(temporary.path(), &installed_record);
    component_paths.set_install_override("A3S_USE_INSTALL_DIR", use_install);
    let config_path = temporary.path().join("config/a3s.acl");
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let new_manager = || {
        Arc::new(PluginManager::new_with_policy(
            config_path.clone(),
            workspace.clone(),
            component_paths.clone(),
            RegistryStore::new(registry_store.root()),
            PluginManagerPolicy {
                offline: false,
                authorization: Default::default(),
            },
        ))
    };
    let manager = new_manager();
    let host =
        ManagedPluginHostManager::new(manager.clone(), "host:node-01", "cli:0.11.1:managed-test")
            .unwrap();
    let scope = managed_scope(7, 'f');
    host.fence_store().initialize(scope.clone()).await.unwrap();
    let capabilities = host.capabilities().await.unwrap();
    let capabilities_digest = capabilities.descriptor_digest().unwrap();
    let package_id = PluginPackageId::parse("acme/guide").unwrap();
    let plan_request = PluginHostPlanRequest {
        schema: PLUGIN_HOST_PLAN_REQUEST_SCHEMA.to_string(),
        request_id: "request:plan:managed-guide-0001".to_string(),
        assignment_generation: 11,
        capabilities_digest: capabilities_digest.clone(),
        scope: scope.clone(),
        action: PluginOperationAction::Install,
        package_id: package_id.clone(),
        candidate: Some(verified_catalog.clone()),
        package_lock: Some(package_lock.clone()),
        selected_surfaces: Vec::new(),
    };

    let planned = host.plan(plan_request.clone()).await.unwrap();
    assert!(!planned.replayed);
    assert_eq!(planned.scope, scope);
    assert_eq!(planned.plan.package_lock, Some(package_lock));
    assert_eq!(planned.plan.plan.scope.kind, PlanScopeKind::Workspace);
    assert_eq!(planned.plan.plan.scope.id, "workspace:research");
    assert_eq!(planned.plan.plan.authority.actor, PlanActor::Agent);
    assert_eq!(
        planned.plan.plan.authority.decision,
        PlanPolicyDecision::Ask
    );
    let root = planned
        .plan
        .plan
        .packages
        .iter()
        .find(|package| package.role == PlanPackageRole::Root)
        .unwrap();
    assert_eq!(
        root.after.as_ref(),
        Some(&verified_catalog.selected_state(&[]).unwrap())
    );
    assert!(host.plan(plan_request.clone()).await.unwrap().replayed);

    let mut changed_request = plan_request.clone();
    changed_request.selected_surfaces = vec![PluginSurfaceRef {
        kind: PluginSurfaceKind::Skill,
        id: "main".to_string(),
    }];
    assert_eq!(
        host.plan(changed_request).await.unwrap_err().code,
        "use.plugin.host_request_invalid"
    );
    let mut changed_capabilities = plan_request.clone();
    changed_capabilities.capabilities_digest = format!("sha256:{}", "b".repeat(64));
    assert_eq!(
        host.plan(changed_capabilities).await.unwrap_err().code,
        "use.plugin.host_capabilities_mismatch"
    );
    let mut changed_scope = plan_request;
    changed_scope.scope = managed_scope(8, 'e');
    assert_eq!(
        host.plan(changed_scope).await.unwrap_err().code,
        "use.plugin.managed_scope_fence_mismatch"
    );

    let local_error = manager
        .apply_confirmed_operation(&PluginApplyRequest {
            operation_id: Some(planned.plan.plan.operation_id.clone()),
            action: None,
            component_id: None,
            version: None,
            channel: None,
            plan_digest: planned.plan.plan_digest.clone(),
        })
        .await
        .unwrap_err();
    assert!(matches!(local_error, PluginManagerError::InvalidRequest(_)));

    let confirmation = PluginOperationConfirmation {
        schema: PLUGIN_OPERATION_CONFIRMATION_SCHEMA.to_string(),
        operation_id: planned.plan.plan.operation_id.clone(),
        plan_digest: planned.plan.plan_digest.clone(),
        confirmed_by: PlanActor::User,
        confirmed_at_ms: planned.plan.plan.created_at_ms,
    };
    let apply_request = PluginHostApplyRequest {
        schema: PLUGIN_HOST_APPLY_REQUEST_SCHEMA.to_string(),
        request_id: "request:apply:managed-guide-0001".to_string(),
        assignment_generation: planned.assignment_generation,
        capabilities_digest: capabilities_digest.clone(),
        scope: scope.clone(),
        package_id: package_id.clone(),
        operation_id: planned.plan.plan.operation_id.clone(),
        plan_digest: planned.plan.plan_digest.clone(),
        confirmation: Some(confirmation),
    };
    let applied = host.apply(apply_request.clone()).await.unwrap();
    assert!(!applied.replayed);
    assert_eq!(applied.scope, scope);
    assert_eq!(applied.state.capability_generation, 1);
    assert_eq!(applied.state.desired, PluginDesiredState::Enabled);
    assert_eq!(applied.state.observed, PluginObservedState::Ready);
    assert_eq!(
        applied.state.selected_surfaces,
        vec![PluginSurfaceRef {
            kind: PluginSurfaceKind::Skill,
            id: "main".to_string(),
        }]
    );
    let receipt: a3s_use_extension::ExtensionReceipt =
        serde_json::from_slice(&std::fs::read(&installed_record).unwrap()).unwrap();
    assert_eq!(applied.state.version.as_deref(), Some("1.0.0"));
    assert_eq!(
        applied.state.package_generation,
        receipt.lifecycle_generation
    );
    assert_eq!(
        applied.state.package_digest.as_deref(),
        Some(format!("sha256:{package_sha256}").as_str())
    );
    assert_eq!(
        applied.state.receipt_digest.as_deref(),
        Some(receipt.descriptor_digest().unwrap().as_str())
    );
    assert_eq!(
        std::fs::read_to_string(&planner_calls)
            .unwrap()
            .lines()
            .count(),
        1
    );

    let result_path = component_paths
        .state_root
        .join("plugin-manager/operations/results")
        .join(format!(
            "{:x}.json",
            Sha256::digest(applied.operation_id.as_bytes())
        ));
    let durable_result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(result_path).unwrap()).unwrap();
    let operation_data = durable_result.get("data").unwrap();
    let mut canonical = Vec::new();
    let mut serializer =
        serde_json::Serializer::with_formatter(&mut canonical, CanonicalFormatter::new());
    operation_data.serialize(&mut serializer).unwrap();
    assert_eq!(
        applied.operation_result_digest,
        format!("sha256:{:x}", Sha256::digest(canonical))
    );

    let mut changed_apply = apply_request.clone();
    changed_apply.confirmation.as_mut().unwrap().confirmed_at_ms += 1;
    assert_eq!(
        host.apply(changed_apply).await.unwrap_err().code,
        "use.plugin.host_request_invalid"
    );

    drop(host);
    drop(manager);
    let restarted =
        ManagedPluginHostManager::new(new_manager(), "host:node-01", "cli:0.11.1:managed-test")
            .unwrap();
    restarted
        .fence_store()
        .initialize(scope.clone())
        .await
        .unwrap();
    let replayed = restarted.apply(apply_request).await.unwrap();
    assert!(replayed.replayed);
    assert_eq!(replayed.completed_at_ms, applied.completed_at_ms);
    assert_eq!(
        replayed.operation_result_digest,
        applied.operation_result_digest
    );
    assert_eq!(replayed.state, applied.state);

    let observation_request = PluginHostObservationRequest {
        schema: PLUGIN_HOST_OBSERVATION_REQUEST_SCHEMA.to_string(),
        request_id: "request:observe:managed-guide-0001".to_string(),
        assignment_generation: 11,
        capabilities_digest: capabilities_digest.clone(),
        scope: scope.clone(),
        package_id: package_id.clone(),
    };
    let observed = restarted
        .observe(observation_request.clone())
        .await
        .unwrap();
    let PluginHostObservationStatus::Available {
        state: installed_state,
    } = observed.status
    else {
        panic!("expected the installed managed package state");
    };
    assert_eq!(installed_state.version.as_deref(), Some("1.0.0"));
    assert_eq!(installed_state.desired, PluginDesiredState::Enabled);
    assert_eq!(installed_state.observed, PluginObservedState::Ready);
    assert_eq!(
        installed_state.package_generation,
        applied.state.package_generation
    );

    let package_fingerprint_before = package_fingerprint(&receipt.package_root);
    let graph_record = component_paths
        .state_root
        .join("use/package-graphs/acme/guide.json");
    let graph_before = std::fs::read(&graph_record).unwrap();
    let disable_plan_request = PluginHostEnablementPlanRequest {
        schema: PLUGIN_HOST_ENABLEMENT_PLAN_REQUEST_SCHEMA.to_string(),
        request_id: "request:disable:managed-guide-0001".to_string(),
        assignment_generation: 11,
        capabilities_digest: capabilities_digest.clone(),
        scope: scope.clone(),
        package_id: package_id.clone(),
        expected_package_generation: installed_state.package_generation.unwrap(),
        enabled: false,
    };
    let disable_plan = restarted
        .plan_enablement(disable_plan_request.clone())
        .await
        .unwrap();
    assert!(!disable_plan.replayed);
    assert_eq!(disable_plan.status, PluginHostEnablementPlanStatus::Planned);
    assert_eq!(disable_plan.state, installed_state);
    let disable_envelope = disable_plan.plan.as_ref().unwrap();
    assert_eq!(disable_envelope.plan.action, PluginOperationAction::Disable);
    assert_eq!(disable_envelope.plan.authority.actor, PlanActor::Agent);
    assert_eq!(
        disable_envelope.plan.authority.decision,
        PlanPolicyDecision::Ask
    );
    assert_eq!(disable_envelope.plan.packages.len(), 1);
    assert_eq!(
        disable_envelope.plan.packages[0].change,
        PlanPackageChangeKind::Retain
    );
    assert_eq!(
        disable_envelope.plan.packages[0].before,
        disable_envelope.plan.packages[0].after
    );
    assert!(
        restarted
            .plan_enablement(disable_plan_request.clone())
            .await
            .unwrap()
            .replayed
    );
    let mut conflicting_disable_plan = disable_plan_request.clone();
    conflicting_disable_plan.enabled = true;
    assert_eq!(
        restarted
            .plan_enablement(conflicting_disable_plan)
            .await
            .unwrap_err()
            .code,
        "use.plugin.host_enablement_operation_conflict"
    );

    let disable_confirmation = PluginOperationConfirmation {
        schema: PLUGIN_OPERATION_CONFIRMATION_SCHEMA.to_string(),
        operation_id: disable_envelope.plan.operation_id.clone(),
        plan_digest: disable_envelope.plan_digest.clone(),
        confirmed_by: PlanActor::User,
        confirmed_at_ms: disable_envelope.plan.created_at_ms,
    };
    let disable_apply_request = PluginHostApplyRequest {
        schema: PLUGIN_HOST_APPLY_REQUEST_SCHEMA.to_string(),
        request_id: "request:apply-disable:managed-guide-0001".to_string(),
        assignment_generation: 11,
        capabilities_digest: capabilities_digest.clone(),
        scope: scope.clone(),
        package_id: package_id.clone(),
        operation_id: disable_envelope.plan.operation_id.clone(),
        plan_digest: disable_envelope.plan_digest.clone(),
        confirmation: Some(disable_confirmation),
    };
    let mut substituted_digest = disable_apply_request.clone();
    substituted_digest.plan_digest = format!("sha256:{}", "b".repeat(64));
    substituted_digest.confirmation = None;
    assert_eq!(
        restarted.apply(substituted_digest).await.unwrap_err().code,
        "use.plugin.host_apply_request_mismatch"
    );
    let disabled = restarted
        .apply(disable_apply_request.clone())
        .await
        .unwrap();
    assert!(!disabled.replayed);
    assert_eq!(
        disabled.state.desired,
        PluginDesiredState::InstalledDisabled
    );
    assert_eq!(disabled.state.observed, PluginObservedState::Installed);
    assert!(
        disabled.state.package_generation.unwrap() > installed_state.package_generation.unwrap()
    );
    assert_eq!(
        package_fingerprint(&receipt.package_root),
        package_fingerprint_before
    );
    assert_eq!(std::fs::read(&graph_record).unwrap(), graph_before);

    drop(restarted);
    let restarted =
        ManagedPluginHostManager::new(new_manager(), "host:node-01", "cli:0.11.1:managed-test")
            .unwrap();
    restarted
        .fence_store()
        .initialize(scope.clone())
        .await
        .unwrap();
    let replayed_disable = restarted
        .apply(disable_apply_request.clone())
        .await
        .unwrap();
    assert!(replayed_disable.replayed);
    assert_eq!(replayed_disable.completed_at_ms, disabled.completed_at_ms);
    assert_eq!(
        replayed_disable.operation_result_digest,
        disabled.operation_result_digest
    );
    assert_eq!(replayed_disable.state, disabled.state);

    let mut conflicting_disable = disable_apply_request.clone();
    conflicting_disable
        .confirmation
        .as_mut()
        .unwrap()
        .confirmed_at_ms += 1;
    assert_eq!(
        restarted.apply(conflicting_disable).await.unwrap_err().code,
        "use.plugin.host_enablement_operation_conflict"
    );

    let stale_enable = PluginHostEnablementPlanRequest {
        schema: PLUGIN_HOST_ENABLEMENT_PLAN_REQUEST_SCHEMA.to_string(),
        request_id: "request:enable:managed-guide:stale".to_string(),
        assignment_generation: 11,
        capabilities_digest: capabilities_digest.clone(),
        scope: scope.clone(),
        package_id: package_id.clone(),
        expected_package_generation: installed_state.package_generation.unwrap(),
        enabled: true,
    };
    assert_eq!(
        restarted
            .plan_enablement(stale_enable)
            .await
            .unwrap_err()
            .code,
        "use.plugin.package_generation_changed"
    );

    let enable_plan_request = PluginHostEnablementPlanRequest {
        schema: PLUGIN_HOST_ENABLEMENT_PLAN_REQUEST_SCHEMA.to_string(),
        request_id: "request:enable:managed-guide-0001".to_string(),
        assignment_generation: 11,
        capabilities_digest: capabilities_digest.clone(),
        scope: scope.clone(),
        package_id: package_id.clone(),
        expected_package_generation: disabled.state.package_generation.unwrap(),
        enabled: true,
    };
    let enable_plan = restarted
        .plan_enablement(enable_plan_request)
        .await
        .unwrap();
    assert_eq!(enable_plan.status, PluginHostEnablementPlanStatus::Planned);
    let enable_envelope = enable_plan.plan.as_ref().unwrap();
    assert_eq!(enable_envelope.plan.action, PluginOperationAction::Enable);
    let enable_confirmation = PluginOperationConfirmation {
        schema: PLUGIN_OPERATION_CONFIRMATION_SCHEMA.to_string(),
        operation_id: enable_envelope.plan.operation_id.clone(),
        plan_digest: enable_envelope.plan_digest.clone(),
        confirmed_by: PlanActor::User,
        confirmed_at_ms: enable_envelope.plan.created_at_ms,
    };
    let enable_apply_request = PluginHostApplyRequest {
        schema: PLUGIN_HOST_APPLY_REQUEST_SCHEMA.to_string(),
        request_id: "request:apply-enable:managed-guide-0001".to_string(),
        assignment_generation: 11,
        capabilities_digest: capabilities_digest.clone(),
        scope: scope.clone(),
        package_id: package_id.clone(),
        operation_id: enable_envelope.plan.operation_id.clone(),
        plan_digest: enable_envelope.plan_digest.clone(),
        confirmation: Some(enable_confirmation),
    };
    let enabled = restarted.apply(enable_apply_request).await.unwrap();
    assert_eq!(enabled.state.desired, PluginDesiredState::Enabled);
    assert_eq!(enabled.state.observed, PluginObservedState::Ready);
    assert!(enabled.state.package_generation.unwrap() > disabled.state.package_generation.unwrap());
    assert_eq!(
        package_fingerprint(&receipt.package_root),
        package_fingerprint_before
    );
    assert_eq!(std::fs::read(&graph_record).unwrap(), graph_before);

    let no_change_request = PluginHostEnablementPlanRequest {
        schema: PLUGIN_HOST_ENABLEMENT_PLAN_REQUEST_SCHEMA.to_string(),
        request_id: "request:enable:managed-guide:no-change".to_string(),
        assignment_generation: 11,
        capabilities_digest: capabilities_digest.clone(),
        scope: scope.clone(),
        package_id: package_id.clone(),
        expected_package_generation: enabled.state.package_generation.unwrap(),
        enabled: true,
    };
    let no_change = restarted
        .plan_enablement(no_change_request.clone())
        .await
        .unwrap();
    assert_eq!(no_change.status, PluginHostEnablementPlanStatus::NoChange);
    assert!(no_change.plan.is_none());
    let replayed_no_change = restarted.plan_enablement(no_change_request).await.unwrap();
    assert!(replayed_no_change.replayed);
    assert_eq!(
        replayed_no_change.status,
        PluginHostEnablementPlanStatus::NoChange
    );
    assert!(replayed_no_change.plan.is_none());

    let observed_enabled = restarted.observe(observation_request).await.unwrap();
    let PluginHostObservationStatus::Available {
        state: observed_enabled,
    } = observed_enabled.status
    else {
        panic!("expected the re-enabled managed package state");
    };
    assert_eq!(observed_enabled, enabled.state);
    assert_eq!(
        std::fs::read_to_string(planner_calls)
            .unwrap()
            .lines()
            .count(),
        1
    );

    fn package_fingerprint(root: &std::path::Path) -> (String, u64, u64) {
        fn collect(
            root: &std::path::Path,
            directory: &std::path::Path,
            files: &mut Vec<(String, std::path::PathBuf)>,
        ) {
            let mut entries = std::fs::read_dir(directory)
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let path = entry.path();
                if entry.file_type().unwrap().is_dir() {
                    collect(root, &path, files);
                } else {
                    files.push((
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                        path,
                    ));
                }
            }
        }

        let mut files = Vec::new();
        collect(root, root, &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));
        let mut digest = Sha256::new();
        digest.update(b"a3s-use-expanded-package-v1\0");
        let mut expanded_bytes = 0_u64;
        for (relative, path) in &files {
            let size = std::fs::metadata(path).unwrap().len();
            expanded_bytes += size;
            digest.update((relative.len() as u64).to_be_bytes());
            digest.update(relative.as_bytes());
            digest.update(size.to_be_bytes());
            let mut input = std::fs::File::open(path).unwrap();
            let mut buffer = Vec::new();
            input.read_to_end(&mut buffer).unwrap();
            digest.update(buffer);
        }
        (
            format!("{:x}", digest.finalize()),
            files.len() as u64,
            expanded_bytes,
        )
    }

    fn write_plan_fixture(
        root: &std::path::Path,
        calls_path: &std::path::Path,
        response: &serde_json::Value,
    ) -> std::path::PathBuf {
        let executable = root.join("fake-a3s");
        let calls_path = shell_literal(calls_path.to_string_lossy().as_ref());
        let response = shell_literal(&response.to_string());
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {calls_path}\nprintf '%s\\n' {response}\n"
        );
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        executable
    }

    fn write_capability_fixture(
        root: &std::path::Path,
        installed_record: &std::path::Path,
    ) -> std::path::PathBuf {
        let directory = root.join("use-bin-managed");
        std::fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("a3s-use");
        let installed_record = shell_literal(installed_record.to_string_lossy().as_ref());
        let before = shell_literal(&capability_snapshot(0, &"f".repeat(64)));
        let after = shell_literal(&capability_snapshot(1, &"c".repeat(64)));
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n  --version) printf '%s\\n' 'a3s-use 0.3.0' ;;\n  capability)\n    if [ -f {installed_record} ]; then\n      printf '%s\\n' {after}\n    else\n      printf '%s\\n' {before}\n    fi\n    ;;\n  *) exit 64 ;;\nesac\n"
        );
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }

    fn capability_snapshot(generation: u64, revision: &str) -> String {
        serde_json::json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "registry": {
                    "schemaVersion": 1,
                    "generation": generation,
                    "revision": revision,
                    "capabilities": [],
                },
            },
        })
        .to_string()
    }

    fn shell_literal(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}
