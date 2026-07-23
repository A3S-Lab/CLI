#[test]
fn production_runtime_delegates_the_evidence_first_pipeline_to_one_engine_call() {
    const RUNTIME: &str = include_str!("../../inquiry_runtime.rs");
    let active_start = RUNTIME
        .find("async fn execute_evidence_first_research(")
        .expect("production evidence-first runtime");
    let active_end = RUNTIME[active_start..]
        .find("\n/// Spawn the complete evidence inquiry")
        .map(|offset| active_start + offset)
        .expect("end of production evidence-first runtime");
    let active_runtime = &RUNTIME[active_start..active_end];

    assert_eq!(active_runtime.matches("DeepResearchEngine::new(").count(), 1);
    assert_eq!(active_runtime.matches(".execute(args.clone())").count(), 1);
    assert!(!active_runtime.contains("run_bootstrap_acquisition_stage("));
    assert!(!active_runtime.contains("run_dynamic_workflow("));
    assert!(!active_runtime.contains("run_retrieval_stage("));
    for obsolete in [
        "resolve_questions_with_bounded_follow_up_waves",
        "perspective_research_plan",
        "follow_up_research_plan",
        "scout_plan",
    ] {
        assert!(!RUNTIME.contains(obsolete), "obsolete path: {obsolete}");
    }
}

#[test]
fn production_javascript_is_retrieval_only() {
    let source = super::super::deep_research_workflow_source();

    for required in [
        "web_search",
        "web_fetch",
        "semantic_chunk_ids",
        "select_evidence_chunks",
    ] {
        assert!(source.contains(required), "missing retrieval primitive: {required}");
    }
    for obsolete in [
        "checker",
        "maker",
        "execution_route",
        "research_method",
        "scout",
    ] {
        assert!(
            !source.to_ascii_lowercase().contains(obsolete),
            "JavaScript retained obsolete control plane: {obsolete}"
        );
    }
}
