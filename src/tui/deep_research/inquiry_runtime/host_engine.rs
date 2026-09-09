//! Standalone DeepResearch host-engine adapter for hermetic capability tests.
//!
//! Production Interactive/smoke DeepResearch uses `CodeDeepResearchRunner`.
//! These ports keep evidence-first and product-adapter hermetics honest.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use a3s_code_core::{AgentEvent, AgentSession, ToolCallResult};
use a3s_deep_research::engine::{
    DeepResearchEngine, EngineLimits, GenerationRequest, GenerationStage, ProgressPort,
    PublicationPort, PublicationRequest, ResearchProgress, StructuredGenerationPort,
    WorkflowExecutionPort, WorkflowOutput, WorkflowRequest, WorkflowStage,
};
use serde_json::Value;
use tokio::sync::mpsc;

use self::execution::{
    run_bootstrap_acquisition_stage, run_dynamic_workflow, within_inquiry_stage_timeout_typed,
    InquiryStageError,
};
use super::super::deep_research_artifacts::{
    materialize_deep_research_admitted_report,
    materialize_deep_research_no_evidence_report_in_language,
    materialize_deep_research_source_backed_report_in_language,
    record_deep_research_publication_receipt_in_language, DeepResearchEvidenceFirstPublication,
    ResearchReportArtifacts,
};
use super::super::deep_research_state_journal::{
    load_research_run_started_at_ms, record_workflow_started,
};
use crate::deep_research_checkpoint::recover_initial_retrieval_checkpoint;
use super::deep_research_evidence_first_research_spec;

pub(crate) const PROGRESS_CHANNEL_CAPACITY: usize = 256;
const MIN_INQUIRY_STAGE_TIMEOUT_MS: u64 = 1_000;
const JOURNAL_INITIALIZATION_ATTEMPTS: usize = 8;
const JOURNAL_INITIALIZATION_RETRY_MS: u64 = 10;
const DURABLE_GENERATION_WORKFLOW_SOURCE: &str =
    a3s_deep_research::workflow::GENERATION_WORKFLOW_SOURCE;

#[derive(Clone, Copy, Debug)]
pub(super) struct EvidenceFirstRuntimeLimits {
    pub(super) bootstrap_stage_timeout_ms: u64,
    pub(super) planned_retrieval_stage_timeout_ms: u64,
    pub(super) report_proposal_attempt_timeout_ms: u64,
    pub(super) report_proposal_stage_timeout_ms: u64,
}

pub(super) struct A3sDeepResearchRuntime<'a> {
    pub(super) session: &'a AgentSession,
    pub(super) progress_tx: &'a mpsc::Sender<AgentEvent>,
    pub(super) run_clock: &'a EvidenceFirstRunClock,
}

#[async_trait::async_trait]
impl StructuredGenerationPort for A3sDeepResearchRuntime<'_> {
    async fn generate_object(&self, request: GenerationRequest) -> Result<Value, String> {
        let execution_timeout_ms = if request.stage == GenerationStage::Planning {
            self.run_clock
                .pre_report_stage_timeout_ms(request.execution_timeout_ms)
                .ok_or_else(|| {
                    "the shared DeepResearch deadline left no outline-planner budget after reserving report proposal and finalization"
                        .to_string()
                })?
        } else {
            request.execution_timeout_ms
        };
        let result = execution::call_generation_with_progress(
            self.session,
            request.arguments,
            self.progress_tx,
            self.run_clock,
            request.stage.label(),
            execution_timeout_ms,
            request.max_attempts,
        )
        .await?;
        execution::generated_object::<Value>(&result)
    }
}

#[async_trait::async_trait]
impl WorkflowExecutionPort for A3sDeepResearchRuntime<'_> {
    async fn execute_workflow(&self, mut request: WorkflowRequest) -> Result<WorkflowOutput, String> {
        // Same surgical Core batch-header patch as CodeDeepResearchRuntime: in
        // place only when the legacy matcher is still present, so Host fixture
        // tool rewrites stay intact.
        crate::research::apply_patched_retrieval_workflow_source(&mut request.arguments);
        let recovery_arguments = request.arguments.clone();
        let arguments = crate::research::validate_dynamic_workflow_arguments(request.arguments)?;
        let result = match request.stage {
            WorkflowStage::Bootstrap => {
                run_bootstrap_acquisition_stage(
                    self.session,
                    arguments,
                    self.progress_tx,
                    request.timeout_ms,
                )
                .await
            }
            WorkflowStage::PlannedRetrieval => {
                match within_inquiry_stage_timeout_typed(
                    run_dynamic_workflow(self.session, arguments, self.progress_tx),
                    request.timeout_ms,
                    request.stage.label(),
                )
                .await
                {
                    Ok(result) => Ok(result),
                    Err(error @ InquiryStageError::TimedOut { .. }) => {
                        if let Some(recovered) = recover_initial_retrieval_checkpoint(
                            self.session.workspace(),
                            &recovery_arguments,
                        )
                        .await
                        {
                            return Ok(recovered);
                        }
                        Err(error.to_string())
                    }
                    Err(InquiryStageError::Operation(error)) => Err(error),
                }
            }
        }?;
        Ok(WorkflowOutput {
            output: result.output,
            metadata: result.metadata,
        })
    }
}

