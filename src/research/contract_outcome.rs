//! Deterministic quality classification for an assessed research contract.
//!
//! `InquiryPhase::Completed` records that report production and audit finished.
//! This module separately classifies whether the closed evidence fully
//! satisfied the research contract or supports only a qualified report.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    ContractAssessmentStatus, DiagnosticDisposition, EvidenceRequirementAssessment, InquiryState,
    QuestionStatus, ResearchContractOutcome, ResearchObligation, ResearchObligationAssessment,
};

/// Return whether the inquiry has at least one traceable answer on a material
/// obligation path. Other material obligations may remain explicitly bounded:
/// a qualified report is useful precisely because it preserves supported
/// findings without pretending that every requested dimension was covered.
/// No material answer at all remains an unsatisfied research run.
pub fn material_evidence_floor(state: &InquiryState) -> bool {
    let material_obligations = state
        .obligations
        .iter()
        .filter(|obligation| obligation.material)
        .collect::<Vec<_>>();
    if material_obligations.is_empty() {
        return state
            .questions
            .iter()
            .any(|question| question.material && answered_question_is_traceable(state, question));
    }
    material_obligations.iter().any(|obligation| {
        state.questions.iter().any(|question| {
            question.material
                && question.obligation_ids.contains(&obligation.id)
                && answered_question_is_traceable(state, question)
        })
    })
}

fn answered_question_is_traceable(state: &InquiryState, question: &super::Question) -> bool {
    question.status == QuestionStatus::Answered
        && !question.evidence_ids.is_empty()
        && question.evidence_ids.iter().all(|evidence_id| {
            state
                .evidence_catalog
                .get(evidence_id)
                .is_some_and(|evidence| {
                    !evidence.claim_ids.is_empty() && !evidence.source_ids.is_empty()
                })
        })
}

/// Classify the validated closed-evidence assessment without introducing a
/// second terminal state machine. Semantic sufficiency comes from the model's
/// assessment; the host enforces the replayable minimum evidence floor.
pub fn research_contract_outcome(state: &InquiryState) -> Option<ResearchContractOutcome> {
    if state.obligations.is_empty() {
        return None;
    }
    let assessment = state.contract_assessment.as_ref()?;
    let obligation_assessments = assessment
        .obligations
        .iter()
        .map(|item| (item.obligation_id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    let bounded_diagnostics = assessment
        .diagnostics
        .iter()
        .filter(|item| item.disposition == DiagnosticDisposition::Bounded)
        .flat_map(|item| item.obligation_ids.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();

    if !material_evidence_floor(state) {
        return Some(ResearchContractOutcome::Unsatisfied);
    }

    let mut qualified = state.questions.iter().any(|question| {
        question.status == QuestionStatus::Bounded
            || (question.status == QuestionStatus::Answered && question.bound_reason.is_some())
    }) || !bounded_diagnostics.is_empty();
    for obligation in &state.obligations {
        let item = obligation_assessments.get(obligation.id.as_str())?;
        if obligation.material {
            if material_obligation_is_uncovered(obligation, item) {
                qualified = true;
                continue;
            }
            qualified |= !material_obligation_is_satisfied(obligation, item, &bounded_diagnostics);
        } else {
            qualified |= !material_obligation_is_satisfied(obligation, item, &bounded_diagnostics);
        }
    }

    for condition in &assessment.stop_conditions {
        match condition.status {
            ContractAssessmentStatus::Satisfied => {}
            ContractAssessmentStatus::Bounded => qualified = true,
            ContractAssessmentStatus::Uncovered => {
                qualified = true;
            }
        }
    }

    Some(if qualified {
        ResearchContractOutcome::Qualified
    } else {
        ResearchContractOutcome::Satisfied
    })
}

fn material_obligation_is_uncovered(
    obligation: &ResearchObligation,
    assessment: &ResearchObligationAssessment,
) -> bool {
    assessment
        .criteria
        .iter()
        .any(|criterion| criterion.status == ContractAssessmentStatus::Uncovered)
        || required_assessment_is_uncovered(
            obligation.evidence_requirements.primary_source_required,
            assessment.primary_source.as_ref(),
        )
        || required_assessment_is_uncovered(
            obligation
                .evidence_requirements
                .independent_corroboration_required,
            assessment.independent_corroboration.as_ref(),
        )
}

fn required_assessment_is_uncovered(
    required: bool,
    assessment: Option<&EvidenceRequirementAssessment>,
) -> bool {
    required
        && !assessment
            .is_some_and(|assessment| assessment.status != ContractAssessmentStatus::Uncovered)
}

fn material_obligation_is_satisfied(
    obligation: &ResearchObligation,
    assessment: &ResearchObligationAssessment,
    bounded_diagnostics: &BTreeSet<&str>,
) -> bool {
    assessment
        .criteria
        .iter()
        .all(|criterion| criterion.status == ContractAssessmentStatus::Satisfied)
        && evidence_requirements_satisfied(obligation, assessment)
        && !bounded_diagnostics.contains(obligation.id.as_str())
}

fn evidence_requirements_satisfied(
    obligation: &ResearchObligation,
    assessment: &ResearchObligationAssessment,
) -> bool {
    (!obligation.evidence_requirements.primary_source_required
        || assessment
            .primary_source
            .as_ref()
            .is_some_and(|item| item.status == ContractAssessmentStatus::Satisfied))
        && (!obligation
            .evidence_requirements
            .independent_corroboration_required
            || assessment
                .independent_corroboration
                .as_ref()
                .is_some_and(|item| item.status == ContractAssessmentStatus::Satisfied))
}
