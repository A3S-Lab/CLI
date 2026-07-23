//! Bounded workflow execution and closed-evidence inquiry resolution.

#[cfg(test)]
use a3s::research::{
    decode_question_resolution, question_resolution_events, Question, QuestionResolution,
    QuestionResolutionOutput,
};
use a3s::research::{
    derive_research_contract_assessment, material_evidence_floor,
    research_contract_assessment_event, research_contract_outcome, EvidenceDiagnostic,
    EvidenceDiagnosticKind, EvidenceRef, InquiryEvent, InquiryLimits, InquiryState, QuestionStatus,
    ResearchContractOutcome,
};
use a3s_code_core::{AgentEvent, AgentSession, ToolCallResult};
#[cfg(test)]
use futures::StreamExt;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use tokio::sync::mpsc;

use super::super::{
    accepted_evidence_ledger, deep_research_canonical_workflow_output,
    normalize_research_source_anchor, recover_deep_research_bootstrap_acquisition_from_store,
    AcceptedEvidence,
};
use super::{
    apply_event, apply_event_and_checkpoint, checkpoint_inquiry, InquiryCheckpointWriter,
    DURABLE_GENERATION_WORKFLOW_SOURCE,
};

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
include!("execution/extraction.rs");
