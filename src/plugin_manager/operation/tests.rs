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
    assert!(
        !manager
            .operation_store
            .persist_intent_with_confirmation(&plan, Some(confirmation))
            .await
            .unwrap()
            .resumed
    );
    assert!(manager.operation_store.has_intent(&plan).await.unwrap());
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
