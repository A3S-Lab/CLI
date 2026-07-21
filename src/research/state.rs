//! Replayable projection for inquiry events.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    reduce, EvidenceRef, InquiryAudit, InquiryError, InquiryEvent, InquiryLimits, InquiryPhase,
    InquiryReplayError, OutlineValidationContext, Perspective, Question, QuestionStatus,
    ResearchContractAssessment, ResearchMethod, ResearchObligation, ResearchOutline, SectionDraft,
    SectionRevision,
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
    /// Legacy journal replay state; never populated by a new host inquiry.
    pub scout_completed: bool,
    /// Legacy journal replay state; never populated by a new host inquiry.
    pub scout_source_ids: Vec<String>,
    /// Legacy journal replay state; never populated by a new host inquiry.
    #[serde(default)]
    pub perspective_retrieval_wave_budget: Option<u8>,
    /// Legacy journal replay state; never populated by a new host inquiry.
    pub perspectives: Vec<Perspective>,
    pub questions: Vec<Question>,
    pub evidence_catalog: BTreeMap<String, EvidenceRef>,
    pub claim_catalog: BTreeSet<String>,
    pub source_catalog: BTreeSet<String>,
    pub outline: Option<ResearchOutline>,
    pub drafts: BTreeMap<String, SectionDraft>,
    #[serde(default)]
    pub section_revisions: Vec<SectionRevision>,
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

    pub fn active_section_revision(&self) -> Option<&SectionRevision> {
        self.section_revisions
            .last()
            .filter(|revision| !revision.committed)
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
                .filter(|question| question.material && question.status == QuestionStatus::Answered)
                .map(|question| question.id.clone())
                .collect(),
            required_question_ids: self
                .questions
                .iter()
                .map(|question| question.id.clone())
                .collect(),
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