#[async_trait::async_trait]
impl PublicationPort for A3sDeepResearchRuntime<'_> {
    async fn publish(
        &self,
        request: PublicationRequest,
    ) -> Result<ResearchReportArtifacts, String> {
        match request {
            PublicationRequest::SourceBacked {
                run_id,
                query,
                output_language,
                workflow_output,
                workflow_metadata,
                quality,
            } => {
                self.validate_publication_run_id(&run_id)?;
                let artifacts = materialize_deep_research_source_backed_report_in_language(
                    self.session.workspace(),
                    &query,
                    &workflow_output,
                    workflow_metadata.as_ref(),
                    &output_language,
                )?
                .ok_or_else(|| {
                    "source catalog disappeared before deterministic publication".to_string()
                })?;
                record_deep_research_publication_receipt_in_language(
                    self.session.workspace(),
                    &query,
                    &output_language,
                    &run_id,
                    DeepResearchEvidenceFirstPublication::SourceBacked,
                    quality,
                    &artifacts,
                )?;
                Ok(artifacts)
            }
            PublicationRequest::Synthesized {
                run_id,
                query,
                output_language,
                report,
                publication,
                quality,
            } => {
                self.validate_publication_run_id(&run_id)?;
                if !matches!(
                    publication,
                    DeepResearchEvidenceFirstPublication::Synthesized
                        | DeepResearchEvidenceFirstPublication::Qualified
                ) {
                    return Err(
                        "generated report publication requested a non-generated outcome"
                            .to_string(),
                    );
                }
                let artifacts = materialize_deep_research_admitted_report(
                    self.session.workspace(),
                    &query,
                    &report,
                )?;
                record_deep_research_publication_receipt_in_language(
                    self.session.workspace(),
                    &query,
                    &output_language,
                    &run_id,
                    publication,
                    quality,
                    &artifacts,
                )?;
                Ok(artifacts)
            }
            PublicationRequest::NoEvidence {
                run_id,
                query,
                output_language,
                quality,
            } => {
                self.validate_publication_run_id(&run_id)?;
                let artifacts = materialize_deep_research_no_evidence_report_in_language(
                    self.session.workspace(),
                    &query,
                    &output_language,
                )?;
                record_deep_research_publication_receipt_in_language(
                    self.session.workspace(),
                    &query,
                    &output_language,
                    &run_id,
                    DeepResearchEvidenceFirstPublication::NoEvidence,
                    quality,
                    &artifacts,
                )?;
                Ok(artifacts)
            }
        }
    }
}

impl A3sDeepResearchRuntime<'_> {
    fn validate_publication_run_id(&self, run_id: &str) -> Result<(), String> {
        if run_id == self.run_clock.run_id() {
            Ok(())
        } else {
            Err("publication request belongs to a different DeepResearch run".to_string())
        }
    }
}

#[async_trait::async_trait]
impl ProgressPort for A3sDeepResearchRuntime<'_> {
    async fn report_progress(&self, _progress: ResearchProgress) -> Result<(), String> {
        // A3S forwards the finer-grained tool event streams from each port.
        Ok(())
    }
}

#[path = "execution.rs"]
mod execution;

pub(super) async fn run_evidence_first_research_with_limits(
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
    let run_clock = EvidenceFirstRunClock::initialize(&session, &args).await?;
    execute_evidence_first_research(&session, &args, &progress_tx, &run_clock, limits).await
}

