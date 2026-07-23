use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use tokio::time::{interval_at, Duration};
use tokio_util::sync::CancellationToken;

use super::streaming::send_code_web_event;
use super::*;
use crate::budget::{budget_plan_for_effort_id, BudgetWorkload};
use crate::commands::code::research_runtime::{
    execute_deepresearch_query_in, DeepResearchReportStatus, DeepResearchReportSynthesis,
};

const RESEARCH_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const MAX_RESEARCH_REPORT_BYTES: u64 = 4 * 1024 * 1024;
const RESEARCH_CANCELLED_MESSAGE: &str = "DeepResearch was cancelled by the user.";

impl KernelService {
    pub(super) async fn stream_deep_research_turn(
        &self,
        session: Arc<AgentSession>,
        turn: CodeWebQueuedTurn,
        visible_content: String,
    ) -> BootResult<BootResponse> {
        if turn.kind != CodeWebQueuedTurnKind::User {
            self.restore_queued_turn(session.session_id(), &turn.id)
                .await?;
            return Err(BootError::BadRequest(
                "DeepResearch mode is only available for user turns".to_string(),
            ));
        }

        let session_id = session.session_id().to_string();
        let cancellation = Arc::new(CancellationToken::new());
        self.state
            .active_research_runs
            .lock()
            .await
            .insert(session_id.clone(), Arc::clone(&cancellation));
        if let Err(error) = self
            .append_message(&session_id, "user", &visible_content, None)
            .await
        {
            self.state
                .active_research_runs
                .lock()
                .await
                .remove(&session_id);
            let _ = self.finish_queued_turn(&session_id, &turn.id, true).await;
            return Err(error);
        }

        let service = Self::new(Arc::clone(&self.state));
        let workspace = session.workspace().to_path_buf();
        let query = turn.content.clone();
        let turn_id = turn.id.clone();
        let (sender, receiver) = tokio::sync::mpsc::channel::<BootResult<SseEvent>>(64);
        tokio::spawn(async move {
            let started_at = Instant::now();
            let tool_id = format!("deep-research-{turn_id}");
            let tool_args = json!({
                "query": query.clone(),
                "scope": "webAndWorkspace",
            });
            let mut events = Vec::new();
            emit_research_event(
                &sender,
                &mut events,
                AgentEvent::Start {
                    prompt: format!("DeepResearch: {query}"),
                },
            )
            .await;
            emit_research_event(
                &sender,
                &mut events,
                AgentEvent::AgentModeChanged {
                    mode: "deep_research".to_string(),
                    agent: "deep-research".to_string(),
                    description: "Collecting and validating web and workspace evidence before publishing a report."
                        .to_string(),
                },
            )
            .await;
            emit_research_event(
                &sender,
                &mut events,
                AgentEvent::ToolStart {
                    id: tool_id.clone(),
                    name: "deep_research".to_string(),
                },
            )
            .await;
            emit_research_event(
                &sender,
                &mut events,
                AgentEvent::ToolExecutionStart {
                    id: tool_id.clone(),
                    name: "deep_research".to_string(),
                    args: tool_args.clone(),
                },
            )
            .await;

            let mut code_config = service.state.code_config_snapshot();
            if let Some(model) = service.session_response_model(&session_id).await {
                code_config.default_model = Some(model);
            }
            let memory_dir = research_memory_dir(&workspace, code_config.memory_dir.as_deref());
            let controls = service.session_controls_snapshot(&session_id).await;
            let budget =
                budget_plan_for_effort_id(&controls.effort, None, BudgetWorkload::DeepResearch);
            let research = execute_deepresearch_query_in(
                &query,
                Some(crate::tui::DeepResearchEvidenceScope::WebAndWorkspace),
                budget,
                &workspace,
                code_config,
                memory_dir,
            );
            tokio::pin!(research);
            let mut heartbeat = interval_at(
                tokio::time::Instant::now() + RESEARCH_HEARTBEAT_INTERVAL,
                RESEARCH_HEARTBEAT_INTERVAL,
            );
            let completion = loop {
                tokio::select! {
                    _ = cancellation.cancelled() => break ResearchCompletion::Cancelled,
                    result = &mut research => {
                        break match result {
                            Ok(synthesis) => ResearchCompletion::Published(synthesis),
                            Err(error) => ResearchCompletion::Failed(error.to_string()),
                        };
                    }
                    _ = heartbeat.tick() => {
                        emit_research_event(
                            &sender,
                            &mut events,
                            AgentEvent::ToolOutputDelta {
                                id: tool_id.clone(),
                                name: "deep_research".to_string(),
                                delta: "DeepResearch is still gathering and validating sources.\n".to_string(),
                            },
                        ).await;
                    }
                }
            };

            let succeeded = match completion {
                ResearchCompletion::Published(synthesis) => {
                    publish_research_completion(
                        &service,
                        &sender,
                        &mut events,
                        &session_id,
                        &workspace,
                        &tool_id,
                        tool_args,
                        started_at,
                        synthesis,
                    )
                    .await
                }
                ResearchCompletion::Cancelled => {
                    publish_research_cancellation(
                        &service,
                        &sender,
                        &mut events,
                        &session_id,
                        &tool_id,
                        tool_args,
                        started_at,
                    )
                    .await;
                    false
                }
                ResearchCompletion::Failed(error) => {
                    let message = format!("DeepResearch failed: {error}");
                    publish_research_failure(
                        &service,
                        &sender,
                        &mut events,
                        &session_id,
                        &tool_id,
                        tool_args,
                        started_at,
                        &message,
                    )
                    .await;
                    false
                }
            };

            let mut active = service.state.active_research_runs.lock().await;
            if active
                .get(&session_id)
                .is_some_and(|current| Arc::ptr_eq(current, &cancellation))
            {
                active.remove(&session_id);
            }
            drop(active);
            let _ = service
                .finish_queued_turn(&session_id, &turn_id, !succeeded)
                .await;
        });

        let stream = futures::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|event| (event, receiver))
        });
        Ok(BootResponse::sse(stream))
    }

    pub(in crate::api::code_web) async fn read_deep_research_report(
        &self,
        session_id: &str,
        relative_path: String,
    ) -> BootResult<Vec<u8>> {
        let session = self.kernel_session(session_id).await?;
        let report_path = validated_report_path(session.workspace(), &relative_path).await?;
        let metadata = tokio::fs::metadata(&report_path)
            .await
            .map_err(report_io_error)?;
        if metadata.len() > MAX_RESEARCH_REPORT_BYTES {
            return Err(BootError::BadRequest(format!(
                "DeepResearch report exceeds the {MAX_RESEARCH_REPORT_BYTES}-byte display limit"
            )));
        }
        tokio::fs::read(report_path).await.map_err(report_io_error)
    }
}

