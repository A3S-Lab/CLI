use super::*;
use crate::components::ComponentPaths;
use crate::plugin_manager::operation::store::{
    PluginPlanIdentity, StoredPluginPlan, OPERATION_RECORD_SCHEMA,
};
use crate::plugin_manager::policy::tests::install_plan;
use crate::plugin_manager::process::PluginLifecycleAction;
use crate::plugin_manager::{PluginAuthorizationPolicy, PluginManagerPolicy};
use crate::registry::RegistryStore;

#[test]
fn operation_id_apply_rejects_legacy_identity_fields() {
    let request = PluginApplyRequest {
        operation_id: Some("plugin-install-abc".to_string()),
        action: Some(PluginLifecycleAction::Install),
        component_id: None,
        version: None,
        channel: None,
        plan_digest: "a".repeat(64),
    };

    assert!(apply_identity(&request).is_err());
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

#[test]
fn operation_id_apply_accepts_a_canonical_prefixed_digest() {
    let digest = "c".repeat(64);
    let request: PluginApplyRequest = serde_json::from_value(serde_json::json!({
        "operationId": "plugin-install-abc",
        "planDigest": format!("sha256:{digest}"),
    }))
    .unwrap();

    let (operation_id, legacy_request) = apply_identity(&request).unwrap();

    assert_eq!(operation_id.as_deref(), Some("plugin-install-abc"));
    assert!(legacy_request.is_none());
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
    };
    let raw_plan = planner::attach_draft(&request, &installation, None, 1, raw_plan).unwrap();
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
    let prepared = plan_artifact::prepare(
        &policy,
        &request,
        PlanActor::User,
        plan_artifact::ObservedPlanState {
            capability: &capability,
            state_revision: 1,
        },
        &identity,
        upstream_digest,
        raw_plan,
    )
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
        .create_plan_for_actor(crate::plugin_manager::operation::store::NewPluginPlan {
            identity,
            request,
            actor: PlanActor::User,
            plan_digest: prepared.plan_digest.clone(),
            upstream_plan_digest: prepared.upstream_plan_digest,
            capability_state: capability,
            plan: prepared.plan,
            plugin_operation_plan: prepared.plugin_operation_plan,
        })
        .await
        .unwrap();
    let apply_request = PluginApplyRequest {
        operation_id: Some(stored.operation_id.clone()),
        action: None,
        component_id: None,
        version: None,
        channel: None,
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

    std::fs::write(
        manager.registry_store.root().join("fixture.acl"),
        format!(
            "registry \"fixture\" {{\n  url = \"{}\"\n  trust_root = \"sha256:{}\"\n}}\n",
            server.base_url(),
            "b".repeat(64)
        ),
    )
    .unwrap();
    let drift = crate::components::apply_reviewed_cognitive_package(
        &envelope,
        Some(&confirmation),
        &component_paths,
        &manager.registry_store,
    )
    .await
    .unwrap_err();
    assert!(drift
        .to_string()
        .contains("no longer matches its locked URL and trust root"));
}

#[tokio::test]
async fn safe_plan_parent_lifecycle_is_durable_and_replayable() {
    let (_temporary, manager, plan) = full_plan_record(
        PluginAuthorizationPolicy::default(),
        a3s_use_core::PlanActor::User,
    );
    let confirmation = verify_new_apply_authority(&manager, &plan, true, plan.created_at_ms)
        .unwrap()
        .unwrap();
    let intent = manager
        .operation_store
        .persist_intent_with_confirmation(&plan, Some(confirmation))
        .await
        .unwrap();
    let first = manager
        .operation_store
        .begin_lifecycle(&plan, intent.started_at_ms)
        .await
        .unwrap()
        .unwrap();
    let replay = manager
        .operation_store
        .begin_lifecycle(&plan, intent.started_at_ms)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(first, replay);
    assert!(first.cutover.is_none());
    let capability_after = PluginCapabilityEvidence {
        status: PluginCapabilityEvidenceStatus::Verified,
        observed_at_ms: intent.started_at_ms + 1,
        generation: Some(first.binding.capability_generation_after()),
        revision: Some("c".repeat(64)),
        error: None,
    };
    let completed = manager
        .operation_store
        .complete_lifecycle(
            &plan,
            &capability_after,
            first.binding.state_revision_after(),
            intent.started_at_ms + 2,
        )
        .await
        .unwrap()
        .unwrap();
    let completed_replay = manager
        .operation_store
        .complete_lifecycle(
            &plan,
            &capability_after,
            first.binding.state_revision_after(),
            intent.started_at_ms + 3,
        )
        .await
        .unwrap()
        .unwrap();

    assert_eq!(completed, completed_replay);
    assert_eq!(
        lifecycle_completed_at_ms(Some(&completed_replay), intent.started_at_ms + 3,).unwrap(),
        intent.started_at_ms + 2
    );
    assert_eq!(
        completed
            .cutover
            .as_ref()
            .unwrap()
            .capability_snapshot_digest(),
        format!("sha256:{}", "c".repeat(64))
    );
    let completed_at_ms = intent.started_at_ms + 2;
    let data = applied_output(
        serde_json::json!({
            "planDigest": plan.upstream_plan_digest(),
            "operations": [],
        }),
        &plan,
        AppliedStateEvidence {
            capability_before: &plan.capability_state,
            capability_after: &capability_after,
            state_revision_after: first.binding.state_revision_after(),
            lifecycle: Some(&completed),
        },
        completed_at_ms,
        false,
        false,
    )
    .unwrap();
    assert_eq!(
        data["lifecycleBindingDigest"],
        completed.binding.binding_digest()
    );
    assert_eq!(
        data["lifecycleCutoverDigest"],
        completed.cutover.as_ref().unwrap().cutover_digest()
    );
    let result = StoredOperationResult {
        schema: OPERATION_RECORD_SCHEMA.to_string(),
        operation_id: plan.operation_id.clone(),
        plan_digest: plan.plan_digest.clone(),
        completed_at_ms,
        capability_before: plan.capability_state.clone(),
        capability_after,
        data,
    };
    manager
        .operation_store
        .validate_lifecycle_result_sync(&plan, &result)
        .unwrap();

    let mut drifted = result;
    drifted.data["capabilitySnapshotDigest"] =
        serde_json::Value::String(format!("sha256:{}", "d".repeat(64)));
    assert!(manager
        .operation_store
        .validate_lifecycle_result_sync(&plan, &drifted)
        .is_err());
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
    };
    let created_at_ms = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap();
    let identity = PluginPlanIdentity {
        operation_id: "plugin-install-host".to_string(),
        created_at_ms,
        expires_at_ms: created_at_ms + 60_000,
    };
    let prepared = plan_artifact::prepare(
        &policy,
        &request,
        actor,
        plan_artifact::ObservedPlanState {
            capability: &capability_state,
            state_revision,
        },
        &identity,
        "b".repeat(64),
        serde_json::json!({
            "dryRun": true,
            "planDigest": "b".repeat(64),
            "pluginOperationPlan": draft,
        }),
    )
    .unwrap();
    let plan = StoredPluginPlan {
        schema: OPERATION_RECORD_SCHEMA.to_string(),
        operation_id: identity.operation_id,
        created_at_ms: identity.created_at_ms,
        expires_at_ms: identity.expires_at_ms,
        request,
        actor,
        plan_digest: prepared.plan_digest,
        upstream_plan_digest: prepared.upstream_plan_digest,
        capability_state,
        plan: prepared.plan,
        plugin_operation_plan: prepared.plugin_operation_plan,
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
        RegistryStore::new(root.join("registries")),
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

#[cfg(unix)]
fn write_capability_use_fixture(
    root: &std::path::Path,
    installed_record: &std::path::Path,
) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let directory = root.join("use-bin");
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("a3s-use");
    let installed_record = installed_record
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
