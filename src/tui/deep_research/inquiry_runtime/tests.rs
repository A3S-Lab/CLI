use a3s::research::{InquiryEvent, InquiryLimits, InquiryState, PerspectiveDiscoveryOutput};
use serde_json::Value;

use super::plan::{
    defer_or_bound_question_batch, perspective_research_plan, questions_scheduled_for_retrieval,
    queue_plan_questions, scout_plan, single_wave_research_plan, validate_plan,
    workflow_args_with_plan,
};

fn production_planner_plan(method: &str, scouts: &[&str]) -> Value {
    serde_json::json!({
        "answer_shape": "investigation",
        "report_title": "A bounded investigation",
        "freshness_required": true,
        "workspace_evidence_required": false,
        "research_method": method,
        "execution_route": "direct_then_review",
        "phases": ["collect", "check"],
        "tracks": [{
            "id": "track:material",
            "title": "Material obligation",
            "focus": "Resolve the material evidence obligation",
            "perspective": "",
            "material": true,
            "questions": ["What does the evidence establish?"],
            "completion_criteria": ["A traceable answer or bounded gap"]
        }],
        "scout_queries": scouts,
        "search_queries": ["targeted evidence query"],
        "seed_urls": [],
        "budget": {
            "retrieval_timeout_secs": 90,
            "synthesis_timeout_secs": 180,
            "max_iterations": 2,
            "max_parallel_tasks": 2,
            "max_steps_per_task": 2,
            "direct_searches": 2,
            "direct_fetches": 4
        },
        "stop_conditions": ["The material obligation is resolved"]
    })
}

fn plan(method: &str, scouts: &[&str]) -> Value {
    validate_plan(production_planner_plan(method, scouts))
        .expect("production planner fixture")
        .value
}

#[test]
fn semantic_method_contract_is_enforced_without_keyword_rules() {
    assert!(validate_plan(production_planner_plan("focused", &[])).is_ok());
    assert!(validate_plan(production_planner_plan("focused", &["unexpected scout"])).is_err());
    assert!(validate_plan(production_planner_plan("perspective_guided", &[])).is_err());
    assert!(validate_plan(production_planner_plan(
        "perspective_guided",
        &["discover source landscape"]
    ))
    .is_ok());
}

#[test]
fn planner_must_select_at_least_one_material_track() {
    let mut supporting_only = production_planner_plan("focused", &[]);
    supporting_only["tracks"][0]["material"] = Value::Bool(false);

    let error = validate_plan(supporting_only)
        .expect_err("a plan without a core evidence obligation must fail closed");
    assert!(error.contains("at least one material track"), "{error}");
}

#[test]
fn focused_questions_inherit_the_model_selected_track_materiality() {
    let mut mixed = plan("focused", &[]);
    mixed["tracks"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "track:supporting",
            "title": "Supporting context",
            "focus": "Collect useful context that does not gate the core answer",
            "perspective": "",
            "material": false,
            "questions": ["Which supporting context remains available?"],
            "completion_criteria": ["Retain evidence or an explicit bounded limitation"]
        }));

    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut events = Vec::new();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::StrategySelected {
            method: a3s::research::ResearchMethod::Focused,
        },
        &limits,
    )
    .expect("strategy");
    queue_plan_questions(&mixed, None, &mut state, &mut events, &limits)
        .expect("focused questions");

    assert!(state.questions[0].material);
    assert!(!state.questions[1].material);
}

#[test]
fn focused_maker_plan_uses_track_questions_when_search_queries_are_empty() {
    let mut local = production_planner_plan("focused", &[]);
    local["execution_route"] = Value::String("maker_first".to_string());
    local["search_queries"] = serde_json::json!([]);
    let local = validate_plan(local).expect("focused maker plan").value;

    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut events = Vec::new();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::StrategySelected {
            method: a3s::research::ResearchMethod::Focused,
        },
        &limits,
    )
    .expect("strategy");
    queue_plan_questions(&local, None, &mut state, &mut events, &limits)
        .expect("track-driven focused questions");

    assert_eq!(
        state.questions[0].retrieval_query.as_deref(),
        Some("What does the evidence establish?")
    );
    assert_eq!(
        questions_scheduled_for_retrieval(&state, &local)
            .expect("track-driven resolution batch")
            .len(),
        1
    );
}

