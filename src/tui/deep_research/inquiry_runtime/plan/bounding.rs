pub(super) fn bound_questions(
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    reason: &str,
) -> Result<(), String> {
    let queued = state
        .questions
        .iter()
        .filter(|question| question.status == a3s::research::QuestionStatus::Queued)
        .cloned()
        .collect::<Vec<_>>();
    bound_question_batch(state, events, limits, &queued, reason)
}

pub(super) fn bound_question_batch(
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    questions: &[Question],
    reason: &str,
) -> Result<(), String> {
    let reason = bounded_question_event_reason(reason, limits.max_text_chars);
    for question in questions {
        apply_event(
            state,
            events,
            InquiryEvent::QuestionBounded {
                question_id: question.id.clone(),
                reason: reason.clone(),
            },
            limits,
        )?;
    }
    Ok(())
}

/// Tool/provider diagnostics may include the entire rejected schema or model
/// payload. Durable question events retain a concise single-line prefix and
/// must never fail merely because an upstream error exceeded reducer limits.
fn bounded_question_event_reason(reason: &str, maximum: usize) -> String {
    let normalized = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    let detail = if normalized.is_empty() {
        "question resolution ended without a diagnostic"
    } else {
        normalized.as_str()
    };
    detail.chars().take(maximum).collect()
}

