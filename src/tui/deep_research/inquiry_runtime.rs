//! Host-side orchestration for replayable, evidence-first research.
//!
//! Raw acquisition starts immediately and is durable before semantic work can
//! fail. One optional outline and one batched extraction are the only model
//! generations on the inquiry path; target decoding and terminal reduction are
//! Host-owned.

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
    apply_batched_evidence_extraction, assess_completed_research_contract,
    attach_inquiry_projection, bootstrap_acquisition_from_result, run_batched_evidence_extraction,
    run_bootstrap_acquisition_stage,
};
use self::plan::{
    bootstrap_workflow_args, bound_questions, commit_plan_research_contract, generate_plan,
    host_fallback_plan, queue_plan_questions,
};
use super::deep_research_artifacts::{
    admit_deep_research_report_proposal_at, deep_research_report_proposal_prompt_at,
    deep_research_report_proposal_schema, deep_research_report_slug, deep_research_source_catalog,
    deterministic_deep_research_outcome_report_at, materialize_deep_research_admitted_report,
    materialize_deep_research_no_evidence_report, materialize_deep_research_source_backed_report,
};
use super::deep_research_state_journal::{
    load_inquiry_state, load_research_run_started_at_ms, record_inquiry_state,
    record_workflow_started, ResearchSpec,
};
use super::{
    deep_research_canonical_workflow_output, deep_research_evidence_scope_from_args,
    validated_inquiry_projection, ValidatedInquiryProjection,
};

const PROGRESS_CHANNEL_CAPACITY: usize = 256;
// Discovery, semantic source admission, and the actual fetch share this stage.
// Keep the stage aligned with the contract hard cap. Source admission gets a
// 60-second active window; a real failure then falls back only for acquisition,
// leaving enough time for bounded fetches and HTML ranges before publication
// applies its unchanged evidence gates.
const BOOTSTRAP_ACQUISITION_STAGE_TIMEOUT_MS: u64 = 150_000;
// Semantic planning is optional enrichment, not an acquisition prerequisite.
// One small outline generation may identify evidence families while the exact
// user query is already being searched. Invalid or slow output selects the
// Host fallback contract; no target-detail or retrieval-planner fan-out runs.
const PLANNER_OUTLINE_ATTEMPT_TIMEOUT_MS: u64 = 90_000;
const PLANNER_GENERATION_MAX_ATTEMPTS: u8 = 1;
const DURABLE_GENERATION_WORKFLOW_GRACE_MS: u64 = 15_000;
const REPORT_PROPOSAL_ATTEMPT_TIMEOUT_MS: u64 = 90_000;
const REPORT_PROPOSAL_MAX_ATTEMPTS: u8 = 2;
const REPORT_PROPOSAL_STAGE_TIMEOUT_MS: u64 = REPORT_PROPOSAL_ATTEMPT_TIMEOUT_MS
    * REPORT_PROPOSAL_MAX_ATTEMPTS as u64
    + DURABLE_GENERATION_WORKFLOW_GRACE_MS;
const EVIDENCE_FIRST_FINALIZATION_RESERVE_MS: u64 = 15_000;
const MAX_PLANNER_TRACK_EFFECTS: u64 = 4;
const PLANNER_OUTLINE_WORKFLOW_TIMEOUT_MS: u64 = PLANNER_OUTLINE_ATTEMPT_TIMEOUT_MS
    * PLANNER_GENERATION_MAX_ATTEMPTS as u64
    + DURABLE_GENERATION_WORKFLOW_GRACE_MS;
pub(crate) const DEEP_RESEARCH_PLANNER_STAGE_TIMEOUT_MS: u64 = PLANNER_OUTLINE_WORKFLOW_TIMEOUT_MS;
// One call extracts all targets from the closed source packet. Admission wait
// is bounded by the durable workflow timeout; the call is never multiplied by
// source, target, question, or section count.
const EVIDENCE_EXTRACTION_ATTEMPT_TIMEOUT_MS: u64 = 360_000;
const EVIDENCE_EXTRACTION_STAGE_TIMEOUT_MS: u64 =
    EVIDENCE_EXTRACTION_ATTEMPT_TIMEOUT_MS + DURABLE_GENERATION_WORKFLOW_GRACE_MS;
