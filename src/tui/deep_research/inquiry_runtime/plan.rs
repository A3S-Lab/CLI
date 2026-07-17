//! LLM-authored inquiry plan validation and bounded plan transformations.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use a3s::research::{
    EvidenceQualityRequirements, InquiryEvent, InquiryLimits, InquiryState,
    PerspectiveDiscoveryOutput, Question, ResearchMethod, ResearchObligation,
};
use a3s_code_core::{AgentEvent, AgentSession};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use super::execution::{call_tool_with_progress, generated_object};
use super::{apply_event, MAX_FOLLOW_UP_QUESTIONS_PER_WAVE};

#[derive(Clone, Debug)]
pub(super) struct PlannedInquiry {
    pub(super) value: Value,
    pub(super) method: ResearchMethod,
    pub(super) scout_queries: Vec<String>,
}

pub(super) async fn generate_plan(
    session: &AgentSession,
    workflow_args: &Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
) -> Result<PlannedInquiry, String> {
    let planner = workflow_args
        .pointer("/input/loop_contract/planner")
        .and_then(Value::as_object)
        .ok_or_else(|| "DeepResearch host did not receive a planner contract".to_string())?;
    let schema = planner
        .get("output_schema")
        .cloned()
        .ok_or_else(|| "DeepResearch planner contract has no output schema".to_string())?;
    let prompt = planner
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|prompt| !prompt.trim().is_empty())
        .ok_or_else(|| "DeepResearch planner contract has no prompt".to_string())?;
    let timeout_ms = planner
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(120_000)
        .clamp(1_000, 600_000);
    let generation_args = serde_json::json!({
        "schema": schema,
        "schema_name": "deep_research_plan",
        "schema_description": "LLM-authored adaptive DeepResearch plan and budget",
        "prompt": prompt,
        "mode": "auto",
        "max_repair_attempts": 1,
        "include_raw_text": false,
        "timeout_ms": timeout_ms,
    });
    let generated = call_tool_with_progress(
        session,
        "generate_object",
        generation_args,
        progress_tx,
        false,
    )
    .await?;
    let value: Value = generated_object(&generated)?;
    validate_plan(value)
}

pub(super) fn validate_plan(value: Value) -> Result<PlannedInquiry, String> {
    let value = normalize_planner_budget(value)?;
    let object = value
        .as_object()
        .ok_or_else(|| "DeepResearch planner returned a non-object plan".to_string())?;
    let method = match object.get("research_method").and_then(Value::as_str) {
        Some("focused") => ResearchMethod::Focused,
        Some("perspective_guided") => ResearchMethod::PerspectiveGuided,
        Some(other) => return Err(format!("unknown DeepResearch research method `{other}`")),
        None => return Err("DeepResearch plan omitted research_method".to_string()),
    };
    let scout_queries = string_array(object.get("scout_queries"), "scout_queries", 4)?;
    // Empty direct queries are valid for maker-first and local-only focused
    // plans. Stable track questions remain the host-owned inquiry obligations.
    let _search_queries = string_array(object.get("search_queries"), "search_queries", 4)?;
    match method {
        ResearchMethod::Focused if !scout_queries.is_empty() => {
            return Err("focused DeepResearch plan must not contain scout queries".to_string())
        }
        ResearchMethod::PerspectiveGuided if scout_queries.is_empty() => {
            return Err(
                "perspective-guided DeepResearch plan must contain at least one scout query"
                    .to_string(),
            )
        }
        _ => {}
    }
    let (obligations, _) = research_contract_from_plan(&value)?;
    let tracks = object
        .get("tracks")
        .and_then(Value::as_array)
        .ok_or_else(|| "DeepResearch plan did not contain stable research tracks".to_string())?;
    for track in tracks {
        let track = track
            .as_object()
            .ok_or_else(|| "DeepResearch planner returned a non-object track".to_string())?;
        let questions = string_array(track.get("questions"), "track questions", 3)?;
        if questions.is_empty() {
            return Err("DeepResearch track has no research question".to_string());
        }
    }
    debug_assert!(obligations.iter().any(|obligation| obligation.material));
    Ok(PlannedInquiry {
        value,
        method,
        scout_queries,
    })
}