pub(super) fn queue_plan_questions(
    plan: &Value,
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
) -> Result<(), String> {
    let tracks = plan
        .get("tracks")
        .and_then(Value::as_array)
        .ok_or_else(|| "DeepResearch plan has no tracks".to_string())?;
    let mut questions = Vec::new();
    for (track_index, track) in tracks.iter().enumerate() {
        let track = track
            .as_object()
            .ok_or_else(|| "DeepResearch plan contains a non-object track".to_string())?;
        let obligation_id = required_text(track, "id")?;
        let material = track
            .get("material")
            .and_then(Value::as_bool)
            .ok_or_else(|| "DeepResearch plan track omitted boolean `material`".to_string())?;
        let prompts = string_array(
            track.get("questions"),
            "track questions",
            limits.max_questions,
        )?;
        let completion_criterion_count = track
            .get("completion_criteria")
            .and_then(Value::as_array)
            .map(Vec::len)
            .filter(|count| *count > 0)
            .ok_or_else(|| {
                format!("DeepResearch plan track `{obligation_id}` has no completion criteria")
            })?;
        let question_count = prompts.len();
        for (question_index, prompt) in prompts.into_iter().enumerate() {
            // Planner-authored IDs are display metadata and may contain
            // provider-dependent punctuation. Inquiry identity is owned by
            // the host so replay and downstream closed schemas remain stable.
            let id = format!("question:plan-{}-{}", track_index + 1, question_index + 1);
            let mut question = Question::queued(id, None, prompt);
            question.obligation_ids = vec![obligation_id.to_string()];
            question.completion_criterion_indexes = if question_count == completion_criterion_count
            {
                vec![question_index]
            } else if question_count == 1 {
                (0..completion_criterion_count).collect()
            } else if completion_criterion_count == 1 {
                vec![0]
            } else {
                return Err(format!(
                        "DeepResearch plan track `{obligation_id}` cannot map {question_count} questions onto {completion_criterion_count} completion criteria"
                    ));
            };
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
    // Flow compares this input byte-for-byte when a stable run is resumed, so
    // wall-clock origins belong to Flow history rather than durable input.
    exact_string_array(plan.get("search_queries"), "search_queries", 4)?;
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
    input.remove("run_started_at_ms");
    if let Some(run_id) = run_id {
        args.as_object_mut()
            .ok_or_else(|| "DeepResearch workflow args are not an object".to_string())?
            .insert("run_id".to_string(), Value::String(run_id.to_string()));
    }
    Ok(args)
}

/// Build the minimum Host-owned contract that keeps acquisition and qualified
/// reporting available when semantic planning is slow, invalid, or absent.
/// The original query remains the first provider query. One deterministic
/// outcome-oriented companion query broadens accountable publisher recall without
/// reopening semantic planning or creating an unbounded query fan-out.
pub(super) fn host_fallback_plan(workflow_args: &Value) -> Result<PlannedInquiry, String> {
    let query = workflow_args
        .pointer("/input/query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| "DeepResearch fallback contract requires a non-empty query".to_string())?;
    let local_only = workflow_args
        .pointer("/input/evidence_scope")
        .and_then(Value::as_str)
        == Some("local_only");
    let report_title = bounded_fallback_text(query, 160);
    let focus = bounded_fallback_text(query, 500);
    let question = bounded_fallback_text(query, 240);
    let plan = serde_json::json!({
        "report_title": report_title,
        "freshness_required": false,
        "workspace_evidence_required": local_only,
        "tracks": [{
            "id": "request.primary",
            "title": bounded_fallback_text(query, 160),
            "focus": focus,
            "material": true,
            "questions": [question.clone()],
            "completion_criteria": [question],
            "evidence_requirements": {
                "primary_source_required": false,
                "independent_corroboration_required": false
            }
        }],
        "search_queries": host_web_search_queries(workflow_args, query, local_only),
        "seed_urls": [],
        "budget": {
            "retrieval_timeout_ms": 150_000,
            "direct_searches": if local_only { 0 } else { 2 },
            "direct_fetches": if local_only { 0 } else { 8 }
        },
        "stop_conditions": [
            "Material evidence is retained or the request is explicitly bounded."
        ]
    });
    validate_plan(plan)
}

/// Complete one small model-authored outline into the legacy retrieval and
/// Inquiry contract without another semantic generation. The Host owns
/// questions, completion semantics, evidence-quality defaults, transport
/// budgets, and the provider query used by the bootstrap acquisition.
pub(super) fn host_plan_from_outline(
    workflow_args: &Value,
    outline: Value,
) -> Result<PlannedInquiry, String> {
    let query = workflow_args
        .pointer("/input/query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| "DeepResearch outline contract requires a non-empty query".to_string())?;
    let local_only = workflow_args
        .pointer("/input/evidence_scope")
        .and_then(Value::as_str)
        == Some("local_only");
    let mut outline = close_semantic_outline(outline)?;
    let targets = semantic_outline_track_targets(&outline)?;
    let tracks = targets
        .into_iter()
        .map(|target| {
            let mut target = target
                .as_object()
                .cloned()
                .ok_or_else(|| "DeepResearch outline contains a non-object target".to_string())?;
            let title = required_text(&target, "title")?.to_string();
            target.insert("focus".to_string(), Value::String(title.clone()));
            target.insert(
                "questions".to_string(),
                Value::Array(vec![Value::String(title.clone())]),
            );
            target.insert(
                "completion_criteria".to_string(),
                Value::Array(vec![Value::String(title)]),
            );
            target.insert(
                "evidence_requirements".to_string(),
                serde_json::json!({
                    "primary_source_required": false,
                    "independent_corroboration_required": false
                }),
            );
            Ok(Value::Object(target))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let object = outline
        .as_object_mut()
        .ok_or_else(|| "DeepResearch outline planner returned a non-object fragment".to_string())?;
    object.insert("tracks".to_string(), Value::Array(tracks));
    if local_only {
        object.insert("workspace_evidence_required".to_string(), Value::Bool(true));
    }
    object.insert(
        "search_queries".to_string(),
        serde_json::to_value(host_web_search_queries(workflow_args, query, local_only))
            .map_err(|error| format!("encode Host search queries: {error}"))?,
    );
    object.insert("seed_urls".to_string(), Value::Array(Vec::new()));
    object.insert(
        "budget".to_string(),
        serde_json::json!({
            "retrieval_timeout_ms": 150_000,
            "direct_searches": if local_only { 0 } else { 2 },
            "direct_fetches": if local_only { 0 } else { 8 }
        }),
    );
    validate_plan(outline)
}

pub(super) fn bootstrap_workflow_args(args: Value, run_id: &str) -> Result<Value, String> {
    let plan = host_fallback_plan(&args)?;
    let mut args = workflow_args_with_plan(args, plan.value, Some(run_id))?;
    args.pointer_mut("/input/execution_mode")
        .ok_or_else(|| "DeepResearch bootstrap args omitted execution mode".to_string())?
        .clone_from(&Value::String("bootstrap_acquisition".to_string()));
    Ok(args)
}

#[cfg(test)]
pub(super) fn attach_bootstrap_acquisition(
    workflow_args: &mut Value,
    acquisition: Value,
) -> Result<(), String> {
    let sources = acquisition
        .pointer("/packet/sources")
        .and_then(Value::as_array)
        .filter(|sources| !sources.is_empty())
        .ok_or_else(|| {
            "DeepResearch bootstrap acquisition contains no reusable raw source packet".to_string()
        })?;
    if sources.len() > 16 {
        return Err(
            "DeepResearch bootstrap acquisition exceeds the source catalog limit".to_string(),
        );
    }
    workflow_args
        .get_mut("input")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "DeepResearch workflow args have no input object".to_string())?
        .insert("bootstrap_acquisition".to_string(), acquisition);
    Ok(())
}

fn bounded_fallback_text(value: &str, maximum_chars: usize) -> String {
    value.chars().take(maximum_chars).collect()
}

fn host_web_search_queries(workflow_args: &Value, query: &str, local_only: bool) -> Vec<String> {
    if local_only {
        return Vec::new();
    }
    let current_date = workflow_args
        .pointer("/input/current_date")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|date| !date.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| chrono::Local::now().date_naive().to_string());
    vec![
        query.to_string(),
        host_authority_companion_query(query, &current_date),
    ]
}

fn host_authority_companion_query(query: &str, current_date: &str) -> String {
    if query.chars().any(host_query_han_character) {
        let localized_date = chrono::NaiveDate::parse_from_str(current_date, "%Y-%m-%d")
            .map(|date| date.format("%Y年%-m月%-d日").to_string())
            .unwrap_or_else(|_| current_date.to_string());
        format!("{query} {localized_date} 最新进展 最终结果 新闻")
    } else {
        format!("{query} {current_date} latest development final outcome news")
    }
}

fn host_query_han_character(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F
    )
}

fn normalize_planner_budget(mut plan: Value) -> Result<Value, String> {
    // The provider-facing planner schema uses seconds, while the workflow
    // runtime contract uses milliseconds. Injected host plans bypass the
    // JavaScript planner-result normalizer, so close that boundary here.
    let Some(budget) = plan.get_mut("budget").and_then(Value::as_object_mut) else {
        return Ok(plan);
    };
    let Some(seconds) = budget.remove("retrieval_timeout_secs") else {
        return Ok(plan);
    };
    let seconds = seconds
        .as_u64()
        .filter(|seconds| *seconds > 0)
        .ok_or_else(|| {
            "DeepResearch plan budget `retrieval_timeout_secs` must be a positive integer"
                .to_string()
        })?;
    let milliseconds = seconds.checked_mul(1_000).ok_or_else(|| {
        "DeepResearch plan budget `retrieval_timeout_secs` exceeds millisecond range".to_string()
    })?;
    budget.insert(
        "retrieval_timeout_ms".to_string(),
        Value::from(milliseconds),
    );
    Ok(plan)
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

pub(super) fn exact_string_array(
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
            let value = value.as_str().ok_or_else(|| {
                format!("DeepResearch plan {resource} contains a non-string item")
            })?;
            if value.is_empty() || value.trim().is_empty() {
                return Err(format!(
                    "DeepResearch plan {resource} contains a blank item"
                ));
            }
            if value.trim() != value {
                return Err(format!(
                    "DeepResearch plan {resource} contains an item with surrounding whitespace"
                ));
            }
            Ok(value.to_string())
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
