//! Host-side orchestration for replayable, perspective-guided research.
//!
//! DynamicWorkflow remains the bounded retrieval executor. Strategy selection,
//! scout/result boundaries, and inquiry state live in Rust so the workflow
//! JavaScript does not grow into a second research state machine.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use a3s::research::{
    research_contract_assessment_event, CompletionCriterionAssessment, ContractAssessmentStatus,
    DiagnosticDisposition, EvidenceDiagnosticAssessment, EvidenceRequirementAssessment,
    InquiryEvent, InquiryLimits, InquiryPhase, InquiryState, QuestionStatus,
    ResearchContractAssessment, ResearchMethod, ResearchObligationAssessment,
    StopConditionAssessment,
};
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
    bound_questions, bound_workflow_timeout, commit_plan_research_contract, generate_plan,
    plan_max_iterations, queue_plan_questions, single_wave_research_plan, workflow_args_with_plan,
};
use super::deep_research_state_journal::{
    load_inquiry_state, record_inquiry_state, record_workflow_started, ResearchSpec,
};
use super::{
    deep_research_canonical_workflow_output, deep_research_evidence_scope_from_args,
    validated_inquiry_projection, ValidatedInquiryProjection,
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
const MIN_INQUIRY_STAGE_TIMEOUT_MS: u64 = 1_000;
const JOURNAL_INITIALIZATION_ATTEMPTS: usize = 8;
const JOURNAL_INITIALIZATION_RETRY_MS: u64 = 10;

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

    let checkpoint = InquiryCheckpointWriter::initialize(&session, &args).await?;
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
            checkpoint.checkpoint(&inquiry_events, &state).await?;
            let follow_up_waves_remaining = plan_max_iterations(&plan.value)
                .saturating_sub(1)
                .min(limits.max_question_round as u64)
                as usize;
            let retrieval_plan = single_wave_research_plan(&plan.value)?;
            let mut workflow_args =
                workflow_args_with_plan(args.clone(), retrieval_plan.clone(), None)?;
            let requested_timeout =
                configured_workflow_timeout_ms(&workflow_args, FOLLOW_UP_WORKFLOW_TIMEOUT_MS);
            let Some(timeout_ms) = checkpoint.stage_timeout_ms(requested_timeout) else {
                let reason = "the shared inquiry deadline left no retrieval budget after reserving finalization time";
                terminalize_budget_exhaustion(
                    Some(&checkpoint),
                    &mut state,
                    &mut inquiry_events,
                    &limits,
                    reason,
                )
                .await?;
                return attach_inquiry_projection(
                    budget_terminal_result(&args, reason),
                    &inquiry_events,
                    &state,
                );
            };
            bound_workflow_timeout(&mut workflow_args, timeout_ms)?;
            InquiryExecution {
                result: run_dynamic_workflow(&session, workflow_args.clone(), &progress_tx).await?,
                retrieval_plan,
                workflow_args,
                follow_up_waves_remaining,
            }
        }
        ResearchMethod::PerspectiveGuided => {
            checkpoint.checkpoint(&inquiry_events, &state).await?;
            run_perspective_guided(
                &session,
                args,
                plan,
                &progress_tx,
                &mut state,
                &mut inquiry_events,
                &limits,
                &checkpoint,
            )
            .await?
        }
    };

    if !state.phase.is_terminal() {
        resolve_questions_with_bounded_follow_up_waves(
            &session,
            &progress_tx,
            &mut execution,
            &mut state,
            &mut inquiry_events,
            &limits,
            Some(&checkpoint),
        )
        .await?;
    }

    if state.phase == a3s::research::InquiryPhase::Outlining {
        let outcome = assess_completed_research_contract(
            &session,
            &progress_tx,
            &execution,
            &mut state,
            &mut inquiry_events,
            &limits,
            Some(&checkpoint),
        )
        .await?;
        if outcome == a3s::research::ResearchContractOutcome::Unsatisfied
            && state.phase != InquiryPhase::Exhausted
        {
            apply_event_and_checkpoint(
                Some(&checkpoint),
                &mut state,
                &mut inquiry_events,
                InquiryEvent::BudgetExhausted {
                    reason: "the LLM-selected retrieval budget ended before the material research contract and stop conditions were satisfied".to_string(),
                },
                &limits,
            )
            .await?;
        }
    }

    attach_inquiry_projection(execution.result, &inquiry_events, &state)
}