#[test]
fn transient_question_failure_defers_until_no_retry_wave_remains() {
    let focused = plan("focused", &[]);
    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut events = Vec::new();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::StrategySelected {
            method: a3s::research::ResearchMethod::Focused,
        },
        &limits,
    )
    .expect("strategy");
    queue_plan_questions(&focused, None, &mut state, &mut events, &limits)
        .expect("focused question");
    let question = state.questions[0].clone();

    defer_or_bound_question_batch(
        &mut state,
        &mut events,
        &limits,
        std::slice::from_ref(&question),
        true,
        "transient assessment failure",
    )
    .expect("retryable failure");
    assert_eq!(
        state.questions[0].status,
        a3s::research::QuestionStatus::Queued
    );
    assert!(matches!(
        events.last(),
        Some(InquiryEvent::QuestionDeferred { .. })
    ));

    defer_or_bound_question_batch(
        &mut state,
        &mut events,
        &limits,
        std::slice::from_ref(&question),
        false,
        "terminal assessment failure",
    )
    .expect("terminal failure");
    assert_eq!(
        state.questions[0].status,
        a3s::research::QuestionStatus::Bounded
    );
    assert!(matches!(
        events.last(),
        Some(InquiryEvent::QuestionBounded { .. })
    ));
}

#[test]
fn production_planner_clocks_are_normalized_for_each_workflow_wave() {
    let planned =
        validate_plan(production_planner_plan("focused", &[])).expect("production planner output");
    assert_eq!(planned.value["budget"]["retrieval_timeout_ms"], 90_000);
    assert_eq!(planned.value["budget"]["synthesis_timeout_ms"], 180_000);
    assert!(planned.value["budget"]
        .get("retrieval_timeout_secs")
        .is_none());
    assert!(planned.value["budget"]
        .get("synthesis_timeout_secs")
        .is_none());

    let args = serde_json::json!({
        "run_id": "wave-clock-fixture",
        "input": {
            "query": "fixture inquiry",
            "run_started_at_ms": u64::MAX,
        },
        "limits": { "timeoutMs": 30_000 }
    });
    let wave = workflow_args_with_plan(
        args,
        production_planner_plan("focused", &[]),
        Some("wave-clock-fixture-focused"),
    )
    .expect("focused workflow wave");
    assert_ne!(wave["input"]["run_started_at_ms"], Value::from(u64::MAX));
    assert_eq!(
        wave["input"]["research_plan"]["budget"]["retrieval_timeout_ms"],
        90_000
    );
    assert_eq!(
        wave["input"]["research_plan"]["budget"]["synthesis_timeout_ms"],
        180_000
    );
    assert!(wave["input"]["research_plan"]["budget"]
        .get("retrieval_timeout_secs")
        .is_none());
    assert!(wave["input"]["research_plan"]["budget"]
        .get("synthesis_timeout_secs")
        .is_none());
}

#[test]
fn host_inquiry_owns_cross_wave_iteration() {
    let plan = plan("focused", &[]);
    assert_eq!(plan["budget"]["max_iterations"], 2);
    let wave = single_wave_research_plan(&plan).expect("single host wave");
    assert_eq!(wave["budget"]["max_iterations"], 1);
    assert_eq!(plan["budget"]["max_iterations"], 2);
}

#[test]
fn planner_track_ids_must_be_unique_stable_ascii_metadata() {
    let mut invalid = plan("focused", &[]);
    invalid["tracks"][0]["id"] = Value::String("track with spaces".to_string());
    assert!(validate_plan(invalid).is_err());

    let mut duplicate = plan("focused", &[]);
    let repeated = duplicate["tracks"][0].clone();
    duplicate["tracks"].as_array_mut().unwrap().push(repeated);
    assert!(validate_plan(duplicate).is_err());
}

