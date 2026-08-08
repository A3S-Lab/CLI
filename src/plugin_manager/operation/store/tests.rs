use super::*;
use crate::plugin_manager::capability::PluginCapabilityEvidenceStatus;
use crate::plugin_manager::process::PluginLifecycleAction;

fn request() -> PluginPlanRequest {
    PluginPlanRequest {
        action: PluginLifecycleAction::Install,
        component_id: "use/acme/research".to_string(),
        version: Some("2.0.0".to_string()),
        channel: Some("stable".to_string()),
    }
}

fn evidence(generation: u64, revision: char) -> PluginCapabilityEvidence {
    PluginCapabilityEvidence {
        status: PluginCapabilityEvidenceStatus::Verified,
        observed_at_ms: 42,
        generation: Some(generation),
        revision: Some(revision.to_string().repeat(64)),
        error: None,
    }
}

fn plan_value(plan_digest: &str) -> Value {
    serde_json::json!({
        "dryRun": true,
        "planDigest": plan_digest,
        "plans": [],
    })
}

fn operation_result(plan: &StoredPluginPlan, completed_at_ms: u64) -> StoredOperationResult {
    let capability_before = evidence(7, 'b');
    let capability_after = evidence(8, 'c');
    StoredOperationResult {
        schema: OPERATION_RECORD_SCHEMA.to_string(),
        operation_id: plan.operation_id.clone(),
        plan_digest: plan.plan_digest.clone(),
        completed_at_ms,
        capability_before: capability_before.clone(),
        capability_after: capability_after.clone(),
        data: serde_json::json!({
            "planDigest": plan.plan_digest,
            "canonicalPlanDigest": format!("sha256:{}", plan.plan_digest),
            "operations": [],
            "operationId": plan.operation_id,
            "capabilityBefore": capability_before,
            "capabilityAfter": capability_after,
            "completedAtMs": completed_at_ms,
            "stateRevisionAfter": 2,
            "resumed": false,
            "replayed": false,
        }),
    }
}

#[tokio::test]
async fn durable_plan_intent_and_result_are_append_only_and_replayable() {
    let temporary = tempfile::tempdir().unwrap();
    let store = PluginOperationStore::new(temporary.path().join("operations"));
    let plan_digest = "a".repeat(64);
    let plan = store
        .create_plan(
            request(),
            plan_digest.clone(),
            evidence(7, 'b'),
            plan_value(&plan_digest),
        )
        .await
        .unwrap();

    let resolved = store
        .resolve_plan(plan.operation_id.clone(), plan_digest)
        .await
        .unwrap();
    assert_eq!(resolved, plan);
    assert!(!store.persist_intent(&plan).await.unwrap().resumed);
    assert!(store.persist_intent(&plan).await.unwrap().resumed);
    let intent =
        read_required_record::<StoredApplyIntent>(&store.intent_path(&plan.operation_id)).unwrap();
    let result = operation_result(&plan, intent.started_at_ms);

    let (first, created) = store.persist_result(result.clone()).await.unwrap();
    assert!(created);
    assert_eq!(first, result);
    let (replayed, created) = store.persist_result(result.clone()).await.unwrap();
    assert!(!created);
    assert_eq!(replayed, result);
    assert_eq!(store.result(&plan).await.unwrap(), Some(result));
}

#[tokio::test]
async fn reviewed_plan_persists_the_host_selected_actor() {
    let temporary = tempfile::tempdir().unwrap();
    let store = PluginOperationStore::new(temporary.path().join("operations"));
    let plan_digest = "6".repeat(64);
    let identity = store
        .allocate_plan_identity(PluginLifecycleAction::Install)
        .await
        .unwrap();
    let plan = store
        .create_plan_for_actor(NewPluginPlan {
            identity,
            request: request(),
            actor: a3s_use_core::PlanActor::Agent,
            scope: crate::plugin_manager::default_plan_scope(),
            plan_digest: plan_digest.clone(),
            upstream_plan_digest: None,
            capability_state: evidence(7, 'b'),
            plan: plan_value(&plan_digest),
            plugin_operation_plan: None,
            planning_bundles: std::collections::BTreeMap::new(),
            grant_snapshot: None,
            managed_plan_request: None,
        })
        .await
        .unwrap();

    let resolved = store
        .resolve_plan(plan.operation_id.clone(), plan_digest)
        .await
        .unwrap();

    assert_eq!(resolved.actor, a3s_use_core::PlanActor::Agent);
}

