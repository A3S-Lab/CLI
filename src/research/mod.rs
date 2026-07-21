//! Deterministic, replayable state machine for bounded research inquiries.

mod assessment;
mod contract_outcome;
mod error;
mod event;
mod evidence;
mod model;
mod outline;
mod questioning;
mod reducer;
mod state;
mod validation;

pub use error::{InquiryError, InquiryReplayError};
pub use event::InquiryEvent;
pub use evidence::{
    EvidenceDiagnostic, EvidenceDiagnosticKind, EvidenceRef, SourceCoverageBinding,
    SourceEvidenceRole,
};
pub use model::{
    CompletionCriterionAssessment, ContractAssessmentStatus, DiagnosticDisposition,
    EvidenceDiagnosticAssessment, EvidenceQualityRequirements, EvidenceRequirementAssessment,
    InquiryAudit, InquiryLimits, InquiryPhase, Perspective, Question, QuestionStatus,
    ResearchContractAssessment, ResearchContractOutcome, ResearchMethod, ResearchObligation,
    ResearchObligationAssessment, SectionDraft, SectionRevision, StopConditionAssessment,
};
pub use outline::{
    research_outline_json_schema, validate_research_outline, OutlineIdKind, OutlineSection,
    OutlineValidationContext, OutlineValidationError, ResearchOutline,
};
pub use questioning::{
    decode_question_resolution, question_resolution_events, question_resolution_generation_params,
    question_resolution_json_schema, validate_question_resolution, QuestionResolution,
    QuestionResolutionGenerationParams, QuestionResolutionOutput,
    QuestionResolutionValidationError,
};
pub use reducer::reduce;
pub use state::{replay, InquiryState};

#[cfg(test)]
mod tests;
pub use assessment::{
    aggregate_research_contract_assessments, decode_research_contract_assessment,
    decode_research_contract_assessment_chunk, derive_research_contract_assessment,
    research_contract_assessment_event, research_contract_assessment_generation_chunks,
    research_contract_assessment_generation_params, research_contract_assessment_json_schema,
    validate_research_contract_assessment, ResearchContractAssessmentError,
    ResearchContractAssessmentGenerationChunk, ResearchContractAssessmentGenerationParams,
    RESEARCH_CONTRACT_ASSESSMENT_SCHEMA_BUDGET_BYTES,
};
pub use contract_outcome::{material_evidence_floor, research_contract_outcome};