/// Convert the LLM-authored stable tracks into the typed coverage contract
/// consumed by the replayable Inquiry reducer. This is the only planner-to-
/// state boundary for research obligations and stopping conditions.
pub(super) fn research_contract_from_plan(
    plan: &Value,
) -> Result<(Vec<ResearchObligation>, Vec<String>), String> {
    let object = plan
        .as_object()
        .ok_or_else(|| "DeepResearch planner returned a non-object plan".to_string())?;
    let tracks = object
        .get("tracks")
        .and_then(Value::as_array)
        .filter(|tracks| !tracks.is_empty())
        .ok_or_else(|| "DeepResearch plan did not contain stable research tracks".to_string())?;
    let limits = InquiryLimits::default();
    if tracks.len() > limits.max_obligations {
        return Err(format!(
            "DeepResearch plan has {} stable research tracks; maximum is {}",
            tracks.len(),
            limits.max_obligations
        ));
    }

    let mut track_ids = BTreeSet::new();
    let mut obligations = Vec::with_capacity(tracks.len());
    for track in tracks {
        let track = track
            .as_object()
            .ok_or_else(|| "DeepResearch planner returned a non-object track".to_string())?;
        let id = required_text(track, "id")?;
        if !is_stable_plan_id(id) {
            return Err(format!(
                "DeepResearch track id `{id}` is not a stable ASCII identifier"
            ));
        }
        if !track_ids.insert(id) {
            return Err(format!("duplicate DeepResearch track id `{id}`"));
        }
        let title = required_text(track, "title")?;
        let focus = required_text(track, "focus")?;
        let material = track
            .get("material")
            .and_then(Value::as_bool)
            .ok_or_else(|| format!("DeepResearch track `{id}` omitted boolean `material`"))?;
        let completion_criteria = string_array(
            track.get("completion_criteria"),
            "track completion_criteria",
            limits.max_completion_criteria_per_obligation,
        )?;
        if completion_criteria.is_empty() {
            return Err(format!(
                "DeepResearch track `{id}` has no completion criterion"
            ));
        }
        let evidence_requirements = track
            .get("evidence_requirements")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                format!("DeepResearch track `{id}` omitted object `evidence_requirements`")
            })?;
        let primary_source_required = evidence_requirements
            .get("primary_source_required")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                format!("DeepResearch track `{id}` omitted boolean `primary_source_required`")
            })?;
        let independent_corroboration_required = evidence_requirements
            .get("independent_corroboration_required")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                format!(
                    "DeepResearch track `{id}` omitted boolean `independent_corroboration_required`"
                )
            })?;
        obligations.push(
            ResearchObligation::new(id, title, focus, material, completion_criteria)
                .with_evidence_requirements(EvidenceQualityRequirements {
                    primary_source_required,
                    independent_corroboration_required,
                }),
        );
    }
    if !obligations.iter().any(|obligation| obligation.material) {
        return Err("DeepResearch plan must contain at least one material track".to_string());
    }

    let stop_conditions = string_array(
        object.get("stop_conditions"),
        "stop_conditions",
        limits.max_stop_conditions,
    )?;
    if stop_conditions.is_empty() {
        return Err("DeepResearch plan has no stopping condition".to_string());
    }
    Ok((obligations, stop_conditions))
}

pub(super) fn commit_plan_research_contract(
    plan: &Value,
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
) -> Result<(), String> {
    let (obligations, stop_conditions) = research_contract_from_plan(plan)?;
    apply_event(
        state,
        events,
        InquiryEvent::ResearchObligationsCommitted {
            obligations,
            stop_conditions,
        },
        limits,
    )
}

fn is_stable_plan_id(value: &str) -> bool {
    value.len() <= 64
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && value.chars().skip(1).all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
}