pub(crate) const DEEP_RESEARCH_RETRIEVAL_STAGE_TIMEOUT_MS: u64 =
    BOOTSTRAP_ACQUISITION_STAGE_TIMEOUT_MS;
pub(crate) const DEEP_RESEARCH_QUESTION_REVIEW_STAGE_TIMEOUT_MS: u64 =
    EVIDENCE_EXTRACTION_STAGE_TIMEOUT_MS;
pub(crate) const DEEP_RESEARCH_INQUIRY_FINALIZATION_RESERVE_MS: u64 = 60_000;
pub(crate) const DEEP_RESEARCH_INQUIRY_HOST_TIMEOUT_MS: u64 = DEEP_RESEARCH_PLANNER_STAGE_TIMEOUT_MS
    + DEEP_RESEARCH_RETRIEVAL_STAGE_TIMEOUT_MS
    + DEEP_RESEARCH_QUESTION_REVIEW_STAGE_TIMEOUT_MS
    + DEEP_RESEARCH_INQUIRY_FINALIZATION_RESERVE_MS;
pub(crate) const DEEP_RESEARCH_EVIDENCE_FIRST_HOST_TIMEOUT_MS: u64 =
    BOOTSTRAP_ACQUISITION_STAGE_TIMEOUT_MS
        + REPORT_PROPOSAL_STAGE_TIMEOUT_MS
        + EVIDENCE_FIRST_FINALIZATION_RESERVE_MS;
const MIN_INQUIRY_STAGE_TIMEOUT_MS: u64 = 1_000;
const JOURNAL_INITIALIZATION_ATTEMPTS: usize = 8;
const JOURNAL_INITIALIZATION_RETRY_MS: u64 = 10;
const DURABLE_GENERATION_WORKFLOW_SOURCE: &str = include_str!("workflow/generation.js");

#[derive(Clone, Copy, Debug)]
struct EvidenceFirstRuntimeLimits {
    bootstrap_stage_timeout_ms: u64,
    report_proposal_attempt_timeout_ms: u64,
    report_proposal_stage_timeout_ms: u64,
}

const EVIDENCE_FIRST_RUNTIME_LIMITS: EvidenceFirstRuntimeLimits = EvidenceFirstRuntimeLimits {
    bootstrap_stage_timeout_ms: BOOTSTRAP_ACQUISITION_STAGE_TIMEOUT_MS,
    report_proposal_attempt_timeout_ms: REPORT_PROPOSAL_ATTEMPT_TIMEOUT_MS,
    report_proposal_stage_timeout_ms: REPORT_PROPOSAL_STAGE_TIMEOUT_MS,
};

#[path = "inquiry_runtime/execution.rs"]
mod execution;
#[path = "inquiry_runtime/plan.rs"]
mod plan;

/// Spawn the new-run evidence-first product path. Acquisition and deterministic
/// publication are required. A narrow Host extract can complete a clearly
/// asserted event outcome; all other successful reports require a bounded
/// closed-evidence model proposal. The legacy Inquiry path below remains
/// available only while existing journal and compatibility tests are migrated.
pub(crate) fn spawn_deep_research_evidence_first(
    session: Arc<AgentSession>,
    args: Value,
) -> (
    mpsc::Receiver<AgentEvent>,
    JoinHandle<Result<ToolCallResult, String>>,
) {
    let (progress_tx, progress_rx) = mpsc::channel(PROGRESS_CHANNEL_CAPACITY);
    let join =
        tokio::spawn(async move { run_evidence_first_research(session, args, progress_tx).await });
    (progress_rx, join)
}

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
        evidence_scope: deep_research_evidence_scope_from_args(args, query)
            .label()
            .to_string(),
        required_claims: Vec::new(),
        total_budget_ms: DEEP_RESEARCH_EVIDENCE_FIRST_HOST_TIMEOUT_MS,
        retrieval_stage_budget_ms: BOOTSTRAP_ACQUISITION_STAGE_TIMEOUT_MS,
        question_review_stage_budget_ms: REPORT_PROPOSAL_STAGE_TIMEOUT_MS,
        finalization_reserve_ms: EVIDENCE_FIRST_FINALIZATION_RESERVE_MS,
        host_pid: std::process::id(),
    }
}