#[derive(Clone, Debug)]
struct InquiryDeadline {
    deadline: Instant,
    finalization_reserve: Duration,
}

impl InquiryDeadline {
    fn from_args(
        args: &Value,
        total_budget_ms: u64,
        finalization_reserve_ms: u64,
        now: Instant,
    ) -> Self {
        let wall_now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let elapsed_ms = args
            .pointer("/input/run_started_at_ms")
            .and_then(Value::as_u64)
            .map(|started_at_ms| wall_now_ms.saturating_sub(started_at_ms))
            .unwrap_or_default()
            .min(total_budget_ms);
        Self::from_elapsed(now, total_budget_ms, finalization_reserve_ms, elapsed_ms)
    }

    fn from_elapsed(
        now: Instant,
        total_budget_ms: u64,
        finalization_reserve_ms: u64,
        elapsed_ms: u64,
    ) -> Self {
        let remaining_ms = total_budget_ms.saturating_sub(elapsed_ms);
        Self {
            deadline: now
                .checked_add(Duration::from_millis(remaining_ms))
                .unwrap_or(now),
            finalization_reserve: Duration::from_millis(
                finalization_reserve_ms.min(total_budget_ms),
            ),
        }
    }

    fn stage_timeout_ms_at(&self, now: Instant, requested_timeout_ms: u64) -> Option<u64> {
        let available = self
            .deadline
            .saturating_duration_since(now)
            .saturating_sub(self.finalization_reserve);
        let available_ms = available.as_millis().min(u128::from(u64::MAX)) as u64;
        let selected = requested_timeout_ms.min(available_ms);
        (selected >= MIN_INQUIRY_STAGE_TIMEOUT_MS).then_some(selected)
    }
}

/// The host inquiry owns exactly one sequential checkpoint writer. Each write
/// contains the complete validated event prefix, so retries are idempotent and
/// a timeout can recover the last event committed before the next tool await.
#[derive(Debug)]
struct InquiryCheckpointWriter {
    workspace: PathBuf,
    run_id: String,
    persisted_events: AtomicUsize,
    deadline: InquiryDeadline,
}

