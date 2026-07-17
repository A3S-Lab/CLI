//! Facts accepted by the inquiry reducer.

use serde::{Deserialize, Serialize};

use super::{
    EvidenceRef, Perspective, Question, ResearchContractAssessment, ResearchMethod,
    ResearchObligation, ResearchOutline,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InquiryEvent {
    StrategySelected {
        method: ResearchMethod,
    },
    ResearchObligationsCommitted {
        obligations: Vec<ResearchObligation>,
        stop_conditions: Vec<String>,
    },
    ScoutCompleted {
        source_ids: Vec<String>,
    },
    PerspectivesCommitted {
        perspectives: Vec<Perspective>,
    },
    QuestionsQueued {
        questions: Vec<Question>,
    },
    EvidenceAccepted {
        evidence: EvidenceRef,
    },
    QuestionAnswered {
        question_id: String,
        answer: String,
        evidence_ids: Vec<String>,
    },
    /// Records that the current evidence wave could not answer a question,
    /// while keeping it open because a model-authored follow-up was queued.
    QuestionDeferred {
        question_id: String,
        reason: String,
    },
    QuestionBounded {
        question_id: String,
        reason: String,
    },
    ResearchContractAssessed {
        assessment: ResearchContractAssessment,
    },
    OutlineCommitted {
        outline: ResearchOutline,
    },
    SectionDrafted {
        section_id: String,
        content: String,
        citation_ids: Vec<String>,
    },
    AuditCompleted {
        passed: bool,
        issues: Vec<String>,
    },
    BudgetExhausted {
        reason: String,
    },
}

impl InquiryEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::StrategySelected { .. } => "strategy_selected",
            Self::ResearchObligationsCommitted { .. } => "research_obligations_committed",
            Self::ScoutCompleted { .. } => "scout_completed",
            Self::PerspectivesCommitted { .. } => "perspectives_committed",
            Self::QuestionsQueued { .. } => "questions_queued",
            Self::EvidenceAccepted { .. } => "evidence_accepted",
            Self::QuestionAnswered { .. } => "question_answered",
            Self::QuestionDeferred { .. } => "question_deferred",
            Self::QuestionBounded { .. } => "question_bounded",
            Self::ResearchContractAssessed { .. } => "research_contract_assessed",
            Self::OutlineCommitted { .. } => "outline_committed",
            Self::SectionDrafted { .. } => "section_drafted",
            Self::AuditCompleted { .. } => "audit_completed",
            Self::BudgetExhausted { .. } => "budget_exhausted",
        }
    }
}
