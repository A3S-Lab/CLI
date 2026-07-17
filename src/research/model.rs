//! Typed values shared by inquiry planners, CLI runners, and the TUI.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchMethod {
    Focused,
    PerspectiveGuided,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InquiryPhase {
    #[default]
    StrategySelection,
    Scouting,
    PerspectiveDiscovery,
    Questioning,
    Outlining,
    Drafting,
    Auditing,
    Completed,
    Exhausted,
}

impl InquiryPhase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Exhausted)
    }
}

/// A stable, planner-authored coverage contract that must remain traceable
/// through questions, accepted evidence, and the final report.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceQualityRequirements {
    /// The obligation needs at least one direct, original, or first-party
    /// source rather than only derivative commentary.
    #[serde(default)]
    pub primary_source_required: bool,
    /// The obligation needs corroboration by separately attributable sources
    /// rather than support from only one source identity.
    #[serde(default)]
    pub independent_corroboration_required: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchObligation {
    pub id: String,
    pub title: String,
    pub focus: String,
    pub material: bool,
    pub completion_criteria: Vec<String>,
    /// Semantic evidence-quality constraints selected per obligation by the
    /// planner. Legacy journals default to no additional quality constraint.
    #[serde(default, skip_serializing_if = "EvidenceQualityRequirements::is_empty")]
    pub evidence_requirements: EvidenceQualityRequirements,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractAssessmentStatus {
    Satisfied,
    Bounded,
    Uncovered,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionCriterionAssessment {
    pub criterion_index: usize,
    pub status: ContractAssessmentStatus,
    pub rationale: String,
    pub evidence_ids: Vec<String>,
}

/// Closed-evidence assessment of one planner-declared source-quality
/// requirement. Source roles are judged semantically by the assessment model;
/// the Host validates that every cited source belongs to the cited accepted
/// evidence on this obligation's question path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRequirementAssessment {
    pub status: ContractAssessmentStatus,
    pub rationale: String,
    pub evidence_ids: Vec<String>,
    pub source_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchObligationAssessment {
    pub obligation_id: String,
    pub criteria: Vec<CompletionCriterionAssessment>,
    /// Present exactly when the planner required a primary source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_source: Option<EvidenceRequirementAssessment>,
    /// Present exactly when the planner required independent corroboration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub independent_corroboration: Option<EvidenceRequirementAssessment>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StopConditionAssessment {
    pub condition_index: usize,
    pub status: ContractAssessmentStatus,
    pub rationale: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticDisposition {
    Resolved,
    Bounded,
    Irrelevant,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceDiagnosticAssessment {
    pub diagnostic_id: String,
    pub disposition: DiagnosticDisposition,
    pub obligation_ids: Vec<String>,
    pub rationale: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchContractAssessment {
    pub obligations: Vec<ResearchObligationAssessment>,
    pub stop_conditions: Vec<StopConditionAssessment>,
    pub diagnostics: Vec<EvidenceDiagnosticAssessment>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchContractOutcome {
    Satisfied,
    Qualified,
    Unsatisfied,
}

impl ResearchObligation {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        focus: impl Into<String>,
        material: bool,
        completion_criteria: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            focus: focus.into(),
            material,
            completion_criteria,
            evidence_requirements: EvidenceQualityRequirements::default(),
        }
    }

    pub fn with_evidence_requirements(
        mut self,
        evidence_requirements: EvidenceQualityRequirements,
    ) -> Self {
        self.evidence_requirements = evidence_requirements;
        self
    }
}

impl EvidenceQualityRequirements {
    pub fn is_empty(&self) -> bool {
        !self.primary_source_required && !self.independent_corroboration_required
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Perspective {
    pub id: String,
    pub title: String,
    pub focus: String,
    pub source_ids: Vec<String>,
}

impl Perspective {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        focus: impl Into<String>,
        source_ids: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            focus: focus.into(),
            source_ids,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionStatus {
    Queued,
    Answered,
    Bounded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub id: String,
    pub perspective_id: Option<String>,
    pub parent_question_id: Option<String>,
    /// Stable research obligations this question is responsible for closing.
    /// Legacy journals may omit the field, but host-managed Inquiry runs fail
    /// closed unless every planned obligation is linked before outlining.
    #[serde(default)]
    pub obligation_ids: Vec<String>,
    /// A model-authored, search-engine-ready query for retrieving evidence for
    /// this question. The human-facing question remains in `prompt`.
    #[serde(default)]
    pub retrieval_query: Option<String>,
    pub material: bool,
    #[serde(alias = "iteration")]
    pub round: u32,
    pub prompt: String,
    pub status: QuestionStatus,
    pub answer: Option<String>,
    pub bound_reason: Option<String>,
    pub evidence_ids: Vec<String>,
}

impl Question {
    pub fn queued(
        id: impl Into<String>,
        perspective_id: Option<String>,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            perspective_id,
            parent_question_id: None,
            obligation_ids: Vec::new(),
            retrieval_query: None,
            material: true,
            round: 0,
            prompt: prompt.into(),
            status: QuestionStatus::Queued,
            answer: None,
            bound_reason: None,
            evidence_ids: Vec::new(),
        }
    }

    pub fn follow_up(
        id: impl Into<String>,
        perspective_id: Option<String>,
        parent_question_id: impl Into<String>,
        round: u32,
        prompt: impl Into<String>,
    ) -> Self {
        let mut question = Self::queued(id, perspective_id, prompt);
        question.parent_question_id = Some(parent_question_id.into());
        question.round = round;
        question
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SectionDraft {
    pub section_id: String,
    pub content: String,
    pub citation_ids: Vec<String>,
}

/// A durable, replayable section-revision attempt.
///
/// The attempt is counted when it starts, not when a model call happens to
/// return. This keeps the global repair budget stable across process restarts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SectionRevision {
    pub round: usize,
    pub section_ids: Vec<String>,
    pub input_digest: String,
    pub drafted_section_ids: Vec<String>,
    pub committed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InquiryAudit {
    pub passed: bool,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InquiryLimits {
    pub max_events: usize,
    pub max_identifier_chars: usize,
    pub max_text_chars: usize,
    pub max_obligations: usize,
    pub max_completion_criteria_per_obligation: usize,
    pub max_stop_conditions: usize,
    pub max_scout_sources: usize,
    pub max_perspectives: usize,
    pub max_questions: usize,
    pub max_question_round: usize,
    pub max_outline_sections: usize,
    pub max_evidence_ids_per_answer: usize,
    pub max_citation_ids_per_section: usize,
    pub max_answer_chars: usize,
    pub max_total_answer_chars: usize,
    pub max_section_chars: usize,
    pub max_total_draft_chars: usize,
    pub max_section_revision_rounds: usize,
    pub max_audit_issues: usize,
    pub max_audit_attempts: usize,
}

impl Default for InquiryLimits {
    fn default() -> Self {
        Self {
            max_events: 256,
            max_identifier_chars: 160,
            max_text_chars: 4_000,
            max_obligations: 16,
            max_completion_criteria_per_obligation: 8,
            max_stop_conditions: 8,
            max_scout_sources: 16,
            max_perspectives: 4,
            max_questions: 32,
            max_question_round: 4,
            max_outline_sections: 16,
            max_evidence_ids_per_answer: 16,
            max_citation_ids_per_section: 64,
            max_answer_chars: 12_000,
            max_total_answer_chars: 64_000,
            max_section_chars: 30_000,
            max_total_draft_chars: 120_000,
            max_section_revision_rounds: 2,
            max_audit_issues: 32,
            max_audit_attempts: 4,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InquiryBudgetInput {
    pub events_applied: usize,
    pub scout_sources: usize,
    pub perspectives: usize,
    pub questions: usize,
    pub material_questions: usize,
    pub answered_questions: usize,
    pub bounded_questions: usize,
    pub outline_sections: usize,
    pub drafted_sections: usize,
    pub answer_chars: usize,
    pub draft_chars: usize,
    pub audit_attempts: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InquiryConvergenceInput {
    pub method: Option<ResearchMethod>,
    pub phase: InquiryPhase,
    pub scout_completed: bool,
    pub perspectives_required: bool,
    pub perspectives_committed: usize,
    pub questions_queued: usize,
    pub material_questions: usize,
    pub questions_answered: usize,
    pub questions_bounded: usize,
    pub unresolved_questions: usize,
    pub unresolved_material_questions: usize,
    pub research_obligations: usize,
    pub material_obligations: usize,
    pub contract_assessed: bool,
    pub contract_outcome: Option<ResearchContractOutcome>,
    pub outline_sections: usize,
    pub drafted_sections: usize,
    pub undrafted_sections: usize,
    pub audit_passed: Option<bool>,
    pub budget_exhausted: bool,
}
