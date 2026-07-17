//! Deterministic, replayable state machine for bounded research inquiries.

mod assessment;
mod error;
mod event;
mod evidence;
mod model;
mod outline;
mod perspective;
mod questioning;
mod reducer;
mod state;
mod validation;

pub use error::{InquiryError, InquiryReplayError};
pub use event::InquiryEvent;
pub use evidence::{EvidenceDiagnostic, EvidenceDiagnosticKind, EvidenceRef};
pub use model::{
    CompletionCriterionAssessment, ContractAssessmentStatus, DiagnosticDisposition,
    EvidenceDiagnosticAssessment, EvidenceQualityRequirements, EvidenceRequirementAssessment,
    InquiryAudit, InquiryBudgetInput, InquiryConvergenceInput, InquiryLimits, InquiryPhase,
    Perspective, Question, QuestionStatus, ResearchContractAssessment, ResearchContractOutcome,
    ResearchMethod, ResearchObligation, ResearchObligationAssessment, SectionDraft,
    SectionRevision, StopConditionAssessment,
};
pub use outline::{
    research_outline_json_schema, validate_research_outline, OutlineIdKind, OutlineSection,
    OutlineValidationContext, OutlineValidationError, ResearchOutline,
};
pub use perspective::{
    perspective_discovery_events, perspective_discovery_generation_params,
    perspective_discovery_json_schema, validate_perspective_discovery, DiscoveredPerspective,
    DiscoveredQuestion, PerspectiveDiscoveryGenerationParams, PerspectiveDiscoveryOutput,
    PerspectiveDiscoveryValidationError,
};
pub use questioning::{
    question_resolution_events, question_resolution_generation_params,
    question_resolution_json_schema, validate_question_resolution, FollowUpQuestion,
    QuestionResolution, QuestionResolutionGenerationParams, QuestionResolutionOutput,
    QuestionResolutionValidationError,
};
pub use reducer::reduce;
pub use state::{replay, InquiryState};

#[cfg(test)]
mod tests;
pub use assessment::{
    research_contract_assessment_event, research_contract_assessment_generation_params,
    research_contract_assessment_json_schema, research_contract_outcome,
    validate_research_contract_assessment, ResearchContractAssessmentError,
    ResearchContractAssessmentGenerationParams,
};