impl InquiryCheckpointWriter {
    async fn initialize(session: &AgentSession, args: &Value) -> Result<Self, String> {
        let run_id = args
            .get("run_id")
            .and_then(Value::as_str)
            .filter(|run_id| !run_id.trim().is_empty())
            .ok_or_else(|| "DeepResearch inquiry checkpointing requires a run_id".to_string())?
            .to_string();
        let workspace = session.workspace().to_path_buf();
        if let Some((events, _)) = load_inquiry_state(&workspace, &run_id)
            .await
            .map_err(|error| format!("load existing DeepResearch inquiry prefix: {error}"))?
        {
            if !events.is_empty() {
                return Err(format!(
                    "DeepResearch inquiry `{run_id}` has a durable {}-event prefix but no complete execution payload; replaying planner or retrieval work would diverge, so this run cannot be resumed",
                    events.len()
                ));
            }
        }
        let query = args
            .pointer("/input/query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let total_budget_ms = DEEP_RESEARCH_INQUIRY_HOST_TIMEOUT_MS;
        let finalization_reserve_ms = total_budget_ms.saturating_mul(15) / 100;
        let spec = ResearchSpec {
            query: query.to_string(),
            current_date: args
                .pointer("/input/current_date")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| chrono::Local::now().date_naive().to_string()),
            evidence_scope: deep_research_evidence_scope_from_args(args, query)
                .label()
                .to_string(),
            required_claims: Vec::new(),
            total_budget_ms,
            finalization_reserve_ms,
            host_pid: std::process::id(),
        };
        let deadline = InquiryDeadline::from_args(
            args,
            total_budget_ms,
            finalization_reserve_ms,
            Instant::now(),
        );
        let mut last_error = None;
        for attempt in 0..JOURNAL_INITIALIZATION_ATTEMPTS {
            match record_workflow_started(&workspace, &run_id, spec.clone()).await {
                Ok(()) => {
                    return Ok(Self {
                        workspace,
                        run_id,
                        persisted_events: AtomicUsize::new(0),
                        deadline,
                    })
                }
                Err(error) => {
                    last_error = Some(error);
                    if attempt + 1 < JOURNAL_INITIALIZATION_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(JOURNAL_INITIALIZATION_RETRY_MS))
                            .await;
                    }
                }
            }
        }
        let detail = last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "initialization attempts ended without an error".to_string());
        Err(format!("initialize DeepResearch inquiry journal: {detail}"))
    }

    fn stage_timeout_ms(&self, requested_timeout_ms: u64) -> Option<u64> {
        self.deadline
            .stage_timeout_ms_at(Instant::now(), requested_timeout_ms)
    }

    async fn checkpoint(
        &self,
        events: &[InquiryEvent],
        state: &InquiryState,
    ) -> Result<(), String> {
        let persisted = self.persisted_events.load(Ordering::Acquire);
        if events.len() < persisted {
            return Err(format!(
                "DeepResearch inquiry checkpoint for `{}` regressed from {persisted} to {} events",
                self.run_id,
                events.len()
            ));
        }
        if events.len() == persisted {
            let replayed = a3s::research::replay(events, &InquiryLimits::default())
                .map_err(|error| format!("strictly replay unchanged inquiry prefix: {error}"))?;
            if &replayed != state {
                return Err(
                    "unchanged DeepResearch inquiry prefix differs from its strict replay"
                        .to_string(),
                );
            }
            return Ok(());
        }
        record_inquiry_state(&self.workspace, &self.run_id, events, state)
            .await
            .map_err(|error| {
                format!(
                    "durably checkpoint DeepResearch inquiry event prefix for `{}`: {error}",
                    self.run_id
                )
            })?;
        self.persisted_events.store(events.len(), Ordering::Release);
        Ok(())
    }
}

async fn checkpoint_inquiry(
    checkpoint: Option<&InquiryCheckpointWriter>,
    events: &[InquiryEvent],
    state: &InquiryState,
) -> Result<(), String> {
    match checkpoint {
        Some(checkpoint) => checkpoint.checkpoint(events, state).await,
        None => Ok(()),
    }
}

async fn apply_event_and_checkpoint(
    checkpoint: Option<&InquiryCheckpointWriter>,
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    event: InquiryEvent,
    limits: &InquiryLimits,
) -> Result<(), String> {
    apply_event(state, events, event, limits)?;
    checkpoint_inquiry(checkpoint, events, state).await
}

async fn terminalize_budget_exhaustion(
    checkpoint: Option<&InquiryCheckpointWriter>,
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    reason: &str,
) -> Result<(), String> {
    if state.phase == InquiryPhase::Exhausted {
        return checkpoint_inquiry(checkpoint, events, state).await;
    }
    if state.phase == InquiryPhase::Questioning {
        bound_questions(state, events, limits, reason)?;
    }
    if state.phase == InquiryPhase::Outlining
        && state.contract_assessment.is_none()
        && state
            .questions
            .iter()
            .filter(|question| question.material)
            .all(|question| question.status == QuestionStatus::Answered)
    {
        let event = research_contract_assessment_event(
            state,
            budget_exhausted_contract_assessment(state, reason),
        )
        .map_err(|error| format!("build deadline-bounded research assessment: {error}"))?;
        apply_event(state, events, event, limits)?;
    }
    apply_event(
        state,
        events,
        InquiryEvent::BudgetExhausted {
            reason: reason.to_string(),
        },
        limits,
    )?;
    checkpoint_inquiry(checkpoint, events, state).await
}

