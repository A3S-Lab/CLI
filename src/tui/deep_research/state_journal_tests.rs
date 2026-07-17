use super::*;

fn spec() -> ResearchSpec {
    ResearchSpec {
        query: "event sourced research".to_string(),
        current_date: "2026-07-12".to_string(),
        evidence_scope: "web_and_workspace".to_string(),
        required_claims: vec!["architecture".to_string()],
        total_budget_ms: 60_000,
        finalization_reserve_ms: 9_000,
        host_pid: 0,
    }
}

fn event(sequence: u64, name: &str, payload: serde_json::Value) -> ResearchDomainEvent {
    ResearchDomainEvent {
        source: "flow".to_string(),
        source_sequence: sequence,
        source_event_id: format!("flow-{sequence}"),
        name: name.to_string(),
        payload,
    }
}

#[tokio::test]
async fn strictly_replays_persisted_projection() {
    let temp = tempfile::tempdir().unwrap();
    let mut journal = DeepResearchStateJournal::create(temp.path(), "run-1", spec())
        .await
        .unwrap();
    journal
        .append(event(
            1,
            "research.track.scheduled",
            serde_json::json!({"step_id": "sources"}),
        ))
        .await
        .unwrap();
    drop(journal);

    let restored = DeepResearchStateJournal::open(temp.path(), "run-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.projection().unwrap().active_steps, ["sources"]);
    assert!(restored.projection().unwrap().can_schedule());
}

#[tokio::test]
async fn duplicate_external_event_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let mut journal = DeepResearchStateJournal::create(temp.path(), "run-2", spec())
        .await
        .unwrap();
    let scheduled = event(
        1,
        "research.track.scheduled",
        serde_json::json!({"step_id": "sources"}),
    );
    assert!(journal.append(scheduled.clone()).await.unwrap());
    assert!(!journal.append(scheduled).await.unwrap());
    assert_eq!(journal.projection().unwrap().active_steps.len(), 1);
}

