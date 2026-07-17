#[test]
fn production_workflow_uses_an_llm_loop_contract_without_a_precomputed_rule_plan() {
    let args = super::deep_research_workflow_args_with_scope(
        "A semantically ambiguous question",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let contract = &args["input"]["loop_contract"];
    assert_eq!(contract["pattern"], "adaptive-deep-research");
    assert_eq!(contract["maker_role"], "evidence-researcher");
    assert_eq!(contract["checker_role"], "evidence-coverage-checker");
    assert_eq!(contract["planner"]["agent"], "loop-planner");
    assert_eq!(contract["planner"]["timeout_ms"], 120_000);
    assert_eq!(contract["checker"]["timeout_ms"], 180_000);
    assert!(contract["planner"]["output_schema"]["properties"]
        .get("intent_summary")
        .is_none());
    assert!(
        contract["planner"]["output_schema"]["properties"]["budget"]["properties"]
            .get("per_task_timeout_secs")
            .is_none()
    );
    let planner_properties = &contract["planner"]["output_schema"]["properties"];
    assert_eq!(
        planner_properties["research_method"]["enum"],
        serde_json::json!(["focused", "perspective_guided"])
    );
    assert_eq!(planner_properties["tracks"]["items"]["type"], "object");
    assert_eq!(
        planner_properties["tracks"]["items"]["additionalProperties"],
        false
    );
    let track_properties = &planner_properties["tracks"]["items"]["properties"];
    for field in [
        "id",
        "title",
        "focus",
        "perspective",
        "material",
        "questions",
        "completion_criteria",
        "evidence_requirements",
    ] {
        assert!(track_properties[field].is_object(), "missing {field}");
    }
    assert_eq!(track_properties["questions"]["minItems"], 1);
    assert_eq!(track_properties["questions"]["maxItems"], 3);
    assert_eq!(
        track_properties["id"]["pattern"],
        "^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$"
    );
    assert_eq!(track_properties["completion_criteria"]["minItems"], 1);
    assert_eq!(track_properties["completion_criteria"]["maxItems"], 3);
    assert!(track_properties["perspective"].get("minLength").is_none());
    assert_eq!(track_properties["material"]["type"], "boolean");
    assert_eq!(
        track_properties["evidence_requirements"]["additionalProperties"],
        false
    );
    assert_eq!(
        track_properties["evidence_requirements"]["properties"]["primary_source_required"]["type"],
        "boolean"
    );
    assert_eq!(
        track_properties["evidence_requirements"]["properties"]
            ["independent_corroboration_required"]["type"],
        "boolean"
    );
    let track_required = planner_properties["tracks"]["items"]["required"]
        .as_array()
        .expect("planner track required fields");
    for field in [
        "id",
        "title",
        "focus",
        "perspective",
        "material",
        "questions",
        "completion_criteria",
        "evidence_requirements",
    ] {
        assert!(
            track_required.contains(&serde_json::json!(field)),
            "{field} must be required"
        );
    }
    assert!(contract["planner"]["output_schema"]["properties"]
        .get("plan_rationale")
        .is_none());
    assert_eq!(
        contract["planner"]["output_schema"]["properties"]["tracks"]["maxItems"],
        4
    );
    assert_eq!(planner_properties["scout_queries"]["maxItems"], 4);
    assert!(contract["planner"]["output_schema"]["required"]
        .as_array()
        .is_some_and(|required| {
            required.contains(&serde_json::json!("research_method"))
                && required.contains(&serde_json::json!("scout_queries"))
        }));
    assert_eq!(
        contract["planner"]["output_schema"]["properties"]["budget"]["properties"]
            ["max_steps_per_task"]["maximum"],
        2
    );
    assert_eq!(contract["checker"]["agent"], "loop-checker");
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt
            .contains("Never infer stages, depth, route, or budget from keyword counts")));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains(
            "Choose research_method from the meaning of this request, never from keywords, named entities, or a task template"
        )));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains(
            "perspective_guided must scout first and derive its perspectives from the retrieved material"
        )));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains(
            "focused skips the scout-and-perspective expansion and must return an empty scout_queries array"
        )));
    assert!(contract["planner"]["prompt"].as_str().is_some_and(
        |prompt| prompt.contains("it must return one to four broad but targeted scout_queries")
    ));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains(
            "The initial tracks are stable research obligations and initial questions, not final perspectives"
        )));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains(
            "never claim that this initial plan has already discovered the final viewpoints"
        )));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains(
            "Set material=true only when failure to resolve that track would prevent a defensible answer"
        )));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains("Neither requirement is a global default")));
    assert!(contract["planner"]["output_schema"]["properties"]["phases"].is_object());
    assert_eq!(
        contract["planner"]["output_schema"]["properties"]["phases"]["items"]["type"],
        "string"
    );
    assert_eq!(
        contract["planner"]["output_schema"]["properties"]["execution_route"]["enum"],
        serde_json::json!([
            "direct_only",
            "direct_then_review",
            "direct_then_maker",
            "maker_first"
        ])
    );
    assert!(contract["planner"]["output_schema"]["properties"]
        .get("strategy")
        .is_none());
    assert_eq!(
        contract["planner"]["output_schema"]["properties"]["report_title"]["type"],
        "string"
    );
    assert_eq!(
        contract["planner"]["output_schema"]["properties"]["workspace_evidence_required"]["type"],
        "boolean"
    );
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains(
            "explicitly asks about this repository, a local codebase, or attached/local artifacts"
        )));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains("direct_then_review")));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| {
            prompt.contains("Ordinary public-web research defaults to direct_then_review")
        }));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| {
            prompt.contains(
                "Reserve maker routes for genuinely adaptive or parallel evidence production",
            )
        }));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| !prompt.contains("normally needs direct_then_maker")));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains("same language as the query")));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains("primary or authoritative evidence")));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains("every explicitly requested alternative")));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt.contains("directly fetchable")));
    assert!(contract["planner"]["prompt"]
        .as_str()
        .is_some_and(|prompt| prompt
            .contains("total number of evidence passes including initial collection")));
    assert!(contract["planner"]["output_schema"]["properties"]["search_queries"].is_object());
    assert_eq!(
        contract["planner"]["output_schema"]["properties"]["search_queries"]["maxItems"],
        4
    );
    assert_eq!(
        contract["planner"]["output_schema"]["properties"]["budget"]["properties"]
            ["direct_fetches"]["maximum"],
        8
    );
    assert!(contract["planner"]["output_schema"]["properties"]
        .get("source_targets")
        .is_none());
    assert!(contract["planner"]["output_schema"]["properties"]["seed_urls"].is_object());
    assert!(contract["planner"]["output_schema"]["properties"]["budget"].is_object());
    assert_eq!(
        contract["planner"]["output_schema"]["properties"]["budget"]["properties"]
            ["synthesis_timeout_secs"]["minimum"],
        120
    );
    assert_eq!(
        contract["planner"]["output_schema"]["properties"]["budget"]["properties"]
            ["synthesis_timeout_secs"]["maximum"],
        180
    );
    assert_eq!(contract["hard_caps"]["synthesis_timeout_ms"], 180_000);
    assert_eq!(
        contract["checker"]["output_schema"]["properties"]["next_action"]["enum"],
        serde_json::json!(["none", "direct_retrieval", "maker"])
    );
    assert!(contract["checker"]["output_schema"]["properties"]["report_summary"].is_object());
    assert!(contract["checker"]["output_schema"]["properties"]["verified_findings"].is_object());
    assert!(contract["checker"]["output_schema"]["properties"]["track_assessments"].is_object());
    assert!(
        contract["checker"]["output_schema"]["properties"]["stop_condition_assessments"]
            .is_object()
    );
    assert_eq!(
        contract["checker"]["output_schema"]["properties"]["track_assessments"]["items"]
            ["properties"]["status"]["enum"],
        serde_json::json!(["supported", "bounded", "uncovered"])
    );
    assert!(contract["checker"]["output_schema"]["required"]
        .as_array()
        .is_some_and(
            |required| required.contains(&serde_json::json!("track_assessments"))
                && required.contains(&serde_json::json!("stop_condition_assessments"))
        ));
    assert!(args["input"].get("research_plan").is_none());
    assert!(args["source"].as_str().is_some_and(|source| {
        source.contains("engineered_loop_enabled: engineeredLoopEnabled")
            && source.contains("return await collectDirectWebResearch()")
            && source.contains("const localMinSuccessCount")
            && source.contains("Authoritative scope: web_only")
            && source.contains("minItems: 1")
            && source.contains("const plannerWorkflowRetry")
            && source.contains("max_attempts: 1")
            && source.contains("const checkerWorkflowRetry = {max_attempts: 1")
            && source.contains("step_name: \"generate_object\"")
            && source.contains("schema_name: \"deep_research_plan\"")
            && source.contains("schema_name: \"deep_research_check\"")
            && source.contains("normalizePlannerBudget")
            && source.contains("ctx.tool(\"batch\"")
            && source.contains("batchOutputSections")
            && source.contains("tool: \"web_search\"")
            && source.contains("tool: \"web_fetch\"")
            && source.contains("directIteration === 0")
            && source.contains("excluded_urls")
            && source.contains("retrieval_elapsed_ms")
            && source.contains("retrievalBudgetUsedMs")
            && source.contains("plannerObservedLatencyMs")
            && source.contains("observedCheckerLatencyMs")
            && !source.contains("plannedSynthesisTimeoutMs")
            && source.contains("plannerStructuredMode")
            && source.contains("packMakerTracks")
            && source.contains("checkerReserveMs")
            && source.contains("promptMakerReserveMs")
            && source.contains("makerAndCheckerFloorMs")
            && source.contains("step_elapsed_ms")
            && source.contains("plannedSeedEvidenceContext")
            && source.contains("researchPlan.execution_route === \"direct_then_maker\"")
            && source.contains("return the existing source-backed evidence without a tool call")
            && source.contains("hasReusableEvidencePackage")
            && source.contains("Public web gaps use direct_retrieval")
            && source.contains("Findings state facts")
            && source.contains("A URL, title, or search snippet alone is a discovery lead")
            && source.contains("discovery_leads_not_evidence")
            && source.contains("raw.githubusercontent.com")
            && source.contains("raw.githubusercontent.com/wiki")
            && source.contains("directIteration + 1 < maxResearchRounds")
            && source.contains("exact supporting source URL")
            && source.contains("validateCheckerDecision")
            && source.contains("finalize_gate_passed")
            && !source.contains("researchPlan.answer_shape ===")
    }));
    assert_eq!(
        contract["checker"]["output_schema"]["properties"]["report_summary"]["maxLength"],
        4800
    );
}
#[test]
fn llm_plan_controls_synthesis_timeout_and_visible_status() {
    let output = serde_json::json!({
        "plan": {
            "answer_shape": "briefing",
            "budget": {
                "synthesis_timeout_ms": 42000,
                "max_iterations": 2,
                "max_parallel_tasks": 3,
                "retrieval_timeout_ms": 75000
            }
        }
    })
    .to_string();
    assert_eq!(
        super::deep_research_planned_synthesis_timeout_ms(Some(&output)),
        Some(42_000)
    );
    let status = super::deep_research_plan_status(&output).unwrap();
    assert!(status.contains("briefing"), "{status}");
    assert!(status.contains("≤2 iterations"), "{status}");
    assert!(status.contains("75s retrieval"), "{status}");

    for (planned, expected) in [(5_000, 10_000), (180_000, 180_000), (250_000, 180_000)] {
        let output = serde_json::json!({
            "plan": { "budget": { "synthesis_timeout_ms": planned } }
        })
        .to_string();
        assert_eq!(
            super::deep_research_planned_synthesis_timeout_ms(Some(&output)),
            Some(expected),
            "the host must preserve the planner clock up to the shared synthesis ceiling"
        );
    }
}
