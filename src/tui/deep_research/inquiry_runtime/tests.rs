use a3s::research::{InquiryEvent, InquiryLimits, InquiryState, PerspectiveDiscoveryOutput};
use serde_json::Value;
use std::time::{Duration, Instant};

use super::super::deep_research_evidence_ledger::{
    AcceptedClaim, AcceptedEvidence, AcceptedSource, SourceTier,
};
use super::execution::balanced_evidence_packet;
use super::plan::{
    attempted_retrieval_frontiers, commit_plan_research_contract, defer_or_bound_question_batch,
    follow_up_research_plan, perspective_research_plan, questions_scheduled_for_retrieval,
    queue_plan_questions, queued_questions_for_next_wave, scout_plan, single_wave_research_plan,
    validate_plan, workflow_args_with_plan,
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
            "completion_criteria": ["A traceable answer or bounded gap"],
            "evidence_requirements": {
                "primary_source_required": false,
                "independent_corroboration_required": false
            }
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

fn evidence_fixture(index: usize) -> AcceptedEvidence {
    AcceptedEvidence {
        id: format!("evidence:{index}"),
        summary: format!("Evidence {index}"),
        confidence: Some("high".to_string()),
        sources: vec![AcceptedSource {
            id: format!("source:{index}"),
            anchor: format!("https://example.com/{index}"),
            title: None,
            date: None,
            reliability: None,
            quote_or_fact: Some(format!("Fact {index}")),
            tier: SourceTier::Secondary,
        }],
        claims: vec![AcceptedClaim {
            id: format!("claim:{index}"),
            text: format!("Claim {index}"),
        }],
        contradictions: Vec::new(),
        gaps: Vec::new(),
    }
}

#[test]
fn shared_inquiry_deadline_clamps_each_stage_and_preserves_finalization() {
    let now = Instant::now();
    let deadline = super::InquiryDeadline::from_elapsed(now, 720_000, 108_000, 500_000);

    assert_eq!(
        deadline.stage_timeout_ms_at(now, 180_000),
        Some(112_000),
        "a stage may use only the work budget before the finalization reserve"
    );
    assert_eq!(
        deadline.stage_timeout_ms_at(now + Duration::from_millis(111_000), 90_000),
        Some(1_000)
    );
    assert_eq!(
        deadline.stage_timeout_ms_at(now + Duration::from_millis(111_001), 90_000),
        None,
        "sub-second work must not start a model or retrieval operation"
    );
}

#[test]
fn shared_inquiry_deadline_accounts_for_time_spent_before_initialization() {
    let now = Instant::now();
    let exhausted = super::InquiryDeadline::from_elapsed(now, 720_000, 108_000, 620_000);
    let bounded = super::InquiryDeadline::from_elapsed(now, 720_000, 108_000, 600_000);

    assert_eq!(exhausted.stage_timeout_ms_at(now, 90_000), None);
    assert_eq!(bounded.stage_timeout_ms_at(now, 90_000), Some(12_000));
}

#[test]
fn resolver_packet_preserves_initial_and_latest_waves() {
    let evidence = (0..8).map(evidence_fixture).collect::<Vec<_>>();
    let packet = balanced_evidence_packet(&evidence, 4);
    assert_eq!(
        packet
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["evidence:0", "evidence:1", "evidence:6", "evidence:7"]
    );
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
fn planner_must_define_bounded_completion_and_stop_conditions() {
    let mut missing_criteria = production_planner_plan("focused", &[]);
    missing_criteria["tracks"][0]["completion_criteria"] = serde_json::json!([]);
    let error = validate_plan(missing_criteria)
        .expect_err("a stable obligation without completion criteria must fail closed");
    assert!(error.contains("completion criterion"), "{error}");

    let mut missing_stop = production_planner_plan("focused", &[]);
    missing_stop["stop_conditions"] = serde_json::json!([]);
    let error = validate_plan(missing_stop)
        .expect_err("an inquiry without a stopping condition must fail closed");
    assert!(error.contains("stopping condition"), "{error}");
}

#[test]
fn planner_must_explicitly_select_per_obligation_evidence_requirements() {
    let mut missing = production_planner_plan("focused", &[]);
    missing["tracks"][0]
        .as_object_mut()
        .expect("track")
        .remove("evidence_requirements");
    let error = validate_plan(missing)
        .expect_err("a new plan must explicitly select evidence requirements per obligation");
    assert!(error.contains("evidence_requirements"), "{error}");

    let mut focused = plan("focused", &[]);
    focused["tracks"][0]["evidence_requirements"] = serde_json::json!({
        "primary_source_required": true,
        "independent_corroboration_required": true
    });
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
    commit_plan_research_contract(&focused, &mut state, &mut events, &limits)
        .expect("typed research contract");

    assert!(
        state.obligations[0]
            .evidence_requirements
            .primary_source_required
    );
    assert!(
        state.obligations[0]
            .evidence_requirements
            .independent_corroboration_required
    );
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
            "completion_criteria": ["Retain evidence or an explicit bounded limitation"],
            "evidence_requirements": {
                "primary_source_required": false,
                "independent_corroboration_required": false
            }
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
    commit_plan_research_contract(&mixed, &mut state, &mut events, &limits)
        .expect("stable research contract");
    queue_plan_questions(&mixed, None, &mut state, &mut events, &limits)
        .expect("focused questions");

    assert!(state.questions[0].material);
    assert!(!state.questions[1].material);
    assert_eq!(state.questions[0].obligation_ids, ["track:material"]);
    assert_eq!(state.questions[1].obligation_ids, ["track:supporting"]);
    assert_eq!(state.obligations.len(), 2);
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
    commit_plan_research_contract(&local, &mut state, &mut events, &limits)
        .expect("stable research contract");
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
    commit_plan_research_contract(&focused, &mut state, &mut events, &limits)
        .expect("stable research contract");
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
    commit_plan_research_contract(&focused, &mut state, &mut events, &limits)
        .expect("stable research contract");
    queue_plan_questions(&focused, None, &mut state, &mut events, &limits).expect("host questions");

    let ids = state
        .questions
        .iter()
        .map(|question| question.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["question:plan-1-1", "question:plan-1-2"]);
    assert!(ids.iter().all(|id| !id.contains("material.v2")));
    assert_eq!(
        state.questions[0].retrieval_query.as_deref(),
        Some("targeted evidence query")
    );
    assert_eq!(
        state.questions[1].retrieval_query.as_deref(),
        Some("What survives independent checking?")
    );
    assert_eq!(
        questions_scheduled_for_retrieval(&state, &focused)
            .expect("initial question retrieval opportunity")
            .iter()
            .map(|question| question.id.as_str())
            .collect::<Vec<_>>(),
        ["question:plan-1-1"]
    );
}

#[test]
fn renamed_equivalent_query_cannot_repeat_an_attempted_frontier() {
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
    commit_plan_research_contract(&focused, &mut state, &mut events, &limits)
        .expect("stable research contract");
    queue_plan_questions(&focused, None, &mut state, &mut events, &limits)
        .expect("initial question");
    let root = state.questions[0].clone();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::QuestionDeferred {
            question_id: root.id.clone(),
            reason: "the completed wave retained no new material evidence".to_string(),
        },
        &limits,
    )
    .expect("completed retrieval opportunity");

    let mut renamed = a3s::research::Question::follow_up(
        "question:model-renamed",
        None,
        root.id.clone(),
        1,
        "Can the same search be tried under another question id?",
    );
    renamed.retrieval_query = Some("  TARGETED   evidence QUERY  ".to_string());
    renamed.obligation_ids.clone_from(&root.obligation_ids);
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::QuestionsQueued {
            questions: vec![renamed],
        },
        &limits,
    )
    .expect("renamed follow-up");

    let selection = queued_questions_for_next_wave(&state, &events, &limits)
        .expect("stable frontier selection");
    assert!(selection.novel.is_empty());
    assert_eq!(
        selection
            .already_attempted
            .iter()
            .map(|question| question.id.as_str())
            .collect::<Vec<_>>(),
        ["question:plan-1-1", "question:model-renamed"]
    );
}

