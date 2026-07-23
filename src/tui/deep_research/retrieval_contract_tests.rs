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
    assert_eq!(
        properties["tracks"]["items"]["properties"]
            .as_object()
            .expect("outline track properties")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        ["id", "material", "title"].into_iter().collect()
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

    assert!(prompt.contains("Do not route by keywords"));
    assert!(prompt.contains("Use the query language"));
    assert!(prompt.contains("original provider query"));
    assert!(prompt.contains("no target-detail or retrieval-planner call follows"));
    assert!(prompt.contains("one material track for each coherent evidence family"));
    assert!(prompt.contains("absence can be disclosed"));
    assert!(prompt.contains("independently published projects"));
    assert!(prompt.contains("Do not return focus, questions"));
    assert!(prompt.contains("universal stop conditions"));
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
        "bounded_discovery_fallback",
        "exact workspace paths observed with read or grep",
    ] {
        assert!(
            lower.contains(required),
            "missing retrieval behavior: {required}"
        );
    }
    assert!(!source.contains(r#"["anysearch", "tavily", "ddg"]"#));
    assert!(!source.contains("WEB_SEARCH_ENGINES"));
    assert!(!source.contains(r#"["ddg", "wiki"]"#));
    assert!(!source.contains("provider_round_robin"));
    assert!(!source.contains("deterministicWebCandidates"));
    assert!(source.contains("boundedDiscoveryFallback"));
    assert!(source.contains(
        "continued with bounded cross-query discovery candidates for deterministic Host review"
    ));
    assert!(source.contains("fallbackCandidatePriority"));
    assert!(source.contains("protectedPublisherLookalike"));
    assert!(source
        .contains("Reject unrelated results even when fetch slots remain; return an empty list"));
    assert!(
        source.contains("Reject social or community posts, anonymous score trackers")
            && source
                .contains("Do not infer authority from a title or snippet claiming to be official"),
        "semantic source admission must distinguish topicality from source trust"
    );
    assert!(
        source.contains("publisher only provides storage")
            && source.contains("earlier-stage snapshot"),
        "source admission must reject self-publishing disclaimers and stale event phases"
    );
    assert!(
        source.contains("limit: 16"),
        "exact-query discovery must expose enough candidates for authority-aware admission"
    );
    assert!(source.contains("MODEL_GENERATION_ACTIVE_TIMEOUT_MS = 300_000"));
    assert!(source.contains("WEB_SOURCE_SELECTION_ACTIVE_TIMEOUT_MS = 60_000"));
    assert!(source.contains("timeout_ms: WEB_SOURCE_SELECTION_ACTIVE_TIMEOUT_MS"));
    assert!(source.contains("const webSourceSelectionRetry"));
    assert!(source.contains("const bootstrapWebSourceSelectionRetry"));
    assert!(source.contains("retry: bootstrapWebSourceSelectionRetry"));
    assert!(source.contains("semantic_web_selection_retry: webSourceSelectionRetry"));
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
    let web_source_quality = include_str!("workflow/retrieval_web_source_quality.js");
    let web = include_str!("workflow/retrieval_web.js");
    let selection = include_str!("workflow/retrieval_selection.js");
    let reduction = include_str!("workflow/retrieval_reduction.js");
    let materialization = include_str!("workflow/retrieval_materialization.js");
    let loop_source = include_str!("workflow/retrieval_loop.js");
    let local = include_str!("workflow/retrieval_local.js");
    let local_collection = include_str!("workflow/retrieval_local_collection.js");
    let execution = include_str!("workflow/retrieval_execution.js");

    assert!(foundation.lines().count() < 1_000);
    assert!(web_source_quality.lines().count() < 1_000);
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
        "{foundation}{web_source_quality}{web}{selection}{reduction}{materialization}{loop_source}{local}{local_collection}{execution}"
    );
    assert!(combined.starts_with("async function run(ctx, inputs)"));
    assert!(combined.trim_end().ends_with('}'));
    assert_eq!(
        combined.matches("async function run(ctx, inputs)").count(),
        1
    );
}