enum ResearchCompletion {
    Published(DeepResearchReportSynthesis),
    Cancelled,
    Failed(String),
}

#[allow(clippy::too_many_arguments)]
async fn publish_research_completion(
    service: &KernelService,
    sender: &tokio::sync::mpsc::Sender<BootResult<SseEvent>>,
    events: &mut Vec<AgentEvent>,
    session_id: &str,
    workspace: &Path,
    tool_id: &str,
    tool_args: Value,
    started_at: Instant,
    synthesis: DeepResearchReportSynthesis,
) -> bool {
    let artifacts = match research_artifact_metadata(workspace, &synthesis) {
        Ok(artifacts) => artifacts,
        Err(error) => {
            publish_research_failure(
                service, sender, events, session_id, tool_id, tool_args, started_at, &error,
            )
            .await;
            return false;
        }
    };
    let status = report_status_id(synthesis.status);
    emit_research_event(
        sender,
        events,
        AgentEvent::ToolEnd {
            id: tool_id.to_string(),
            name: "deep_research".to_string(),
            args: Some(tool_args),
            output: format!("DeepResearch published a {status} report."),
            exit_code: 0,
            metadata: Some(json!({
                "duration_ms": duration_millis(started_at),
                "report": artifacts,
            })),
            error_kind: None,
        },
    )
    .await;
    let end = AgentEvent::End {
        text: synthesis.text.clone(),
        usage: TokenUsage::default(),
        verification_summary: Box::new(
            a3s_code_core::verification::VerificationSummary::from_reports(&[]),
        ),
        meta: None,
    };
    events.push(end.clone());
    if let Err(error) = service
        .append_message_with_events(
            session_id,
            "assistant",
            &synthesis.text,
            service.session_response_model(session_id).await,
            events,
        )
        .await
    {
        events.pop();
        let message = format!(
            "DeepResearch report was written but its Web response could not be saved: {error}"
        );
        emit_research_event(sender, events, AgentEvent::Error { message }).await;
        return false;
    }
    send_code_web_event(sender, &end).await;
    true
}

