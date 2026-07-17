//! Replayable projection for inquiry events.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    reduce, research_contract_outcome, EvidenceRef, InquiryAudit, InquiryBudgetInput,
    InquiryConvergenceInput, InquiryError, InquiryEvent, InquiryLimits, InquiryPhase,
    InquiryReplayError, OutlineValidationContext, Perspective, Question, QuestionStatus,
    ResearchContractAssessment, ResearchMethod, ResearchObligation, ResearchOutline, SectionDraft,
};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InquiryState {
    pub phase: InquiryPhase,
    pub method: Option<ResearchMethod>,
    #[serde(default)]
    pub obligations: Vec<ResearchObligation>,
    #[serde(default)]
    pub stop_conditions: Vec<String>,
    #[serde(default)]
    pub contract_assessment: Option<ResearchContractAssessment>,
    pub scout_completed: bool,
    pub scout_source_ids: Vec<String>,
    pub perspectives: Vec<Perspective>,
    pub questions: Vec<Question>,
    pub evidence_catalog: BTreeMap<String, EvidenceRef>,
    pub claim_catalog: BTreeSet<String>,
    pub source_catalog: BTreeSet<String>,
    pub outline: Option<ResearchOutline>,
    pub drafts: BTreeMap<String, SectionDraft>,
    pub audit: Option<InquiryAudit>,
    pub audit_attempts: usize,
    pub budget_exhausted_reason: Option<String>,
    pub events_applied: usize,
}

impl InquiryState {
    pub fn apply(
        &mut self,
        event: &InquiryEvent,
        limits: &InquiryLimits,
    ) -> Result<(), InquiryError> {
        *self = reduce(self, event, limits)?;
        Ok(())
    }

    pub fn question(&self, id: &str) -> Option<&Question> {
        self.questions.iter().find(|question| question.id == id)
    }

    pub fn evidence(&self, id: &str) -> Option<&EvidenceRef> {
        self.evidence_catalog.get(id)
    }

    pub fn outline_validation_context(&self) -> OutlineValidationContext {
        OutlineValidationContext {
            allowed_perspective_ids: self
                .perspectives
                .iter()
                .map(|perspective| perspective.id.clone())
                .collect(),
            allowed_question_ids: self
                .questions
                .iter()
                .map(|question| question.id.clone())
                .collect(),
            allowed_claim_ids: self.claim_catalog.clone(),
            allowed_source_ids: self.source_catalog.clone(),
            evidence_catalog: self.evidence_catalog.clone(),
            question_evidence_ids: self
                .questions
                .iter()
                .map(|question| {
                    (
                        question.id.clone(),
                        question.evidence_ids.iter().cloned().collect(),
                    )
                })
                .collect(),
            material_perspective_ids: self
                .perspectives
                .iter()
                .filter(|perspective| {
                    self.questions.iter().any(|question| {
                        question.material
                            && question.perspective_id.as_deref() == Some(perspective.id.as_str())
                    })
                })
                .map(|perspective| perspective.id.clone())
                .collect(),
            material_question_ids: self
                .questions
                .iter()
                .filter(|question| question.material)
                .map(|question| question.id.clone())
                .collect(),
            required_question_ids: self
                .questions
                .iter()
                .map(|question| question.id.clone())
                .collect(),
        }
    }

    pub fn budget_input(&self) -> InquiryBudgetInput {
        let answered_questions = self
            .questions
            .iter()
            .filter(|question| question.status == QuestionStatus::Answered)
            .count();
        let bounded_questions = self
            .questions
            .iter()
            .filter(|question| question.status == QuestionStatus::Bounded)
            .count();
        InquiryBudgetInput {
            events_applied: self.events_applied,
            scout_sources: self.scout_source_ids.len(),
            perspectives: self.perspectives.len(),
            questions: self.questions.len(),
            material_questions: self
                .questions
                .iter()
                .filter(|question| question.material)
                .count(),
            answered_questions,
            bounded_questions,
            outline_sections: self
                .outline
                .as_ref()
                .map_or(0, |outline| outline.sections.len()),
            drafted_sections: self.drafts.len(),
            answer_chars: self
                .questions
                .iter()
                .filter_map(|question| question.answer.as_deref())
                .map(char_count)
                .sum(),
            draft_chars: self
                .drafts
                .values()
                .map(|draft| char_count(&draft.content))
                .sum(),
            audit_attempts: self.audit_attempts,
        }
    }

    pub fn convergence_input(&self) -> InquiryConvergenceInput {
        let budget = self.budget_input();
        let unresolved_questions = budget
            .questions
            .saturating_sub(budget.answered_questions + budget.bounded_questions);
        let unresolved_material_questions = self
            .questions
            .iter()
            .filter(|question| question.material && question.status == QuestionStatus::Queued)
            .count();
        InquiryConvergenceInput {
            method: self.method,
            phase: self.phase,
            scout_completed: self.scout_completed,
            perspectives_required: self.method == Some(ResearchMethod::PerspectiveGuided),
            perspectives_committed: budget.perspectives,
            questions_queued: budget.questions,
            material_questions: budget.material_questions,
            questions_answered: budget.answered_questions,
            questions_bounded: budget.bounded_questions,
            unresolved_questions,
            unresolved_material_questions,
            research_obligations: self.obligations.len(),
            material_obligations: self
                .obligations
                .iter()
                .filter(|obligation| obligation.material)
                .count(),
            contract_assessed: self.contract_assessment.is_some(),
            contract_outcome: research_contract_outcome(self),
            outline_sections: budget.outline_sections,
            drafted_sections: budget.drafted_sections,
            undrafted_sections: budget
                .outline_sections
                .saturating_sub(budget.drafted_sections),
            audit_passed: self.audit.as_ref().map(|audit| audit.passed),
            budget_exhausted: self.phase == InquiryPhase::Exhausted,
        }
    }
}

pub fn replay(
    events: &[InquiryEvent],
    limits: &InquiryLimits,
) -> Result<InquiryState, InquiryReplayError> {
    events
        .iter()
        .enumerate()
        .try_fold(InquiryState::default(), |state, (event_index, event)| {
            reduce(&state, event, limits).map_err(|error| InquiryReplayError { event_index, error })
        })
}

fn char_count(value: &str) -> usize {
    value.chars().count()
}