pub(super) fn scout_plan(base: &Value, scout_queries: &[String]) -> Result<Value, String> {
    let mut plan = base
        .as_object()
        .cloned()
        .ok_or_else(|| "DeepResearch plan is not an object".to_string())?;
    plan.insert(
        "execution_route".to_string(),
        Value::String("direct_only".to_string()),
    );
    plan.insert(
        "search_queries".to_string(),
        serde_json::to_value(scout_queries)
            .map_err(|error| format!("encode scout queries: {error}"))?,
    );
    plan.insert("seed_urls".to_string(), Value::Array(Vec::new()));
    plan.insert(
        "tracks".to_string(),
        Value::Array(
            scout_queries
                .iter()
                .enumerate()
                .map(|(index, query)| {
                    serde_json::json!({
                        "id": format!("scout:{}", index + 1),
                        "title": query,
                        "focus": query,
                        "perspective": "",
                        "material": false,
                        "questions": [query],
                        "completion_criteria": ["Retain a traceable source-backed fact or explicitly record the evidence gap"],
                        "evidence_requirements": {
                            "primary_source_required": false,
                            "independent_corroboration_required": false
                        }
                    })
                })
                .collect(),
        ),
    );
    plan.insert(
        "stop_conditions".to_string(),
        serde_json::json!([
            "The source landscape is sufficient for evidence-grounded perspective discovery"
        ]),
    );
    if let Some(budget) = plan.get_mut("budget").and_then(Value::as_object_mut) {
        budget.insert("max_iterations".to_string(), Value::from(1));
        budget.insert("max_parallel_tasks".to_string(), Value::from(1));
        raise_budget_floor(budget, "direct_searches", scout_queries.len(), 4);
        raise_budget_floor(
            budget,
            "direct_fetches",
            scout_queries.len().saturating_mul(2),
            8,
        );
    }
    Ok(Value::Object(plan))
}

pub(super) fn perspective_research_plan(
    base: &Value,
    discovery: &PerspectiveDiscoveryOutput,
) -> Result<Value, String> {
    let mut plan = base
        .as_object()
        .cloned()
        .ok_or_else(|| "DeepResearch plan is not an object".to_string())?;
    let tracks = discovery
        .perspectives
        .iter()
        .map(|perspective| {
            let title = perspective.reader_title();
            let material = perspective
                .questions
                .iter()
                .any(|question| question.material);
            let evidence_requirements = merged_evidence_requirements(
                base,
                perspective
                    .questions
                    .iter()
                    .flat_map(|question| question.obligation_ids.iter().map(String::as_str)),
            );
            serde_json::json!({
                "id": perspective.id,
                "title": title,
                "focus": perspective.focus,
                "perspective": title,
                "material": material,
                "questions": perspective.questions.iter().map(|question| question.prompt.as_str()).collect::<Vec<_>>(),
                "completion_criteria": ["Resolve each linked question with traceable evidence or an explicit bounded gap"],
                "evidence_requirements": evidence_requirements
            })
        })
        .collect::<Vec<_>>();
    // Preserve coverage across source-derived perspectives before allocating a
    // second query to any one perspective. This is deterministic scheduling of
    // model-authored queries, not task/domain routing.
    let mut search_queries = Vec::new();
    let maximum_questions = discovery
        .perspectives
        .iter()
        .map(|perspective| perspective.questions.len())
        .max()
        .unwrap_or_default();
    for question_index in 0..maximum_questions {
        for perspective in &discovery.perspectives {
            // Material questions lead within a perspective, but every
            // supporting-only perspective still receives a retrieval
            // opportunity before a second question is scheduled elsewhere.
            let ordered = perspective
                .questions
                .iter()
                .filter(|question| question.material)
                .chain(
                    perspective
                        .questions
                        .iter()
                        .filter(|question| !question.material),
                );
            if let Some(question) = ordered.into_iter().nth(question_index) {
                search_queries.push(question.retrieval_query.clone());
                if search_queries.len() == 4 {
                    break;
                }
            }
        }
        if search_queries.len() == 4 {
            break;
        }
    }
    if search_queries.is_empty() {
        return Err("perspective discovery produced no retrieval question".to_string());
    }
    let search_count = search_queries.len();
    let track_count = tracks.len();
    plan.insert("tracks".to_string(), Value::Array(tracks));
    plan.insert(
        "search_queries".to_string(),
        serde_json::to_value(search_queries)
            .map_err(|error| format!("encode perspective questions: {error}"))?,
    );
    plan.insert("scout_queries".to_string(), Value::Array(Vec::new()));
    if let Some(budget) = plan.get_mut("budget").and_then(Value::as_object_mut) {
        // Perspective-guided iteration is owned by the Rust inquiry loop. A
        // workflow invocation is one retrieval wave, never a nested loop.
        budget.insert("max_iterations".to_string(), Value::from(1));
        raise_budget_floor(budget, "max_parallel_tasks", track_count, 4);
        raise_budget_floor(budget, "direct_searches", search_count, 4);
        raise_budget_floor(budget, "direct_fetches", search_count.saturating_mul(2), 8);
    }
    Ok(Value::Object(plan))
}

