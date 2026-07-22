use std::time::{Duration, Instant};

use a3s::research::{
    InquiryEvent, InquiryLimits, InquiryState, Question, QuestionResolution,
    QuestionResolutionOutput, QuestionStatus, ResearchMethod,
};
use serde_json::Value;

use super::super::deep_research_evidence_ledger::{
    AcceptedClaim, AcceptedEvidence, AcceptedSource,
};
use super::execution::{
    isolated_question_resolution_events, isolated_wire_question_resolution_events,
    prepare_question_evidence_packet, question_group_evidence, question_review_groups,
};
use super::plan::{
    attach_bootstrap_acquisition, bootstrap_workflow_args, bound_questions,
    commit_plan_research_contract, host_fallback_plan, host_plan_from_outline,
    queue_plan_questions, validate_plan, validated_loop_planner, workflow_args_with_plan,
};

fn minimal_plan() -> Value {
    serde_json::json!({
        "report_title": "跨语言证据报告",
        "freshness_required": true,
        "workspace_evidence_required": false,
        "tracks": [{
            "id": "material.claim",
            "title": "核心结论",
            "focus": "核实问题所要求的核心结论",
            "material": true,
            "questions": ["¿Qué establece la fuente primaria?", "独立来源是否支持该结论？"],
            "completion_criteria": ["保留可追溯的直接证据，或明确记录证据缺口"],
            "evidence_requirements": {
                "primary_source_required": true,
                "independent_corroboration_required": true
            }
        }],
        "search_queries": [
            "fuente primaria informe oficial",
            "独立来源 结论 证据"
        ],
        "seed_urls": [],
        "budget": {
            "retrieval_timeout_secs": 90,
            "direct_searches": 2,
            "direct_fetches": 4
        },
        "stop_conditions": ["核心结论具有可追溯证据，或其限制已被明确界定"]
    })
}

fn automatic_loop_workflow_args(query: &str) -> Value {
    serde_json::json!({
        "input": {
            "query": query,
            "current_date": "2026-07-19",
            "loop_contract": crate::tui::loop_engineering::deep_research_loop_contract(
                query,
                "2026-07-19",
                "web and workspace evidence",
                4,
            )
        }
    })
}

#[test]
fn host_fallback_contract_preserves_the_original_query_and_adds_one_authority_query() {
    let query = "截至 2026 年核实一个 planner 失败后仍必须检索的公开结论";
    let mut args = automatic_loop_workflow_args(query);
    args["input"]["evidence_scope"] = serde_json::json!("web_and_workspace");

    let fallback = host_fallback_plan(&args).expect("host fallback plan");

    assert_eq!(fallback.value["search_queries"][0], query);
    assert_eq!(
        fallback.value["search_queries"][1],
        format!("{query} 2026年7月19日 最新进展 最终结果 新闻")
    );
    assert_eq!(
        fallback.value["search_queries"].as_array().unwrap().len(),
        2
    );
    assert_eq!(fallback.value["budget"]["direct_searches"], 2);
    assert_eq!(fallback.value["budget"]["direct_fetches"], 8);
    assert_eq!(fallback.value["budget"]["retrieval_timeout_ms"], 150_000);
    assert_eq!(fallback.value["tracks"][0]["id"], "request.primary");
    assert_eq!(fallback.value["tracks"][0]["material"], true);
    assert_eq!(
        fallback.value["stop_conditions"],
        serde_json::json!(["Material evidence is retained or the request is explicitly bounded."])
    );
}