pub(super) async fn execute_evidence_first_research(
    session: &AgentSession,
    args: &Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
    run_clock: &EvidenceFirstRunClock,
    limits: EvidenceFirstRuntimeLimits,
) -> Result<ToolCallResult, String> {
    let runtime = A3sDeepResearchRuntime {
        session,
        progress_tx,
        run_clock,
    };
    let engine_limits = EngineLimits {
        bootstrap_stage_timeout_ms: limits.bootstrap_stage_timeout_ms,
        planned_retrieval_stage_timeout_ms: limits.planned_retrieval_stage_timeout_ms,
        report_attempt_timeout_ms: limits.report_proposal_attempt_timeout_ms,
        report_stage_timeout_ms: limits.report_proposal_stage_timeout_ms,
        ..EngineLimits::default()
    };
    let run = DeepResearchEngine::new(&runtime, &runtime, &runtime, &runtime)
        .with_limits(engine_limits)
        .execute(args.clone())
        .await
        .map_err(|error| error.to_string())?;
    Ok(ToolCallResult {
        name: "dynamic_workflow".to_string(),
        output: run.output_json(),
        exit_code: 0,
        metadata: None,
        error_kind: None,
    })
}

#[derive(Clone, Debug)]
pub(super) struct EvidenceFirstDeadline {
    deadline: Instant,
    report_reserve: Duration,
    finalization_reserve: Duration,
}

impl EvidenceFirstDeadline {
    fn from_started_at_ms(
        started_at_ms: u64,
        total_budget_ms: u64,
        report_reserve_ms: u64,
        finalization_reserve_ms: u64,
        now: Instant,
    ) -> Self {
        let elapsed_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .and_then(|wall_now_ms| wall_now_ms.checked_sub(started_at_ms))
            .unwrap_or(u64::MAX);
        let remaining_ms = total_budget_ms.saturating_sub(elapsed_ms);
        Self {
            deadline: now
                .checked_add(Duration::from_millis(remaining_ms))
                .unwrap_or(now),
            report_reserve: Duration::from_millis(report_reserve_ms.min(total_budget_ms)),
            finalization_reserve: Duration::from_millis(
                finalization_reserve_ms.min(total_budget_ms),
            ),
        }
    }

    fn pre_report_stage_timeout_ms(&self, now: Instant, requested_timeout_ms: u64) -> Option<u64> {
        let available = self
            .deadline
            .saturating_duration_since(now)
            .saturating_sub(self.report_reserve)
            .saturating_sub(self.finalization_reserve);
        let available_ms = available.as_millis().min(u128::from(u64::MAX)) as u64;
        let selected = requested_timeout_ms.min(available_ms);
        (selected >= MIN_INQUIRY_STAGE_TIMEOUT_MS).then_some(selected)
    }
}

/// Carries the durable run identity and one absolute budget origin shared by
/// every standalone-engine stage. It does not interpret query text or source
/// content.
#[derive(Debug)]
pub(super) struct EvidenceFirstRunClock {
    run_id: String,
    deadline: EvidenceFirstDeadline,
}

impl EvidenceFirstRunClock {
    pub(super) async fn initialize(session: &AgentSession, args: &Value) -> Result<Self, String> {
        let run_id = args
            .get("run_id")
            .and_then(Value::as_str)
            .filter(|run_id| !run_id.trim().is_empty())
            .ok_or_else(|| "DeepResearch runtime requires a run_id".to_string())?
            .to_string();
        let workspace = session.workspace();
        let spec = deep_research_evidence_first_research_spec(args);
        let total_budget_ms = spec.total_budget_ms;
        let report_reserve_ms = spec.question_review_stage_budget_ms;
        let finalization_reserve_ms = spec.finalization_reserve_ms;
        let mut last_error = None;
        for attempt in 0..JOURNAL_INITIALIZATION_ATTEMPTS {
            match record_workflow_started(workspace, &run_id, spec.clone()).await {
                Ok(()) => {
                    let started_at_ms = load_research_run_started_at_ms(workspace, &run_id)
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
                    return Ok(Self {
                        run_id,
                        deadline: EvidenceFirstDeadline::from_started_at_ms(
                            started_at_ms,
                            total_budget_ms,
                            report_reserve_ms,
                            finalization_reserve_ms,
                            Instant::now(),
                        ),
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
        Err(format!("initialize DeepResearch run journal: {detail}"))
    }

    fn pre_report_stage_timeout_ms(&self, requested_timeout_ms: u64) -> Option<u64> {
        self.deadline
            .pre_report_stage_timeout_ms(Instant::now(), requested_timeout_ms)
    }

    fn run_id(&self) -> &str {
        &self.run_id
    }
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