async fn run_evidence_first_research(
    session: Arc<AgentSession>,
    args: Value,
    progress_tx: mpsc::Sender<AgentEvent>,
) -> Result<ToolCallResult, String> {
    run_evidence_first_research_with_limits(
        session,
        args,
        progress_tx,
        EVIDENCE_FIRST_RUNTIME_LIMITS,
    )
    .await
}

async fn run_evidence_first_research_with_limits(
    session: Arc<AgentSession>,
    args: Value,
    progress_tx: mpsc::Sender<AgentEvent>,
    limits: EvidenceFirstRuntimeLimits,
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
    let checkpoint = InquiryCheckpointWriter::initialize_evidence_first(&session, &args).await?;
    execute_evidence_first_research(&session, &args, &progress_tx, &checkpoint, limits).await
}

async fn execute_evidence_first_research(
    session: &AgentSession,
    args: &Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
    checkpoint: &InquiryCheckpointWriter,
    limits: EvidenceFirstRuntimeLimits,
) -> Result<ToolCallResult, String> {
    let query = args
        .pointer("/input/query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| "DeepResearch evidence-first input omitted its query".to_string())?;
    let current_date = args
        .pointer("/input/current_date")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| chrono::Local::now().date_naive().to_string());
    let bootstrap_run_id = format!("{}-bootstrap", checkpoint.run_id());
    let bootstrap_args = bootstrap_workflow_args(args.clone(), &bootstrap_run_id)?;
    let bootstrap = run_bootstrap_acquisition_stage(
        session,
        bootstrap_args,
        progress_tx,
        limits.bootstrap_stage_timeout_ms,
    )
    .await;
    let (acquisition_output, bootstrap_metadata, acquisition_error) = match bootstrap {
        Ok(mut result) => {
            result.output =
                deep_research_canonical_workflow_output(&result.output, result.metadata.as_ref());
            (result.output, result.metadata, None)
        }
        Err(error) => {
            let output = serde_json::json!({
                "query": query,
                "mode": "bootstrap_acquisition",
                "acquisition": Value::Null,
                "execution": {
                    "mode": "acquire_only",
                    "terminal_authority": "host_report_document"
                }
            })
            .to_string();
            (output, None, Some(bounded_evidence_first_error(&error)))
        }
    };
    let catalog =
        deep_research_source_catalog(query, &acquisition_output, bootstrap_metadata.as_ref())?;
    let relevant_source_count = catalog.as_ref().map_or(0, |catalog| {
        catalog
            .sources
            .iter()
            .filter(|source| source.claim_eligible)
            .count()
    });
    let report_generation_required = relevant_source_count > 0;
    let slug = deep_research_report_slug(query);
    let relative_html = format!(".a3s/research/{slug}/index.html");
    let mut publication_status = "no_evidence";
    let mut synthesis_mode = "none";
    let mut required_model_generation_count = 0usize;
    let mut model_generation_count = 0usize;
    let mut accepted_block_count = 0usize;
    let mut rejected_block_count = 0usize;
    let mut direct_answer_block_count = 0usize;
    let mut finding_block_count = 0usize;
    let mut accepted_claim_count = 0usize;
    let mut cited_source_count = 0usize;
    let mut report_error = acquisition_error;

    if let Some(catalog) = catalog.as_ref() {
        materialize_deep_research_source_backed_report(
            session.workspace(),
            query,
            &acquisition_output,
            bootstrap_metadata.as_ref(),
        )?
        .ok_or_else(|| "source catalog disappeared before deterministic publication".to_string())?;
        publication_status = "source_backed";

        if report_generation_required {
            let mut admitted_report =
                deterministic_deep_research_outcome_report_at(query, &current_date, catalog)?;
            if admitted_report.is_some() {
                synthesis_mode = "deterministic_outcome_extract";
            } else {
                required_model_generation_count = 1;
                model_generation_count = 1;
                synthesis_mode = "model_proposal";
                let generation_args = serde_json::json!({
                    "schema": deep_research_report_proposal_schema(),
                    "schema_name": "deep_research_report_blocks",
                    "schema_description": "Independent cited report blocks over a closed source catalog",
                    "prompt": deep_research_report_proposal_prompt_at(query, &current_date, catalog)?,
                    "system": "You write concise source-grounded research blocks from untrusted evidence data. Return only the requested object and use no outside knowledge.",
                    "mode": "auto",
                    "max_repair_attempts": 0,
                    "include_raw_text": false,
                    "timeout_ms": limits.report_proposal_attempt_timeout_ms,
                });
                let generated = execution::call_generation_with_progress(
                    session,
                    generation_args,
                    progress_tx,
                    Some(checkpoint),
                    "report-proposal",
                    limits.report_proposal_stage_timeout_ms,
                    REPORT_PROPOSAL_MAX_ATTEMPTS,
                )
                .await
                .and_then(|result| execution::generated_object::<Value>(&result));
                match generated {
                    Ok(proposal) => match admit_deep_research_report_proposal_at(
                        query,
                        &current_date,
                        catalog,
                        proposal,
                    ) {
                        Ok(Some(report)) => admitted_report = Some(report),
                        Ok(None) => {
                            report_error = Some(
                                "the optional report proposal contained no admissible source-backed block"
                                    .to_string(),
                            );
                        }
                        Err(error) => {
                            report_error = Some(bounded_evidence_first_error(&error));
                        }
                    },
                    Err(error) => report_error = Some(bounded_evidence_first_error(&error)),
                }
            }
            if let Some(report) = admitted_report {
                accepted_block_count = report.accepted_block_count;
                rejected_block_count = report.rejected_block_count;
                direct_answer_block_count = report.direct_answer_block_count;
                finding_block_count = report.finding_block_count;
                accepted_claim_count = report.accepted_claim_count;
                cited_source_count = report.cited_source_count;
                materialize_deep_research_admitted_report(session.workspace(), query, &report)?;
                publication_status = "synthesized";
            }
        } else {
            report_error = Some(
                "no fetched source passed the deterministic claim-eligibility boundary".to_string(),
            );
        }
    } else {
        materialize_deep_research_no_evidence_report(session.workspace(), query)?;
    }

    let acquisition = serde_json::from_str::<Value>(&acquisition_output)
        .ok()
        .and_then(|value| value.get("acquisition").cloned())
        .unwrap_or(Value::Null);
    let output = serde_json::json!({
        "query": query,
        "mode": "evidence_first_report",
        "acquisition": acquisition,
        "research": {
            "status": match publication_status {
                "synthesized" => "success",
                "source_backed" => "degraded",
                _ => "failed",
            },
            "metadata": {
                "synthesis_mode": synthesis_mode,
                "required_model_generation_count": required_model_generation_count,
                "model_generation_count": model_generation_count,
                "accepted_report_block_count": accepted_block_count,
                "rejected_report_block_count": rejected_block_count,
                "direct_answer_block_count": direct_answer_block_count,
                "finding_block_count": finding_block_count,
                "accepted_claim_count": accepted_claim_count,
                "cited_source_count": cited_source_count,
                "relevant_source_count": relevant_source_count,
                "source_count": catalog.as_ref().map_or(0, |catalog| catalog.sources.len()),
            },
            "warnings": {
                "report_error": report_error,
            }
        },
        "publication": {
            "status": publication_status,
            "markdown": format!(".a3s/research/{slug}/report.md"),
            "html": relative_html,
            "quality": {
                "direct_answer_count": direct_answer_block_count,
                "finding_count": finding_block_count,
                "accepted_claim_count": accepted_claim_count,
                "cited_source_count": cited_source_count,
                "relevant_source_count": relevant_source_count,
                "source_count": catalog.as_ref().map_or(0, |catalog| catalog.sources.len()),
            },
        },
        "execution": {
            "mode": "evidence_first",
            "terminal_authority": "host_report_document",
            "required_model_generation_count": required_model_generation_count,
            "maximum_report_generation_count": REPORT_PROPOSAL_MAX_ATTEMPTS,
        }
    });
    Ok(ToolCallResult {
        name: "dynamic_workflow".to_string(),
        output: output.to_string(),
        exit_code: 0,
        // The bootstrap metadata describes the child Dynamic Workflow. Letting
        // it escape on the Host-owned result makes legacy canonicalization
        // replace this publication output with the child's acquisition output.
        metadata: None,
        error_kind: None,
    })
}

