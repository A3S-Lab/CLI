#[test]
fn production_contract_has_one_optional_outline_and_host_owned_retrieval() {
    let args = super::deep_research_workflow_args_with_scope(
        "跨语言核实一个公开结论",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let contract = &args["input"]["loop_contract"];

    assert!(args["input"].get("semantic_plan_contract").is_none());
    assert_eq!(contract["version"], 1);
    assert_eq!(contract["pattern"], "evidence-first-deep-research");
    assert_eq!(contract["goal"], "跨语言核实一个公开结论");
    assert_eq!(contract["controller"], "host_inquiry_reducer");
    assert_eq!(contract["quota"]["mode"], "bounded");
    assert_eq!(contract["execution"]["mode"], "progressively_publishable");
    assert_eq!(
        contract["execution"]["stages"],
        serde_json::json!([
            "bootstrap_acquisition",
            "optional_outline",
            "batched_evidence_extraction",
            "host_coverage_reduction",
            "optional_gap_acquisition",
            "optional_gap_extraction",
            "report_document_generation",
            "deterministic_publication"
        ])
    );
    for field in [
        "outline_generations",
        "initial_extractions",
        "gap_extractions",
        "report_generations",
        "report_repairs",
    ] {
        assert_eq!(contract["cardinality"][field], 1, "{field}");
    }
    assert_eq!(contract["planner"]["agent"], "research-planner");
    assert_eq!(contract["planner"]["max_steps"], 1);
    assert_eq!(contract["planner"]["timeout_ms"], 90_000);
    assert!(contract["planner"].get("outline_prompt").is_none());
    assert!(contract["planner"].get("track_prompt").is_none());
    assert!(contract["planner"].get("retrieval_prompt").is_none());
    assert_eq!(contract["hard_caps"]["max_searches"], 4);
    assert_eq!(contract["hard_caps"]["max_fetches"], 8);
    assert_eq!(contract["hard_caps"]["max_supplemental_fetches"], 2);
    for obsolete in [
        "checker",
        "maker",
        "scout",
        "perspective",
        "adaptive_route",
        "adaptive_routes",
    ] {
        assert!(contract.get(obsolete).is_none(), "{obsolete}");
    }
    assert!(args["input"].get("research_plan").is_none());

    let properties = &contract["planner"]["output_schema"]["properties"];
    let expected = [
        "report_title",
        "research_scope",
        "freshness_required",
        "supplemental_queries",
        "workspace_evidence_required",
        "tracks",
    ];
    let actual = properties
        .as_object()
        .expect("planner properties")
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, expected.into_iter().collect());
    assert_eq!(
        properties["tracks"]["items"]["properties"]
            .as_object()
            .expect("outline track properties")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "completion_criteria",
            "evidence_requirements",
            "focus",
            "id",
            "material",
            "title",
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn optional_outline_prompt_is_language_agnostic_and_host_closes_the_contract() {
    let args = super::deep_research_workflow_args_with_scope(
        "¿Qué demuestra la evidencia?",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let prompt = args["input"]["loop_contract"]["planner"]["prompt"]
        .as_str()
        .expect("optional outline prompt");
    assert!(prompt.chars().count() < 3_000);

    assert!(prompt.contains("Do not use fixed topic taxonomies"));
    assert!(prompt.contains("Use the query language"));
    assert!(prompt.contains("always searches the exact user query first"));
    assert!(prompt.contains("one to four coherent evidence tracks"));
    assert!(prompt.contains("observable completion criteria"));
    assert!(prompt.contains("zero to three supplemental_queries"));
    assert!(prompt.contains("not a URL"));
    assert!(prompt.contains("Do not return URLs"));
    for obsolete in [
        "research_method",
        "execution_route",
        "scout_queries",
        "checker",
        "maker",
    ] {
        assert!(
            !prompt.contains(obsolete),
            "obsolete planner field: {obsolete}"
        );
    }
}

#[test]
fn tui_uses_the_standalone_retrieval_workflow_without_a_local_fork() {
    assert_eq!(
        super::deep_research_workflow_source(),
        a3s_deep_research::workflow::retrieval_workflow_source()
    );
}

#[test]
fn retrieval_fragments_remain_small_and_reassemble_as_valid_source() {
    let foundation = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../deep-research/src/workflow/retrieval_foundation.js"
    ));
    let web = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../deep-research/src/workflow/retrieval_web.js"
    ));
    let selection = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../deep-research/src/workflow/retrieval_selection.js"
    ));
    let reduction = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../deep-research/src/workflow/retrieval_reduction.js"
    ));
    let materialization = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../deep-research/src/workflow/retrieval_materialization.js"
    ));
    let loop_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../deep-research/src/workflow/retrieval_loop.js"
    ));
    let local = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../deep-research/src/workflow/retrieval_local.js"
    ));
    let local_collection = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../deep-research/src/workflow/retrieval_local_collection.js"
    ));
    let execution = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../deep-research/src/workflow/retrieval_execution.js"
    ));

    assert!(foundation.lines().count() < 1_000);
    assert!(web.lines().count() < 1_000);
    assert!(selection.lines().count() < 1_000);
    assert!(reduction.lines().count() < 1_000);
    assert!(materialization.lines().count() < 1_000);
    assert!(loop_source.lines().count() < 1_000);
    assert!(local.lines().count() < 1_000);
    assert!(local_collection.lines().count() < 1_000);
    assert!(execution.lines().count() < 1_000);
    assert!(local_collection.contains("const collectLocal"));
    assert!(reduction.contains("const reducedSelectorPacket"));
    assert!(materialization.contains("const materializeEvidence"));
    for misplaced_responsibility in [
        "const collectLocal",
        "const reducedSelectorPacket",
        "const materializeEvidence",
    ] {
        assert!(
            !execution.contains(misplaced_responsibility),
            "execution scheduling must not absorb {misplaced_responsibility}"
        );
    }
    let combined = format!(
        "{foundation}{web}{selection}{reduction}{materialization}{loop_source}{local}{local_collection}{execution}"
    );
    assert!(combined.starts_with("async function run(ctx, inputs)"));
    assert!(combined.trim_end().ends_with('}'));
    assert_eq!(
        combined.matches("async function run(ctx, inputs)").count(),
        1
    );
}
