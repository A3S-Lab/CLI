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
    /// Legacy journal replay only. New inquiries begin with planner-authored
    /// questions and never execute a scout phase.
    ScoutCompleted {
        source_ids: Vec<String>,
    },
    /// Legacy journal replay only. The active runtime has a fixed bounded
    /// coverage loop and no perspective-wave budget.
    PerspectiveBudgetSelected {
        total_retrieval_waves: u8,
    },
    /// Legacy journal replay only. New plans use stable research obligations
    /// and questions directly.
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
    /// A traceable answer that materially advances the question but retains a
    /// consequential evidence limitation. The projection keeps it answerable
    /// for material-evidence purposes while `bound_reason` preserves why the
    /// linked completion criterion remains qualified.
    QuestionPartiallyAnswered {
        question_id: String,
        answer: String,
        limitation: String,
        evidence_ids: Vec<String>,
    },
    /// Legacy journal replay only. The active closed-evidence review resolves
    /// every question exactly once as answered or bounded.
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
    SectionRevisionStarted {
        round: usize,
        section_ids: Vec<String>,
        input_digest: String,
    },
    SectionRevisionCommitted {
        round: usize,
        input_digest: String,
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
            Self::PerspectiveBudgetSelected { .. } => "perspective_budget_selected",
            Self::PerspectivesCommitted { .. } => "perspectives_committed",
            Self::QuestionsQueued { .. } => "questions_queued",
            Self::EvidenceAccepted { .. } => "evidence_accepted",
            Self::QuestionAnswered { .. } => "question_answered",
            Self::QuestionPartiallyAnswered { .. } => "question_partially_answered",
            Self::QuestionDeferred { .. } => "question_deferred",
            Self::QuestionBounded { .. } => "question_bounded",
            Self::ResearchContractAssessed { .. } => "research_contract_assessed",
            Self::OutlineCommitted { .. } => "outline_committed",
            Self::SectionDrafted { .. } => "section_drafted",
            Self::SectionRevisionStarted { .. } => "section_revision_started",
            Self::SectionRevisionCommitted { .. } => "section_revision_committed",
            Self::AuditCompleted { .. } => "audit_completed",
            Self::BudgetExhausted { .. } => "budget_exhausted",
        }
    }
}