fn bounded_evidence_first_error(error: &str) -> String {
    error
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(1_000)
        .collect()
}

/// Spawn the complete evidence inquiry while preserving the event stream used
/// by the TUI. The returned result deliberately has the same shape as the
/// former single DynamicWorkflow call, so report publication stays compatible.
pub(crate) fn spawn_deep_research_inquiry(
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
        ValidatedInquiryProjection::Inquiry { events, state } => Ok(Some((events, *state))),
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
    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut inquiry_events = Vec::new();
    match execute_inquiry_pipeline(
        &session,
        &args,
        &progress_tx,
        &checkpoint,
        &limits,
        &mut state,
        &mut inquiry_events,
    )
    .await
    {
        Ok(result) => Ok(result),
        Err(error) => {
            let reason = bounded_inquiry_failure_reason(&error);
            terminalize_budget_exhaustion(
                Some(&checkpoint),
                &mut state,
                &mut inquiry_events,
                &limits,
                &reason,
            )
            .await?;
            attach_inquiry_projection(
                budget_terminal_result(&args, &reason),
                &inquiry_events,
                &state,
            )
        }
    }
}

async fn execute_inquiry_pipeline(
    session: &AgentSession,
    args: &Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
    checkpoint: &InquiryCheckpointWriter,
    limits: &InquiryLimits,
    state: &mut InquiryState,
    inquiry_events: &mut Vec<InquiryEvent>,
) -> Result<ToolCallResult, String> {
    let bootstrap_run_id = format!("{}-bootstrap", checkpoint.run_id());
    let bootstrap_args = bootstrap_workflow_args(args.clone(), &bootstrap_run_id)?;
    let query = args
        .pointer("/input/query")
        .and_then(Value::as_str)
        .ok_or_else(|| "DeepResearch inquiry input omitted its query".to_string())?;
    let (planned, bootstrap) = tokio::join!(
        generate_plan(session, args, progress_tx, checkpoint),
        run_bootstrap_acquisition_stage(
            session,
            bootstrap_args,
            progress_tx,
            BOOTSTRAP_ACQUISITION_STAGE_TIMEOUT_MS,
        ),
    );
    let plan = match planned {
        Ok(plan) => plan,
        Err(error) => {
            tracing::warn!(
                %error,
                "DeepResearch semantic planning failed; continuing with the Host fallback contract"
            );
            host_fallback_plan(args)?
        }
    };
    apply_event(
        state,
        inquiry_events,
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        limits,
    )?;
    commit_plan_research_contract(&plan.value, state, inquiry_events, limits)?;
    queue_plan_questions(&plan.value, state, inquiry_events, limits)?;
    checkpoint.checkpoint(inquiry_events, state).await?;
    let acquisition = match bootstrap {
        Ok(result) => match bootstrap_acquisition_from_result(&result, query) {
            Some(acquisition) => Some(acquisition),
            None => {
                tracing::warn!(
                    "DeepResearch bootstrap acquisition returned no reusable raw source packet"
                );
                None
            }
        },
        Err(error) => {
            tracing::warn!(
                %error,
                "DeepResearch bootstrap acquisition failed; the run will retain an explicit acquisition gap"
            );
            None
        }
    };
    let Some(extraction_timeout_ms) =
        checkpoint.question_review_stage_timeout_ms(EVIDENCE_EXTRACTION_STAGE_TIMEOUT_MS)
    else {
        let reason =
            "the shared inquiry deadline left no batched evidence-extraction budget after acquisition";
        terminalize_budget_exhaustion(Some(checkpoint), state, inquiry_events, limits, reason)
            .await?;
        return attach_inquiry_projection(
            budget_terminal_result(args, reason),
            inquiry_events,
            state,
        );
    };
    let extraction = run_batched_evidence_extraction(
        session,
        query,
        &plan.value,
        acquisition.as_ref(),
        progress_tx,
        checkpoint,
        extraction_timeout_ms,
    )
    .await;
    apply_batched_evidence_extraction(&extraction, state, inquiry_events, limits, Some(checkpoint))
        .await?;
    let result = extraction.result;

    if state.phase == a3s::research::InquiryPhase::Outlining {
        let outcome = match assess_completed_research_contract(
            state,
            inquiry_events,
            limits,
            Some(checkpoint),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                let reason = bounded_inquiry_failure_reason(&error);
                terminalize_budget_exhaustion(
                    Some(checkpoint),
                    state,
                    inquiry_events,
                    limits,
                    &reason,
                )
                .await?;
                return attach_inquiry_projection(result, inquiry_events, state);
            }
        };
        if outcome == a3s::research::ResearchContractOutcome::Unsatisfied
            && state.phase != InquiryPhase::Exhausted
        {
            apply_event_and_checkpoint(
                Some(checkpoint),
                state,
                inquiry_events,
                InquiryEvent::BudgetExhausted {
                    reason: "the semantic-plan retrieval pass ended before the material research contract reached its minimum evidence floor".to_string(),
                },
                limits,
            )
            .await?;
        }
    }

    attach_inquiry_projection(result, inquiry_events, state)
}

