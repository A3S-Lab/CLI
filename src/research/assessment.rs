//! Closed-evidence assessment of the planner-authored research contract.
//!
//! Question resolution alone cannot prove that the plan's semantic completion
//! criteria or stop conditions were met. This module creates a closed schema
//! over the replayed inquiry graph and validates the model's final assessment
//! before the reducer permits outlining.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{
    material_evidence_floor, CompletionCriterionAssessment, ContractAssessmentStatus,
    DiagnosticDisposition, EvidenceDiagnostic, EvidenceDiagnosticAssessment,
    EvidenceRequirementAssessment, InquiryEvent, InquiryState, QuestionStatus,
    ResearchContractAssessment, ResearchObligation, ResearchObligationAssessment,
    SourceEvidenceRole, StopConditionAssessment,
};

const MAX_QUERY_CHARS: usize = 8_000;
const MAX_PACKET_CHARS: usize = 96_000;
const MAX_RATIONALE_CHARS: usize = 4_000;
const MAX_ASSESSMENT_PROMPT_BYTES: usize = 120 * 1024;
const MIN_TIMEOUT_MS: u64 = 1_000;
const MAX_TIMEOUT_MS: u64 = 600_000;

/// Leaves headroom below `generate_object`'s hard 64 KiB schema limit for
/// provider envelopes and future schema metadata.
pub const RESEARCH_CONTRACT_ASSESSMENT_SCHEMA_BUDGET_BYTES: usize = 56 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchContractAssessmentGenerationParams {
    pub schema: serde_json::Value,
    pub schema_name: String,
    pub schema_description: String,
    pub prompt: String,
    pub mode: String,
    pub max_repair_attempts: u8,
    pub include_raw_text: bool,
    pub timeout_ms: u64,
}

/// One independently generated, identity-closed slice of a research-contract
/// assessment.
///
/// Callers submit `params` to `generate_object`, decode the response with
/// [`decode_research_contract_assessment_chunk`], then combine every decoded
/// part with [`aggregate_research_contract_assessments`]. Private scope and
/// reference metadata prevent callers from constructing a chunk whose wire
/// identities disagree with the host-owned inquiry state.
#[derive(Clone, Debug, PartialEq)]
pub struct ResearchContractAssessmentGenerationChunk {
    pub chunk_index: usize,
    pub chunk_count: usize,
    pub params: ResearchContractAssessmentGenerationParams,
    scope: AssessmentChunkScope,
    reference_catalog: AssessmentReferenceCatalog,
    reference_encoding: AssessmentReferenceEncoding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssessmentReferenceEncoding {
    ExactIds,
    Indexed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssessmentUnit {
    Obligation(usize),
    StopCondition(usize),
    Diagnostic(usize),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct AssessmentChunkScope {
    units: Vec<AssessmentUnit>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
struct AssessmentReferenceCatalog {
    obligations: BTreeMap<String, ObligationReferenceCatalog>,
    stop_condition_evidence_ids: Vec<String>,
    diagnostics: BTreeMap<String, DiagnosticReferenceCatalog>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
struct ObligationReferenceCatalog {
    evidence_ids: Vec<String>,
    source_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
struct DiagnosticReferenceCatalog {
    obligation_ids: Vec<String>,
    resolved_evidence_ids: Vec<String>,
    bounded_evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResearchContractAssessmentError {
    message: String,
}

impl ResearchContractAssessmentError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ResearchContractAssessmentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ResearchContractAssessmentError {}

include!("assessment/generation.rs");
include!("assessment/decoding.rs");
include!("assessment/validation.rs");
include!("assessment/derivation.rs");
include!("assessment/tests.rs");