#[test]
fn one_follow_up_wave_coalesces_equivalent_normalized_frontiers() {
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
    commit_plan_research_contract(&focused, &mut state, &mut events, &limits)
        .expect("stable research contract");
    let mut first = a3s::research::Question::queued(
        "question:first-equivalent",
        None,
        "First equivalent question",
    );
    first.retrieval_query = Some("Targeted evidence QUERY".to_string());
    first.obligation_ids = vec!["track:material".to_string()];
    let mut renamed = a3s::research::Question::queued(
        "question:second-equivalent",
        None,
        "Renamed equivalent question",
    );
    renamed.retrieval_query = Some("  targeted   EVIDENCE query ".to_string());
    renamed.obligation_ids = vec!["track:material".to_string()];

    let wave = follow_up_research_plan(&focused, &[first, renamed], state.obligations.as_slice())
        .expect("coalesced follow-up plan");
    assert_eq!(wave["search_queries"].as_array().map(Vec::len), Some(1));
    assert_eq!(wave["tracks"].as_array().map(Vec::len), Some(1));
    assert_eq!(wave["budget"]["max_parallel_tasks"], 1);
}

#[test]
fn new_query_or_obligation_remains_a_novel_retrieval_frontier() {
    let mut focused = plan("focused", &[]);
    focused["tracks"]
        .as_array_mut()
        .expect("tracks")
        .push(serde_json::json!({
            "id": "track:independent",
            "title": "Independent obligation",
            "focus": "Resolve an independent evidence obligation",
            "perspective": "",
            "material": false,
            "questions": ["Which independent evidence is available?"],
            "completion_criteria": ["Retain independent evidence or a bounded gap"],
            "evidence_requirements": {
                "primary_source_required": false,
                "independent_corroboration_required": false
            }
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
    commit_plan_research_contract(&focused, &mut state, &mut events, &limits)
        .expect("stable research contract");
    queue_plan_questions(&focused, None, &mut state, &mut events, &limits)
        .expect("planned questions");
    let root = state.questions[0].clone();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::QuestionDeferred {
            question_id: root.id.clone(),
            reason: "retrieval opportunity completed".to_string(),
        },
        &limits,
    )
    .expect("attempt root");

    let mut new_query = a3s::research::Question::follow_up(
        "question:new-query",
        None,
        root.id.clone(),
        1,
        "Which different query can close the same obligation?",
    );
    new_query.retrieval_query = Some("independent source for material claim".to_string());
    new_query.obligation_ids.clone_from(&root.obligation_ids);
    let mut new_obligation = a3s::research::Question::queued(
        "question:new-obligation",
        None,
        "Can the same query address an independent obligation?",
    );
    new_obligation.retrieval_query = root.retrieval_query.clone();
    new_obligation.obligation_ids = vec!["track:independent".to_string()];
    new_obligation.material = false;
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::QuestionsQueued {
            questions: vec![new_query, new_obligation],
        },
        &limits,
    )
    .expect("novel frontiers");

    let selection = queued_questions_for_next_wave(&state, &events, &limits)
        .expect("stable frontier selection");
    let novel_ids = selection
        .novel
        .iter()
        .map(|question| question.id.as_str())
        .collect::<Vec<_>>();
    assert!(novel_ids.contains(&"question:new-query"));
    assert!(novel_ids.contains(&"question:new-obligation"));
    assert!(!novel_ids.contains(&"question:plan-1-1"));
}

#[test]
fn accepted_evidence_advances_the_replayable_frontier_head() {
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
    commit_plan_research_contract(&focused, &mut state, &mut events, &limits)
        .expect("stable research contract");
    queue_plan_questions(&focused, None, &mut state, &mut events, &limits)
        .expect("initial question");
    let root = state.questions[0].clone();
    let mut independent = a3s::research::Question::queued(
        "question:independent",
        None,
        "Which independent source changes the evidence frontier?",
    );
    independent.retrieval_query = Some("independent evidence frontier".to_string());
    independent.obligation_ids.clone_from(&root.obligation_ids);
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::QuestionsQueued {
            questions: vec![independent],
        },
        &limits,
    )
    .expect("independent question");
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::QuestionDeferred {
            question_id: root.id.clone(),
            reason: "first frontier completed without gain".to_string(),
        },
        &limits,
    )
    .expect("first frontier completion");
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::EvidenceAccepted {
            evidence: a3s::research::EvidenceRef::new(
                "evidence:independent",
                vec!["claim:independent".to_string()],
                vec!["source:independent".to_string()],
            ),
        },
        &limits,
    )
    .expect("new accepted evidence");
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::QuestionAnswered {
            question_id: "question:independent".to_string(),
            answer: "The independent evidence advances the retained frontier.".to_string(),
            evidence_ids: vec!["evidence:independent".to_string()],
        },
        &limits,
    )
    .expect("independent coverage");

    let selection = queued_questions_for_next_wave(&state, &events, &limits)
        .expect("advanced frontier selection");
    assert_eq!(
        selection
            .novel
            .iter()
            .map(|question| question.id.as_str())
            .collect::<Vec<_>>(),
        ["question:plan-1-1"]
    );
    assert!(selection.already_attempted.is_empty());
}