pub(super) fn plan_max_iterations(plan: &Value) -> u64 {
    plan.pointer("/budget/max_iterations")
        .and_then(Value::as_u64)
        .unwrap_or(1)
}

/// DynamicWorkflow executes one evidence wave at a time. The Rust inquiry
/// loop owns cross-wave questioning and convergence, so the workflow must not
/// run a second, hidden research loop from the same plan.
pub(super) fn single_wave_research_plan(base: &Value) -> Result<Value, String> {
    let mut plan = base
        .as_object()
        .cloned()
        .ok_or_else(|| "DeepResearch plan is not an object".to_string())?;
    let budget = plan
        .get_mut("budget")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "DeepResearch plan omitted its budget".to_string())?;
    budget.insert("max_iterations".to_string(), Value::from(1));
    Ok(Value::Object(plan))
}

pub(super) fn follow_up_research_plan(
    base: &Value,
    questions: &[Question],
    obligations: &[ResearchObligation],
) -> Result<Value, String> {
    let mut plan = base
        .as_object()
        .cloned()
        .ok_or_else(|| "DeepResearch follow-up plan base is not an object".to_string())?;
    let mut unique_frontiers = BTreeSet::new();
    let bounded = questions
        .iter()
        .filter(|question| unique_frontiers.insert(question_frontier_key(question)))
        .take(MAX_FOLLOW_UP_QUESTIONS_PER_WAVE)
        .collect::<Vec<_>>();
    if bounded.is_empty() {
        return Err("DeepResearch follow-up wave has no queued questions".to_string());
    }
    let retrieval_queries = bounded
        .iter()
        .map(|question| {
            question
                .retrieval_query
                .as_deref()
                .map(str::trim)
                .filter(|query| !query.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    format!(
                        "DeepResearch follow-up question `{}` omitted its model-authored retrieval query",
                        question.id
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    plan.insert(
        "tracks".to_string(),
        Value::Array(
            bounded
                .iter()
                .map(|question| {
                    let evidence_requirements = merged_typed_evidence_requirements(
                        obligations,
                        question.obligation_ids.iter().map(String::as_str),
                    );
                    serde_json::json!({
                        "id": question.id,
                        "title": question.prompt,
                        "focus": question.prompt,
                        "perspective": question.perspective_id.clone().unwrap_or_default(),
                        "material": question.material,
                        "questions": [question.prompt],
                        "completion_criteria": ["Return traceable evidence or an explicit bounded gap"],
                        "evidence_requirements": evidence_requirements
                    })
                })
                .collect(),
        ),
    );
    plan.insert(
        "search_queries".to_string(),
        serde_json::to_value(&retrieval_queries)
            .map_err(|error| format!("encode follow-up retrieval queries: {error}"))?,
    );
    plan.insert("scout_queries".to_string(), Value::Array(Vec::new()));
    if let Some(budget) = plan.get_mut("budget").and_then(Value::as_object_mut) {
        budget.insert("max_iterations".to_string(), Value::from(1));
        budget.insert("max_parallel_tasks".to_string(), Value::from(bounded.len()));
        raise_budget_floor(budget, "direct_searches", bounded.len(), 4);
        raise_budget_floor(budget, "direct_fetches", bounded.len().saturating_mul(2), 8);
    }
    Ok(Value::Object(plan))
}

fn merged_typed_evidence_requirements<'a>(
    obligations: &[ResearchObligation],
    obligation_ids: impl Iterator<Item = &'a str>,
) -> Value {
    let obligation_ids = obligation_ids.collect::<BTreeSet<_>>();
    let mut primary_source_required = false;
    let mut independent_corroboration_required = false;
    for obligation in obligations {
        if !obligation_ids.contains(obligation.id.as_str()) {
            continue;
        }
        primary_source_required |= obligation.evidence_requirements.primary_source_required;
        independent_corroboration_required |= obligation
            .evidence_requirements
            .independent_corroboration_required;
    }
    serde_json::json!({
        "primary_source_required": primary_source_required,
        "independent_corroboration_required": independent_corroboration_required,
    })
}

/// Preserve the strongest planner-authored evidence contract when a host
/// wave groups stable obligations into source-derived or follow-up tracks.
/// This is an ID join over model-authored obligations, not semantic routing.
fn merged_evidence_requirements<'a>(
    base: &Value,
    obligation_ids: impl Iterator<Item = &'a str>,
) -> Value {
    let obligation_ids = obligation_ids.collect::<BTreeSet<_>>();
    let mut primary_source_required = false;
    let mut independent_corroboration_required = false;
    for track in base
        .get("tracks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
    {
        let Some(id) = track.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !obligation_ids.contains(id) {
            continue;
        }
        let Some(requirements) = track
            .get("evidence_requirements")
            .and_then(Value::as_object)
        else {
            continue;
        };
        primary_source_required |= requirements
            .get("primary_source_required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        independent_corroboration_required |= requirements
            .get("independent_corroboration_required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    serde_json::json!({
        "primary_source_required": primary_source_required,
        "independent_corroboration_required": independent_corroboration_required,
    })
}

fn raise_budget_floor(
    budget: &mut Map<String, Value>,
    key: &str,
    requested: usize,
    hard_cap: usize,
) {
    let current = budget
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default();
    budget.insert(
        key.to_string(),
        Value::from(current.max(requested).min(hard_cap)),
    );
}

pub(super) fn queued_questions(state: &InquiryState) -> Vec<Question> {
    state
        .questions
        .iter()
        .filter(|question| question.status == a3s::research::QuestionStatus::Queued)
        .cloned()
        .collect()
}

pub(super) fn questions_scheduled_for_retrieval(
    state: &InquiryState,
    plan: &Value,
) -> Result<Vec<Question>, String> {
    let mut scheduled_queries = string_array(plan.get("search_queries"), "search_queries", 4)?
        .into_iter()
        .map(|query| normalize_retrieval_query(&query))
        .collect::<BTreeSet<_>>();
    if scheduled_queries.is_empty() {
        let tracks = plan
            .get("tracks")
            .and_then(Value::as_array)
            .ok_or_else(|| "DeepResearch plan has no tracks".to_string())?;
        for track in tracks {
            let questions = string_array(track.get("questions"), "track questions", 3)?;
            scheduled_queries.extend(
                questions
                    .into_iter()
                    .map(|query| normalize_retrieval_query(&query)),
            );
        }
    }
    let mut scheduled_ids = state
        .questions
        .iter()
        .filter(|question| question.status == a3s::research::QuestionStatus::Queued)
        .filter(|question| {
            question
                .retrieval_query
                .as_deref()
                .map(normalize_retrieval_query)
                .is_some_and(|query| scheduled_queries.contains(&query))
        })
        .map(|question| question.id.clone())
        .collect::<BTreeSet<_>>();
    if scheduled_ids.is_empty() {
        return Err(
            "DeepResearch retrieval wave did not match any queued model-authored query".to_string(),
        );
    }

    // A follow-up query is authored to close its parent's evidence gap. Keep
    // any still-queued ancestor addressable by that evidence without spending
    // another search slot on the ancestor's already-attempted query.
    let mut frontier = scheduled_ids.iter().cloned().collect::<Vec<_>>();
    while let Some(question_id) = frontier.pop() {
        let Some(parent_id) = state
            .questions
            .iter()
            .find(|question| question.id == question_id)
            .and_then(|question| question.parent_question_id.as_deref())
        else {
            continue;
        };
        let Some(parent) = state.questions.iter().find(|question| {
            question.id == parent_id && question.status == a3s::research::QuestionStatus::Queued
        }) else {
            continue;
        };
        if scheduled_ids.insert(parent.id.clone()) {
            frontier.push(parent.id.clone());
        }
    }

    Ok(state
        .questions
        .iter()
        .filter(|question| scheduled_ids.contains(&question.id))
        .cloned()
        .collect())
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RetrievalFrontierFingerprint {
    normalized_query: String,
    obligation_ids: Vec<String>,
    evidence_head: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct RetrievalWaveSelection {
    pub(super) novel: Vec<Question>,
    pub(super) already_attempted: Vec<Question>,
}

/// Select only retrieval frontiers that have not already completed an
/// opportunity in this replayable Inquiry prefix. Question IDs are excluded
/// deliberately: a model cannot buy another identical search by renaming a
/// follow-up question.
pub(super) fn queued_questions_for_next_wave(
    state: &InquiryState,
    events: &[InquiryEvent],
    limits: &InquiryLimits,
) -> Result<RetrievalWaveSelection, String> {
    let attempted = attempted_retrieval_frontiers(events, limits)?;
    let attempted_query_frontiers = attempted
        .iter()
        .map(|fingerprint| {
            (
                fingerprint.normalized_query.clone(),
                fingerprint.obligation_ids.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    let mut selection = RetrievalWaveSelection::default();
    let mut fresh = Vec::new();
    let mut advanced = Vec::new();
    for question in queued_questions(state) {
        let fingerprint = retrieval_frontier_fingerprint(state, &question)?;
        if attempted.contains(&fingerprint) {
            selection.already_attempted.push(question);
        } else if attempted_query_frontiers.contains(&question_frontier_key(&question)) {
            advanced.push(question);
        } else {
            fresh.push(question);
        }
    }
    // Cover every never-attempted query/obligation frontier before revisiting
    // one whose evidence head advanced because another wave made progress.
    // This preserves breadth without forbidding a bounded, evidence-justified
    // revisit after the fresh frontier is exhausted.
    selection.novel = if fresh.is_empty() { advanced } else { fresh };
    // A genuinely novel child query is allowed to close an already-attempted
    // ancestor's gap. Keep that ancestor queued so the resolver can address it
    // from the child's evidence, without putting the ancestor's old query back
    // into the retrieval plan.
    let mut carried_ancestor_ids = BTreeSet::new();
    let mut frontier = selection
        .novel
        .iter()
        .filter_map(|question| question.parent_question_id.clone())
        .collect::<Vec<_>>();
    while let Some(question_id) = frontier.pop() {
        if !carried_ancestor_ids.insert(question_id.clone()) {
            continue;
        }
        if let Some(parent_id) = state
            .question(&question_id)
            .and_then(|question| question.parent_question_id.clone())
        {
            frontier.push(parent_id);
        }
    }
    selection
        .already_attempted
        .retain(|question| !carried_ancestor_ids.contains(&question.id));
    Ok(selection)
}

/// Reconstruct attempted retrieval frontiers solely from the durable event
/// prefix. Resolution events are the host-owned completion boundary for a
/// retrieval opportunity. The evidence head is taken after the event so any
/// evidence accepted immediately before resolution belongs to that completed
/// frontier on both live execution and strict replay.
pub(super) fn attempted_retrieval_frontiers(
    events: &[InquiryEvent],
    limits: &InquiryLimits,
) -> Result<BTreeSet<RetrievalFrontierFingerprint>, String> {
    let mut state = InquiryState::default();
    let mut attempted = BTreeSet::new();
    for (event_index, event) in events.iter().enumerate() {
        let resolved_question = match event {
            InquiryEvent::QuestionAnswered { question_id, .. }
            | InquiryEvent::QuestionDeferred { question_id, .. }
            | InquiryEvent::QuestionBounded { question_id, .. } => {
                state.question(question_id).cloned()
            }
            _ => None,
        };
        state.apply(event, limits).map_err(|error| {
            format!(
                "derive attempted retrieval frontiers from inquiry event {event_index}: {error}"
            )
        })?;
        if let Some(question) = resolved_question {
            attempted.insert(retrieval_frontier_fingerprint(&state, &question)?);
        }
    }
    Ok(attempted)
}

pub(super) fn retrieval_frontier_fingerprint(
    state: &InquiryState,
    question: &Question,
) -> Result<RetrievalFrontierFingerprint, String> {
    let query = question
        .retrieval_query
        .as_deref()
        .map(normalize_retrieval_query)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| {
            format!(
                "DeepResearch question `{}` has no normalized retrieval query",
                question.id
            )
        })?;
    Ok(RetrievalFrontierFingerprint {
        normalized_query: query,
        obligation_ids: normalized_obligation_ids(question),
        evidence_head: retrieval_evidence_head(state)?,
    })
}

fn question_frontier_key(question: &Question) -> (String, Vec<String>) {
    (
        question
            .retrieval_query
            .as_deref()
            .map(normalize_retrieval_query)
            .unwrap_or_default(),
        normalized_obligation_ids(question),
    )
}

fn normalized_obligation_ids(question: &Question) -> Vec<String> {
    question
        .obligation_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalize_retrieval_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|token| {
            token
                .chars()
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn retrieval_evidence_head(state: &InquiryState) -> Result<String, String> {
    // BTree collections and typed evidence make this encoding stable across
    // process replay. Only the typed resolution relation for a contradiction
    // is included, not model-authored rationale prose.
    let resolved_diagnostics = state
        .contract_assessment
        .iter()
        .flat_map(|assessment| assessment.diagnostics.iter())
        .filter(|diagnostic| {
            diagnostic.disposition == a3s::research::DiagnosticDisposition::Resolved
        })
        .map(|diagnostic| {
            let obligation_ids = diagnostic
                .obligation_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            let evidence_ids = diagnostic
                .evidence_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            (
                diagnostic.diagnostic_id.clone(),
                obligation_ids,
                evidence_ids,
            )
        })
        .collect::<BTreeSet<_>>();
    let encoded = serde_json::to_vec(&(
        &state.evidence_catalog,
        &state.source_catalog,
        resolved_diagnostics,
    ))
    .map_err(|error| format!("encode DeepResearch evidence head: {error}"))?;
    let mut digest = Sha256::new();
    digest.update(encoded);
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn bound_questions(
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    reason: &str,
) -> Result<(), String> {
    let queued = queued_questions(state);
    bound_question_batch(state, events, limits, &queued, reason)
}

pub(super) fn defer_or_bound_question_batch(
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    questions: &[Question],
    retryable: bool,
    reason: &str,
) -> Result<(), String> {
    if !retryable {
        return bound_question_batch(state, events, limits, questions, reason);
    }
    for question in questions {
        apply_event(
            state,
            events,
            InquiryEvent::QuestionDeferred {
                question_id: question.id.clone(),
                reason: reason.to_string(),
            },
            limits,
        )?;
    }
    Ok(())
}

pub(super) fn bound_question_batch(
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    questions: &[Question],
    reason: &str,
) -> Result<(), String> {
    for question in questions {
        apply_event(
            state,
            events,
            InquiryEvent::QuestionBounded {
                question_id: question.id.clone(),
                reason: reason.to_string(),
            },
            limits,
        )?;
    }
    Ok(())
}

pub(super) fn queue_plan_questions(
    plan: &Value,
    perspective_id: Option<&str>,
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
) -> Result<(), String> {
    let tracks = plan
        .get("tracks")
        .and_then(Value::as_array)
        .ok_or_else(|| "DeepResearch plan has no tracks".to_string())?;
    let retrieval_queries = string_array(plan.get("search_queries"), "search_queries", 4)?;
    let mut questions = Vec::new();
    let mut retrieval_index = 0usize;
    for (track_index, track) in tracks.iter().enumerate() {
        let track = track
            .as_object()
            .ok_or_else(|| "DeepResearch plan contains a non-object track".to_string())?;
        let obligation_id = required_text(track, "id")?;
        let material = track
            .get("material")
            .and_then(Value::as_bool)
            .ok_or_else(|| "DeepResearch plan track omitted boolean `material`".to_string())?;
        for (question_index, prompt) in track
            .get("questions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .enumerate()
        {
            // Planner-authored IDs are display metadata and may contain
            // provider-dependent punctuation. Inquiry identity is owned by
            // the host so replay and downstream closed schemas remain stable.
            let id = format!("question:plan-{}-{}", track_index + 1, question_index + 1);
            let mut question =
                Question::queued(id, perspective_id.map(str::to_string), prompt.to_string());
            question.obligation_ids = vec![obligation_id.to_string()];
            question.retrieval_query = Some(
                retrieval_queries
                    .get(retrieval_index)
                    .cloned()
                    .unwrap_or_else(|| prompt.trim().to_string()),
            );
            retrieval_index = retrieval_index.saturating_add(1);
            question.material = material;
            question.round = 0;
            questions.push(question);
        }
    }
    if questions.is_empty() {
        return Err("DeepResearch plan did not queue any research question".to_string());
    }
    apply_event(
        state,
        events,
        InquiryEvent::QuestionsQueued { questions },
        limits,
    )
}

pub(super) fn workflow_args_with_plan(
    mut args: Value,
    plan: Value,
    run_id: Option<&str>,
) -> Result<Value, String> {
    // Focused, scout, main, and follow-up retrieval all cross this boundary.
    // Give each wave its own elapsed-time origin instead of inheriting time
    // spent by the planner or an earlier wave.
    let plan = normalize_planner_budget(plan)?;
    let input = args
        .get_mut("input")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "DeepResearch workflow args have no input object".to_string())?;
    input.insert("research_plan".to_string(), plan);
    input.insert(
        "execution_mode".to_string(),
        Value::String("collect_only".to_string()),
    );
    input.insert("research_plan_fixture".to_string(), Value::Bool(false));
    input.insert(
        "run_started_at_ms".to_string(),
        Value::from(current_time_ms()),
    );
    if let Some(run_id) = run_id {
        args.as_object_mut()
            .ok_or_else(|| "DeepResearch workflow args are not an object".to_string())?
            .insert("run_id".to_string(), Value::String(run_id.to_string()));
    }
    Ok(args)
}

fn normalize_planner_budget(mut plan: Value) -> Result<Value, String> {
    // The provider-facing planner schema uses seconds, while the workflow
    // runtime contract uses milliseconds. Injected host plans bypass the
    // JavaScript planner-result normalizer, so close that boundary here.
    let Some(budget) = plan.get_mut("budget").and_then(Value::as_object_mut) else {
        return Ok(plan);
    };
    for (seconds_key, milliseconds_key) in [
        ("retrieval_timeout_secs", "retrieval_timeout_ms"),
        ("synthesis_timeout_secs", "synthesis_timeout_ms"),
        ("per_task_timeout_secs", "per_task_timeout_ms"),
    ] {
        let Some(seconds) = budget.remove(seconds_key) else {
            continue;
        };
        let seconds = seconds
            .as_u64()
            .filter(|seconds| *seconds > 0)
            .ok_or_else(|| {
                format!("DeepResearch plan budget `{seconds_key}` must be a positive integer")
            })?;
        let milliseconds = seconds.checked_mul(1_000).ok_or_else(|| {
            format!("DeepResearch plan budget `{seconds_key}` exceeds millisecond range")
        })?;
        budget.insert(milliseconds_key.to_string(), Value::from(milliseconds));
    }
    Ok(plan)
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

pub(super) fn bound_workflow_timeout(args: &mut Value, timeout_ms: u64) -> Result<(), String> {
    args.get_mut("limits")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "DeepResearch workflow args have no limits object".to_string())?
        .insert("timeoutMs".to_string(), Value::from(timeout_ms));
    args.pointer_mut("/input/workflow_timeout_ms")
        .ok_or_else(|| "DeepResearch workflow args omitted workflow_timeout_ms".to_string())?
        .clone_from(&Value::from(timeout_ms));
    Ok(())
}

fn string_array(
    value: Option<&Value>,
    resource: &str,
    maximum: usize,
) -> Result<Vec<String>, String> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("DeepResearch plan {resource} is not an array"))?;
    if values.len() > maximum {
        return Err(format!(
            "DeepResearch plan {resource} has {} items; maximum is {maximum}",
            values.len()
        ));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("DeepResearch plan {resource} contains a blank item"))
        })
        .collect()
}

fn required_text<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("DeepResearch plan omitted non-empty `{key}`"))
}