#[test]
fn authority_companion_uses_the_current_date_and_a_query_script_hint() {
    let query = "Verify the current World Cup result";
    let mut args = automatic_loop_workflow_args(query);
    args["input"]["evidence_scope"] = serde_json::json!("web_and_workspace");

    let fallback = host_fallback_plan(&args).expect("host fallback plan");

    assert_eq!(fallback.value["search_queries"][0], query);
    assert_eq!(
        fallback.value["search_queries"][1],
        "Verify the current World Cup result 2026-07-19 latest development final outcome news"
    );
    assert_eq!(
        fallback.value["search_queries"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn bootstrap_args_are_transport_only_and_reusable_by_final_retrieval() {
    let query = "Acquire raw evidence before planning";
    let mut args = automatic_loop_workflow_args(query);
    args["input"]["evidence_scope"] = serde_json::json!("web_and_workspace");
    args["limits"] = serde_json::json!({
        "timeoutMs": 90_000,
        "maxToolCalls": 32,
        "maxOutputBytes": 2 * 1024 * 1024
    });
    let bootstrap =
        bootstrap_workflow_args(args.clone(), "run-bootstrap").expect("bootstrap workflow args");
    assert_eq!(bootstrap["run_id"], "run-bootstrap");
    assert_eq!(
        bootstrap["input"]["execution_mode"],
        "bootstrap_acquisition"
    );
    assert_eq!(
        bootstrap["input"]["research_plan"]["search_queries"][0],
        query
    );

    let plan = host_fallback_plan(&args).expect("fallback plan");
    let mut final_args =
        workflow_args_with_plan(args, plan.value, Some("run-final")).expect("final workflow args");
    attach_bootstrap_acquisition(
        &mut final_args,
        serde_json::json!({
            "status": "success",
            "packet": {
                "version": 1,
                "focuses": [],
                "sources": [{
                    "source_id": "bootstrap-web-source-1",
                    "title": "Source",
                    "url_or_path": "https://example.test/source",
                    "reliability": "fetched",
                    "chunks": [{
                        "chunk_id": "bootstrap-web-source-1:chunk:1",
                        "text": "traceable source text"
                    }]
                }]
            },
            "errors": [],
            "metadata": {}
        }),
    )
    .expect("attach reusable bootstrap packet");
    assert_eq!(
        final_args["input"]["bootstrap_acquisition"]["packet"]["sources"][0]["source_id"],
        "bootstrap-web-source-1"
    );
}

fn planned_state(plan: &Value) -> (InquiryState, Vec<InquiryEvent>, InquiryLimits) {
    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut events = Vec::new();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        &limits,
    )
    .expect("focused strategy");
    commit_plan_research_contract(plan, &mut state, &mut events, &limits)
        .expect("research contract");
    queue_plan_questions(plan, &mut state, &mut events, &limits).expect("planned questions");
    (state, events, limits)
}

#[test]
fn automatic_loop_contract_is_bounded_and_progressively_publishable() {
    let args = automatic_loop_workflow_args("跨语言核实公开结论");
    let planner = validated_loop_planner(&args).expect("valid automatic loop contract");
    let contract = &args["input"]["loop_contract"];

    assert!(planner["output_schema"].is_object());
    assert_eq!(
        planner["output_schema"]["properties"]
            .as_object()
            .expect("outline properties")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "report_title",
            "freshness_required",
            "workspace_evidence_required",
            "tracks"
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(planner["max_steps"], 1);
    assert_eq!(planner["timeout_ms"], 90_000);
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
}

#[test]
fn one_optional_outline_becomes_a_complete_host_owned_plan() {
    let query = "跨语言核实公开结论";
    let mut args = automatic_loop_workflow_args(query);
    args["input"]["evidence_scope"] = serde_json::json!("web_and_workspace");
    let outline = serde_json::json!({
        "report_title": "公开结论核实",
        "freshness_required": true,
        "workspace_evidence_required": true,
        "tracks": [{
            "id": "publisher.primary",
            "title": "发布方原始记录",
            "material": true
        }, {
            "id": "independent.context",
            "title": "独立背景材料",
            "material": false
        }]
    });

    let plan = host_plan_from_outline(&args, outline).expect("Host-completed outline plan");

    assert_eq!(plan.value["search_queries"][0], query);
    assert_eq!(
        plan.value["search_queries"][1],
        format!("{query} 2026年7月19日 最新进展 最终结果 新闻")
    );
    assert_eq!(plan.value["seed_urls"], serde_json::json!([]));
    assert_eq!(plan.value["budget"]["direct_searches"], 2);
    assert_eq!(plan.value["budget"]["direct_fetches"], 8);
    assert_eq!(plan.value["budget"]["retrieval_timeout_ms"], 150_000);
    assert_eq!(plan.value["workspace_evidence_required"], true);
    assert_eq!(plan.value["tracks"][0]["focus"], "发布方原始记录");
    assert_eq!(
        plan.value["tracks"][0]["questions"],
        serde_json::json!(["发布方原始记录"])
    );
    assert_eq!(
        plan.value["tracks"][0]["completion_criteria"],
        serde_json::json!(["发布方原始记录"])
    );
    assert_eq!(
        plan.value["tracks"][0]["evidence_requirements"],
        serde_json::json!({
            "primary_source_required": false,
            "independent_corroboration_required": false
        })
    );
    assert_eq!(
        plan.value["stop_conditions"],
        serde_json::json!([
            "Every material evidence target is resolved from traceable evidence or explicitly bounded.",
            "Any remaining limitation is disclosed and cannot make the qualified answer misleading."
        ])
    );
}

#[test]
fn local_only_host_fallback_reserves_no_web_transport() {
    let mut args = automatic_loop_workflow_args("inspect this workspace");
    args["input"]["evidence_scope"] = serde_json::json!("local_only");

    let fallback = host_fallback_plan(&args).expect("local-only Host fallback");

    assert_eq!(fallback.value["search_queries"], serde_json::json!([]));
    assert_eq!(fallback.value["budget"]["direct_searches"], 0);
    assert_eq!(fallback.value["budget"]["direct_fetches"], 0);
    assert_eq!(fallback.value["workspace_evidence_required"], true);
}

#[test]
fn mutated_loop_identity_quota_graph_or_cardinality_fails_closed() {
    let mutations = [
        (
            "/input/loop_contract/goal",
            serde_json::json!("different goal"),
            "goal differs",
        ),
        (
            "/input/loop_contract/quota/mode",
            serde_json::json!("metered"),
            "must be `bounded`",
        ),
        (
            "/input/loop_contract/execution/stages",
            serde_json::json!([
                "bootstrap_acquisition",
                "optional_outline",
                "batched_evidence_extraction",
                "deterministic_publication"
            ]),
            "stage graph differs",
        ),
        (
            "/input/loop_contract/cardinality/initial_extractions",
            serde_json::json!(3),
            "must be exactly 1",
        ),
    ];

    for (pointer, replacement, expected) in mutations {
        let mut args = automatic_loop_workflow_args("跨语言核实公开结论");
        *args.pointer_mut(pointer).expect("mutation target") = replacement;
        let error = validated_loop_planner(&args)
            .expect_err("mutated automatic Loop Engineering contract must fail closed");
        assert!(error.contains(expected), "{pointer}: {error}");
    }
}

#[test]
fn hidden_loop_control_or_relaxed_safety_fuse_fails_closed() {
    let mut hidden_control = automatic_loop_workflow_args("核实结论");
    hidden_control["input"]["loop_contract"]["planner"]["checker"] = serde_json::json!("legacy");
    let error =
        validated_loop_planner(&hidden_control).expect_err("hidden checker must fail closed");
    assert!(error.contains("checker"), "{error}");

    let mut relaxed_fuse = automatic_loop_workflow_args("核实结论");
    relaxed_fuse["input"]["loop_contract"]["hard_caps"]["max_searches"] = serde_json::json!(5);
    let error =
        validated_loop_planner(&relaxed_fuse).expect_err("relaxed safety fuse must fail closed");
    assert!(error.contains("max_searches"), "{error}");
}

#[test]
fn planner_contract_is_language_agnostic_and_normalizes_only_retrieval_time() {
    let planned = validate_plan(minimal_plan()).expect("minimal multilingual plan");

    assert_eq!(
        planned.value["budget"]["retrieval_timeout_ms"],
        serde_json::json!(90_000)
    );
    assert!(planned.value["budget"]
        .get("retrieval_timeout_secs")
        .is_none());
    assert!(planned.value.get("research_method").is_none());
    assert!(planned.value.get("execution_route").is_none());
    assert!(planned.value.get("scout_queries").is_none());
    assert!(planned.value.get("phases").is_none());
}

#[test]
fn planner_queries_preserve_case_punctuation_unicode_and_internal_spacing() {
    let query = "MiXeD Case?!  日本語／中文 — café №42";
    let mut plan = minimal_plan();
    plan["search_queries"] = serde_json::json!([query]);

    let planned = validate_plan(plan).expect("exact provider query");
    assert_eq!(planned.value["search_queries"][0], query);

    let (state, _, _) = planned_state(&planned.value);
    assert!(state
        .questions
        .iter()
        .all(|question| question.retrieval_query.is_none()));
}

#[test]
fn planner_queries_reject_blank_or_surrounding_whitespace() {
    for query in ["", "   ", " leading", "trailing ", "\twrapped\n"] {
        let mut plan = minimal_plan();
        plan["search_queries"] = serde_json::json!([query]);
        let error = validate_plan(plan).expect_err("invalid provider query must fail closed");
        assert!(
            error.contains("blank") || error.contains("surrounding whitespace"),
            "{query:?}: {error}"
        );
    }
}

#[test]
fn planner_allows_more_seed_candidates_than_direct_fetch_slots() {
    let mut plan = minimal_plan();
    plan["seed_urls"] = serde_json::json!([
        "https://official.example/one",
        "https://official.example/two"
    ]);
    plan["budget"]["direct_fetches"] = serde_json::json!(1);

    let planned =
        validate_plan(plan).expect("seed URLs are semantic candidates, not reserved slots");

    assert_eq!(planned.value["seed_urls"].as_array().unwrap().len(), 2);
    assert_eq!(planned.value["budget"]["direct_fetches"], 1);
}

#[test]
fn planner_rejects_more_than_two_questions_or_completion_criteria_per_track() {
    for field in ["questions", "completion_criteria"] {
        let mut plan = minimal_plan();
        plan["tracks"][0][field] = serde_json::json!(["one", "two", "three"]);
        let error = validate_plan(plan).expect_err("over-wide track must fail closed");
        assert!(error.contains(field), "{field}: {error}");
        assert!(error.contains("maximum is 2"), "{field}: {error}");
    }
}

#[test]
fn host_rejects_obsolete_or_unknown_planner_control_fields() {
    for field in [
        "research_method",
        "execution_route",
        "scout_queries",
        "checker",
        "maker",
    ] {
        let mut plan = minimal_plan();
        plan[field] = serde_json::json!("obsolete");
        let error = validate_plan(plan).expect_err("unknown control field must fail closed");
        assert!(error.contains(field), "{error}");
    }
}

#[test]
fn active_runtime_cannot_emit_legacy_research_control_events() {
    let active_sources = [
        include_str!("../../inquiry_runtime.rs"),
        include_str!("../plan.rs"),
        include_str!("../plan/planning.rs"),
        include_str!("../plan/bounding.rs"),
        include_str!("../execution.rs"),
        include_str!("../execution/resolution.rs"),
        include_str!("../execution/evidence.rs"),
        include_str!("../execution/tools.rs"),
        include_str!("../../host_workflow.rs"),
        include_str!("../../../app/research_workflow.rs"),
    ];
    for forbidden in [
        "ScoutCompleted",
        "PerspectiveBudgetSelected",
        "PerspectivesCommitted",
        "QuestionDeferred",
        "PerspectiveGuided",
        "Question::follow_up",
        "Perspective::new",
    ] {
        assert!(
            active_sources
                .iter()
                .all(|source| !source.contains(forbidden)),
            "active DeepResearch runtime references legacy control `{forbidden}`"
        );
    }

    let model = include_str!("../../../../research/model.rs");
    assert!(!model.contains("pub fn follow_up"));
    assert!(!model.contains("impl Perspective"));
}

#[test]
fn one_plan_queues_one_closed_question_set_with_typed_criterion_edges() {
    let plan = validate_plan(minimal_plan()).expect("minimal plan").value;
    let (state, _, _) = planned_state(&plan);

    assert_eq!(state.questions.len(), 2);
    assert!(state
        .questions
        .iter()
        .all(|question| question.status == QuestionStatus::Queued));
    assert!(state
        .questions
        .iter()
        .all(|question| question.retrieval_query.is_none()));
    assert!(state
        .questions
        .iter()
        .all(|question| question.completion_criterion_indexes == [0]));
    assert!(state.questions.iter().all(|question| question.round == 0));
    assert!(state
        .questions
        .iter()
        .all(|question| question.perspective_id.is_none()));
}

#[test]
fn host_attached_inquiry_authority_cannot_fall_back_to_a_legacy_checker() {
    let plan = validate_plan(minimal_plan()).expect("minimal plan").value;
    let (state, events, _) = planned_state(&plan);
    let result = a3s_code_core::ToolCallResult {
        name: "dynamic_workflow".to_string(),
        output: serde_json::json!({
            "query": "跨语言研究",
            "mode": "direct_web",
            "execution": {
                "terminal_authority": "legacy_checker"
            },
            "checker": {
                "decision": "finalize"
            }
        })
        .to_string(),
        exit_code: 0,
        metadata: None,
        error_kind: None,
    };

    let attached = super::execution::attach_inquiry_projection(result, &events, &state).unwrap();
    let value: Value = serde_json::from_str(&attached.output).unwrap();
    assert_eq!(
        value.pointer("/execution/terminal_authority"),
        Some(&serde_json::json!("host_inquiry_reducer"))
    );
    assert!(matches!(
        super::validated_inquiry_projection(&value),
        Ok(super::ValidatedInquiryProjection::Inquiry { .. })
    ));
}

#[test]
fn workflow_boundary_preserves_the_plan_and_removes_wall_clock_input() {
    let plan = minimal_plan();
    let loop_contract =
        automatic_loop_workflow_args("跨语言研究")["input"]["loop_contract"].clone();
    let args = serde_json::json!({
        "run_id": "original-run",
        "input": {
            "query": "跨语言研究",
            "run_started_at_ms": u64::MAX,
            "workflow_timeout_ms": 30_000,
            "loop_contract": loop_contract.clone()
        },
        "limits": { "timeoutMs": 30_000 }
    });

    let wave = workflow_args_with_plan(args, plan, Some("stable-run"))
        .expect("coverage-driven retrieval arguments");

    assert_eq!(wave["run_id"], "stable-run");
    assert_eq!(wave["input"]["execution_mode"], "collect_only");
    assert_eq!(wave["input"]["research_plan_fixture"], false);
    assert!(wave["input"].get("run_started_at_ms").is_none());
    assert_eq!(wave["input"]["loop_contract"], loop_contract);
    assert_eq!(
        wave["input"]["research_plan"]["search_queries"],
        minimal_plan()["search_queries"]
    );
    assert_eq!(
        wave["input"]["research_plan"]["budget"]["retrieval_timeout_ms"],
        90_000
    );
}

#[test]
fn closed_retrieval_failure_bounds_every_unanswered_question() {
    let plan = validate_plan(minimal_plan()).expect("minimal plan").value;
    let (mut state, mut events, limits) = planned_state(&plan);

    bound_questions(
        &mut state,
        &mut events,
        &limits,
        "the bounded retrieval contract retained no supporting evidence",
    )
    .expect("bounded questions");

    assert!(state
        .questions
        .iter()
        .all(|question| question.status == QuestionStatus::Bounded));
    assert!(!events
        .iter()
        .any(|event| matches!(event, InquiryEvent::QuestionDeferred { .. })));
}

#[test]
fn one_batched_extraction_leaves_contract_reduction_to_the_host() {
    const EXTRACTION: &str = include_str!("../execution/extraction.rs");
    const GENERATION: &str = include_str!("../../workflow/generation.js");

    assert_eq!(super::EVIDENCE_EXTRACTION_ATTEMPT_TIMEOUT_MS, 360_000);
    assert_eq!(super::EVIDENCE_EXTRACTION_STAGE_TIMEOUT_MS, 375_000);
    assert!(GENERATION.contains("inputs.input.max_attempts"));
    assert!(GENERATION.contains("retry: { max_attempts: maxAttempts"));
    assert!(GENERATION.contains("exitCode ?? result.exit_code"));
    assert!(GENERATION.contains("throw new Error(`Durable structured generation failed"));
    assert!(EXTRACTION.contains("run_batched_evidence_extraction"));
    assert!(EXTRACTION.contains("apply_batched_evidence_extraction"));
    assert!(EXTRACTION.contains("Target decoding and coverage reduction are Host-owned"));
    assert!(EXTRACTION.contains("execution_timeout_ms,\n        1,"));
}

#[test]
fn one_obligation_review_groups_its_linked_questions() {
    let plan = validate_plan(minimal_plan()).expect("minimal plan").value;
    let (state, _events, _limits) = planned_state(&plan);

    let groups = question_review_groups(&state.questions);

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].len(), 2);
    assert_eq!(groups[0][0].obligation_ids, groups[0][1].obligation_ids);
}

#[test]
fn obligation_review_packet_keeps_scoped_and_legacy_evidence_only() {
    let evidence = [
        ("evidence:one", vec!["obligation:one".to_string()]),
        ("evidence:two", vec!["obligation:two".to_string()]),
        ("evidence:legacy", Vec::new()),
    ]
    .into_iter()
    .map(|(id, relevant_obligation_ids)| AcceptedEvidence {
        id: id.to_string(),
        summary: "Scoped evidence".to_string(),
        confidence: None,
        sources: vec![AcceptedSource {
            id: format!("source:{id}"),
            anchor: format!("https://example.com/{id}"),
            title: None,
            date: None,
            reliability: None,
            quote_or_fact: None,
            evidence_excerpts: Vec::new(),
        }],
        claims: Vec::new(),
        source_coverage: Vec::new(),
        relevant_obligation_ids,
        contradictions: Vec::new(),
        gaps: Vec::new(),
    })
    .collect::<Vec<_>>();
    let mut question = Question::queued("question:one", None, "Resolve obligation one");
    question.obligation_ids = vec!["obligation:one".to_string()];

    let selected = question_group_evidence(&evidence, &[question]);

    assert_eq!(
        selected
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["evidence:one", "evidence:legacy"]
    );
}

#[test]
fn invalid_evidence_id_bounds_only_its_question_in_a_shared_review() {
    let mut first = Question::queued("question:first", None, "First question");
    first.obligation_ids = vec!["obligation:shared".to_string()];
    let mut second = Question::queued("question:second", None, "Second question");
    second.obligation_ids = vec!["obligation:shared".to_string()];
    let questions = vec![first, second];
    let output = QuestionResolutionOutput {
        resolutions: vec![
            QuestionResolution::Answered {
                question_id: "question:first".to_string(),
                answer: "Traceable answer".to_string(),
                evidence_ids: vec!["evidence:allowed".to_string()],
            },
            QuestionResolution::Answered {
                question_id: "question:second".to_string(),
                answer: "Invalid answer".to_string(),
                evidence_ids: vec!["evidence:outside".to_string()],
            },
        ],
    };

    let events = isolated_question_resolution_events(
        &output,
        &questions,
        &["evidence:allowed".to_string()].into_iter().collect(),
    );

    assert!(matches!(events[0], InquiryEvent::QuestionAnswered { .. }));
    assert!(matches!(events[1], InquiryEvent::QuestionBounded { .. }));
}

#[test]
fn invalid_wire_entry_preserves_its_valid_shared_review_sibling() {
    let mut first = Question::queued("question:first", None, "First question");
    first.obligation_ids = vec!["obligation:shared".to_string()];
    let mut second = Question::queued("question:second", None, "Second question");
    second.obligation_ids = vec!["obligation:shared".to_string()];
    let questions = vec![first, second];
    let wire = serde_json::json!({
        "resolutions": {
            "question:first": {
                "status": "answered",
                "content": "Traceable answer",
                "limitation": "",
                "evidence_refs": ["E1"]
            },
            "question:second": {
                "status": "bounded",
                "content": "Invalid bound",
                "limitation": "",
                "evidence_refs": ["E1"]
            }
        }
    });

    let events = isolated_wire_question_resolution_events(
        wire,
        &questions,
        &["evidence:allowed".to_string()].into_iter().collect(),
    );

    assert!(matches!(events[0], InquiryEvent::QuestionAnswered { .. }));
    assert!(matches!(events[1], InquiryEvent::QuestionBounded { .. }));
}

#[test]
fn shared_inquiry_deadline_reserves_review_then_releases_it_for_review() {
    let now = Instant::now();
    let deadline = super::InquiryDeadline::from_elapsed(now, 720_000, 180_000, 60_000, 400_000);

    assert_eq!(
        deadline.pre_review_stage_timeout_ms_at(now, 180_000),
        Some(80_000)
    );
    assert_eq!(
        deadline.question_review_stage_timeout_ms_at(now, 180_000),
        Some(180_000)
    );
    assert_eq!(
        deadline.pre_review_stage_timeout_ms_at(now + Duration::from_millis(79_001), 90_000),
        None
    );
}

#[test]
fn regressed_wall_clock_cannot_grant_a_fresh_inquiry_budget() {
    let now = Instant::now();
    let deadline =
        super::InquiryDeadline::from_wall_clock(50_000, 49_999, 720_000, 180_000, 60_000, now);

    assert_eq!(deadline.deadline, now);
    assert_eq!(deadline.pre_review_stage_timeout_ms_at(now, 180_000), None);
    assert_eq!(
        deadline.question_review_stage_timeout_ms_at(now, 180_000),
        None
    );
}

#[test]
fn inquiry_budget_keeps_the_full_closed_review_reserve_after_retrieval() {
    assert_eq!(super::PLANNER_OUTLINE_ATTEMPT_TIMEOUT_MS, 90_000);
    assert_eq!(super::PLANNER_GENERATION_MAX_ATTEMPTS, 1);
    assert_eq!(super::PLANNER_OUTLINE_WORKFLOW_TIMEOUT_MS, 105_000);
    assert_eq!(super::MAX_PLANNER_TRACK_EFFECTS, 4);
    assert_eq!(super::DEEP_RESEARCH_PLANNER_STAGE_TIMEOUT_MS, 105_000);
    assert_eq!(super::BOOTSTRAP_ACQUISITION_STAGE_TIMEOUT_MS, 150_000);
    assert_eq!(super::DEEP_RESEARCH_RETRIEVAL_STAGE_TIMEOUT_MS, 150_000);
    let accounted = super::DEEP_RESEARCH_PLANNER_STAGE_TIMEOUT_MS
        + super::DEEP_RESEARCH_RETRIEVAL_STAGE_TIMEOUT_MS
        + super::DEEP_RESEARCH_QUESTION_REVIEW_STAGE_TIMEOUT_MS
        + super::DEEP_RESEARCH_INQUIRY_FINALIZATION_RESERVE_MS;
    assert_eq!(accounted, super::DEEP_RESEARCH_INQUIRY_HOST_TIMEOUT_MS);

    let now = Instant::now();
    let elapsed = super::DEEP_RESEARCH_PLANNER_STAGE_TIMEOUT_MS
        + super::DEEP_RESEARCH_RETRIEVAL_STAGE_TIMEOUT_MS;
    let deadline = super::InquiryDeadline::from_elapsed(
        now,
        super::DEEP_RESEARCH_INQUIRY_HOST_TIMEOUT_MS,
        super::DEEP_RESEARCH_QUESTION_REVIEW_STAGE_TIMEOUT_MS,
        super::DEEP_RESEARCH_INQUIRY_FINALIZATION_RESERVE_MS,
        elapsed,
    );
    assert_eq!(
        deadline.question_review_stage_timeout_ms_at(
            now,
            super::DEEP_RESEARCH_QUESTION_REVIEW_STAGE_TIMEOUT_MS,
        ),
        Some(super::DEEP_RESEARCH_QUESTION_REVIEW_STAGE_TIMEOUT_MS)
    );
}

#[tokio::test]
async fn retrieval_stage_timeout_bounds_the_whole_future() {
    let started = Instant::now();
    let result = super::execution::within_inquiry_stage_timeout(
        std::future::pending::<Result<(), String>>(),
        20,
        "retrieval",
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        "DeepResearch retrieval stage timed out after 20 ms"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn review_stage_timeout_keeps_completed_siblings_and_drops_pending_work() {
    use futures::StreamExt;
    use std::future::Future;
    use std::pin::Pin;

    let futures: Vec<Pin<Box<dyn Future<Output = u8>>>> = vec![
        Box::pin(async { 1 }),
        Box::pin(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            2
        }),
    ];
    let stream = futures::stream::iter(futures).buffer_unordered(2);
    let (results, timed_out) = super::execution::collect_inquiry_stage_results(stream, 20).await;

    assert_eq!(results, [1]);
    assert!(timed_out);
}

#[test]
fn closed_question_packet_preserves_the_whole_final_evidence_set_or_fails_closed() {
    let evidence = (1..=3)
        .map(|index| AcceptedEvidence {
            id: format!("evidence:{index}"),
            summary: String::new(),
            confidence: Some("high".to_string()),
            sources: vec![AcceptedSource {
                id: format!("source:{index}"),
                anchor: format!("https://example.com/{index}"),
                title: None,
                date: None,
                reliability: None,
                quote_or_fact: Some(format!("Exact retained fact {index}")),
                evidence_excerpts: Vec::new(),
            }],
            claims: vec![AcceptedClaim {
                id: format!("claim:{index}"),
                text: format!("Supported claim {index}"),
            }],
            source_coverage: Vec::new(),
            relevant_obligation_ids: Vec::new(),
            contradictions: Vec::new(),
            gaps: Vec::new(),
        })
        .collect::<Vec<_>>();

    let packet = prepare_question_evidence_packet(&evidence, evidence.len(), 100_000).unwrap();
    assert_eq!(
        packet.allowed_evidence_ids,
        ["evidence:1", "evidence:2", "evidence:3"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
    let payload: Value = serde_json::from_str(&packet.payload).unwrap();
    assert_eq!(payload["evidence_items"].as_array().unwrap().len(), 3);
    assert_eq!(
        payload["evidence_items"][0]["claims"][0]["text"],
        "Supported claim 1"
    );
    assert_eq!(
        payload["evidence_items"][0]["sources"][0]["url_or_path"],
        "https://example.com/1"
    );
    assert!(payload["evidence_items"][0]["sources"][0]
        .get("additional_facts")
        .is_none());
    assert!(payload["evidence_items"][0]["sources"][0]
        .get("quote_or_fact")
        .is_none());
    assert!(payload.get("report_context").is_none());

    let item_error =
        prepare_question_evidence_packet(&evidence, evidence.len() - 1, 100_000).unwrap_err();
    assert!(item_error.contains("3 accepted evidence items"));
    assert!(item_error.contains("limit is 2"));

    let size_error = prepare_question_evidence_packet(&evidence, evidence.len(), 1).unwrap_err();
    assert!(size_error.contains("closed-evidence question packet"));
    assert!(size_error.contains("1-character"));
}
