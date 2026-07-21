#[test]
fn production_contract_is_one_plan_with_one_bounded_coverage_supplement() {
    let args = super::deep_research_workflow_args_with_scope(
        "跨语言核实一个公开结论",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let contract = &args["input"]["loop_contract"];

    assert!(args["input"].get("semantic_plan_contract").is_none());
    assert_eq!(contract["version"], 1);
    assert_eq!(contract["pattern"], "minimal-deep-research");
    assert_eq!(contract["goal"], "跨语言核实一个公开结论");
    assert_eq!(contract["controller"], "host_inquiry_reducer");
    assert_eq!(contract["quota"]["mode"], "unlimited");
    assert_eq!(contract["execution"]["mode"], "coverage_driven");
    assert_eq!(
        contract["execution"]["stages"],
        serde_json::json!([
            "semantic_plan",
            "initial_retrieval",
            "semantic_chunk_selection",
            "typed_coverage_evaluation",
            "optional_supplemental_retrieval",
            "final_closed_question_review",
            "host_contract_reduction",
            "sectioned_report_transaction"
        ])
    );
    for field in [
        "semantic_iterations",
        "retrieval_passes",
        "semantic_selections",
        "section_revision_rounds",
    ] {
        assert_eq!(contract["cardinality"][field], 2, "{field}");
    }
    for field in [
        "question_reviews",
        "contract_assessments",
        "report_transactions",
    ] {
        assert_eq!(contract["cardinality"][field], 1, "{field}");
    }
    assert_eq!(contract["planner"]["agent"], "research-planner");
    assert_eq!(contract["planner"]["max_steps"], 1);
    assert_eq!(contract["planner"]["timeout_ms"], 90_000);
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
        "freshness_required",
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
    assert!(properties.get("budget").is_none());
    assert!(properties.get("search_queries").is_none());
    assert!(properties.get("stop_conditions").is_none());
}

#[test]
fn outline_prompt_is_language_agnostic_and_provider_query_is_host_owned() {
    let args = super::deep_research_workflow_args_with_scope(
        "¿Qué demuestra la evidencia?",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let outline_prompt = args["input"]["loop_contract"]["planner"]["prompt"]
        .as_str()
        .expect("outline planner prompt");
    assert!(outline_prompt.chars().count() < 3_000);

    assert!(outline_prompt.contains("Do not route by keywords"));
    assert!(outline_prompt.contains("Use the query language"));
    assert!(outline_prompt.contains("original provider query"));
    assert!(outline_prompt.contains("one material track for each coherent evidence family"));
    assert!(outline_prompt.contains("absence can be disclosed"));
    assert!(outline_prompt.contains("Do not return focus, questions"));
    assert!(outline_prompt.contains("universal stop conditions"));
    for obsolete in [
        "research_method",
        "execution_route",
        "scout_queries",
        "checker",
        "maker",
    ] {
        assert!(
            !outline_prompt.contains(obsolete),
            "obsolete planner field: {obsolete}"
        );
    }
}

#[test]
fn production_javascript_contains_retrieval_and_no_control_plane() {
    let source = super::deep_research_workflow_source();
    let lower = source.to_ascii_lowercase();

    for required in [
        "web_search",
        "web_fetch",
        "select_web_sources",
        "select_evidence_chunks",
        "checkpoint_initial_retrieval",
        "deep_research_web_source_selection",
        "semantic_chunk_ids",
        "chunk_ids",
        "source_coverage",
        "source_relevance",
        "completion_criterion_indexes",
        "validatedsourcecoverage",
        "validatedsourcerelevance",
        "initial_attempts",
        "fetch_failed transport surface",
        "judge meaning across languages",
        "model_generation_active_timeout_ms",
        "document_range_count",
        "exact workspace paths observed with read or grep",
    ] {
        assert!(
            lower.contains(required),
            "missing retrieval behavior: {required}"
        );
    }
    assert!(source.contains(r#"["anysearch", "tavily", "ddg"]"#));
    assert!(!source.contains(r#"["ddg", "wiki"]"#));
    assert!(source.contains("MODEL_GENERATION_ACTIVE_TIMEOUT_MS = 300_000"));
    for obsolete in [
        "checker",
        "maker",
        "execution_route",
        "research_method",
        "scout",
        "queryterms",
        "querytermmatches",
        "sourcerelevancescore",
    ] {
        assert!(
            !lower.contains(obsolete),
            "retrieval source retained obsolete control or lexical logic: {obsolete}"
        );
    }
    assert!(
        source.contains("supporting: { type: \"boolean\", enum: [true] }"),
        "semantic selection must encode mandatory support in its schema"
    );
    assert!(
        source.contains("required: [\"supporting\", \"primary\", \"independent\"]"),
        "semantic selection must return a complete typed source-role object"
    );
    assert!(
        source.contains("otherwise primary must be false")
            && source.contains("otherwise independent must be false"),
        "semantic selection instructions must agree with the closed role schema"
    );
    assert!(
        source.contains(
            "selected fetched source text itself directly resolves every material element"
        ) && source.contains("partial answer is not criterion coverage"),
        "typed coverage must remain conservative enough to trigger the bounded supplemental pass"
    );
    assert!(
        source.contains(
            "Chunk retention, partial obligation relevance, and full criterion coverage are separate decisions"
        )
            && source.contains("must not by itself make chunk_ids empty"),
        "partial source text must remain reviewable without manufacturing a full criterion edge"
    );
    assert!(
        source.contains("resilient alternatives when a fetch failure")
            && source.contains("do not minimize the set below the declared evidence needs"),
        "semantic source admission must preserve coverage under ordinary fetch failures"
    );
    assert!(
        !source.contains("date: item.date || undefined"),
        "unverified provider dates must not be promoted into closed evidence"
    );
    assert!(
        source.contains("releases.atom")
            && source.contains("atomFeedSegments")
            && source.contains("/<entry(?:\\s|>)/gi")
            && source.contains("output_truncated")
            && source.contains("invokeBatchWithOutputRecovery"),
        "bounded web retrieval must preserve complete Atom entries, avoid GitHub release chrome, and recover truncated batch children"
    );
    assert!(
        source.contains("sources: [source]")
            && !source.contains("const entries = packet.sources.flatMap"),
        "selector shards must stay source-local so one model call never arbitrates unrelated source contracts"
    );
    assert!(
        source.contains("MODEL_GENERATION_SHARD_ACTIVE_TIMEOUT_MS = 270_000")
            && source.contains("const semanticShardSelectionRetry")
            && source.contains("max_attempts: 1"),
        "source-local selectors need one long-tail attempt without starving the complete catalog"
    );
    for forbidden_lexical_role_rule in [
        "keywordmatch",
        "tokenoverlap",
        "stopwords",
        "detectlanguage",
        "url.includes(\"official\")",
        "title.includes(\"official\")",
    ] {
        assert!(
            !lower.contains(forbidden_lexical_role_rule),
            "typed source coverage must not use lexical inference: {forbidden_lexical_role_rule}"
        );
    }
    for hidden_admission_policy in ["evenlySpaced", "seenHosts"] {
        assert!(
            !source.contains(hidden_admission_policy),
            "retrieval must preserve provider order and either admit the complete catalog or fail closed: {hidden_admission_policy}"
        );
    }
    assert!(source.contains("closed catalog limit"));
    assert!(
        !source.contains("quote_or_fact: bounded(safe.quote_or_fact"),
        "local evidence text must be restored from the closed chunk catalog, not authored by a task"
    );
}

#[test]
fn retrieval_fragments_remain_small_and_reassemble_as_valid_source() {
    let foundation = include_str!("workflow/retrieval_foundation.js");
    let web = include_str!("workflow/retrieval_web.js");
    let selection = include_str!("workflow/retrieval_selection.js");
    let loop_source = include_str!("workflow/retrieval_loop.js");
    let local = include_str!("workflow/retrieval_local.js");
    let execution = include_str!("workflow/retrieval_execution.js");
    let dispatch = include_str!("workflow/retrieval_dispatch.js");

    assert!(foundation.lines().count() < 1_000);
    assert!(web.lines().count() < 1_000);
    assert!(selection.lines().count() < 1_000);
    assert!(loop_source.lines().count() < 1_000);
    assert!(local.lines().count() < 1_000);
    assert!(execution.lines().count() < 1_000);
    assert!(dispatch.lines().count() < 1_000);
    let combined = format!("{foundation}{web}{selection}{loop_source}{local}{execution}{dispatch}");
    assert!(combined.starts_with("async function run(ctx, inputs)"));
    assert!(combined.trim_end().ends_with('}'));
    assert_eq!(
        combined.matches("async function run(ctx, inputs)").count(),
        1
    );
}