#[allow(clippy::too_many_arguments)]
async fn publish_research_failure(
    service: &KernelService,
    sender: &tokio::sync::mpsc::Sender<BootResult<SseEvent>>,
    events: &mut Vec<AgentEvent>,
    session_id: &str,
    tool_id: &str,
    tool_args: Value,
    started_at: Instant,
    message: &str,
) {
    emit_research_event(
        sender,
        events,
        AgentEvent::ToolEnd {
            id: tool_id.to_string(),
            name: "deep_research".to_string(),
            args: Some(tool_args),
            output: message.to_string(),
            exit_code: 1,
            metadata: Some(json!({ "duration_ms": duration_millis(started_at) })),
            error_kind: None,
        },
    )
    .await;
    let error = AgentEvent::Error {
        message: message.to_string(),
    };
    events.push(error.clone());
    let _ = service
        .append_message_with_events(
            session_id,
            "assistant",
            message,
            service.session_response_model(session_id).await,
            events,
        )
        .await;
    send_code_web_event(sender, &error).await;
}

#[allow(clippy::too_many_arguments)]
async fn publish_research_cancellation(
    service: &KernelService,
    sender: &tokio::sync::mpsc::Sender<BootResult<SseEvent>>,
    events: &mut Vec<AgentEvent>,
    session_id: &str,
    tool_id: &str,
    tool_args: Value,
    started_at: Instant,
) {
    let (tool_end, terminal) =
        research_cancellation_events(tool_id, tool_args, duration_millis(started_at));
    emit_research_event(sender, events, tool_end).await;
    events.push(terminal.clone());
    if let Err(error) = service
        .append_message_with_events(
            session_id,
            "assistant",
            RESEARCH_CANCELLED_MESSAGE,
            service.session_response_model(session_id).await,
            events,
        )
        .await
    {
        events.pop();
        let message =
            format!("DeepResearch was cancelled but its Web response could not be saved: {error}");
        emit_research_event(sender, events, AgentEvent::Error { message }).await;
        return;
    }
    send_code_web_event(sender, &terminal).await;
}

fn research_cancellation_events(
    tool_id: &str,
    tool_args: Value,
    duration_ms: u64,
) -> (AgentEvent, AgentEvent) {
    (
        AgentEvent::ToolEnd {
            id: tool_id.to_string(),
            name: "deep_research".to_string(),
            args: Some(tool_args),
            output: RESEARCH_CANCELLED_MESSAGE.to_string(),
            exit_code: 1,
            metadata: Some(json!({
                "duration_ms": duration_ms,
                "cancelled": true,
                "message": RESEARCH_CANCELLED_MESSAGE,
            })),
            error_kind: Some(ToolErrorKind::Cancelled {
                op: "deep_research".to_string(),
            }),
        },
        AgentEvent::End {
            text: RESEARCH_CANCELLED_MESSAGE.to_string(),
            usage: TokenUsage::default(),
            verification_summary: Box::new(
                a3s_code_core::verification::VerificationSummary::from_reports(&[]),
            ),
            meta: None,
        },
    )
}

async fn emit_research_event(
    sender: &tokio::sync::mpsc::Sender<BootResult<SseEvent>>,
    events: &mut Vec<AgentEvent>,
    event: AgentEvent,
) {
    send_code_web_event(sender, &event).await;
    events.push(event);
}

fn research_memory_dir(workspace: &Path, configured: Option<&Path>) -> PathBuf {
    match configured {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => workspace.join(path),
        None => workspace.join(".a3s/memory"),
    }
}

fn research_artifact_metadata(
    workspace: &Path,
    synthesis: &DeepResearchReportSynthesis,
) -> Result<Value, String> {
    Ok(json!({
        "status": report_status_id(synthesis.status),
        "markdownPath": relative_research_artifact(workspace, &synthesis.artifacts.markdown, "report.md")?,
        "htmlPath": relative_research_artifact(workspace, &synthesis.artifacts.html, "index.html")?,
    }))
}

