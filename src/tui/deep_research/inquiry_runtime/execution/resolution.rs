pub(super) async fn resolve_questions_once(
    session: &AgentSession,
    progress_tx: &mpsc::Sender<AgentEvent>,
    execution: &mut InquiryExecution,
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    checkpoint: Option<&InquiryCheckpointWriter>,
) -> Result<(), String> {
    let resolution_timeout_ms = match checkpoint {
        Some(checkpoint) => {
            let Some(timeout_ms) = checkpoint.question_review_stage_timeout_ms(
                DEEP_RESEARCH_QUESTION_REVIEW_STAGE_TIMEOUT_MS,
            ) else {
                terminalize_budget_exhaustion(
                    Some(checkpoint),
                    state,
                    events,
                    limits,
                    "the shared inquiry deadline left no closed-evidence review budget after reserving finalization",
                )
                .await?;
                return Ok(());
            };
            timeout_ms
        }
        None => DEEP_RESEARCH_QUESTION_REVIEW_STAGE_TIMEOUT_MS,
    };
    let queued = state
        .questions
        .iter()
        .filter(|question| question.status == QuestionStatus::Queued)
        .cloned()
        .collect::<Vec<_>>();
    resolve_queued_questions(
        session,
        progress_tx,
        &execution.result,
        &queued,
        state,
        events,
        limits,
        checkpoint,
        resolution_timeout_ms,
    )
    .await?;
    bound_questions(
        state,
        events,
        limits,
        "the single closed-evidence review retained no support for this question",
    )?;
    exhaust_if_material_evidence_floor_missing(state, events, limits)?;
    checkpoint_inquiry(checkpoint, events, state).await
}

pub(super) async fn assess_completed_research_contract(
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    checkpoint: Option<&InquiryCheckpointWriter>,
) -> Result<ResearchContractOutcome, String> {
    if state.phase != a3s::research::InquiryPhase::Outlining {
        return Err(format!(
            "research contract assessment requires Outlining; current phase is {:?}",
            state.phase
        ));
    }
    let assessment =
        derive_research_contract_assessment(state).map_err(|error| error.to_string())?;
    let event =
        research_contract_assessment_event(state, assessment).map_err(|error| error.to_string())?;
    apply_event_and_checkpoint(checkpoint, state, events, event, limits).await?;
    research_contract_outcome(state)
        .ok_or_else(|| "host contract reduction produced no terminal outcome".to_string())
}

fn exhaust_if_material_evidence_floor_missing(
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
) -> Result<(), String> {
    if state.phase == a3s::research::InquiryPhase::Exhausted {
        return Ok(());
    }
    if material_evidence_floor(state) {
        return Ok(());
    }
    let uncovered_material = state
        .obligations
        .iter()
        .filter(|obligation| obligation.material)
        .filter(|obligation| {
            !state.questions.iter().any(|question| {
                question.material
                    && question.status == QuestionStatus::Answered
                    && !question.evidence_ids.is_empty()
                    && question.obligation_ids.contains(&obligation.id)
            })
        })
        .count()
        .max(1);
    apply_event(
        state,
        events,
        InquiryEvent::BudgetExhausted {
            reason: format!(
                "{uncovered_material} material research obligation(s) had no traceable answered evidence path after the retrieval pass"
            ),
        },
        limits,
    )
}