fn budget_exhausted_contract_assessment(
    state: &InquiryState,
    reason: &str,
) -> ResearchContractAssessment {
    let rationale = format!("The host could not complete closed-evidence assessment: {reason}");
    let obligations = state
        .obligations
        .iter()
        .map(|obligation| ResearchObligationAssessment {
            obligation_id: obligation.id.clone(),
            criteria: obligation
                .completion_criteria
                .iter()
                .enumerate()
                .map(|(criterion_index, _)| CompletionCriterionAssessment {
                    criterion_index,
                    status: ContractAssessmentStatus::Uncovered,
                    rationale: rationale.clone(),
                    evidence_ids: Vec::new(),
                })
                .collect(),
            primary_source: obligation
                .evidence_requirements
                .primary_source_required
                .then(|| EvidenceRequirementAssessment {
                    status: ContractAssessmentStatus::Uncovered,
                    rationale: rationale.clone(),
                    evidence_ids: Vec::new(),
                    source_ids: Vec::new(),
                }),
            independent_corroboration: obligation
                .evidence_requirements
                .independent_corroboration_required
                .then(|| EvidenceRequirementAssessment {
                    status: ContractAssessmentStatus::Uncovered,
                    rationale: rationale.clone(),
                    evidence_ids: Vec::new(),
                    source_ids: Vec::new(),
                }),
        })
        .collect();
    let stop_conditions = state
        .stop_conditions
        .iter()
        .enumerate()
        .map(|(condition_index, _)| StopConditionAssessment {
            condition_index,
            status: ContractAssessmentStatus::Uncovered,
            rationale: rationale.clone(),
            evidence_ids: Vec::new(),
        })
        .collect();
    let diagnostics = state
        .evidence_catalog
        .values()
        .flat_map(|evidence| {
            let rationale = rationale.clone();
            evidence.diagnostics.iter().map(move |diagnostic| {
                let obligation_ids = state
                    .questions
                    .iter()
                    .filter(|question| question.evidence_ids.contains(&evidence.evidence_id))
                    .flat_map(|question| question.obligation_ids.iter().cloned())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let linked = !obligation_ids.is_empty();
                EvidenceDiagnosticAssessment {
                    diagnostic_id: diagnostic.id.clone(),
                    disposition: if linked {
                        DiagnosticDisposition::Bounded
                    } else {
                        DiagnosticDisposition::Irrelevant
                    },
                    obligation_ids,
                    rationale: rationale.clone(),
                    evidence_ids: if linked {
                        vec![evidence.evidence_id.clone()]
                    } else {
                        Vec::new()
                    },
                }
            })
        })
        .collect();
    ResearchContractAssessment {
        obligations,
        stop_conditions,
        diagnostics,
    }
}

fn configured_workflow_timeout_ms(args: &Value, fallback_ms: u64) -> u64 {
    args.pointer("/limits/timeoutMs")
        .and_then(Value::as_u64)
        .or_else(|| {
            args.pointer("/input/workflow_timeout_ms")
                .and_then(Value::as_u64)
        })
        .filter(|timeout_ms| *timeout_ms > 0)
        .unwrap_or(fallback_ms)
}

fn budget_terminal_result(args: &Value, reason: &str) -> ToolCallResult {
    let query = args
        .pointer("/input/query")
        .and_then(Value::as_str)
        .unwrap_or("DeepResearch inquiry");
    ToolCallResult {
        name: "dynamic_workflow".to_string(),
        output: serde_json::json!({
            "query": query,
            "structured": {
                "summary": reason,
                "sources": [],
                "key_evidence": [],
                "contradictions": [],
                "gaps": [reason],
                "confidence": "low"
            }
        })
        .to_string(),
        exit_code: 0,
        metadata: None,
        error_kind: None,
    }
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