fn bounded_inquiry_failure_reason(error: &str) -> String {
    const MAX_DETAIL_CHARS: usize = 1_000;

    let detail = error.split_whitespace().collect::<Vec<_>>().join(" ");
    let detail = if detail.is_empty() {
        "the stage returned no diagnostic"
    } else {
        detail.as_str()
    };
    let bounded = detail.chars().take(MAX_DETAIL_CHARS).collect::<String>();
    format!("DeepResearch inquiry stopped before completion: {bounded}")
}

#[derive(Clone, Debug)]
struct InquiryDeadline {
    deadline: Instant,
    question_review_reserve: Duration,
    finalization_reserve: Duration,
}

impl InquiryDeadline {
    fn from_started_at_ms(
        started_at_ms: u64,
        total_budget_ms: u64,
        question_review_reserve_ms: u64,
        finalization_reserve_ms: u64,
        now: Instant,
    ) -> Self {
        let Ok(wall_now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return Self::from_elapsed(
                now,
                total_budget_ms,
                question_review_reserve_ms,
                finalization_reserve_ms,
                u64::MAX,
            );
        };
        let wall_now_ms = wall_now.as_millis().min(u128::from(u64::MAX)) as u64;
        Self::from_wall_clock(
            started_at_ms,
            wall_now_ms,
            total_budget_ms,
            question_review_reserve_ms,
            finalization_reserve_ms,
            now,
        )
    }

