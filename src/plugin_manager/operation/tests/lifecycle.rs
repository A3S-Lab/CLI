use super::*;

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
        lifecycle_completed_at_ms(Some(&completed_replay), intent.started_at_ms + 3).unwrap(),
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
