//! Bounded workflow execution and closed-evidence inquiry resolution.

use a3s::research::{
    decode_question_resolution, derive_research_contract_assessment, material_evidence_floor,
    question_resolution_events, question_resolution_generation_params,
    research_contract_assessment_event, research_contract_outcome, EvidenceDiagnostic,
    EvidenceDiagnosticKind, EvidenceRef, InquiryEvent, InquiryLimits, InquiryState, Question,
    QuestionResolution, QuestionResolutionOutput, QuestionStatus, ResearchContractOutcome,
};
use a3s_code_core::{AgentEvent, AgentSession, ToolCallResult};
use futures::{stream, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use tokio::sync::mpsc;

use super::super::{
    accepted_evidence_ledger, deep_research_canonical_workflow_output,
    recover_deep_research_initial_retrieval_from_store, AcceptedEvidence,
};
use super::plan::{bound_question_batch, bound_questions};
use super::{
    apply_event, apply_event_and_checkpoint, checkpoint_inquiry, terminalize_budget_exhaustion,
    InquiryCheckpointWriter, DEEP_RESEARCH_QUESTION_REVIEW_STAGE_TIMEOUT_MS,
    DURABLE_GENERATION_WORKFLOW_SOURCE, MAX_CONCURRENT_QUESTION_REVIEWS,
    MAX_QUESTION_EVIDENCE_ITEMS, MAX_QUESTION_EVIDENCE_PACKET_CHARS,
    QUESTION_RESOLUTION_ATTEMPT_TIMEOUT_MS, QUESTION_RESOLUTION_MAX_ATTEMPTS,
    QUESTION_RESOLUTION_WORKFLOW_TIMEOUT_MS,
};

#[derive(Debug)]
pub(super) struct InquiryExecution {
    pub(super) result: ToolCallResult,
}

struct AbortInnerToolOnDrop(Option<tokio::task::AbortHandle>);

impl AbortInnerToolOnDrop {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for AbortInnerToolOnDrop {
    fn drop(&mut self) {
        if let Some(abort) = self.0.take() {
            abort.abort();
        }
    }
}

include!("execution/resolution.rs");
include!("execution/tools.rs");
include!("execution/evidence.rs");
