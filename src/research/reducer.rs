//! Transition and hard-limit validation for the inquiry projection.

use std::collections::HashSet;

use super::validation::{
    ensure_limit, ensure_nonempty, ensure_string, ensure_strings, ensure_unique_ids,
    validate_outline_limits, validate_queued_questions, validate_research_obligations,
    validate_section_citations,
};
use super::{
    research_contract_outcome, validate_research_contract_assessment, validate_research_outline,
    InquiryAudit, InquiryError, InquiryEvent, InquiryLimits, InquiryPhase, InquiryState,
    QuestionStatus, ResearchContractOutcome, ResearchMethod, SectionDraft,
};

pub fn reduce(
    state: &InquiryState,
    event: &InquiryEvent,
    limits: &InquiryLimits,
) -> Result<InquiryState, InquiryError> {
    ensure_limit(
        "events",
        state.events_applied.saturating_add(1),
        limits.max_events,
    )?;
    if state.phase.is_terminal() {
        return Err(invalid_transition(state, event));
    }

    let mut next = state.clone();
    match event {
        InquiryEvent::StrategySelected { method } => {
            require_phase(state, event, &[InquiryPhase::StrategySelection])?;
            next.method = Some(*method);
            next.phase = match method {
                ResearchMethod::Focused => InquiryPhase::Questioning,
                ResearchMethod::PerspectiveGuided => InquiryPhase::Scouting,
            };
        }
        InquiryEvent::ResearchObligationsCommitted {
            obligations,
            stop_conditions,
        } => {
            require_phase(
                state,
                event,
                &[InquiryPhase::Scouting, InquiryPhase::Questioning],
            )?;
            if !state.obligations.is_empty()
                || !state.stop_conditions.is_empty()
                || state.scout_completed
                || !state.perspectives.is_empty()
                || !state.questions.is_empty()
                || !state.evidence_catalog.is_empty()
            {
                return Err(invalid_transition(state, event));
            }
            validate_research_obligations(obligations, stop_conditions, limits)?;
            next.obligations.clone_from(obligations);
            next.stop_conditions.clone_from(stop_conditions);
        }
        InquiryEvent::ScoutCompleted { source_ids } => {
            require_phase(state, event, &[InquiryPhase::Scouting])?;
            ensure_nonempty(source_ids, "scout sources")?;
            ensure_limit("scout sources", source_ids.len(), limits.max_scout_sources)?;
            ensure_unique_ids(source_ids.iter().map(String::as_str), "scout source")?;
            ensure_strings(source_ids, "scout source", limits.max_text_chars)?;
            next.scout_completed = true;
            next.scout_source_ids.clone_from(source_ids);
            next.phase = InquiryPhase::PerspectiveDiscovery;
        }
        InquiryEvent::PerspectivesCommitted { perspectives } => {
            require_phase(state, event, &[InquiryPhase::PerspectiveDiscovery])?;
            ensure_nonempty(perspectives, "perspectives")?;
            ensure_limit("perspectives", perspectives.len(), limits.max_perspectives)?;
            ensure_unique_ids(
                perspectives
                    .iter()
                    .map(|perspective| perspective.id.as_str()),
                "perspective",
            )?;
            let scout_sources = state
                .scout_source_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            for perspective in perspectives {
                ensure_string(
                    &perspective.id,
                    "perspective id",
                    limits.max_identifier_chars,
                )?;
                ensure_string(
                    &perspective.title,
                    "perspective title",
                    limits.max_text_chars,
                )?;
                ensure_string(
                    &perspective.focus,
                    "perspective focus",
                    limits.max_text_chars,
                )?;
                ensure_nonempty(&perspective.source_ids, "perspective source ids")?;
                ensure_limit(
                    "perspective source ids",
                    perspective.source_ids.len(),
                    limits.max_scout_sources,
                )?;
                ensure_unique_ids(
                    perspective.source_ids.iter().map(String::as_str),
                    "perspective source",
                )?;
                for source_id in &perspective.source_ids {
                    if !scout_sources.contains(source_id.as_str()) {
                        return Err(InquiryError::UnknownId {
                            resource: "scout source",
                            id: source_id.clone(),
                        });
                    }
                }
            }
            next.perspectives.clone_from(perspectives);
            next.phase = InquiryPhase::Questioning;
        }
        InquiryEvent::QuestionsQueued { questions } => {
            require_phase(
                state,
                event,
                &[InquiryPhase::Questioning, InquiryPhase::Outlining],
            )?;
            if state.outline.is_some() {
                return Err(invalid_transition(state, event));
            }
            validate_queued_questions(state, questions, limits)?;
            next.questions.extend(questions.iter().cloned());
            next.phase = InquiryPhase::Questioning;
        }
        InquiryEvent::EvidenceAccepted { evidence } => {
            require_phase(
                state,
                event,
                &[
                    InquiryPhase::Scouting,
                    InquiryPhase::PerspectiveDiscovery,
                    InquiryPhase::Questioning,
                ],
            )?;
            ensure_string(
                &evidence.evidence_id,
                "evidence id",
                limits.max_identifier_chars,
            )?;
            ensure_nonempty(&evidence.claim_ids, "evidence claim ids")?;
            ensure_nonempty(&evidence.source_ids, "evidence source ids")?;
            ensure_limit(
                "evidence claim ids",
                evidence.claim_ids.len(),
                limits.max_citation_ids_per_section,
            )?;
            ensure_limit(
                "evidence source ids",
                evidence.source_ids.len(),
                limits.max_citation_ids_per_section,
            )?;
            ensure_unique_ids(
                evidence.claim_ids.iter().map(String::as_str),
                "evidence claim",
            )?;
            ensure_unique_ids(
                evidence.source_ids.iter().map(String::as_str),
                "evidence source",
            )?;
            ensure_strings(
                &evidence.claim_ids,
                "evidence claim id",
                limits.max_identifier_chars,
            )?;
            ensure_strings(
                &evidence.source_ids,
                "evidence source id",
                limits.max_identifier_chars,
            )?;
            ensure_limit(
                "evidence diagnostics",
                evidence.diagnostics.len(),
                limits.max_citation_ids_per_section,
            )?;
            ensure_unique_ids(
                evidence
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.id.as_str()),
                "evidence diagnostic",
            )?;
            for diagnostic in &evidence.diagnostics {
                ensure_string(
                    &diagnostic.id,
                    "evidence diagnostic id",
                    limits.max_identifier_chars,
                )?;
                ensure_string(
                    &diagnostic.detail,
                    "evidence diagnostic detail",
                    limits.max_text_chars,
                )?;
            }
            if let Some(accepted) = state.evidence_catalog.get(&evidence.evidence_id) {
                return if accepted == evidence {
                    Err(InquiryError::DuplicateId {
                        resource: "evidence",
                        id: evidence.evidence_id.clone(),
                    })
                } else {
                    Err(InquiryError::ConflictingEvidence {
                        id: evidence.evidence_id.clone(),
                    })
                };
            }
            next.claim_catalog
                .extend(evidence.claim_ids.iter().cloned());
            next.source_catalog
                .extend(evidence.source_ids.iter().cloned());
            next.evidence_catalog
                .insert(evidence.evidence_id.clone(), evidence.clone());
        }
        InquiryEvent::QuestionAnswered {
            question_id,
            answer,
            evidence_ids,
        } => {
            require_phase(state, event, &[InquiryPhase::Questioning])?;
            ensure_string(question_id, "question id", limits.max_identifier_chars)?;
            ensure_string(answer, "question answer", limits.max_answer_chars)?;
            ensure_nonempty(evidence_ids, "answer evidence ids")?;
            ensure_limit(
                "answer evidence ids",
                evidence_ids.len(),
                limits.max_evidence_ids_per_answer,
            )?;
            ensure_unique_ids(evidence_ids.iter().map(String::as_str), "evidence")?;
            ensure_strings(evidence_ids, "evidence id", limits.max_identifier_chars)?;
            for evidence_id in evidence_ids {
                if !state.evidence_catalog.contains_key(evidence_id) {
                    return Err(InquiryError::UnknownId {
                        resource: "accepted evidence",
                        id: evidence_id.clone(),
                    });
                }
            }
            let question = queued_question_mut(&mut next, question_id)?;
            question.status = QuestionStatus::Answered;
            question.answer = Some(answer.clone());
            question.bound_reason = None;
            question.evidence_ids.clone_from(evidence_ids);
            let answer_chars = next
                .questions
                .iter()
                .filter_map(|question| question.answer.as_deref())
                .map(char_count)
                .sum();
            ensure_limit(
                "total answer chars",
                answer_chars,
                limits.max_total_answer_chars,
            )?;
            advance_after_question_resolution(&mut next);
        }
        InquiryEvent::QuestionDeferred {
            question_id,
            reason,
        } => {
            require_phase(state, event, &[InquiryPhase::Questioning])?;
            ensure_string(question_id, "question id", limits.max_identifier_chars)?;
            ensure_string(reason, "question defer reason", limits.max_text_chars)?;
            let question = queued_question_mut(&mut next, question_id)?;
            question.bound_reason = Some(reason.clone());
        }
        InquiryEvent::QuestionBounded {
            question_id,
            reason,
        } => {
            require_phase(state, event, &[InquiryPhase::Questioning])?;
            ensure_string(question_id, "question id", limits.max_identifier_chars)?;
            ensure_string(reason, "question bound reason", limits.max_text_chars)?;
            let question = queued_question_mut(&mut next, question_id)?;
            question.status = QuestionStatus::Bounded;
            question.bound_reason = Some(reason.clone());
            advance_after_question_resolution(&mut next);
        }
        InquiryEvent::ResearchContractAssessed { assessment } => {
            require_phase(state, event, &[InquiryPhase::Outlining])?;
            if state.contract_assessment.is_some() {
                return Err(invalid_transition(state, event));
            }
            validate_obligation_coverage(state)?;
            validate_research_contract_assessment(state, assessment).map_err(|error| {
                InquiryError::InvalidResearchPlan {
                    reason: error.to_string(),
                }
            })?;
            next.contract_assessment = Some(assessment.clone());
        }
        InquiryEvent::OutlineCommitted { outline } => {
            require_phase(state, event, &[InquiryPhase::Outlining])?;
            let unresolved = unresolved_questions(state);
            if unresolved > 0 {
                return Err(InquiryError::UnresolvedQuestions { count: unresolved });
            }
            let material_questions = state
                .questions
                .iter()
                .filter(|question| question.material)
                .count();
            let unresolved_material = state
                .questions
                .iter()
                .filter(|question| question.material && question.status != QuestionStatus::Answered)
                .count();
            if material_questions == 0 {
                return Err(InquiryError::InvalidOutline {
                    reason:
                        "at least one material research question must be answered before outlining"
                            .to_string(),
                });
            }
            if unresolved_material > 0 {
                return Err(InquiryError::InvalidOutline {
                    reason: format!(
                        "every material research question must be answered before outlining; {unresolved_material} remain bounded"
                    ),
                });
            }
            validate_obligation_coverage(state)?;
            if !state.obligations.is_empty()
                && !matches!(
                    research_contract_outcome(state),
                    Some(ResearchContractOutcome::Satisfied | ResearchContractOutcome::Qualified)
                )
            {
                return Err(InquiryError::InvalidOutline {
                    reason:
                        "research completion criteria and stop conditions have not been satisfied"
                            .to_string(),
                });
            }
            validate_outline_limits(outline, limits)?;
            validate_research_outline(outline, &state.outline_validation_context()).map_err(
                |error| InquiryError::InvalidOutline {
                    reason: error.to_string(),
                },
            )?;
            next.outline = Some(outline.clone());
            next.phase = InquiryPhase::Drafting;
        }
        InquiryEvent::SectionDrafted {
            section_id,
            content,
            citation_ids,
        } => {
            require_phase(
                state,
                event,
                &[InquiryPhase::Drafting, InquiryPhase::Auditing],
            )?;
            ensure_string(
                section_id,
                "outline section id",
                limits.max_identifier_chars,
            )?;
            ensure_string(content, "section content", limits.max_section_chars)?;
            let outline = state
                .outline
                .as_ref()
                .ok_or_else(|| invalid_transition(state, event))?;
            let section = outline
                .sections
                .iter()
                .find(|section| section.id == *section_id)
                .ok_or_else(|| InquiryError::UnknownId {
                    resource: "outline section",
                    id: section_id.clone(),
                })?;
            validate_section_citations(section, citation_ids, limits)?;
            next.drafts.insert(
                section_id.clone(),
                SectionDraft {
                    section_id: section_id.clone(),
                    content: content.clone(),
                    citation_ids: citation_ids.clone(),
                },
            );
            let draft_chars = next
                .drafts
                .values()
                .map(|draft| char_count(&draft.content))
                .sum();
            ensure_limit(
                "total draft chars",
                draft_chars,
                limits.max_total_draft_chars,
            )?;
            next.audit = None;
            next.phase = if next.drafts.len() == outline.sections.len() {
                InquiryPhase::Auditing
            } else {
                InquiryPhase::Drafting
            };
        }
        InquiryEvent::AuditCompleted { passed, issues } => {
            require_phase(state, event, &[InquiryPhase::Auditing])?;
            let missing = state.outline.as_ref().map_or(0, |outline| {
                outline.sections.len().saturating_sub(state.drafts.len())
            });
            if missing > 0 {
                return Err(InquiryError::IncompleteSections { count: missing });
            }
            ensure_limit("audit issues", issues.len(), limits.max_audit_issues)?;
            ensure_strings(issues, "audit issue", limits.max_text_chars)?;
            ensure_limit(
                "audit attempts",
                state.audit_attempts.saturating_add(1),
                limits.max_audit_attempts,
            )?;
            next.audit = Some(InquiryAudit {
                passed: *passed,
                issues: issues.clone(),
            });
            next.audit_attempts += 1;
            next.phase = if *passed {
                InquiryPhase::Completed
            } else {
                InquiryPhase::Drafting
            };
        }
        InquiryEvent::BudgetExhausted { reason } => {
            ensure_string(reason, "budget exhaustion reason", limits.max_text_chars)?;
            next.budget_exhausted_reason = Some(reason.clone());
            next.phase = InquiryPhase::Exhausted;
        }
    }
    next.events_applied += 1;
    Ok(next)
}