async fn resolve_queued_questions(
    session: &AgentSession,
    progress_tx: &mpsc::Sender<AgentEvent>,
    result: &ToolCallResult,
    queued: &[Question],
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    checkpoint: Option<&InquiryCheckpointWriter>,
    resolution_timeout_ms: u64,
) -> Result<(), String> {
    if queued.is_empty() {
        return Err("DeepResearch retrieval pass has no scheduled questions".to_string());
    }
    let canonical =
        deep_research_canonical_workflow_output(&result.output, result.metadata.as_ref());
    let evidence = accepted_evidence_ledger(&canonical, result.metadata.as_ref());
    let evidence_catalog = prepare_evidence_catalog(state, &evidence)?;
    if evidence_catalog.addressable.is_empty() {
        bound_question_batch(
            state,
            events,
            limits,
            queued,
            "no accepted evidence was retained for this question",
        )?;
        checkpoint_inquiry(checkpoint, events, state).await?;
        return Ok(());
    }
    let query = canonical_query(&canonical);
    // The request participates in the stable Flow identity. Keep its active
    // generation fuse constant across process recovery; the review collector
    // separately enforces the remaining whole-stage wall-clock budget,
    // including admission queue time.
    let generation_timeout_ms = QUESTION_RESOLUTION_ATTEMPT_TIMEOUT_MS;
    let question_groups = question_review_groups(queued);
    let question_group_count = question_groups.len();
    let review_units = question_groups
        .into_iter()
        .enumerate()
        .map(|(ordinal, questions)| {
            let linked_obligations = state
                .obligations
                .iter()
                .filter(|obligation| {
                    questions.iter().any(|question| {
                        question.obligation_ids.contains(&obligation.id)
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            let group_evidence = question_group_evidence(&evidence_catalog.addressable, &questions);
            let prepared_packet = prepare_question_evidence_packet(
                &group_evidence,
                MAX_QUESTION_EVIDENCE_ITEMS,
                MAX_QUESTION_EVIDENCE_PACKET_CHARS,
            );
            let (allowed_evidence_ids, generation_args) = match prepared_packet {
                Ok(prepared_packet) => {
                    let generation_args = question_resolution_generation_params(
                        query.as_deref().unwrap_or("DeepResearch inquiry"),
                        &questions,
                        &linked_obligations,
                        &state.stop_conditions,
                        &prepared_packet.allowed_evidence_ids,
                        &prepared_packet.payload,
                        generation_timeout_ms,
                    )
                    .map_err(|error| error.to_string())
                    .and_then(|args| {
                        serde_json::to_value(args).map_err(|error| {
                            format!("encode question resolution request: {error}")
                        })
                    });
                    (prepared_packet.allowed_evidence_ids, generation_args)
                }
                Err(error) => (
                    BTreeSet::new(),
                    Err(format!(
                        "closed-evidence question packet could not be prepared: {error}"
                    )),
                ),
            };
            QuestionReviewUnit {
                ordinal,
                questions,
                stage_label: question_review_stage_label(ordinal, question_group_count),
                generation_args,
                allowed_evidence_ids,
            }
        })
        .collect::<Vec<_>>();
    let review_stream = stream::iter(review_units.into_iter().map(|unit| async move {
        execute_question_review_unit(
            session,
            progress_tx,
            checkpoint,
            unit,
            QUESTION_RESOLUTION_WORKFLOW_TIMEOUT_MS,
        )
        .await
    }))
    .buffer_unordered(MAX_CONCURRENT_QUESTION_REVIEWS);
    let (mut review_results, review_timed_out) =
        collect_inquiry_stage_results(review_stream, resolution_timeout_ms).await;
    review_results.sort_by_key(|result| result.ordinal);

    apply_pending_evidence(state, events, limits, evidence_catalog.pending)?;
    for result in review_results {
        let group_events = match result.events {
            Ok(group_events) => group_events,
            Err(error) => {
                tracing::warn!(
                    question_ids = %result.questions.iter()
                        .map(|question| question.id.as_str())
                        .collect::<Vec<_>>()
                        .join(","),
                    stage = %result.stage_label,
                    %error,
                    "DeepResearch closed-evidence obligation review unit did not produce valid resolutions"
                );
                result
                    .questions
                    .into_iter()
                    .map(|question| InquiryEvent::QuestionBounded {
                        question_id: question.id,
                        reason: "closed-evidence assessment did not establish a valid, traceable answer for this question".to_string(),
                    })
                    .collect()
            }
        };
        for event in group_events {
            apply_event(state, events, event, limits)?;
        }
    }
    if review_timed_out {
        bound_questions(
            state,
            events,
            limits,
            "the bounded closed-evidence review stage reached its deadline before this question completed",
        )?;
    }
    checkpoint_inquiry(checkpoint, events, state).await?;
    Ok(())
}

struct QuestionReviewUnit {
    ordinal: usize,
    questions: Vec<Question>,
    stage_label: String,
    generation_args: Result<Value, String>,
    allowed_evidence_ids: BTreeSet<String>,
}

struct QuestionReviewResult {
    ordinal: usize,
    questions: Vec<Question>,
    stage_label: String,
    events: Result<Vec<InquiryEvent>, String>,
}

async fn execute_question_review_unit(
    session: &AgentSession,
    progress_tx: &mpsc::Sender<AgentEvent>,
    checkpoint: Option<&InquiryCheckpointWriter>,
    unit: QuestionReviewUnit,
    resolution_timeout_ms: u64,
) -> QuestionReviewResult {
    let QuestionReviewUnit {
        ordinal,
        questions,
        stage_label,
        generation_args,
        allowed_evidence_ids,
    } = unit;
    let events = match generation_args {
        Ok(generation_args) => call_generation_with_progress(
            session,
            generation_args,
            progress_tx,
            checkpoint,
            &stage_label,
            resolution_timeout_ms,
            QUESTION_RESOLUTION_MAX_ATTEMPTS,
        )
        .await
        .and_then(|generated| generated_object::<Value>(&generated))
        .map(|value| {
            isolated_wire_question_resolution_events(
                value,
                &questions,
                &allowed_evidence_ids,
            )
        }),
        Err(error) => Err(error),
    };
    QuestionReviewResult {
        ordinal,
        questions,
        stage_label,
        events,
    }
}

/// Decode each expected answer independently after validating the shared wire
/// envelope. A malformed sibling must not erase a valid answer from the same
/// obligation review; the malformed entry alone fails closed to `bounded`.
pub(super) fn isolated_wire_question_resolution_events(
    value: Value,
    questions: &[Question],
    allowed_evidence_ids: &BTreeSet<String>,
) -> Vec<InquiryEvent> {
    let Some(root) = value.as_object() else {
        return bounded_review_events(questions);
    };
    if root.len() != 1 {
        return bounded_review_events(questions);
    }
    let Some(entries) = root.get("resolutions").and_then(Value::as_object) else {
        return bounded_review_events(questions);
    };
    let expected_ids = questions
        .iter()
        .map(|question| question.id.as_str())
        .collect::<BTreeSet<_>>();
    if entries
        .keys()
        .any(|question_id| !expected_ids.contains(question_id.as_str()))
    {
        return bounded_review_events(questions);
    }

    questions
        .iter()
        .map(|question| {
            let Some(entry) = entries.get(&question.id) else {
                return bounded_review_event(question);
            };
            let mut singleton_entries = Map::new();
            singleton_entries.insert(question.id.clone(), entry.clone());
            let mut singleton = Map::new();
            singleton.insert(
                "resolutions".to_string(),
                Value::Object(singleton_entries),
            );
            let resolution = decode_question_resolution(
                Value::Object(singleton),
                allowed_evidence_ids,
            );
            let Ok(resolution) = resolution else {
                return bounded_review_event(question);
            };
            isolated_question_resolution_events(
                &resolution,
                std::slice::from_ref(question),
                allowed_evidence_ids,
            )
            .into_iter()
            .next()
            .unwrap_or_else(|| bounded_review_event(question))
        })
        .collect()
}

fn bounded_review_events(questions: &[Question]) -> Vec<InquiryEvent> {
    questions.iter().map(bounded_review_event).collect()
}

fn bounded_review_event(question: &Question) -> InquiryEvent {
    InquiryEvent::QuestionBounded {
        question_id: question.id.clone(),
        reason: "closed-evidence assessment did not establish a valid, traceable answer for this question".to_string(),
    }
}

pub(super) fn isolated_question_resolution_events(
    output: &QuestionResolutionOutput,
    questions: &[Question],
    allowed_evidence_ids: &BTreeSet<String>,
) -> Vec<InquiryEvent> {
    questions
        .iter()
        .map(|question| {
            let matches = output
                .resolutions
                .iter()
                .filter(|resolution| match resolution {
                    QuestionResolution::Answered { question_id, .. }
                    | QuestionResolution::Partial { question_id, .. }
                    | QuestionResolution::Bounded { question_id, .. } => {
                        question_id == &question.id
                    }
                })
                .cloned()
                .collect::<Vec<_>>();
            if matches.len() == 1 {
                let single = QuestionResolutionOutput {
                    resolutions: matches,
                };
                if let Ok(mut events) = question_resolution_events(
                    &single,
                    std::slice::from_ref(question),
                    allowed_evidence_ids,
                ) {
                    if events.len() == 1 {
                        return events.remove(0);
                    }
                }
            }
            bounded_review_event(question)
        })
        .collect()
}

/// Questions linked to the same obligation contract share one closed-evidence
/// generation. This preserves exact question identities while avoiding
/// repeatedly sending the same evidence packet through a single-flight model.
pub(super) fn question_review_groups(queued: &[Question]) -> Vec<Vec<Question>> {
    let mut groups: Vec<Vec<Question>> = Vec::new();
    for question in queued {
        if let Some(group) = groups.iter_mut().find(|group| {
            group.first().is_some_and(|first| {
                first.obligation_ids == question.obligation_ids
            })
        }) {
            group.push(question.clone());
        } else {
            groups.push(vec![question.clone()]);
        }
    }
    groups
}

fn question_review_stage_label(ordinal: usize, question_count: usize) -> String {
    if question_count == 1 {
        "question-review".to_string()
    } else {
        format!("question-review-{}", ordinal + 1)
    }
}