    fn from_wall_clock(
        started_at_ms: u64,
        wall_now_ms: u64,
        total_budget_ms: u64,
        question_review_reserve_ms: u64,
        finalization_reserve_ms: u64,
        now: Instant,
    ) -> Self {
        // Monotonic time does not survive a process restart. If the durable
        // wall-clock origin is now in the future, fail closed instead of
        // granting this Inquiry another complete budget.
        let elapsed_ms = wall_now_ms.checked_sub(started_at_ms).unwrap_or(u64::MAX);
        Self::from_elapsed(
            now,
            total_budget_ms,
            question_review_reserve_ms,
            finalization_reserve_ms,
            elapsed_ms,
        )
    }

    fn from_elapsed(
        now: Instant,
        total_budget_ms: u64,
        question_review_reserve_ms: u64,
        finalization_reserve_ms: u64,
        elapsed_ms: u64,
    ) -> Self {
        let remaining_ms = total_budget_ms.saturating_sub(elapsed_ms);
        Self {
            deadline: now
                .checked_add(Duration::from_millis(remaining_ms))
                .unwrap_or(now),
            question_review_reserve: Duration::from_millis(
                question_review_reserve_ms.min(total_budget_ms),
            ),
            finalization_reserve: Duration::from_millis(
                finalization_reserve_ms.min(total_budget_ms),
            ),
        }
    }

