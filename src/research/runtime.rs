use std::sync::Arc;
use std::time::Duration;

use a3s_code_core::{AgentEvent, AgentSession, ToolCallResult};
use a3s_deep_research::engine::{
    DeepResearchEvent, GenerationRequest, ProgressPort, PublicationPort, PublicationRequest,
    StructuredGenerationPort, WorkflowExecutionPort, WorkflowOutput, WorkflowRequest,
    WorkflowStage,
};
use a3s_deep_research::report::{
    materialize_deep_research_admitted_report_for_run,
    materialize_deep_research_no_evidence_report_for_run_in_language,
    materialize_deep_research_source_backed_report_for_run_in_language,
    record_deep_research_publication_receipt_in_language, DeepResearchEvidenceFirstPublication,
    ResearchReportArtifacts,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::journal::CodeDeepResearchJournal;
use super::CodeDeepResearchEvent;
use crate::deep_research_checkpoint::recover_initial_retrieval_checkpoint;

pub(super) struct CodeDeepResearchRuntime {
    session: Arc<AgentSession>,
    run_id: String,
    journal: Arc<CodeDeepResearchJournal>,
    events: mpsc::Sender<CodeDeepResearchEvent>,
}

impl CodeDeepResearchRuntime {
    pub(super) fn new(
        session: Arc<AgentSession>,
        run_id: String,
        journal: Arc<CodeDeepResearchJournal>,
        events: mpsc::Sender<CodeDeepResearchEvent>,
    ) -> Self {
        Self {
            session,
            run_id,
            journal,
            events,
        }
    }

    fn validate_run_id(&self, run_id: &str) -> Result<(), String> {
        if run_id == self.run_id {
            Ok(())
        } else {
            Err("publication request belongs to a different DeepResearch run".to_string())
        }
    }

    fn emit_agent_event(&self, event: AgentEvent) {
        let _ = self.events.try_send(CodeDeepResearchEvent::Agent(event));
    }

    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        filter_dynamic_workflow_envelope: bool,
    ) -> Result<ToolCallResult, String> {
        let (mut progress_rx, mut join) = self.session.tool_with_events(name, args);
        let abort = join.abort_handle();
        let mut abort_on_drop = AbortInnerToolOnDrop(Some(abort));
        let mut progress_open = true;
        let result = loop {
            if !progress_open {
                let result = join
                    .await
                    .map_err(|error| format!("{name} task failed: {error}"))?
                    .map_err(|error| format!("{name} failed: {error}"));
                abort_on_drop.disarm();
                break result;
            }
            tokio::select! {
                biased;
                event = progress_rx.recv() => {
                    let Some(event) = event else {
                        progress_open = false;
                        continue;
                    };
                    if filter_dynamic_workflow_envelope
                        && is_dynamic_workflow_envelope(&event)
                    {
                        continue;
                    }
                    self.emit_agent_event(event);
                }
                result = &mut join => {
                    let result = result
                        .map_err(|error| format!("{name} task failed: {error}"))?
                        .map_err(|error| format!("{name} failed: {error}"));
                    abort_on_drop.disarm();
                    break result;
                }
            }
        };
        while let Ok(event) = progress_rx.try_recv() {
            if filter_dynamic_workflow_envelope && is_dynamic_workflow_envelope(&event) {
                continue;
            }
            self.emit_agent_event(event);
        }
        result
    }

    async fn call_generation(&self, request: GenerationRequest) -> Result<ToolCallResult, String> {
        if !(1..=2).contains(&request.max_attempts) {
            return Err("structured generation requires one or two attempts".to_string());
        }
        let stage_label = request.stage.label();
        // Host-direct generate_object: nested dynamic_workflow/PTC for a single
        // structured call was collapsing to opaque `program script error:` and
        // degrading planner/report quality to no_evidence. Retries stay Host-
        // owned under the stage budget DeepResearch already declares.
        let attempt_timeout_ms = request
            .arguments
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .unwrap_or(request.execution_timeout_ms);
        let deadline = Instant::now() + Duration::from_millis(request.execution_timeout_ms.max(1));
        let mut last_error = None;
        for attempt in 1..=request.max_attempts {
            let Some(budget_ms) = generation_attempt_budget_ms(deadline, attempt_timeout_ms) else {
                return Err(format!(
                    "{stage_label} generation timed out after {} ms",
                    request.execution_timeout_ms
                ));
            };
            let mut arguments = request.arguments.clone();
            if let Some(object) = arguments.as_object_mut() {
                object.insert("timeout_ms".to_string(), Value::from(budget_ms));
            }
            match tokio::time::timeout(
                Duration::from_millis(budget_ms),
                self.call_tool("generate_object", arguments, false),
            )
            .await
            {
                Ok(Ok(result)) if result.exit_code == 0 => return Ok(result),
                Ok(Ok(result)) => {
                    last_error = Some(
                        result
                            .output
                            .lines()
                            .next()
                            .unwrap_or("structured generation failed")
                            .to_string(),
                    );
                }
                Ok(Err(error)) => last_error = Some(error),
                Err(_) => {
                    last_error = Some(format!(
                        "{stage_label} generation attempt {attempt} timed out after {budget_ms} ms"
                    ));
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            format!(
                "{stage_label} generation failed after {} attempts",
                request.max_attempts
            )
        }))
    }
}