#[test]
fn attempted_frontiers_are_derived_identically_after_event_replay() {
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
    commit_plan_research_contract(&focused, &mut state, &mut events, &limits)
        .expect("stable research contract");
    queue_plan_questions(&focused, None, &mut state, &mut events, &limits).expect("question");
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::QuestionDeferred {
            question_id: "question:plan-1-1".to_string(),
            reason: "completed without evidence gain".to_string(),
        },
        &limits,
    )
    .expect("retrieval completion");

    let encoded = serde_json::to_vec(&events).expect("serialize event prefix");
    let restored_events: Vec<InquiryEvent> =
        serde_json::from_slice(&encoded).expect("restore event prefix");
    let restored_state = a3s::research::replay(&restored_events, &limits).expect("strict replay");

    assert_eq!(restored_state, state);
    assert_eq!(
        attempted_retrieval_frontiers(&events, &limits).expect("live fingerprints"),
        attempted_retrieval_frontiers(&restored_events, &limits).expect("replayed fingerprints")
    );
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
                obligation_ids: vec!["track:material".to_string()],
                material: true,
                round: 0,
            }],
        }],
    };
    let mut base = plan("perspective_guided", &["source landscape"]);
    base["tracks"][0]["evidence_requirements"] = serde_json::json!({
        "primary_source_required": true,
        "independent_corroboration_required": true
    });
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
    assert_eq!(
        enriched["tracks"][0]["evidence_requirements"],
        serde_json::json!({
            "primary_source_required": true,
            "independent_corroboration_required": true
        })
    );
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
                obligation_ids: vec!["track:material".to_string()],
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