    fn pre_review_stage_timeout_ms_at(
        &self,
        now: Instant,
        requested_timeout_ms: u64,
    ) -> Option<u64> {
        let available = self
            .deadline
            .saturating_duration_since(now)
            .saturating_sub(self.question_review_reserve)
            .saturating_sub(self.finalization_reserve);
        bounded_stage_timeout_ms(available, requested_timeout_ms)
    }

    fn question_review_stage_timeout_ms_at(
        &self,
        now: Instant,
        requested_timeout_ms: u64,
    ) -> Option<u64> {
        let available = self
            .deadline
            .saturating_duration_since(now)
            .saturating_sub(self.finalization_reserve);
        bounded_stage_timeout_ms(available, requested_timeout_ms)
    }
}

fn bounded_stage_timeout_ms(available: Duration, requested_timeout_ms: u64) -> Option<u64> {
    let available_ms = available.as_millis().min(u128::from(u64::MAX)) as u64;
    let selected = requested_timeout_ms.min(available_ms);
    (selected >= MIN_INQUIRY_STAGE_TIMEOUT_MS).then_some(selected)
}

/// The host inquiry owns exactly one sequential checkpoint writer. Each write
/// contains the complete validated event prefix, so retries are idempotent and
/// a timeout can recover the last event committed before the next tool await.
#[derive(Debug)]
struct InquiryCheckpointWriter {
    workspace: PathBuf,
    run_id: String,
    durable_events: Vec<InquiryEvent>,
    durable_state: InquiryState,
    persisted_events: AtomicUsize,
    deadline: InquiryDeadline,
}

