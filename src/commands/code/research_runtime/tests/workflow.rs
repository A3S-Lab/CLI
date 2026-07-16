use super::*;

#[test]
fn deepresearch_cli_reuses_the_engineered_tui_workflow_contract() {
    let args = deep_research_workflow_args(
        "Compare two general-purpose storage approaches using current evidence",
        false,
    );
    let source = args["source"].as_str().expect("workflow source");

    assert!(
        args["input"]["loop_contract"]["planner"].is_object(),
        "the shared workflow must start from the semantic LLM planner"
    );
    assert!(
        args["input"]["loop_contract"]["checker"].is_object(),
        "the shared workflow must retain the independent checker"
    );
    assert!(source.contains("research_planner"), "{source}");
    assert!(source.contains("track_assessments"), "{source}");
    assert!(source.contains("stop_condition_assessments"), "{source}");
    assert!(
        !source.contains("fallbackTracks") && !source.contains("complexityMarkers"),
        "the removed CLI workflow must not survive as a hidden rule engine"
    );
}

#[test]
fn deepresearch_cli_uses_the_shared_hard_safety_envelope() {
    let args = deep_research_workflow_args("Investigate a broad technical decision", false);

    assert_eq!(args["input"]["os_runtime"], false);
    assert_eq!(args["input"]["evidence_scope"], "web_and_workspace");
    assert!(args["input"]["local_max_parallel_tasks"]
        .as_u64()
        .is_some_and(|value| value <= 4));
    assert!(
        args["input"]["local_max_steps"]
            .as_u64()
            .is_some_and(|value| value <= 2),
        "a child task must never inherit the old 80-240 turn budget"
    );
    assert_eq!(args["input"]["local_parallel_task_timeout_ms"], 120_000);
    assert_eq!(args["limits"]["timeoutMs"], 300_000);
    assert_eq!(
        deep_research_workflow_host_timeout_ms(&args),
        300_000 + DEEP_RESEARCH_WORKFLOW_HOST_GRACE_MS
    );
}

#[test]
fn deepresearch_cli_and_tui_share_authoritative_evidence_scope() {
    let local =
        deep_research_workflow_args("仅本地分析当前工作区，不要联网，并说明证据缺口", false);
    let web = deep_research_workflow_args("Compare current public documentation", false);

    assert_eq!(local["input"]["evidence_scope"], "local_only");
    assert_eq!(local["limits"]["timeoutMs"], 210_000);
    assert_eq!(web["input"]["evidence_scope"], "web_and_workspace");
    assert_eq!(web["limits"]["timeoutMs"], 300_000);
    assert_eq!(local["source"], web["source"]);
}
