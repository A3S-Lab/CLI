use super::*;
use crate::components::ComponentPaths;
use crate::plugin_manager::operation::store::{
    PluginPlanIdentity, StoredPluginPlan, OPERATION_RECORD_SCHEMA,
};
use crate::plugin_manager::policy::tests::install_plan;
use crate::plugin_manager::process::PluginLifecycleAction;
use crate::plugin_manager::{
    PluginAuthorizationPolicy, PluginEnablementApplyRequest, PluginEnablementPlanRequest,
    PluginManagerPolicy,
};
use crate::registry::RegistryStore;

#[path = "tests/grant_forwarding.rs"]
mod grant_forwarding;

#[path = "tests/lifecycle.rs"]
mod lifecycle;

#[test]
fn apply_contract_accepts_only_exact_reviewed_identity() {
    let error = serde_json::from_value::<PluginApplyRequest>(serde_json::json!({
        "operationId": "plugin-install-abc",
        "planDigest": "a".repeat(64),
        "action": "install",
    }))
    .unwrap_err();

    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn verified_capability_drift_fails_before_mutation() {
    let planned = PluginCapabilityEvidence {
        status: PluginCapabilityEvidenceStatus::Verified,
        observed_at_ms: 1,
        generation: Some(7),
        revision: Some("a".repeat(64)),
        error: None,
    };
    let current = PluginCapabilityEvidence {
        status: PluginCapabilityEvidenceStatus::Verified,
        observed_at_ms: 2,
        generation: Some(8),
        revision: Some("b".repeat(64)),
        error: None,
    };

    assert!(ensure_capability_state_unchanged(&planned, &current).is_err());
    assert!(ensure_capability_precondition(&planned, &current, true).is_ok());
}

#[test]
fn newly_available_capability_state_requires_a_new_plan() {
    let planned = PluginCapabilityEvidence {
        status: PluginCapabilityEvidenceStatus::Unavailable,
        observed_at_ms: 1,
        generation: None,
        revision: None,
        error: Some("A3S Use is not ready".to_string()),
    };
    let current = PluginCapabilityEvidence {
        status: PluginCapabilityEvidenceStatus::Verified,
        observed_at_ms: 2,
        generation: Some(1),
        revision: Some("a".repeat(64)),
        error: None,
    };

    assert!(ensure_capability_state_unchanged(&planned, &current).is_err());
}

#[tokio::test]
async fn new_apply_intent_rejects_registry_source_revision_drift() {
    let (temporary, manager, mut plan) = full_plan_record(
        PluginAuthorizationPolicy::default(),
        a3s_use_core::PlanActor::User,
    );
    let reviewed_revision = manager.registry_store.snapshot().await.unwrap().revision;
    plan.registry_source_revision = Some(reviewed_revision);
    manager
        .registry_store
        .add_test_source("changed", "https://changed.example/", &"9".repeat(64))
        .await
        .unwrap();

    let error = verify_registry_source_precondition(&manager, &plan)
        .await
        .unwrap_err();
    assert!(matches!(error, PluginManagerError::OperationFailed(_)));

    drop(temporary);
}

#[test]
fn operation_id_apply_accepts_a_canonical_prefixed_digest() {
    let digest = "c".repeat(64);
    let request: PluginApplyRequest = serde_json::from_value(serde_json::json!({
        "operationId": "plugin-install-abc",
        "planDigest": format!("sha256:{digest}"),
    }))
    .unwrap();

    assert_eq!(request.operation_id, "plugin-install-abc");
    assert_eq!(normalize_plan_digest(&request.plan_digest).unwrap(), digest);
}

#[test]
fn ask_plan_requires_exact_trusted_confirmation_before_intent() {
    let policy = PluginAuthorizationPolicy::default();
    let (temporary, manager, plan) = full_plan_record(policy, a3s_use_core::PlanActor::User);
    let now_ms = plan.created_at_ms;

    let error = verify_new_apply_authority(&manager, &plan, false, now_ms).unwrap_err();
    assert!(error.to_string().contains("confirmation"));

    let confirmation = verify_new_apply_authority(&manager, &plan, true, now_ms)
        .unwrap()
        .unwrap();
    assert_eq!(confirmation.operation_id, plan.operation_id);
    assert_eq!(
        confirmation.plan_digest,
        plan.plugin_operation_plan.as_ref().unwrap().plan_digest
    );

    drop(temporary);
}

#[test]
fn changed_policy_or_agent_denial_fails_before_intent() {
    let (temporary, _, user_plan) = full_plan_record(
        PluginAuthorizationPolicy::default(),
        a3s_use_core::PlanActor::User,
    );
    let changed_policy = PluginAuthorizationPolicy::from_acl(
        r#"
plugins {
  schema = "a3s.plugin-policy.v1"
  max_packages = 1
}
"#,
    )
    .unwrap();
    let changed_manager = manager(temporary.path(), changed_policy);
    let now_ms = user_plan.created_at_ms;
    assert!(
        verify_new_apply_authority(&changed_manager, &user_plan, true, now_ms)
            .unwrap_err()
            .to_string()
            .contains("changed")
    );

    let deny_policy = PluginAuthorizationPolicy::from_acl(
        r#"
plugins {
  schema = "a3s.plugin-policy.v1"
  agent_install = "deny"
}
"#,
    )
    .unwrap();
    let (_, agent_manager, agent_plan) =
        full_plan_record(deny_policy, a3s_use_core::PlanActor::Agent);
    assert!(verify_new_apply_authority(
        &agent_manager,
        &agent_plan,
        true,
        agent_plan.created_at_ms,
    )
    .unwrap_err()
    .to_string()
    .contains("denies"));
}

#[tokio::test]
async fn durable_intent_requires_and_preserves_exact_confirmation() {
    let (_temporary, manager, plan) = full_plan_record(
        PluginAuthorizationPolicy::default(),
        a3s_use_core::PlanActor::User,
    );
    let confirmation = verify_new_apply_authority(&manager, &plan, true, plan.created_at_ms)
        .unwrap()
        .unwrap();

    assert!(manager
        .operation_store
        .persist_intent_with_confirmation(&plan, None)
        .await
        .is_err());
    assert!(!manager.operation_store.has_intent(&plan).await.unwrap());
    let first = manager
        .operation_store
        .persist_intent_with_confirmation(&plan, Some(confirmation.clone()))
        .await
        .unwrap();
    assert!(!first.resumed);
    assert_eq!(first.confirmation.as_ref(), Some(&confirmation));
    let replay = manager
        .operation_store
        .persist_intent_with_confirmation(&plan, None)
        .await
        .unwrap();
    assert!(replay.resumed);
    assert_eq!(replay.confirmation.as_ref(), Some(&confirmation));
    assert!(manager.operation_store.has_intent(&plan).await.unwrap());
}

#[cfg(unix)]
#[tokio::test]
async fn reviewed_apply_uses_the_in_process_adapter_and_preserves_host_authority() {
    use crate::plugin_manager::capability::PluginInstallationSnapshot;
    use crate::tuf_test_support::{
        host_target, package_directory_archive, TestRepository, TestServer, TestTarget, FUTURE,
    };
    use a3s_use_core::{
        CatalogArchive, CatalogAvailability, CatalogPackage, CatalogSurface, PlanActor,
        PluginCatalogRecord, PluginPermissionCeiling, PluginReleaseChannel, PluginSurfaceKind,
        PLUGIN_CATALOG_SCHEMA_V3, PLUGIN_PERMISSION_SCHEMA,
    };
    use sha2::{Digest, Sha256};

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
        "---\nname: guide\ndescription: Reviewed guide fixture\n---\n# Guide\n",
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
        description: "Reviewed static guide fixture.".to_string(),
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
        73,
        FUTURE,
    );
    let server = TestServer::start(repository.routes.clone());
    let registry_store = RegistryStore::for_test(temporary.path());
    registry_store
        .add_test_source("fixture", server.base_url(), &repository.root_sha256)
        .await
        .unwrap();
    let mut component_paths = ComponentPaths::for_test(temporary.path());
    let resolved = registry_store
        .resolve_package(Some("fixture"), "acme/guide", Some("1.0.0"), "stable")
        .await
        .unwrap();
    let verified_catalog = resolved.verified_catalog.clone();
    let package_lock = registry_store
        .resolve_cognitive_package_lock(&resolved)
        .await
        .unwrap();
    let registry_source_revision = resolved.registry_source_revision.clone();
    let upstream_digest = "a".repeat(64);
    let raw_plan = serde_json::json!({
        "dryRun": true,
        "planDigest": upstream_digest,
        "plans": [{
            "component": "use/acme/guide",
            "action": "install",
            "mutates": true,
            "registrySourceRevision": registry_source_revision,
            "resolvedRegistryPackages": {"use/acme/guide": resolved.package},
            "verifiedPluginCatalogRecords": {"use/acme/guide": verified_catalog},
            "cognitivePackageLocks": {"use/acme/guide": package_lock},
        }],
    });
    let installation = PluginInstallationSnapshot {
        schema_version: 1,
        available: true,
        observed_at_ms: 1,
        generation: Some(0),
        revision: Some("f".repeat(64)),
        items: Vec::new(),
        error: None,
    };
    let request = PluginPlanRequest {
        action: PluginLifecycleAction::Install,
        component_id: "use/acme/guide".to_string(),
        version: Some("1.0.0".to_string()),
        channel: Some("stable".to_string()),
        registry_name: Some("fixture".to_string()),
    };
    let raw_plan = planner::attach_draft(&request, &installation, None, &[], 1, raw_plan).unwrap();
    let policy = PluginAuthorizationPolicy::default();
    let identity = PluginPlanIdentity {
        operation_id: "plugin-install-reviewed-guide".to_string(),
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
    let grant_snapshot = a3s_use_core::PluginWorkspaceGrantSnapshot {
        schema: a3s_use_core::PLUGIN_WORKSPACE_GRANT_SNAPSHOT_SCHEMA.to_string(),
        scope_id: crate::plugin_manager::default_plan_scope().id,
        state_revision: 1,
        grants: Vec::new(),
    };
    let installed_generations = std::collections::BTreeMap::new();
    let runtime_host = crate::plugin_manager::PluginRuntimeHost::default();
    let prepared = plan_artifact::prepare(
        plan_artifact::HostPlanContext {
            authorization: &policy,
            actor: PlanActor::User,
            scope: &crate::plugin_manager::default_plan_scope(),
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
        upstream_digest,
        None,
        raw_plan,
    )
    .await
    .unwrap();
    let envelope = prepared.plugin_operation_plan.as_ref().unwrap().clone();
    let child_mutation_log = temporary.path().join("forbidden-child-mutation.log");
    let use_install = write_capability_use_fixture(
        temporary.path(),
        &component_paths
            .state_root
            .join("use/extensions/acme/guide.json"),
    );
    component_paths.set_install_override("A3S_USE_INSTALL_DIR", use_install);
    component_paths.current_exe = write_forbidden_a3s(temporary.path(), &child_mutation_log);
    let workspace = temporary.path().join("workspace");
    let config_path = temporary.path().join("config/a3s.acl");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let manager = PluginManager::new_with_policy(
        config_path.clone(),
        workspace.clone(),
        component_paths.clone(),
        registry_store.clone(),
        PluginManagerPolicy {
            offline: false,
            authorization: policy.clone(),
        },
    );
    let stored = manager
        .operation_store
        .create_plan_for_actor(crate::plugin_manager::operation::store::NewPluginPlan {
            identity,
            request,
            actor: PlanActor::User,
            scope: crate::plugin_manager::default_plan_scope(),
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

    let applied = manager
        .apply_confirmed_operation(&apply_request)
        .await
        .unwrap();

    let graph = &applied["operations"][0]["packageGraph"];
    assert_eq!(
        graph["plan"]["plan"]["operationId"],
        envelope.plan.operation_id
    );
    assert_eq!(graph["plan"]["planDigest"], envelope.plan_digest);
    assert_eq!(applied["operationId"], stored.operation_id);
    assert_eq!(applied["replayed"], false);
    assert_eq!(applied["stateRevisionAfter"], 2);
    assert!(component_paths
        .state_root
        .join("use/extensions/acme/guide.json")
        .is_file());
    assert!(
        !child_mutation_log.exists(),
        "complete plans must not launch a child a3s mutation"
    );

    let persisted_intent = manager
        .operation_store
        .persist_intent_with_confirmation(&stored, None)
        .await
        .unwrap();
    let confirmation = persisted_intent.confirmation.unwrap();
    assert!(persisted_intent.resumed);
    assert_eq!(confirmation.operation_id, envelope.plan.operation_id);
    assert_eq!(confirmation.plan_digest, envelope.plan_digest);

    let replayed = manager
        .apply_confirmed_operation(&apply_request)
        .await
        .unwrap();
    assert_eq!(replayed["replayed"], true);
    assert_eq!(replayed["operations"], applied["operations"]);
    assert!(!child_mutation_log.exists());

    Box::pin(async {
        let package_manager = crate::components::code_cognitive_package_manager(
            &component_paths,
            crate::plugin_manager::default_plan_scope(),
        )
        .unwrap();
        let enabled_state = package_manager.observe_package("acme/guide").await.unwrap();
        let disable_plan = manager
            .plan_package_enablement(&PluginEnablementPlanRequest {
                component_id: "use/acme/guide".to_string(),
                enabled: false,
                expected_package_generation: enabled_state.package_generation,
            })
            .await
            .unwrap();
        assert_eq!(disable_plan["status"], "planned");
        assert_eq!(disable_plan["enabled"], false);
        assert_eq!(disable_plan["state"]["desired"], "enabled");
        let disable_request = PluginEnablementApplyRequest {
            operation_id: disable_plan["operationId"].as_str().unwrap().to_string(),
            plan_digest: disable_plan["canonicalPlanDigest"]
                .as_str()
                .unwrap()
                .to_string(),
        };
        let unconfirmed = manager
            .apply_package_enablement(&disable_request)
            .await
            .unwrap_err();
        assert!(unconfirmed.to_string().contains("confirmation"));
        let disabled = manager
            .apply_confirmed_package_enablement(&disable_request)
            .await
            .unwrap();
        assert_eq!(disabled["durableEnablement"], true);
        assert_eq!(disabled["changed"], true);
        assert_eq!(disabled["replayed"], false);
        assert_eq!(disabled["state"]["desired"], "installed-disabled");
        assert_eq!(disabled["operationId"], disable_request.operation_id);
        let disabled_generation = disabled["state"]["packageGeneration"].as_u64().unwrap();
        assert!(disabled_generation > enabled_state.package_generation.unwrap());
        assert!(
            !package_manager
                .registry()
                .get("acme/guide")
                .await
                .unwrap()
                .unwrap()
                .receipt
                .enabled
        );

        let restarted = PluginManager::new_with_policy(
            config_path,
            workspace,
            component_paths.clone(),
            registry_store.clone(),
            PluginManagerPolicy {
                offline: false,
                authorization: policy,
            },
        );
        let replayed_disable = restarted
            .apply_confirmed_package_enablement(&disable_request)
            .await
            .unwrap();
        assert_eq!(replayed_disable["replayed"], true);
        assert_eq!(
            replayed_disable["operationResultDigest"],
            disabled["operationResultDigest"]
        );
        assert_eq!(replayed_disable["state"], disabled["state"]);

        let conflicting = PluginEnablementApplyRequest {
            plan_digest: format!("sha256:{}", "f".repeat(64)),
            ..disable_request.clone()
        };
        let conflict = restarted
            .apply_confirmed_package_enablement(&conflicting)
            .await
            .unwrap_err();
        assert!(conflict.to_string().contains("plan digest"));

        let enable_plan = restarted
            .plan_package_enablement(&PluginEnablementPlanRequest {
                component_id: "use/acme/guide".to_string(),
                enabled: true,
                expected_package_generation: Some(disabled_generation),
            })
            .await
            .unwrap();
        assert_eq!(enable_plan["status"], "planned");
        let enabled = restarted
            .apply_confirmed_package_enablement(&PluginEnablementApplyRequest {
                operation_id: enable_plan["operationId"].as_str().unwrap().to_string(),
                plan_digest: enable_plan["canonicalPlanDigest"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            })
            .await
            .unwrap();
        assert_eq!(enabled["changed"], true);
        assert_eq!(enabled["replayed"], false);
        assert_eq!(enabled["state"]["desired"], "enabled");
        assert!(
            package_manager
                .registry()
                .get("acme/guide")
                .await
                .unwrap()
                .unwrap()
                .receipt
                .enabled
        );
        assert!(
            !child_mutation_log.exists(),
            "schema-v3 enablement must not launch a child mutation"
        );

        let no_change = restarted
            .plan_package_enablement(&PluginEnablementPlanRequest {
                component_id: "use/acme/guide".to_string(),
                enabled: true,
                expected_package_generation: enabled["state"]["packageGeneration"].as_u64(),
            })
            .await
            .unwrap();
        assert_eq!(no_change["status"], "no-change");
        assert!(no_change.get("operationId").is_none());
        assert!(no_change.get("canonicalPlanDigest").is_none());
    })
    .await;

    let source_snapshot = manager.registry_store.snapshot().await.unwrap();
    manager
        .registry_store
        .source_store()
        .replace(
            &source_snapshot.revision,
            a3s_use_extension::RegistrySourceInput::new(
                "fixture",
                server.base_url(),
                "b".repeat(64),
                None,
                a3s_use_extension::VerifiedTargetCachePolicy::default(),
            ),
        )
        .await
        .unwrap();
    let drift = crate::components::apply_reviewed_cognitive_package(
        &envelope,
        Some(&confirmation),
        &component_paths,
        &manager.registry_store,
        manager
            .runtime_host
            .lifecycle_factory(Default::default())
            .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(drift
        .to_string()
        .contains("no longer matches its locked URL and trust root"));
}

#[test]
fn applied_output_separates_manager_and_upstream_plan_digests() {
    let (_temporary, _manager, plan) = full_plan_record(
        PluginAuthorizationPolicy::default(),
        a3s_use_core::PlanActor::User,
    );
    let output = applied_output(
        serde_json::json!({
            "planDigest": plan.upstream_plan_digest(),
            "operations": [],
        }),
        &plan,
        AppliedStateEvidence {
            capability_before: &plan.capability_state,
            capability_after: &plan.capability_state,
            state_revision_after: 4,
            lifecycle: None,
        },
        plan.created_at_ms,
        false,
        false,
    )
    .unwrap();
    let operation_plan = plan.plugin_operation_plan.as_ref().unwrap();

    assert_eq!(output["planDigest"], plan.plan_digest);
    assert_eq!(
        output["canonicalPlanDigest"],
        format!("sha256:{}", plan.plan_digest)
    );
    assert_eq!(
        output["pluginOperationPlanDigest"],
        operation_plan.plan_digest
    );
    assert_eq!(
        output["authority"],
        serde_json::to_value(&operation_plan.plan.authority).unwrap()
    );
    assert_ne!(
        output["planDigest"].as_str(),
        Some(plan.upstream_plan_digest())
    );
    assert_eq!(output["stateRevisionAfter"], 4);
}

#[test]
fn managed_result_digest_is_canonical_and_uninstall_maps_to_removed_state() {
    assert_eq!(
        canonical_value_digest(&serde_json::json!({"z": 1, "a": [true, null]})).unwrap(),
        "sha256:ca6da02fba3343778761e7785f2b55f7fb17b36ce16eee3492dc392fa7c9deaa"
    );

    let (_temporary, _manager, mut plan) = full_plan_record(
        PluginAuthorizationPolicy::default(),
        a3s_use_core::PlanActor::User,
    );
    let mut envelope = plan.plugin_operation_plan.take().unwrap();
    envelope.plan.action = a3s_use_core::PluginOperationAction::Uninstall;
    let result = StoredOperationResult {
        schema: OPERATION_RECORD_SCHEMA.to_string(),
        operation_id: plan.operation_id,
        plan_digest: plan.plan_digest,
        completed_at_ms: plan.created_at_ms,
        capability_before: plan.capability_state,
        capability_after: PluginCapabilityEvidence {
            status: PluginCapabilityEvidenceStatus::Verified,
            observed_at_ms: 2,
            generation: Some(8),
            revision: Some("d".repeat(64)),
            error: None,
        },
        data: serde_json::json!({"operations": []}),
    };

    let state = managed_package_state(&envelope, &result).unwrap();

    assert_eq!(state.desired, PluginDesiredState::Absent);
    assert_eq!(state.observed, PluginObservedState::Removed);
    assert!(state.version.is_none());
    assert!(state.package_generation.is_none());
    assert!(state.package_digest.is_none());
    assert!(state.manifest_digest.is_none());
    assert!(state.receipt_digest.is_none());
    assert!(state.selected_surfaces.is_empty());
    assert_eq!(state.capability_generation, 8);
    assert_eq!(
        state.capability_revision,
        format!("sha256:{}", "d".repeat(64))
    );
}

fn full_plan_record(
    policy: PluginAuthorizationPolicy,
    actor: a3s_use_core::PlanActor,
) -> (tempfile::TempDir, PluginManager, StoredPluginPlan) {
    let temporary = tempfile::tempdir().unwrap();
    let manager = manager(temporary.path(), policy.clone());
    let mut fixture = install_plan();
    fixture.providers.clear();
    fixture.workspace_impacts.clear();
    fixture.secret_changes.clear();
    for package in &mut fixture.packages {
        package
            .surfaces
            .retain(|surface| surface.surface.kind == a3s_use_core::PluginSurfaceKind::Skill);
        if let Some(after) = package.after.as_mut() {
            after
                .release
                .surfaces
                .retain(|surface| surface.kind == a3s_use_core::PluginSurfaceKind::Skill);
            after.permissions.surfaces.clear();
            after.release.permission_ceiling_digest =
                after.permissions.descriptor_digest().unwrap();
        }
    }
    let capability_generation = fixture.state.capability_generation;
    let draft = a3s_use_core::PluginOperationPlanDraft::new(
        fixture.action,
        fixture.package_id,
        fixture.component_id,
        fixture.packages,
        fixture.providers,
        fixture.workspace_impacts,
        fixture.impact,
        fixture.state,
    )
    .unwrap();
    let state_revision = draft.state.state_revision;
    let capability_state = PluginCapabilityEvidence {
        status: PluginCapabilityEvidenceStatus::Verified,
        observed_at_ms: 1,
        generation: Some(capability_generation),
        revision: Some("a".repeat(64)),
        error: None,
    };
    let request = PluginPlanRequest {
        action: PluginLifecycleAction::Install,
        component_id: "use/acme/research".to_string(),
        version: Some("2.0.0".to_string()),
        channel: Some("stable".to_string()),
        registry_name: Some("fixture".to_string()),
    };
    let created_at_ms = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap();
    let identity = PluginPlanIdentity {
        operation_id: "plugin-install-host".to_string(),
        created_at_ms,
        expires_at_ms: created_at_ms + 60_000,
    };
    let installed_generations = std::collections::BTreeMap::new();
    let runtime_host = crate::plugin_manager::PluginRuntimeHost::default();
    let prepared = futures::executor::block_on(plan_artifact::prepare(
        plan_artifact::HostPlanContext {
            authorization: &policy,
            actor,
            scope: &crate::plugin_manager::default_plan_scope(),
            observed: plan_artifact::ObservedPlanState {
                capability: &capability_state,
                state_revision,
            },
            identity: &identity,
            grant_snapshot: None,
            installed_generations: &installed_generations,
            runtime_host: &runtime_host,
        },
        &request,
        "b".repeat(64),
        None,
        serde_json::json!({
            "dryRun": true,
            "planDigest": "b".repeat(64),
            "pluginOperationPlan": draft,
        }),
    ))
    .unwrap();
    let plan = StoredPluginPlan {
        schema: OPERATION_RECORD_SCHEMA.to_string(),
        operation_id: identity.operation_id,
        created_at_ms: identity.created_at_ms,
        expires_at_ms: identity.expires_at_ms,
        request,
        actor,
        scope: crate::plugin_manager::default_plan_scope(),
        plan_digest: prepared.plan_digest,
        upstream_plan_digest: prepared.upstream_plan_digest,
        capability_state,
        plan: prepared.plan,
        registry_source_revision: None,
        plugin_operation_plan: prepared.plugin_operation_plan,
        planning_bundles: prepared.planning_bundles,
        grant_snapshot: None,
        managed_plan_request: None,
        lifecycle_required: true,
    };
    (temporary, manager, plan)
}

fn manager(root: &std::path::Path, authorization: PluginAuthorizationPolicy) -> PluginManager {
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    PluginManager::new_with_policy(
        root.join("config.acl"),
        workspace,
        ComponentPaths::for_test(root),
        RegistryStore::for_test(root),
        PluginManagerPolicy {
            offline: true,
            authorization,
        },
    )
}

#[cfg(unix)]
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

fn write_capability_use_fixture(
    _root: &std::path::Path,
    _installed_record: &std::path::Path,
) -> std::path::PathBuf {
    if let Some(executable) = std::env::var_os("A3S_USE_E2E_BIN").map(std::path::PathBuf::from) {
        assert!(executable.is_absolute(), "A3S_USE_E2E_BIN must be absolute");
        assert_eq!(
            executable.file_name().and_then(std::ffi::OsStr::to_str),
            Some(if cfg!(windows) {
                "a3s-use.exe"
            } else {
                "a3s-use"
            }),
            "A3S_USE_E2E_BIN must name the platform a3s-use executable",
        );
        return executable
            .parent()
            .expect("A3S_USE_E2E_BIN must have a parent directory")
            .to_path_buf();
    }

    #[cfg(windows)]
    panic!("A3S_USE_E2E_BIN is required for the Windows managed Runtime/Grant test");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let directory = _root.join("use-bin");
        std::fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("a3s-use");
        let installed_record = _installed_record
            .display()
            .to_string()
            .replace('\'', "'\"'\"'");
        let before = capability_snapshot(0, &"f".repeat(64));
        let after = capability_snapshot(1, &"c".repeat(64));
        let script = format!(
            "#!/bin/sh\n\
         case \"$1\" in\n\
           --version) printf '%s\\n' 'a3s-use 0.3.0' ;;\n\
           capability)\n\
             if [ -f '{installed_record}' ]; then\n\
               printf '%s\\n' '{after}'\n\
             else\n\
               printf '%s\\n' '{before}'\n\
             fi\n\
             ;;\n\
           *) exit 64 ;;\n\
         esac\n"
        );
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }
}

#[cfg(windows)]
fn write_forbidden_a3s(
    root: &std::path::Path,
    _calls_path: &std::path::Path,
) -> std::path::PathBuf {
    root.join("forbidden-a3s.exe")
}

#[cfg(unix)]
fn write_forbidden_a3s(root: &std::path::Path, calls_path: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let directory = root.join("a3s-bin");
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("a3s");
    let calls_path = calls_path.display().to_string().replace('\'', "'\"'\"'");
    let script = format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{calls_path}'\nexit 97\n");
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    executable
}

fn package_fingerprint(root: &std::path::Path) -> (String, u64, u64) {
    use sha2::{Digest, Sha256};
    use std::io::Read as _;

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
