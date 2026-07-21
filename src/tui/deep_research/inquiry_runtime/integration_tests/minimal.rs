#[test]
fn production_runtime_has_exactly_one_retrieval_call_site() {
    const RUNTIME: &str = include_str!("../../inquiry_runtime.rs");

    assert_eq!(RUNTIME.matches("run_retrieval_stage(").count(), 1);
    assert!(!RUNTIME.contains("run_dynamic_workflow(session"));
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