#[async_trait::async_trait]
impl StructuredGenerationPort for CodeDeepResearchRuntime {
    async fn generate_object(&self, request: GenerationRequest) -> Result<Value, String> {
        let result = self.call_generation(request).await?;
        generated_object::<Value>(&result)
    }
}

#[async_trait::async_trait]
impl WorkflowExecutionPort for CodeDeepResearchRuntime {
    async fn execute_workflow(
        &self,
        mut request: WorkflowRequest,
    ) -> Result<WorkflowOutput, String> {
        // DeepResearch 0.1.5+ embeds Core-compatible staged headers. Older
        // 0.1.4 embeds still need the Host patch for legacy batch headers.
        super::apply_patched_retrieval_workflow_source(&mut request.arguments);
        let recovery_arguments = request.arguments.clone();
        let arguments = validate_dynamic_workflow_arguments(request.arguments)?;
        let result = match tokio::time::timeout(
            Duration::from_millis(request.timeout_ms),
            self.call_tool("dynamic_workflow", arguments, true),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) if request.stage == WorkflowStage::PlannedRetrieval => {
                return recover_initial_retrieval_checkpoint(
                    self.session.workspace(),
                    &recovery_arguments,
                )
                .await
                .ok_or_else(|| {
                    format!(
                        "DeepResearch {} timed out after {} ms",
                        request.stage.label(),
                        request.timeout_ms
                    )
                });
            }
            Err(_) => {
                return Err(format!(
                    "DeepResearch {} timed out after {} ms",
                    request.stage.label(),
                    request.timeout_ms
                ));
            }
        };
        if result.exit_code != 0 {
            return Err(result
                .output
                .lines()
                .next()
                .unwrap_or("dynamic_workflow failed without a diagnostic")
                .to_string());
        }
        Ok(WorkflowOutput {
            output: result.output,
            metadata: result.metadata,
        })
    }
}

/// Enforce the latest bounded generation fan-out contract before forwarding a
/// DeepResearch workflow to Code Core.
///
/// DeepResearch 0.1.3 emits `maxConcurrentGenerations`, and Code Core 8.1.0
/// accepts values from 1 through 4. Older CLI builds stripped the field for an
/// earlier Core schema, silently forcing every workflow back to single-flight
/// generation. Keep the field intact and fail explicitly if an incompatible
/// producer supplies anything outside the pinned schema.
pub(crate) fn validate_dynamic_workflow_arguments(arguments: Value) -> Result<Value, String> {
    let Some(value) = arguments.pointer("/limits/maxConcurrentGenerations") else {
        return Ok(arguments);
    };
    let Some(maximum) = value.as_u64() else {
        return Err(
            "dynamic_workflow limits.maxConcurrentGenerations must be an integer from 1 through 4"
                .to_string(),
        );
    };
    if !(1..=4).contains(&maximum) {
        return Err(
            "dynamic_workflow limits.maxConcurrentGenerations must be between 1 and 4".to_string(),
        );
    }
    Ok(arguments)
}

