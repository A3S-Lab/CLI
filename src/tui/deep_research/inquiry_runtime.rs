//! Host-side DeepResearch budget contract and inquiry projection.
//!
//! Production Code DeepResearch runs through `CodeDeepResearchRunner`. The
//! standalone host-engine adapter (`A3SDeepResearchRuntime`) lives behind
//! `cfg(test)` so hermetic evidence-first capability tests keep exercising the
//! real engine ports without shipping that surface in release binaries.

use a3s::research::{InquiryEvent, InquiryState};
use a3s_deep_research::engine::{
    DEFAULT_PLANNED_RETRIEVAL_STAGE_TIMEOUT_MS, DEFAULT_PLANNING_BOOTSTRAP_STAGE_TIMEOUT_MS,
    DEFAULT_REPORT_STAGE_TIMEOUT_MS,
};
use serde_json::Value;

use super::{
    deep_research_canonical_workflow_output, validated_inquiry_projection, ValidatedInquiryProjection,
};

#[cfg(test)]
use super::deep_research_evidence_scope_from_args;
#[cfg(test)]
use super::deep_research_state_journal::ResearchSpec;

const REPORT_GENERATION_STAGE_COUNT: u64 = 2;
const EVIDENCE_FIRST_FINALIZATION_RESERVE_MS: u64 = 15_000;
pub(crate) const DEEP_RESEARCH_EVIDENCE_FIRST_HOST_TIMEOUT_MS: u64 =
    DEFAULT_PLANNING_BOOTSTRAP_STAGE_TIMEOUT_MS
        + DEFAULT_PLANNED_RETRIEVAL_STAGE_TIMEOUT_MS
        + DEFAULT_REPORT_STAGE_TIMEOUT_MS * REPORT_GENERATION_STAGE_COUNT
        + EVIDENCE_FIRST_FINALIZATION_RESERVE_MS;

/// Spawn the standalone engine for every new evidence-first run. The engine
/// preserves exact-query acquisition, closed semantic evidence selection, and
/// a source-backed artifact before attempting the optional report proposal.
/// The legacy Inquiry path below remains only for journal compatibility.
#[cfg(test)]
pub(crate) fn deep_research_evidence_first_research_spec(args: &Value) -> ResearchSpec {
    let query = args
        .pointer("/input/query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    ResearchSpec {
        query: query.to_string(),
        current_date: args
            .pointer("/input/current_date")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| chrono::Local::now().date_naive().to_string()),
        evidence_scope: deep_research_evidence_scope_from_args(args)
            .label()
            .to_string(),
        required_claims: Vec::new(),
        total_budget_ms: DEEP_RESEARCH_EVIDENCE_FIRST_HOST_TIMEOUT_MS,
        retrieval_stage_budget_ms: DEFAULT_PLANNING_BOOTSTRAP_STAGE_TIMEOUT_MS
            + DEFAULT_PLANNED_RETRIEVAL_STAGE_TIMEOUT_MS,
        question_review_stage_budget_ms: DEFAULT_REPORT_STAGE_TIMEOUT_MS
            * REPORT_GENERATION_STAGE_COUNT,
        finalization_reserve_ms: EVIDENCE_FIRST_FINALIZATION_RESERVE_MS,
        host_pid: std::process::id(),
    }
}

pub(super) fn inquiry_projection_from_workflow(
    workflow_output: &str,
    workflow_metadata: Option<&Value>,
) -> Result<Option<(Vec<InquiryEvent>, InquiryState)>, String> {
    let canonical = deep_research_canonical_workflow_output(workflow_output, workflow_metadata);
    let value = serde_json::from_str::<Value>(&canonical)
        .map_err(|error| format!("decode DeepResearch inquiry projection: {error}"))?;
    match validated_inquiry_projection(&value)? {
        ValidatedInquiryProjection::LegacyCheckedLoop => Ok(None),
        ValidatedInquiryProjection::Inquiry { events, state } => Ok(Some((events, *state))),
    }
}

#[cfg(test)]
#[path = "inquiry_runtime/host_engine.rs"]
mod host_engine;
#[cfg(test)]
use host_engine::*;

// Hermetic adapters and product-adapter tests resolve these through `use super::*`.
#[cfg(test)]
pub(super) use a3s_code_core::{AgentSession, ToolCallResult};
#[cfg(test)]
pub(super) use tokio::sync::mpsc;
#[cfg(test)]
pub(super) use super::deep_research_artifacts::{
    DeepResearchEvidenceFirstPublication, ResearchReportArtifacts,
};
#[cfg(test)]
pub(super) use super::deep_research_state_journal::record_workflow_started;

#[cfg(test)]
#[path = "inquiry_runtime/evidence_first_tests.rs"]
mod evidence_first_tests;
#[cfg(test)]
#[path = "inquiry_runtime/product_adapter_tests.rs"]
mod product_adapter_tests;
