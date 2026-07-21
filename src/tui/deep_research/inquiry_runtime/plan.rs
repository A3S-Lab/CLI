//! LLM-authored inquiry plan validation and bounded plan transformations.

use a3s::research::{
    EvidenceQualityRequirements, InquiryEvent, InquiryLimits, InquiryState, Question,
    ResearchObligation,
};
use a3s_code_core::{AgentEvent, AgentSession};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use tokio::sync::mpsc;

use super::execution::{call_generation_with_progress, generated_object};
use super::{
    apply_event, InquiryCheckpointWriter, DURABLE_GENERATION_WORKFLOW_GRACE_MS,
    MAX_PLANNER_TRACK_EFFECTS, PLANNER_GENERATION_MAX_ATTEMPTS, PLANNER_OUTLINE_ATTEMPT_TIMEOUT_MS,
};

#[derive(Clone, Debug)]
pub(super) struct PlannedInquiry {
    pub(super) value: Value,
}

include!("plan/planning.rs");
include!("plan/bounding.rs");