#[tokio::test]
async fn identical_plans_receive_distinct_operation_ids() {
    let temporary = tempfile::tempdir().unwrap();
    let store = PluginOperationStore::new(temporary.path().join("operations"));
    let plan_digest = "e".repeat(64);
    let first = store
        .create_plan(
            request(),
            plan_digest.clone(),
            evidence(1, 'a'),
            plan_value(&plan_digest),
        )
        .await
        .unwrap();
    let second = store
        .create_plan(
            request(),
            plan_digest.clone(),
            evidence(1, 'a'),
            plan_value(&plan_digest),
        )
        .await
        .unwrap();

    assert_ne!(first.operation_id, second.operation_id);
    assert_eq!(first.plan_digest, second.plan_digest);
}

#[tokio::test]
async fn operation_id_and_digest_mismatch_fails_closed() {
    let temporary = tempfile::tempdir().unwrap();
    let store = PluginOperationStore::new(temporary.path().join("operations"));
    let plan_digest = "f".repeat(64);
    let plan = store
        .create_plan(
            request(),
            plan_digest.clone(),
            evidence(1, 'a'),
            plan_value(&plan_digest),
        )
        .await
        .unwrap();

    let error = store
        .resolve_plan(plan.operation_id, "0".repeat(64))
        .await
        .unwrap_err();

    assert!(matches!(error, PluginManagerError::InvalidRequest(_)));
}

#[tokio::test]
async fn expired_plan_cannot_publish_a_new_apply_intent() {
    let temporary = tempfile::tempdir().unwrap();
    let store = PluginOperationStore::new(temporary.path().join("operations"));
    let plan_digest = "1".repeat(64);
    let plan = StoredPluginPlan {
        schema: OPERATION_RECORD_SCHEMA.to_string(),
        operation_id: "plugin-install-expired".to_string(),
        created_at_ms: 1,
        expires_at_ms: 2,
        request: request(),
        actor: a3s_use_core::PlanActor::User,
        scope: crate::plugin_manager::default_plan_scope(),
        plan_digest: plan_digest.clone(),
        upstream_plan_digest: None,
        capability_state: evidence(1, 'a'),
        plan: plan_value(&plan_digest),
        plugin_operation_plan: None,
        planning_bundles: std::collections::BTreeMap::new(),
        grant_snapshot: None,
        managed_plan_request: None,
        lifecycle_required: false,
    };
    write_new_record(&store.plan_path(&plan.operation_id), &plan).unwrap();

    let error = store.persist_intent(&plan).await.unwrap_err();

    assert!(matches!(error, PluginManagerError::OperationFailed(_)));
}

#[tokio::test]
async fn durable_result_requires_a_prior_apply_intent() {
    let temporary = tempfile::tempdir().unwrap();
    let store = PluginOperationStore::new(temporary.path().join("operations"));
    let plan_digest = "2".repeat(64);
    let plan = store
        .create_plan(
            request(),
            plan_digest.clone(),
            evidence(1, 'a'),
            plan_value(&plan_digest),
        )
        .await
        .unwrap();
    let result = operation_result(&plan, plan.created_at_ms + 1);

    assert!(store.persist_result(result).await.is_err());
}

#[tokio::test]
async fn durable_result_cannot_complete_before_its_apply_intent() {
    let temporary = tempfile::tempdir().unwrap();
    let store = PluginOperationStore::new(temporary.path().join("operations"));
    let plan_digest = "3".repeat(64);
    let plan = store
        .create_plan(
            request(),
            plan_digest.clone(),
            evidence(1, 'a'),
            plan_value(&plan_digest),
        )
        .await
        .unwrap();
    let intent = StoredApplyIntent {
        schema: OPERATION_RECORD_SCHEMA.to_string(),
        operation_id: plan.operation_id.clone(),
        plan_digest: plan.plan_digest.clone(),
        started_at_ms: plan.created_at_ms + 10,
        confirmation: None,
        managed_apply_request: None,
    };
    write_new_record(&store.intent_path(&plan.operation_id), &intent).unwrap();
    let result = operation_result(&plan, plan.created_at_ms + 5);

    let error = store.persist_result(result).await.unwrap_err();

    assert!(matches!(error, PluginManagerError::Infrastructure(_)));
}
