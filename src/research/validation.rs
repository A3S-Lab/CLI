//! Structural and hard-limit checks shared by reducer transitions.

use std::collections::HashSet;

use super::{
    InquiryError, InquiryLimits, InquiryState, OutlineSection, Question, QuestionStatus,
    ResearchMethod, ResearchObligation, ResearchOutline,
};

pub(super) fn validate_research_obligations(
    obligations: &[ResearchObligation],
    stop_conditions: &[String],
    limits: &InquiryLimits,
) -> Result<(), InquiryError> {
    ensure_nonempty(obligations, "research obligations")?;
    ensure_limit(
        "research obligations",
        obligations.len(),
        limits.max_obligations,
    )?;
    ensure_nonempty(stop_conditions, "research stop conditions")?;
    ensure_limit(
        "research stop conditions",
        stop_conditions.len(),
        limits.max_stop_conditions,
    )?;
    ensure_strings(
        stop_conditions,
        "research stop condition",
        limits.max_text_chars,
    )?;
    ensure_unique_ids(
        obligations.iter().map(|obligation| obligation.id.as_str()),
        "research obligation",
    )?;

    let mut material = 0usize;
    for obligation in obligations {
        ensure_string(
            &obligation.id,
            "research obligation id",
            limits.max_identifier_chars,
        )?;
        ensure_string(
            &obligation.title,
            "research obligation title",
            limits.max_text_chars,
        )?;
        ensure_string(
            &obligation.focus,
            "research obligation focus",
            limits.max_text_chars,
        )?;
        ensure_nonempty(
            &obligation.completion_criteria,
            "research obligation completion criteria",
        )?;
        ensure_limit(
            "research obligation completion criteria",
            obligation.completion_criteria.len(),
            limits.max_completion_criteria_per_obligation,
        )?;
        ensure_strings(
            &obligation.completion_criteria,
            "research obligation completion criterion",
            limits.max_text_chars,
        )?;
        material += usize::from(obligation.material);
    }
    if material == 0 {
        return Err(InquiryError::InvalidResearchPlan {
            reason: "at least one research obligation must be material".to_string(),
        });
    }
    Ok(())
}

pub(super) fn validate_queued_questions(
    state: &InquiryState,
    questions: &[Question],
    limits: &InquiryLimits,
) -> Result<(), InquiryError> {
    ensure_nonempty(questions, "questions")?;
    ensure_limit(
        "questions",
        state.questions.len().saturating_add(questions.len()),
        limits.max_questions,
    )?;
    let existing_question_ids = state
        .questions
        .iter()
        .map(|question| question.id.as_str())
        .collect::<HashSet<_>>();
    let mut all_question_ids = existing_question_ids.clone();
    let perspective_ids = state
        .perspectives
        .iter()
        .map(|perspective| perspective.id.as_str())
        .collect::<HashSet<_>>();
    let obligation_ids = state
        .obligations
        .iter()
        .map(|obligation| obligation.id.as_str())
        .collect::<HashSet<_>>();
    for question in questions {
        if !all_question_ids.insert(&question.id) {
            return Err(InquiryError::DuplicateId {
                resource: "question",
                id: question.id.clone(),
            });
        }
        if question.status != QuestionStatus::Queued
            || question.answer.is_some()
            || question.bound_reason.is_some()
            || !question.evidence_ids.is_empty()
        {
            return Err(InquiryError::InvalidQuestionState {
                id: question.id.clone(),
                status: question.status,
            });
        }
        ensure_string(&question.id, "question id", limits.max_identifier_chars)?;
        ensure_string(&question.prompt, "question prompt", limits.max_text_chars)?;
        if obligation_ids.is_empty() && !question.obligation_ids.is_empty() {
            return Err(InquiryError::InvalidResearchPlan {
                reason: format!(
                    "question `{}` references research obligations before a contract was committed",
                    question.id
                ),
            });
        }
        if !obligation_ids.is_empty() {
            ensure_nonempty(&question.obligation_ids, "question obligation ids")?;
            ensure_limit(
                "question obligation ids",
                question.obligation_ids.len(),
                limits.max_obligations,
            )?;
            ensure_unique_ids(
                question.obligation_ids.iter().map(String::as_str),
                "question obligation",
            )?;
            for obligation_id in &question.obligation_ids {
                if !obligation_ids.contains(obligation_id.as_str()) {
                    return Err(InquiryError::UnknownId {
                        resource: "research obligation",
                        id: obligation_id.clone(),
                    });
                }
            }
        }
        ensure_limit(
            "question round",
            question.round as usize,
            limits.max_question_round,
        )?;
        if let Some(parent_id) = question.parent_question_id.as_deref() {
            ensure_string(parent_id, "parent question id", limits.max_identifier_chars)?;
            if !existing_question_ids.contains(parent_id) {
                return Err(InquiryError::UnknownId {
                    resource: "parent question",
                    id: parent_id.to_string(),
                });
            }
            if !obligation_ids.is_empty() {
                let parent = state
                    .questions
                    .iter()
                    .find(|candidate| candidate.id == parent_id)
                    .ok_or_else(|| InquiryError::UnknownId {
                        resource: "parent question",
                        id: parent_id.to_string(),
                    })?;
                let parent_obligations = parent
                    .obligation_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<HashSet<_>>();
                let follow_up_obligations = question
                    .obligation_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<HashSet<_>>();
                if parent_obligations != follow_up_obligations {
                    return Err(InquiryError::InvalidResearchPlan {
                        reason: format!(
                            "follow-up question `{}` changed the obligations of parent `{parent_id}`",
                            question.id
                        ),
                    });
                }
                let expected_round = parent.round.checked_add(1).ok_or_else(|| {
                    InquiryError::InvalidResearchPlan {
                        reason: format!(
                            "parent question `{parent_id}` cannot advance beyond u32::MAX"
                        ),
                    }
                })?;
                if question.round != expected_round {
                    return Err(InquiryError::InvalidResearchPlan {
                        reason: format!(
                            "follow-up question `{}` must use parent round + 1 ({expected_round})",
                            question.id
                        ),
                    });
                }
                if question.perspective_id != parent.perspective_id {
                    return Err(InquiryError::InvalidResearchPlan {
                        reason: format!(
                            "follow-up question `{}` changed the perspective of parent `{parent_id}`",
                            question.id
                        ),
                    });
                }
            }
        }
        match question.perspective_id.as_deref() {
            Some(id) if perspective_ids.contains(id) => {}
            Some(id) => {
                return Err(InquiryError::UnknownId {
                    resource: "perspective",
                    id: id.to_string(),
                });
            }
            None if state.method == Some(ResearchMethod::PerspectiveGuided) => {
                return Err(InquiryError::PerspectiveRequired {
                    question_id: question.id.clone(),
                });
            }
            None => {}
        }
    }
    Ok(())
}