#[test]
fn supporting_only_perspective_receives_a_non_blocking_retrieval_opportunity() {
    let discovery = PerspectiveDiscoveryOutput {
        perspectives: vec![
            a3s::research::DiscoveredPerspective {
                id: "perspective:core".to_string(),
                title: "Core lens".to_string(),
                focus: "Resolve the core obligation".to_string(),
                source_ids: vec!["source:core".to_string()],
                questions: vec![a3s::research::DiscoveredQuestion {
                    id: "question:core".to_string(),
                    prompt: "What closes the core obligation?".to_string(),
                    retrieval_query: "core obligation evidence".to_string(),
                    obligation_ids: vec!["track:material".to_string()],
                    material: true,
                    round: 0,
                }],
            },
            a3s::research::DiscoveredPerspective {
                id: "perspective:context".to_string(),
                title: "Supporting context".to_string(),
                focus: "Collect useful non-gating context".to_string(),
                source_ids: vec!["source:context".to_string()],
                questions: vec![a3s::research::DiscoveredQuestion {
                    id: "question:context".to_string(),
                    prompt: "Which context helps qualify the answer?".to_string(),
                    retrieval_query: "supporting context evidence".to_string(),
                    obligation_ids: vec!["track:supporting".to_string()],
                    material: false,
                    round: 0,
                }],
            },
        ],
    };
    let mut base = plan("perspective_guided", &["source landscape"]);
    base["tracks"]
        .as_array_mut()
        .expect("tracks")
        .push(serde_json::json!({
            "id": "track:supporting",
            "title": "Supporting context",
            "focus": "Collect useful non-gating context",
            "perspective": "",
            "material": false,
            "questions": ["Which context helps qualify the answer?"],
            "completion_criteria": ["Context is retained or explicitly bounded"],
            "evidence_requirements": {
                "primary_source_required": false,
                "independent_corroboration_required": false
            }
        }));

    let enriched = perspective_research_plan(&base, &discovery).expect("perspective plan");
    assert_eq!(
        enriched["search_queries"],
        serde_json::json!(["core obligation evidence", "supporting context evidence"])
    );
    assert_eq!(enriched["tracks"][1]["material"], false);
}