#[test]
fn focused_question_identity_is_owned_by_the_host() {
    let mut focused = plan("focused", &[]);
    focused["tracks"][0]["id"] = Value::String("track:material.v2".to_string());
    focused["tracks"][0]["questions"] = serde_json::json!([
        "What does the first source establish?",
        "What survives independent checking?"
    ]);

    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut events = Vec::new();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::StrategySelected {
            method: a3s::research::ResearchMethod::Focused,
        },
        &limits,
    )
    .expect("strategy");
    queue_plan_questions(&focused, None, &mut state, &mut events, &limits).expect("host questions");

    let ids = state
        .questions
        .iter()
        .map(|question| question.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["question:plan-1-1", "question:plan-1-2"]);
    assert!(ids.iter().all(|id| !id.contains("material.v2")));
    assert!(state.questions.iter().all(|question| {
        question.retrieval_query.as_deref() == Some("targeted evidence query")
    }));
}

#[test]
fn scout_pass_is_bounded_and_does_not_replace_the_stable_plan() {
    let mut base = plan("perspective_guided", &["landscape one", "landscape two"]);
    base["budget"]["direct_searches"] = Value::from(0);
    base["budget"]["direct_fetches"] = Value::from(0);
    let scout = scout_plan(
        &base,
        &["landscape one".to_string(), "landscape two".to_string()],
    )
    .expect("scout plan");
    assert_eq!(scout["execution_route"], "direct_only");
    assert_eq!(scout["budget"]["max_iterations"], 1);
    assert_eq!(scout["search_queries"].as_array().map(Vec::len), Some(2));
    assert_eq!(scout["tracks"].as_array().map(Vec::len), Some(2));
    assert_eq!(scout["budget"]["direct_searches"], 2);
    assert_eq!(scout["budget"]["direct_fetches"], 4);
    assert_eq!(base["tracks"][0]["id"], "track:material");
}

#[test]
fn perspective_questions_become_the_retrieval_plan() {
    let discovery = PerspectiveDiscoveryOutput {
        perspectives: vec![a3s::research::DiscoveredPerspective {
            id: "perspective:evidence".to_string(),
            title: "Evidence tension".to_string(),
            focus: "Resolve the tension found during scouting".to_string(),
            source_ids: vec!["source:one".to_string()],
            questions: vec![a3s::research::DiscoveredQuestion {
                id: "question:evidence".to_string(),
                prompt: "Which conclusion survives cross-checking?".to_string(),
                retrieval_query: "cross-check evidence conclusion".to_string(),
                material: true,
                round: 0,
            }],
        }],
    };
    let mut base = plan("perspective_guided", &["source landscape"]);
    base["budget"]["direct_searches"] = Value::from(0);
    base["budget"]["direct_fetches"] = Value::from(0);
    let enriched = perspective_research_plan(&base, &discovery).expect("enriched plan");
    assert_eq!(enriched["tracks"][0]["id"], "perspective:evidence");
    assert_eq!(
        enriched["search_queries"][0],
        "cross-check evidence conclusion"
    );
    assert_eq!(enriched["scout_queries"], serde_json::json!([]));
    assert_eq!(enriched["budget"]["direct_searches"], 1);
    assert_eq!(enriched["budget"]["direct_fetches"], 2);
}

#[test]
fn perspective_retrieval_budget_covers_each_lens_before_extra_queries() {
    let perspective = |id: &str, queries: &[&str]| a3s::research::DiscoveredPerspective {
        id: format!("perspective:{id}"),
        title: format!("Perspective {id}"),
        focus: format!("Resolve perspective {id}"),
        source_ids: vec![format!("source:{id}")],
        questions: queries
            .iter()
            .enumerate()
            .map(|(index, query)| a3s::research::DiscoveredQuestion {
                id: format!("question:{id}-{index}"),
                prompt: format!("Human-facing question {id}-{index}?"),
                retrieval_query: (*query).to_string(),
                material: true,
                round: 0,
            })
            .collect(),
    };
    let discovery = PerspectiveDiscoveryOutput {
        perspectives: vec![
            perspective("one", &["one primary", "one secondary"]),
            perspective("two", &["two primary", "two secondary"]),
            perspective("three", &["three primary"]),
        ],
    };
    let enriched = perspective_research_plan(
        &plan("perspective_guided", &["source landscape"]),
        &discovery,
    )
    .expect("perspective plan");

    assert_eq!(
        enriched["search_queries"],
        serde_json::json!([
            "one primary",
            "two primary",
            "three primary",
            "one secondary"
        ])
    );
}