fn relative_research_artifact(
    workspace: &Path,
    artifact: &Path,
    expected_name: &str,
) -> Result<String, String> {
    let relative = artifact.strip_prefix(workspace).map_err(|_| {
        format!(
            "DeepResearch artifact escaped the active workspace: {}",
            artifact.display()
        )
    })?;
    let normalized = relative.to_string_lossy().replace('\\', "/");
    if !normalized.starts_with(".a3s/research/") || !normalized.ends_with(expected_name) {
        return Err(format!(
            "DeepResearch returned an invalid report artifact path: {}",
            artifact.display()
        ));
    }
    Ok(normalized)
}

fn report_status_id(status: DeepResearchReportStatus) -> &'static str {
    match status {
        DeepResearchReportStatus::Completed => "completed",
        DeepResearchReportStatus::Qualified => "qualified",
        DeepResearchReportStatus::Degraded => "degraded",
    }
}

fn duration_millis(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

async fn validated_report_path(workspace: &Path, relative_path: &str) -> BootResult<PathBuf> {
    let relative_path = Path::new(relative_path.trim());
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !relative_path.starts_with(".a3s/research")
        || relative_path.file_name().and_then(|name| name.to_str()) != Some("index.html")
    {
        return Err(BootError::BadRequest(
            "path must identify a generated .a3s/research/*/index.html report".to_string(),
        ));
    }
    let workspace = tokio::fs::canonicalize(workspace)
        .await
        .map_err(report_io_error)?;
    let report_root = workspace.join(".a3s/research");
    let candidate = tokio::fs::canonicalize(workspace.join(relative_path))
        .await
        .map_err(report_io_error)?;
    if !candidate.starts_with(&report_root) {
        return Err(BootError::Forbidden(
            "DeepResearch report path escapes the active workspace".to_string(),
        ));
    }
    Ok(candidate)
}

fn report_io_error(error: std::io::Error) -> BootError {
    match error.kind() {
        std::io::ErrorKind::NotFound => BootError::NotFound(error.to_string()),
        std::io::ErrorKind::PermissionDenied => BootError::Forbidden(error.to_string()),
        _ => BootError::Io(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_metadata_exposes_only_workspace_relative_report_paths() {
        let workspace = Path::new("/tmp/workspace");
        let synthesis = DeepResearchReportSynthesis {
            text: "report".to_string(),
            artifacts: crate::commands::code::research_runtime::ResearchReportArtifacts {
                markdown: workspace.join(".a3s/research/topic/report.md"),
                html: workspace.join(".a3s/research/topic/index.html"),
            },
            status: DeepResearchReportStatus::Completed,
        };

        let metadata = research_artifact_metadata(workspace, &synthesis).expect("report metadata");

        assert_eq!(metadata["status"], "completed");
        assert_eq!(metadata["markdownPath"], ".a3s/research/topic/report.md");
        assert_eq!(metadata["htmlPath"], ".a3s/research/topic/index.html");
    }

    #[test]
    fn cancelled_research_uses_a_typed_non_error_terminal_event() {
        let (tool_end, terminal) = research_cancellation_events(
            "deep-research-cancelled",
            json!({ "query": "cancel me" }),
            42,
        );
        let tool_end = serde_json::to_value(tool_end).expect("tool event");
        let terminal = serde_json::to_value(terminal).expect("terminal event");

        assert_eq!(tool_end["type"], "tool_end");
        assert_eq!(tool_end["exit_code"], 1);
        assert_eq!(tool_end["error_kind"]["type"], "cancelled");
        assert_eq!(tool_end["metadata"]["cancelled"], true);
        assert_eq!(terminal["type"], "agent_end");
        assert_eq!(terminal["text"], "DeepResearch was cancelled by the user.");
    }

    #[tokio::test]
    async fn report_path_rejects_traversal_and_symlink_escape() {
        let workspace = tempfile::tempdir().expect("workspace");
        let report_dir = workspace.path().join(".a3s/research/topic");
        std::fs::create_dir_all(&report_dir).expect("report directory");
        std::fs::write(report_dir.join("index.html"), "<!doctype html>").expect("report");

        let valid = validated_report_path(workspace.path(), ".a3s/research/topic/index.html")
            .await
            .expect("valid report path");
        assert!(valid.ends_with(".a3s/research/topic/index.html"));
        assert!(validated_report_path(workspace.path(), "../index.html")
            .await
            .is_err());
        assert!(
            validated_report_path(workspace.path(), ".a3s/research/topic/report.md")
                .await
                .is_err()
        );
    }
}
