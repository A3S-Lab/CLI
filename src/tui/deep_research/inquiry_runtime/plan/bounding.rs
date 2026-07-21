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

pub(super) fn bound_workflow_timeout(args: &mut Value, timeout_ms: u64) -> Result<(), String> {
    args.get_mut("limits")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "DeepResearch workflow args have no limits object".to_string())?
        .insert("timeoutMs".to_string(), Value::from(timeout_ms));
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