pub(super) fn validate_outline_limits(
    outline: &ResearchOutline,
    limits: &InquiryLimits,
) -> Result<(), InquiryError> {
    ensure_nonempty(&outline.sections, "outline sections")?;
    ensure_limit(
        "outline sections",
        outline.sections.len(),
        limits.max_outline_sections,
    )?;
    ensure_unique_ids(
        outline.sections.iter().map(|section| section.id.as_str()),
        "outline section",
    )?;
    for section in &outline.sections {
        ensure_string(
            &section.id,
            "outline section id",
            limits.max_identifier_chars,
        )?;
        for (resource, value) in [
            ("outline heading", section.heading.as_str()),
            ("outline purpose", section.purpose.as_str()),
            ("composition hint", section.composition_hint.as_str()),
        ] {
            ensure_string(value, resource, limits.max_text_chars)?;
        }
        ensure_limit(
            "outline perspective ids",
            section.perspective_ids.len(),
            limits.max_perspectives,
        )?;
        ensure_limit(
            "outline question ids",
            section.question_ids.len(),
            limits.max_questions,
        )?;
        for ids in [
            &section.perspective_ids,
            &section.question_ids,
            &section.claim_ids,
            &section.source_ids,
        ] {
            ensure_strings(ids, "outline reference id", limits.max_identifier_chars)?;
        }
    }
    Ok(())
}

pub(super) fn validate_section_citations(
    section: &OutlineSection,
    citation_ids: &[String],
    limits: &InquiryLimits,
) -> Result<(), InquiryError> {
    ensure_nonempty(citation_ids, "section citation ids")?;
    ensure_limit(
        "section citation ids",
        citation_ids.len(),
        limits.max_citation_ids_per_section,
    )?;
    ensure_unique_ids(citation_ids.iter().map(String::as_str), "citation")?;
    ensure_strings(citation_ids, "citation id", limits.max_identifier_chars)?;

    let claim_ids = section
        .claim_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let source_ids = section
        .source_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for citation_id in citation_ids {
        if !claim_ids.contains(citation_id.as_str()) && !source_ids.contains(citation_id.as_str()) {
            return Err(InquiryError::UnknownId {
                resource: "outline section citation",
                id: citation_id.clone(),
            });
        }
    }
    if !citation_ids
        .iter()
        .any(|citation_id| source_ids.contains(citation_id.as_str()))
    {
        return Err(InquiryError::MissingSourceCitation {
            section_id: section.id.clone(),
        });
    }
    Ok(())
}

pub(super) fn ensure_limit(
    resource: &'static str,
    actual: usize,
    limit: usize,
) -> Result<(), InquiryError> {
    if actual <= limit {
        Ok(())
    } else {
        Err(InquiryError::HardLimitExceeded {
            resource,
            limit,
            actual,
        })
    }
}

pub(super) fn ensure_string(
    value: &str,
    resource: &'static str,
    limit: usize,
) -> Result<(), InquiryError> {
    if value.trim().is_empty() {
        return Err(InquiryError::EmptyValue { resource });
    }
    ensure_limit(resource, char_count(value), limit)
}

pub(super) fn ensure_strings(
    values: &[String],
    resource: &'static str,
    limit: usize,
) -> Result<(), InquiryError> {
    values
        .iter()
        .try_for_each(|value| ensure_string(value, resource, limit))
}

pub(super) fn ensure_unique_ids<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    resource: &'static str,
) -> Result<(), InquiryError> {
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(InquiryError::DuplicateId {
                resource,
                id: id.to_string(),
            });
        }
    }
    Ok(())
}

pub(super) fn ensure_nonempty<T>(values: &[T], resource: &'static str) -> Result<(), InquiryError> {
    if values.is_empty() {
        Err(InquiryError::EmptyBatch { resource })
    } else {
        Ok(())
    }
}

fn char_count(value: &str) -> usize {
    value.chars().count()
}