#[async_trait::async_trait]
impl PublicationPort for CodeDeepResearchRuntime {
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
                self.validate_run_id(&run_id)?;
                let artifacts = materialize_deep_research_source_backed_report_for_run_in_language(
                    self.session.workspace(),
                    &run_id,
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
                self.validate_run_id(&run_id)?;
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
                let artifacts = materialize_deep_research_admitted_report_for_run(
                    self.session.workspace(),
                    &run_id,
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
                self.validate_run_id(&run_id)?;
                let artifacts = materialize_deep_research_no_evidence_report_for_run_in_language(
                    self.session.workspace(),
                    &run_id,
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

#[async_trait::async_trait]
impl ProgressPort for CodeDeepResearchRuntime {
    async fn report_progress(
        &self,
        progress: a3s_deep_research::engine::ResearchProgress,
    ) -> Result<(), String> {
        let event = match progress {
            a3s_deep_research::engine::ResearchProgress::Started(stage) => {
                DeepResearchEvent::StageStarted {
                    run_id: self.run_id.clone(),
                    stage,
                }
            }
            a3s_deep_research::engine::ResearchProgress::Completed(stage) => {
                DeepResearchEvent::StageCompleted {
                    run_id: self.run_id.clone(),
                    stage,
                }
            }
            a3s_deep_research::engine::ResearchProgress::Degraded { stage, reason } => {
                DeepResearchEvent::StageDegraded {
                    run_id: self.run_id.clone(),
                    stage,
                    reason,
                }
            }
        };
        self.report_event(event).await
    }

    async fn report_event(&self, event: DeepResearchEvent) -> Result<(), String> {
        self.journal.append(&event).await?;
        let _ = self.events.try_send(CodeDeepResearchEvent::Engine(event));
        Ok(())
    }
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

fn generation_attempt_budget_ms(deadline: Instant, attempt_timeout_ms: u64) -> Option<u64> {
    let remaining_ms = deadline
        .saturating_duration_since(Instant::now())
        .as_millis();
    if remaining_ms == 0 {
        return None;
    }
    let remaining_ms = u64::try_from(remaining_ms).unwrap_or(u64::MAX);
    let budget = attempt_timeout_ms.max(1).min(remaining_ms);
    Some(budget)
}

fn generated_object<T: DeserializeOwned>(result: &ToolCallResult) -> Result<T, String> {
    if result.exit_code != 0 {
        return Err(result
            .output
            .lines()
            .next()
            .unwrap_or("structured generation failed")
            .to_string());
    }
    let envelope = serde_json::from_str::<Value>(&result.output)
        .map_err(|error| format!("structured generation returned invalid JSON: {error}"))?;
    let object = envelope
        .get("object")
        .cloned()
        .ok_or_else(|| "structured generation response omitted object".to_string())?;
    serde_json::from_value(object)
        .map_err(|error| format!("structured generation object violated its contract: {error}"))
}

fn is_dynamic_workflow_envelope(event: &AgentEvent) -> bool {
    match event {
        AgentEvent::ToolStart { name, .. }
        | AgentEvent::ToolExecutionStart { name, .. }
        | AgentEvent::ToolOutputDelta { name, .. }
        | AgentEvent::ToolEnd { name, .. } => name == "dynamic_workflow",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_arguments_preserve_code_core_8_1_generation_concurrency() {
        let arguments = a3s_deep_research::engine::DeepResearchRequest::new(
            "core-8-1-contract",
            "Audit the runtime contract",
            a3s_deep_research::engine::EvidenceScope::WebAndWorkspace,
        )
        .with_current_date("2026-07-29")
        .to_workflow_arguments()
        .expect("DeepResearch should compile its typed workflow request");

        let adapted = validate_dynamic_workflow_arguments(arguments)
            .expect("Code Core 8.1.0 accepts bounded generation concurrency");

        assert_eq!(adapted["run_id"], "core-8-1-contract");
        assert_eq!(
            adapted["limits"]["maxConcurrentGenerations"],
            a3s_deep_research::engine::DEFAULT_MAX_CONCURRENT_GENERATIONS
        );
    }

    #[test]
    fn workflow_arguments_reject_generation_concurrency_outside_the_core_schema() {
        for maximum in [
            serde_json::json!(0),
            serde_json::json!(5),
            serde_json::json!("2"),
        ] {
            let error = validate_dynamic_workflow_arguments(serde_json::json!({
                "source": "async function run() {}",
                "limits": { "maxConcurrentGenerations": maximum },
            }))
            .expect_err("invalid generation concurrency must fail before tool dispatch");

            assert!(error.contains("maxConcurrentGenerations"), "{error}");
        }
    }

    #[test]
    fn generation_attempt_budget_respects_stage_deadline() {
        let deadline = Instant::now() + Duration::from_millis(5_000);
        let budget = generation_attempt_budget_ms(deadline, 300_000).expect("budget");
        assert!(budget > 0 && budget <= 5_000, "budget={budget}");

        let past = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
        assert!(generation_attempt_budget_ms(past, 1_000).is_none());
    }
}
