//! Host-side orchestration for replayable, perspective-guided research.
//!
//! DynamicWorkflow remains the bounded retrieval executor. Strategy selection,
//! scout/result boundaries, and inquiry state live in Rust so the workflow
//! JavaScript does not grow into a second research state machine.

use std::sync::Arc;

use a3s::research::{InquiryEvent, InquiryLimits, InquiryState, ResearchMethod};
use a3s_code_core::{AgentEvent, AgentSession, ToolCallResult};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use self::execution::{
    assess_completed_research_contract, attach_inquiry_projection,
    resolve_questions_with_bounded_follow_up_waves, run_dynamic_workflow, run_perspective_guided,
    InquiryExecution,
};
use self::plan::{
    commit_plan_research_contract, generate_plan, plan_max_iterations, queue_plan_questions,
    single_wave_research_plan, workflow_args_with_plan,
};
use super::{
    deep_research_canonical_workflow_output, validated_inquiry_projection,
    ValidatedInquiryProjection,
};

const PROGRESS_CHANNEL_CAPACITY: usize = 256;
const PERSPECTIVE_DISCOVERY_TIMEOUT_MS: u64 = 90_000;
const QUESTION_RESOLUTION_TIMEOUT_MS: u64 = 90_000;
const RESEARCH_CONTRACT_ASSESSMENT_TIMEOUT_MS: u64 = 90_000;
const MAX_FOLLOW_UP_QUESTIONS_PER_WAVE: usize = 4;
const MAX_QUESTION_EVIDENCE_ITEMS: usize = 24;
pub(super) const DEEP_RESEARCH_INQUIRY_HOST_TIMEOUT_MS: u64 = 12 * 60 * 1_000;
const SCOUT_WORKFLOW_TIMEOUT_MS: u64 = 150_000;
const FOLLOW_UP_WORKFLOW_TIMEOUT_MS: u64 = 180_000;

#[path = "inquiry_runtime/execution.rs"]
mod execution;
#[path = "inquiry_runtime/plan.rs"]
mod plan;

/// Spawn the complete evidence inquiry while preserving the event stream used
/// by the TUI. The returned result deliberately has the same shape as the
/// former single DynamicWorkflow call, so report publication stays compatible.
pub(super) fn spawn_deep_research_inquiry(
    session: Arc<AgentSession>,
    args: Value,
) -> (
    mpsc::Receiver<AgentEvent>,
    JoinHandle<Result<ToolCallResult, String>>,
) {
    let (progress_tx, progress_rx) = mpsc::channel(PROGRESS_CHANNEL_CAPACITY);
    let join = tokio::spawn(async move { run_inquiry(session, args, progress_tx).await });
    (progress_rx, join)
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
        ValidatedInquiryProjection::Inquiry { events, state } => Ok(Some((events, state))),
    }
}

async fn run_inquiry(
    session: Arc<AgentSession>,
    args: Value,
    progress_tx: mpsc::Sender<AgentEvent>,
) -> Result<ToolCallResult, String> {
    let host_id = format!(
        "host-deep-research-{}",
        args.get("run_id")
            .and_then(Value::as_str)
            .unwrap_or("unassigned")
    );
    send_progress(
        &progress_tx,
        AgentEvent::ToolExecutionStart {
            id: host_id,
            name: "dynamic_workflow".to_string(),
            args: args.clone(),
        },
    )
    .await?;

    let plan = generate_plan(&session, &args, &progress_tx).await?;
    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut inquiry_events = Vec::new();
    apply_event(
        &mut state,
        &mut inquiry_events,
        InquiryEvent::StrategySelected {
            method: plan.method,
        },
        &limits,
    )?;
    commit_plan_research_contract(&plan.value, &mut state, &mut inquiry_events, &limits)?;
    let mut execution = match plan.method {
        ResearchMethod::Focused => {
            queue_plan_questions(&plan.value, None, &mut state, &mut inquiry_events, &limits)?;
            let follow_up_waves_remaining = plan_max_iterations(&plan.value)
                .saturating_sub(1)
                .min(limits.max_question_round as u64)
                as usize;
            let retrieval_plan = single_wave_research_plan(&plan.value)?;
            let workflow_args = workflow_args_with_plan(args, retrieval_plan.clone(), None)?;
            InquiryExecution {
                result: run_dynamic_workflow(&session, workflow_args.clone(), &progress_tx).await?,
                retrieval_plan,
                workflow_args,
                follow_up_waves_remaining,
            }
        }
        ResearchMethod::PerspectiveGuided => {
            run_perspective_guided(
                &session,
                args,
                plan,
                &progress_tx,
                &mut state,
                &mut inquiry_events,
                &limits,
            )
            .await?
        }
    };

    resolve_questions_with_bounded_follow_up_waves(
        &session,
        &progress_tx,
        &mut execution,
        &mut state,
        &mut inquiry_events,
        &limits,
    )
    .await?;

    if state.phase == a3s::research::InquiryPhase::Outlining {
        let outcome = assess_completed_research_contract(
            &session,
            &progress_tx,
            &execution,
            &mut state,
            &mut inquiry_events,
            &limits,
        )
        .await?;
        if outcome == a3s::research::ResearchContractOutcome::Unsatisfied {
            apply_event(
                &mut state,
                &mut inquiry_events,
                InquiryEvent::BudgetExhausted {
                    reason: "the LLM-selected retrieval budget ended before the material research contract and stop conditions were satisfied".to_string(),
                },
                &limits,
            )?;
        }
    }

    attach_inquiry_projection(execution.result, &inquiry_events, &state)
}

fn apply_event(
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    event: InquiryEvent,
    limits: &InquiryLimits,
) -> Result<(), String> {
    state
        .apply(&event, limits)
        .map_err(|error| format!("apply inquiry event `{}`: {error}", event.name()))?;
    events.push(event);
    Ok(())
}

async fn send_progress(
    progress_tx: &mpsc::Sender<AgentEvent>,
    event: AgentEvent,
) -> Result<(), String> {
    progress_tx
        .send(event)
        .await
        .map_err(|_| "DeepResearch progress consumer closed".to_string())
}

#[cfg(test)]
#[path = "inquiry_runtime/integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "inquiry_runtime/retry_tests.rs"]
mod retry_tests;
#[cfg(test)]
#[path = "inquiry_runtime/tests.rs"]
mod tests;