#[tokio::test]
async fn reopening_workflow_requires_the_same_research_spec_identity() {
    let temp = tempfile::tempdir().unwrap();
    let run_id = "run-spec-identity";
    let original = spec();
    record_workflow_started(temp.path(), run_id, original.clone())
        .await
        .unwrap();

    let mut same_identity = original.clone();
    same_identity.host_pid = 42;
    record_workflow_started(temp.path(), run_id, same_identity)
        .await
        .expect("host process identity is not part of the research request");

    let cases = [
        ("query", {
            let mut changed = original.clone();
            changed.query = "different research question".to_string();
            changed
        }),
        ("current_date", {
            let mut changed = original.clone();
            changed.current_date = "2026-07-13".to_string();
            changed
        }),
        ("evidence_scope", {
            let mut changed = original.clone();
            changed.evidence_scope = "local_only".to_string();
            changed
        }),
        ("required_claims", {
            let mut changed = original.clone();
            changed.required_claims = vec!["different claim".to_string()];
            changed
        }),
        ("total_budget_ms", {
            let mut changed = original.clone();
            changed.total_budget_ms += 1;
            changed
        }),
        ("finalization_reserve_ms", {
            let mut changed = original;
            changed.finalization_reserve_ms += 1;
            changed
        }),
    ];
    for (field, changed) in cases {
        let error = record_workflow_started(temp.path(), run_id, changed)
            .await
            .expect_err("a changed research identity must fail closed");
        let detail = format!("{error:#}");
        assert!(detail.contains(field), "{field}: {detail}");
    }

    let journal = DeepResearchStateJournal::open(temp.path(), run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(journal.spec().unwrap(), spec());
    assert_eq!(
        journal.projection().unwrap().active_steps,
        ["evidence-collection"]
    );
}

#[tokio::test]
async fn terminal_event_clears_activity_and_blocks_more_work() {
    let temp = tempfile::tempdir().unwrap();
    let mut journal = DeepResearchStateJournal::create(temp.path(), "run-3", spec())
        .await
        .unwrap();
    journal
        .append(event(
            1,
            "research.track.scheduled",
            serde_json::json!({"step_id": "sources"}),
        ))
        .await
        .unwrap();
    journal
        .append(event(2, "research.run.qualified", serde_json::json!({})))
        .await
        .unwrap();
    let run = journal.projection().unwrap();
    assert_eq!(run.outcome, ResearchOutcome::Qualified);
    assert!(run.active_steps.is_empty());
    assert!(!journal
        .append(event(2, "research.run.qualified", serde_json::json!({}),))
        .await
        .unwrap());
    assert!(journal
        .append(event(
            3,
            "research.track.scheduled",
            serde_json::json!({"step_id": "late"}),
        ))
        .await
        .is_err());
}

#[tokio::test]
async fn terminal_artifact_quality_is_monotonic() {
    let temp = tempfile::tempdir().unwrap();
    let mut journal = DeepResearchStateJournal::create(temp.path(), "run-grade", spec())
        .await
        .unwrap();
    journal
        .append(event(1, "research.run.qualified", serde_json::json!({})))
        .await
        .unwrap();
    assert!(journal
        .append(event(
            2,
            "research.run.outcome_upgraded",
            serde_json::json!({"outcome": "degraded"}),
        ))
        .await
        .is_err());
    journal
        .append(event(
            2,
            "research.run.outcome_upgraded",
            serde_json::json!({"outcome": "completed"}),
        ))
        .await
        .unwrap();
    assert_eq!(
        journal.projection().unwrap().outcome,
        ResearchOutcome::Completed
    );
}

#[tokio::test]
async fn concurrent_writer_cannot_publish_stale_generation() {
    let temp = tempfile::tempdir().unwrap();
    DeepResearchStateJournal::create(temp.path(), "run-4", spec())
        .await
        .unwrap();
    let mut first = DeepResearchStateJournal::open(temp.path(), "run-4")
        .await
        .unwrap()
        .unwrap();
    let mut stale = DeepResearchStateJournal::open(temp.path(), "run-4")
        .await
        .unwrap()
        .unwrap();
    first
        .append(event(
            1,
            "research.track.scheduled",
            serde_json::json!({"step_id": "first"}),
        ))
        .await
        .unwrap();
    assert!(stale
        .append(event(
            1,
            "research.track.scheduled",
            serde_json::json!({"step_id": "stale"}),
        ))
        .await
        .is_err());
}

#[tokio::test]
async fn workflow_boundary_helpers_persist_under_dot_a3s() {
    let temp = tempfile::tempdir().unwrap();
    record_workflow_started(temp.path(), "run-boundary", spec())
        .await
        .unwrap();
    record_workflow_completed(temp.path(), "run-boundary", true)
        .await
        .unwrap();

    let journal = DeepResearchStateJournal::open(temp.path(), "run-boundary")
        .await
        .unwrap()
        .unwrap();
    let projection = journal.projection().unwrap();
    assert!(projection.active_steps.is_empty());
    assert_eq!(projection.last_domain_event, "research.track.completed");
    assert!(temp.path().join(".a3s/research/runs/events").is_dir());
    assert!(temp.path().join(".a3s/research/runs/checkpoints").is_dir());
    assert!(!temp.path().join(".a3s-flow").exists());
}

#[tokio::test]
async fn terminal_helper_publishes_artifact_head_and_clears_activity() {
    let temp = tempfile::tempdir().unwrap();
    let markdown = temp.path().join("report.md");
    let html = temp.path().join("index.html");
    std::fs::write(&markdown, "# Verified report").unwrap();
    std::fs::write(&html, "<!doctype html><h1>Verified report</h1>").unwrap();
    let artifacts = super::super::ResearchReportArtifacts { markdown, html };
    record_workflow_started(temp.path(), "run-terminal", spec())
        .await
        .unwrap();
    record_workflow_completed(temp.path(), "run-terminal", true)
        .await
        .unwrap();
    record_run_terminal(
        temp.path(),
        "run-terminal",
        ResearchOutcome::Qualified,
        Some(&artifacts),
    )
    .await
    .unwrap();

    let journal = DeepResearchStateJournal::open(temp.path(), "run-terminal")
        .await
        .unwrap()
        .unwrap();
    let projection = journal.projection().unwrap();
    assert_eq!(projection.outcome, ResearchOutcome::Qualified);
    assert!(projection.active_steps.is_empty());
    assert_eq!(
        projection.artifact_evidence_head.as_deref().unwrap().len(),
        64
    );
    assert!(!projection.can_schedule());
}

#[tokio::test]
async fn child_lifecycle_is_replayable_and_ordered() {
    let temp = tempfile::tempdir().unwrap();
    record_workflow_started(temp.path(), "run-child", spec())
        .await
        .unwrap();
    record_child_event(
        temp.path(),
        "run-child",
        1,
        "child-a",
        true,
        serde_json::json!({"task_id": "child-a", "agent": "researcher"}),
    )
    .await
    .unwrap();
    let active = DeepResearchStateJournal::open(temp.path(), "run-child")
        .await
        .unwrap()
        .unwrap()
        .projection()
        .unwrap();
    assert_eq!(active.active_children, ["child-a"]);

    assert!(record_child_event(
        temp.path(),
        "run-child",
        3,
        "child-a",
        false,
        serde_json::json!({"task_id": "child-a", "success": true}),
    )
    .await
    .is_err());
    record_child_event(
        temp.path(),
        "run-child",
        2,
        "child-a",
        false,
        serde_json::json!({"task_id": "child-a", "success": true}),
    )
    .await
    .unwrap();
    let settled = DeepResearchStateJournal::open(temp.path(), "run-child")
        .await
        .unwrap()
        .unwrap()
        .projection()
        .unwrap();
    assert!(settled.active_children.is_empty());
}

#[tokio::test]
async fn convergence_reason_survives_strict_replay() {
    let temp = tempfile::tempdir().unwrap();
    record_workflow_started(temp.path(), "run-convergence", spec())
        .await
        .unwrap();
    let decision = super::super::deep_research_convergence::evaluate_convergence(
        super::super::deep_research_convergence::ConvergenceInput {
            accepted_evidence: 0,
            traceable_sources: 0,
            authoritative_sources: 0,
            unresolved_contradictions: 0,
            unresolved_gaps: 1,
            completed_rounds: 1,
            max_rounds: 1,
            rounds_without_material_gain: 1,
            remaining_ms: 20_000,
            finalization_reserve_ms: 10_000,
            evidence_package_complete: false,
        },
    );
    record_convergence(temp.path(), "run-convergence", &decision)
        .await
        .unwrap();
    let restored = DeepResearchStateJournal::open(temp.path(), "run-convergence")
        .await
        .unwrap()
        .unwrap()
        .projection()
        .unwrap();
    assert_eq!(
        restored.convergence_reason.as_deref(),
        Some("bounded research round limit reached")
    );
}

#[tokio::test]
async fn normalized_sources_merge_after_concurrent_head_conflict() {
    let temp = tempfile::tempdir().unwrap();
    record_workflow_started(temp.path(), "run-merge", spec())
        .await
        .unwrap();
    let child = record_child_event(
        temp.path(),
        "run-merge",
        1,
        "child-a",
        true,
        serde_json::json!({"task_id": "child-a"}),
    );
    let workflow = record_workflow_completed(temp.path(), "run-merge", true);
    let (child_result, workflow_result) = tokio::join!(child, workflow);
    child_result.unwrap();
    workflow_result.unwrap();

    let restored = DeepResearchStateJournal::open(temp.path(), "run-merge")
        .await
        .unwrap()
        .unwrap()
        .projection()
        .unwrap();
    assert_eq!(restored.active_children, ["child-a"]);
    assert!(restored.active_steps.is_empty());
}

#[tokio::test]
async fn accepted_evidence_materializes_typed_objects_and_relations() {
    let temp = tempfile::tempdir().unwrap();
    record_workflow_started(temp.path(), "run-evidence", spec())
        .await
        .unwrap();
    let raw = serde_json::json!({
        "structured": {
            "summary": "The documented date is July 12.",
            "sources": [{
                "title": "Release notice",
                "url_or_path": "https://example.gov/release",
                "quote_or_fact": "Published July 12",
                "reliability": "official"
            }],
            "key_evidence": ["The release was published July 12."],
            "contradictions": [],
            "gaps": [],
            "confidence": "high"
        }
    });
    let evidence = super::super::deep_research_evidence_ledger::accepted_evidence_ledger(
        &raw.to_string(),
        None,
    );
    record_evidence_ledger(temp.path(), "run-evidence", &evidence)
        .await
        .unwrap();

    let journal = DeepResearchStateJournal::open(temp.path(), "run-evidence")
        .await
        .unwrap()
        .unwrap();
    let object_types = journal
        .runtime
        .graph()
        .objects()
        .map(|object| object.object_type.as_str())
        .collect::<Vec<_>>();
    assert!(object_types.contains(&SOURCE_OBJECT_TYPE));
    assert!(object_types.contains(&EVIDENCE_OBJECT_TYPE));
    assert!(object_types.contains(&CLAIM_OBJECT_TYPE));
    let relation_types = journal
        .runtime
        .graph()
        .relations()
        .map(|relation| relation.relation_type.as_str())
        .collect::<Vec<_>>();
    assert!(relation_types.contains(&"deep_research.observed_in"));
    assert!(relation_types.contains(&"deep_research.supports"));
    let projection = journal.projection().unwrap();
    assert_eq!(projection.accepted_evidence_count, 1);
    assert_eq!(projection.source_count, 1);
    assert_eq!(projection.claim_count, 1);
    GraphRuntime::strict_replay(journal.runtime.events()).unwrap();
}

#[tokio::test]
async fn failed_claim_audit_downgrades_and_does_not_publish_artifact_head() {
    let temp = tempfile::tempdir().unwrap();
    record_workflow_started(temp.path(), "run-audit", spec())
        .await
        .unwrap();
    let raw = serde_json::json!({
        "structured": {
            "summary": "The documented date is July 12.",
            "sources": [{
                "title": "Release notice",
                "url_or_path": "https://example.gov/release",
                "quote_or_fact": "Published July 12",
                "reliability": "official"
            }],
            "key_evidence": ["The release was published July 12."],
            "contradictions": [],
            "gaps": [],
            "confidence": "high"
        }
    });
    let evidence = super::super::deep_research_evidence_ledger::accepted_evidence_ledger(
        &raw.to_string(),
        None,
    );
    record_evidence_ledger(temp.path(), "run-audit", &evidence)
        .await
        .unwrap();
    let markdown = temp.path().join("report.md");
    let html = temp.path().join("index.html");
    std::fs::write(
        &markdown,
        "# Report\n\nUnrelated conclusion.\n\nhttps://example.gov/release",
    )
    .unwrap();
    std::fs::write(&html, "<h1>Report</h1><p>Unrelated conclusion.</p>").unwrap();
    let projection = record_run_terminal(
        temp.path(),
        "run-audit",
        ResearchOutcome::Completed,
        Some(&super::super::ResearchReportArtifacts { markdown, html }),
    )
    .await
    .unwrap();
    assert_eq!(projection.outcome, ResearchOutcome::Degraded);
    assert!(projection.artifact_evidence_head.is_none());
    assert_eq!(projection.report_claim_coverage_basis_points, Some(0));
    assert!(projection
        .report_audit_reason
        .as_deref()
        .unwrap()
        .contains("less than half"));
}

#[tokio::test]
async fn diagnostics_strictly_replay_explicit_and_latest_runs() {
    let temp = tempfile::tempdir().unwrap();
    record_workflow_started(temp.path(), "run-diagnostic", spec())
        .await
        .unwrap();
    let status = research_diagnostic(
        temp.path(),
        Some("run-diagnostic"),
        ResearchDiagnosticKind::Status,
    )
    .await
    .unwrap();
    assert!(status.contains("DeepResearch run run-diagnostic"));
    assert!(status.contains("active: 1 steps"));

    let replay = research_diagnostic(temp.path(), None, ResearchDiagnosticKind::Replay)
        .await
        .unwrap();
    assert!(replay.contains("strict replay: ok"));
    assert!(replay.contains("graph:"));
    assert!(replay.contains("head:"));
}

#[tokio::test]
async fn restart_reconciliation_cancels_live_children_and_terminalizes_orphans() {
    let temp = tempfile::tempdir().unwrap();
    record_workflow_started(temp.path(), "run-restart", spec())
        .await
        .unwrap();
    record_child_event(
        temp.path(),
        "run-restart",
        1,
        "child-live",
        true,
        serde_json::json!({"task_id": "child-live"}),
    )
    .await
    .unwrap();
    record_child_event(
        temp.path(),
        "run-restart",
        2,
        "child-orphan",
        true,
        serde_json::json!({"task_id": "child-orphan"}),
    )
    .await
    .unwrap();
    let running = HashSet::from(["child-live".to_string()]);
    let recovery = reconcile_interrupted_latest_run(temp.path(), &running)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovery.cancel_children, ["child-live"]);
    assert_eq!(recovery.orphaned_children, ["child-orphan"]);
    let projection = DeepResearchStateJournal::open(temp.path(), "run-restart")
        .await
        .unwrap()
        .unwrap()
        .projection()
        .unwrap();
    assert_eq!(projection.outcome, ResearchOutcome::Failed);
    assert!(projection.active_children.is_empty());
    assert!(projection.active_steps.is_empty());
    assert!(reconcile_interrupted_latest_run(temp.path(), &running)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn recovery_does_not_touch_a_run_owned_by_a_live_host() {
    let temp = tempfile::tempdir().unwrap();
    let mut live_spec = spec();
    live_spec.host_pid = std::process::id();
    record_workflow_started(temp.path(), "run-live-host", live_spec)
        .await
        .unwrap();
    assert!(
        reconcile_interrupted_latest_run(temp.path(), &HashSet::new())
            .await
            .unwrap()
            .is_none()
    );
    let projection = DeepResearchStateJournal::open(temp.path(), "run-live-host")
        .await
        .unwrap()
        .unwrap()
        .projection()
        .unwrap();
    assert_eq!(projection.outcome, ResearchOutcome::Active);
    assert_eq!(projection.active_steps, ["evidence-collection"]);
}

#[tokio::test]
async fn corrupted_checkpoint_falls_back_to_strict_event_replay() {
    let temp = tempfile::tempdir().unwrap();
    record_workflow_started(temp.path(), "run-checkpoint", spec())
        .await
        .unwrap();
    let path = checkpoint_path(&checkpoint_root(temp.path()), "run-checkpoint");
    assert!(path.is_file());
    std::fs::write(&path, b"not-json").unwrap();
    let replay = research_diagnostic(temp.path(), None, ResearchDiagnosticKind::Replay)
        .await
        .unwrap();
    assert!(replay.contains("DeepResearch run run-checkpoint"));
    assert!(replay.contains("strict replay: ok"));
}

#[test]
fn event_payload_bounding_caps_depth_width_and_strings() {
    let value = serde_json::json!({
        "description": "x".repeat(10_000),
        "items": (0..100).collect::<Vec<_>>(),
        "nested": {"a":{"b":{"c":{"d":{"e":{"f":{"g":{"h":{"secret":"hidden"}}}}}}}}}
    });
    let bounded = bounded_event_payload(value, 0);
    assert!(bounded["description"].as_str().unwrap().chars().count() <= 4_001);
    assert_eq!(bounded["items"].as_array().unwrap().len(), 64);
    assert!(!bounded.to_string().contains("hidden"));
}

#[tokio::test]
async fn structural_diff_reports_evidence_and_relation_changes() {
    let temp = tempfile::tempdir().unwrap();
    record_workflow_started(temp.path(), "run-left", spec())
        .await
        .unwrap();
    record_workflow_started(temp.path(), "run-right", spec())
        .await
        .unwrap();
    let raw = serde_json::json!({
        "structured": {
            "summary": "A source-backed difference.",
            "sources": [{
                "url_or_path": "https://example.gov/diff",
                "quote_or_fact": "Observed difference",
                "reliability": "official"
            }],
            "key_evidence": ["The right branch has accepted evidence."],
            "contradictions": [],
            "gaps": [],
            "confidence": "high"
        }
    });
    let evidence = super::super::deep_research_evidence_ledger::accepted_evidence_ledger(
        &raw.to_string(),
        None,
    );
    record_evidence_ledger(temp.path(), "run-right", &evidence)
        .await
        .unwrap();
    let diff = research_diff(temp.path(), "run-left", "run-right")
        .await
        .unwrap();
    assert!(diff.contains("deep_research.evidence +1"), "{diff}");
    assert!(diff.contains("deep_research.source +1"), "{diff}");
    assert!(diff.contains("deep_research.claim +1"), "{diff}");
    assert!(diff.contains("deep_research.observed_in +1"), "{diff}");
    assert!(diff.contains("deep_research.supports +1"), "{diff}");
}

#[tokio::test]
async fn event_point_fork_isolatedly_adds_only_validated_evidence() {
    let temp = tempfile::tempdir().unwrap();
    record_workflow_started(temp.path(), "run-fork", spec())
        .await
        .unwrap();
    let base = DeepResearchStateJournal::open(temp.path(), "run-fork")
        .await
        .unwrap()
        .unwrap();
    let fork_sequence = base.runtime.events().len() as u64;
    let raw = serde_json::json!({
        "structured": {
            "summary": "Alternative official evidence.",
            "sources": [{
                "url_or_path": "https://example.gov/alternative",
                "quote_or_fact": "Alternative fact",
                "reliability": "official"
            }],
            "key_evidence": ["The alternative strategy found a fact."],
            "contradictions": [],
            "gaps": [],
            "confidence": "high"
        }
    });
    let evidence = super::super::deep_research_evidence_ledger::accepted_evidence_ledger(
        &raw.to_string(),
        None,
    );
    let summary = fork_with_validated_evidence(
        temp.path(),
        "run-fork",
        fork_sequence,
        "official-strategy",
        &evidence,
    )
    .await
    .unwrap();
    assert!(summary.objects_added >= 3);
    assert!(summary.relations_added >= 2);
    assert!(DeepResearchStateJournal::open(temp.path(), "run-fork")
        .await
        .unwrap()
        .unwrap()
        .runtime
        .graph()
        .objects()
        .all(|object| object.object_type != EVIDENCE_OBJECT_TYPE));
    let store = FileGraphEventStore::new(store_root(temp.path()));
    let branch_events = store.load(&summary.branch_store_id).await.unwrap().unwrap();
    let branch = GraphRuntime::restore(branch_events).unwrap();
    assert!(branch
        .graph()
        .objects()
        .any(|object| object.object_type == EVIDENCE_OBJECT_TYPE));
    let checkpoint = load_latest_checkpoint(temp.path()).await.unwrap();
    assert_eq!(checkpoint.run_id, "run-fork");
    assert_eq!(checkpoint.projection.accepted_evidence_count, 0);
}