impl InquiryCheckpointWriter {
    async fn initialize(session: &AgentSession, args: &Value) -> Result<Self, String> {
        let query = args
            .pointer("/input/query")
            .and_then(Value::as_str)
            .unwrap_or_default();
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
            total_budget_ms: DEEP_RESEARCH_INQUIRY_HOST_TIMEOUT_MS,
            retrieval_stage_budget_ms: DEEP_RESEARCH_RETRIEVAL_STAGE_TIMEOUT_MS,
            question_review_stage_budget_ms: DEEP_RESEARCH_QUESTION_REVIEW_STAGE_TIMEOUT_MS,
            finalization_reserve_ms: DEEP_RESEARCH_INQUIRY_FINALIZATION_RESERVE_MS,
            host_pid: std::process::id(),
        };
        Self::initialize_with_spec(session, args, spec).await
    }

    async fn initialize_evidence_first(
        session: &AgentSession,
        args: &Value,
    ) -> Result<Self, String> {
        let spec = deep_research_evidence_first_research_spec(args);
        Self::initialize_with_spec(session, args, spec).await
    }

    async fn initialize_with_spec(
        session: &AgentSession,
        args: &Value,
        spec: ResearchSpec,
    ) -> Result<Self, String> {
        let run_id = args
            .get("run_id")
            .and_then(Value::as_str)
            .filter(|run_id| !run_id.trim().is_empty())
            .ok_or_else(|| "DeepResearch inquiry checkpointing requires a run_id".to_string())?
            .to_string();
        let workspace = session.workspace().to_path_buf();
        let (durable_events, durable_state) = load_inquiry_state(&workspace, &run_id)
            .await
            .map_err(|error| format!("load existing DeepResearch inquiry prefix: {error}"))?
            .unwrap_or_else(|| (Vec::new(), InquiryState::default()));
        let total_budget_ms = spec.total_budget_ms;
        let question_review_stage_budget_ms = spec.question_review_stage_budget_ms;
        let finalization_reserve_ms = spec.finalization_reserve_ms;
        let mut last_error = None;
        for attempt in 0..JOURNAL_INITIALIZATION_ATTEMPTS {
            match record_workflow_started(&workspace, &run_id, spec.clone()).await {
                Ok(()) => {
                    let started_at_ms = load_research_run_started_at_ms(&workspace, &run_id)
                        .await
                        .map_err(|error| {
                            format!(
                                "load durable DeepResearch deadline origin for `{run_id}`: {error}"
                            )
                        })?
                        .ok_or_else(|| {
                            format!(
                                "DeepResearch run `{run_id}` persisted without a durable deadline origin"
                            )
                        })?;
                    let deadline = InquiryDeadline::from_started_at_ms(
                        started_at_ms,
                        total_budget_ms,
                        question_review_stage_budget_ms,
                        finalization_reserve_ms,
                        Instant::now(),
                    );
                    let persisted_events = durable_events.len();
                    return Ok(Self {
                        workspace,
                        run_id,
                        durable_events,
                        durable_state,
                        persisted_events: AtomicUsize::new(persisted_events),
                        deadline,
                    });
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

    fn pre_review_stage_timeout_ms(&self, requested_timeout_ms: u64) -> Option<u64> {
        self.deadline
            .pre_review_stage_timeout_ms_at(Instant::now(), requested_timeout_ms)
    }

    fn question_review_stage_timeout_ms(&self, requested_timeout_ms: u64) -> Option<u64> {
        self.deadline
            .question_review_stage_timeout_ms_at(Instant::now(), requested_timeout_ms)
    }

    fn run_id(&self) -> &str {
        &self.run_id
    }

    async fn checkpoint(
        &self,
        events: &[InquiryEvent],
        state: &InquiryState,
    ) -> Result<(), String> {
        let replayed = a3s::research::replay(events, &InquiryLimits::default())
            .map_err(|error| format!("strictly replay inquiry checkpoint prefix: {error}"))?;
        if &replayed != state {
            return Err(
                "DeepResearch inquiry checkpoint differs from its strict replay".to_string(),
            );
        }

        let shared_len = events.len().min(self.durable_events.len());
        if events[..shared_len] != self.durable_events[..shared_len] {
            let mismatch = events
                .iter()
                .zip(&self.durable_events)
                .position(|(replayed, durable)| replayed != durable)
                .unwrap_or(shared_len);
            return Err(format!(
                "DeepResearch inquiry replay for `{}` diverged from its durable prefix at event {}",
                self.run_id,
                mismatch.saturating_add(1)
            ));
        }

        let persisted = self.persisted_events.load(Ordering::Acquire);
        if events.len() < self.durable_events.len() {
            if persisted > self.durable_events.len() {
                return Err(format!(
                    "DeepResearch inquiry checkpoint for `{}` regressed from {persisted} to {} events",
                    self.run_id,
                    events.len()
                ));
            }
            return Ok(());
        }
        if events.len() == self.durable_events.len() {
            if state != &self.durable_state {
                return Err(format!(
                    "DeepResearch inquiry replay for `{}` reached its durable event head with a different state",
                    self.run_id
                ));
            }
            if persisted == events.len() {
                return Ok(());
            }
        }
        if events.len() < persisted {
            return Err(format!(
                "DeepResearch inquiry checkpoint for `{}` regressed from {persisted} to {} events",
                self.run_id,
                events.len()
            ));
        }
        if events.len() == persisted {
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
#[path = "inquiry_runtime/evidence_first_tests.rs"]
mod evidence_first_tests;
#[cfg(test)]
#[path = "inquiry_runtime/integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "inquiry_runtime/process_resume_tests.rs"]
mod process_resume_tests;
#[cfg(test)]
#[path = "inquiry_runtime/tests.rs"]
mod tests;
