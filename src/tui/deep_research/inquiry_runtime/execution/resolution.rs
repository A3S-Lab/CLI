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

/// Decode each expected answer independently after validating the shared wire
/// envelope. A malformed sibling must not erase a valid answer from the same
/// obligation review; the malformed entry alone fails closed to `bounded`.
#[cfg(test)]
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
            let resolution =
                decode_question_resolution(Value::Object(singleton), allowed_evidence_ids);
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

#[cfg(test)]
fn bounded_review_events(questions: &[Question]) -> Vec<InquiryEvent> {
    questions.iter().map(bounded_review_event).collect()
}

#[cfg(test)]
fn bounded_review_event(question: &Question) -> InquiryEvent {
    InquiryEvent::QuestionBounded {
        question_id: question.id.clone(),
        reason: "closed-evidence assessment did not establish a valid, traceable answer for this question".to_string(),
    }
}

#[cfg(test)]
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
                    | QuestionResolution::Bounded { question_id, .. } => question_id == &question.id,
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
/// generation. This preserves exact question identities for replay tests.
#[cfg(test)]
pub(super) fn question_review_groups(queued: &[Question]) -> Vec<Vec<Question>> {
    let mut groups: Vec<Vec<Question>> = Vec::new();
    for question in queued {
        if let Some(group) = groups.iter_mut().find(|group| {
            group
                .first()
                .is_some_and(|first| first.obligation_ids == question.obligation_ids)
        }) {
            group.push(question.clone());
        } else {
            groups.push(vec![question.clone()]);
        }
    }
    groups
}