fn validate_obligation_coverage(state: &InquiryState) -> Result<(), InquiryError> {
    if state.obligations.is_empty() {
        return Ok(());
    }
    for obligation in &state.obligations {
        let linked = state
            .questions
            .iter()
            .filter(|question| question.obligation_ids.contains(&obligation.id))
            .collect::<Vec<_>>();
        if linked.is_empty() {
            return Err(InquiryError::InvalidOutline {
                reason: format!(
                    "research obligation `{}` has no traceable question path",
                    obligation.id
                ),
            });
        }
        if obligation.material {
            let material = linked
                .iter()
                .filter(|question| question.material)
                .collect::<Vec<_>>();
            if material.is_empty() {
                return Err(InquiryError::InvalidOutline {
                    reason: format!(
                        "material research obligation `{}` has no material question",
                        obligation.id
                    ),
                });
            }
            let unresolved = material
                .iter()
                .filter(|question| question.status != QuestionStatus::Answered)
                .count();
            if unresolved > 0 {
                return Err(InquiryError::InvalidOutline {
                    reason: format!(
                        "material research obligation `{}` has {unresolved} unanswered material question(s)",
                        obligation.id
                    ),
                });
            }
        }
    }
    Ok(())
}

fn queued_question_mut<'a>(
    state: &'a mut InquiryState,
    id: &str,
) -> Result<&'a mut super::Question, InquiryError> {
    let question = state
        .questions
        .iter_mut()
        .find(|question| question.id == id)
        .ok_or_else(|| InquiryError::UnknownId {
            resource: "question",
            id: id.to_string(),
        })?;
    if question.status != QuestionStatus::Queued {
        return Err(InquiryError::InvalidQuestionState {
            id: id.to_string(),
            status: question.status,
        });
    }
    Ok(question)
}

fn advance_after_question_resolution(state: &mut InquiryState) {
    if !state.questions.is_empty() && unresolved_questions(state) == 0 {
        state.phase = InquiryPhase::Outlining;
    }
}

fn unresolved_questions(state: &InquiryState) -> usize {
    state
        .questions
        .iter()
        .filter(|question| question.status == QuestionStatus::Queued)
        .count()
}

fn require_phase(
    state: &InquiryState,
    event: &InquiryEvent,
    allowed: &[InquiryPhase],
) -> Result<(), InquiryError> {
    if allowed.contains(&state.phase) {
        Ok(())
    } else {
        Err(invalid_transition(state, event))
    }
}

fn invalid_transition(state: &InquiryState, event: &InquiryEvent) -> InquiryError {
    InquiryError::InvalidTransition {
        phase: state.phase,
        event: event.name(),
    }
}

fn char_count(value: &str) -> usize {
    value.chars().count()
}
