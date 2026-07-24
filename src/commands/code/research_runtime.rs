//! Non-interactive DeepResearch execution and report synthesis.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_code_core::config::CodeConfig;
use a3s_code_core::{Agent, AgentSession, SessionOptions, ToolCallResult};

use crate::budget::{
    budget_plan_for_effort_index, BudgetPlan, BudgetWorkload, DEFAULT_TUI_EFFORT_INDEX,
};

const RESEARCH_TOOL_EXEC_TIMEOUT_MS: u64 = 30 * 60 * 1000;
const RESEARCH_DUPLICATE_TOOL_CALL_THRESHOLD: u32 = 12;

pub(crate) fn deep_research_default_budget() -> BudgetPlan {
    budget_plan_for_effort_index(DEFAULT_TUI_EFFORT_INDEX, None, BudgetWorkload::DeepResearch)
}

#[cfg(test)]
pub(crate) fn deep_research_workflow_args(query: &str) -> serde_json::Value {
    crate::tui::deep_research_cli_workflow_args_for_budget(
        query,
        deep_research_default_budget(),
        None,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeepResearchCliOptions {
    query: String,
    evidence_scope: Option<crate::tui::DeepResearchEvidenceScope>,
}

fn parse_deepresearch_args(args: &[String]) -> anyhow::Result<DeepResearchCliOptions> {
    let mut evidence_scope = None;
    let mut query_parts = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--local" | "--os" => {
                anyhow::bail!(
                    "DeepResearch runtime selection has been removed; use --web or --local-only to choose the evidence scope"
                )
            }
            "--local-only" | "--offline" => {
                if evidence_scope == Some(crate::tui::DeepResearchEvidenceScope::WebAndWorkspace) {
                    anyhow::bail!("--local-only conflicts with --web");
                }
                evidence_scope = Some(crate::tui::DeepResearchEvidenceScope::LocalOnly);
            }
            "--web" => {
                if evidence_scope == Some(crate::tui::DeepResearchEvidenceScope::LocalOnly) {
                    anyhow::bail!("--web conflicts with --local-only");
                }
                evidence_scope = Some(crate::tui::DeepResearchEvidenceScope::WebAndWorkspace);
            }
            "-h" | "--help" | "help" => {
                anyhow::bail!("usage: a3s code deepresearch [--local-only|--web] <query>");
            }
            value if value.starts_with('-') => {
                anyhow::bail!("unknown a3s code deepresearch option `{value}`")
            }
            value => query_parts.push(value.to_string()),
        }
    }
    let query = query_parts.join(" ").trim().to_string();
    if query.is_empty() {
        anyhow::bail!("usage: a3s code deepresearch [--local-only|--web] <query>");
    }
    Ok(DeepResearchCliOptions {
        query,
        evidence_scope,
    })
}

pub(crate) async fn execute_deepresearch_in(
    args: &[String],
    workspace: &Path,
    code_config: CodeConfig,
    memory_dir: PathBuf,
) -> anyhow::Result<DeepResearchReportSynthesis> {
    let opts = parse_deepresearch_args(args)?;
    execute_deepresearch_query_in(
        &opts.query,
        opts.evidence_scope,
        deep_research_default_budget(),
        workspace,
        code_config,
        memory_dir,
    )
    .await
}

pub(crate) async fn execute_deepresearch_query_in(
    query: &str,
    evidence_scope: Option<crate::tui::DeepResearchEvidenceScope>,
    budget: BudgetPlan,
    workspace: &Path,
    code_config: CodeConfig,
    memory_dir: PathBuf,
) -> anyhow::Result<DeepResearchReportSynthesis> {
    let query = query.trim();
    if query.is_empty() {
        anyhow::bail!("DeepResearch query must not be empty");
    }
    let workspace_text = workspace.to_string_lossy().to_string();
    let session = build_deepresearch_session(&workspace_text, code_config, memory_dir).await?;
    eprintln!("deepresearch: gathering evidence via the host-managed workflow…");
    let mut workflow_args =
        crate::tui::deep_research_cli_workflow_args_for_budget(query, budget, evidence_scope);
    let run_id = crate::tui::ensure_deep_research_workflow_run_id(&mut workflow_args)
        .ok_or_else(|| anyhow::anyhow!("failed to assign a DeepResearch workflow run ID"))?;
    let workflow = run_deepresearch_inquiry(Arc::clone(&session), workflow_args.clone()).await;
    let workflow_succeeded = workflow.as_ref().is_ok_and(|result| result.exit_code == 0);
    let (workflow_output, metadata) = match workflow {
        Ok(result) => (result.output, result.metadata),
        Err(error) => (error, None),
    };

    let (mut synthesis, artifact_authority) =
        match crate::tui::resolve_deep_research_run_publication(
            workspace,
            query,
            &run_id,
            &workflow_output,
        ) {
        Ok(Some(published)) => {
            let text = crate::tui::clean_deep_research_final_text_from_artifacts(
                &published.artifacts,
                workspace,
            )
            .unwrap_or_else(|| "DeepResearch report published without a text preview.".to_string());
            let status = match published.publication {
                crate::tui::DeepResearchEvidenceFirstPublication::Synthesized => {
                    DeepResearchReportStatus::Completed
                }
                crate::tui::DeepResearchEvidenceFirstPublication::Qualified => {
                    DeepResearchReportStatus::Qualified
                }
                crate::tui::DeepResearchEvidenceFirstPublication::SourceBacked => {
                    DeepResearchReportStatus::Degraded
                }
                crate::tui::DeepResearchEvidenceFirstPublication::NoEvidence => {
                    DeepResearchReportStatus::Degraded
                }
            };
            (
                DeepResearchReportSynthesis {
                    text,
                    artifacts: ResearchReportArtifacts {
                        markdown: published.artifacts.markdown,
                        html: published.artifacts.html,
                    },
                    status,
                },
                crate::tui::DeepResearchTerminalArtifactAuthority::ValidatedPublication,
            )
        }
        Ok(None) => (
            materialize_deepresearch_cli_recovery(
                workspace,
                query,
                "the standalone DeepResearch engine returned without its required Host publication",
                &workflow_output,
                metadata.as_ref(),
            )?,
            crate::tui::DeepResearchTerminalArtifactAuthority::VerifiedRecovery,
        ),
        Err(error) => (
            materialize_deepresearch_cli_recovery(
                workspace,
                query,
                &format!("the standalone DeepResearch publication failed validation: {error}"),
                &workflow_output,
                metadata.as_ref(),
            )?,
            crate::tui::DeepResearchTerminalArtifactAuthority::VerifiedRecovery,
        ),
    };
    let requested_outcome = match synthesis.status {
        DeepResearchReportStatus::Completed => crate::tui::ResearchOutcome::Completed,
        DeepResearchReportStatus::Qualified => crate::tui::ResearchOutcome::Qualified,
        DeepResearchReportStatus::Degraded => crate::tui::ResearchOutcome::Degraded,
    };
    let journal_artifacts = crate::tui::ResearchReportArtifacts {
        markdown: synthesis.artifacts.markdown.clone(),
        html: synthesis.artifacts.html.clone(),
    };
    let settled_outcome =
        crate::tui::settle_deep_research_cli_run(crate::tui::DeepResearchCliSettlement {
            workspace,
            run_id: &run_id,
            query,
            workflow_succeeded,
            workflow_output: &workflow_output,
            workflow_metadata: metadata.as_ref(),
            requested_outcome,
            artifacts: &journal_artifacts,
            artifact_authority,
        })
        .await
        .map_err(anyhow::Error::msg)?;
    synthesis.status = match settled_outcome {
        crate::tui::ResearchOutcome::Completed => DeepResearchReportStatus::Completed,
        crate::tui::ResearchOutcome::Qualified => DeepResearchReportStatus::Qualified,
        crate::tui::ResearchOutcome::Degraded | crate::tui::ResearchOutcome::Failed => {
            DeepResearchReportStatus::Degraded
        }
        crate::tui::ResearchOutcome::Active => {
            return Err(anyhow::anyhow!(
                "DeepResearch CLI journal remained active after terminal settlement"
            ));
        }
    };
    Ok(synthesis)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeepResearchReportStatus {
    Completed,
    Qualified,
    Degraded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResearchReportArtifacts {
    pub(crate) markdown: PathBuf,
    pub(crate) html: PathBuf,
}

#[derive(Debug)]
pub(crate) struct DeepResearchReportSynthesis {
    pub(crate) text: String,
    pub(crate) artifacts: ResearchReportArtifacts,
    pub(crate) status: DeepResearchReportStatus,
}

fn materialize_deepresearch_cli_recovery(
    workspace: &Path,
    query: &str,
    reason: &str,
    workflow_output: &str,
    metadata: Option<&serde_json::Value>,
) -> anyhow::Result<DeepResearchReportSynthesis> {
    let (text, markdown, html) = crate::tui::materialize_deep_research_cli_recovery_report(
        workspace,
        query,
        reason,
        workflow_output,
        metadata,
    )
    .map_err(anyhow::Error::msg)?;
    Ok(DeepResearchReportSynthesis {
        text,
        artifacts: ResearchReportArtifacts { markdown, html },
        status: DeepResearchReportStatus::Degraded,
    })
}

fn deepresearch_cli_permission_policy() -> a3s_code_core::permissions::PermissionPolicy {
    let mut policy = a3s_code_core::permissions::PermissionPolicy::new()
        .deny_all(&[
            "Write(/**)",
            "Edit(/**)",
            "Write(**/../**)",
            "Edit(**/../**)",
        ])
        .allow_all(&[
            "Read(*)",
            "Grep(*)",
            "Glob(*)",
            "LS(*)",
            "read(*)",
            "grep(*)",
            "glob(*)",
            "ls(*)",
            "web_search(*)",
            "web_fetch(*)",
        ]);
    policy.default_decision = a3s_code_core::permissions::PermissionDecision::Deny;
    policy
}

async fn build_deepresearch_session(
    workspace: &str,
    code_config: CodeConfig,
    memory_dir: PathBuf,
) -> anyhow::Result<Arc<AgentSession>> {
    build_deepresearch_session_with_resolver(
        workspace,
        code_config,
        memory_dir,
        crate::session_llm::resolve_session_llm_client,
    )
    .await
}

async fn build_deepresearch_session_with_resolver<F>(
    workspace: &str,
    code_config: CodeConfig,
    memory_dir: PathBuf,
    resolve_llm_client: F,
) -> anyhow::Result<Arc<AgentSession>>
where
    F: FnOnce(
        &CodeConfig,
        &SessionOptions,
        &str,
    ) -> Result<Arc<dyn a3s_code_core::llm::LlmClient>, String>,
{
    let permission_policy = deepresearch_cli_permission_policy();
    let session_id = deep_research_execution_id();
    let opts = SessionOptions::new()
        .with_session_id(&session_id)
        .with_confirmation_policy(a3s_code_core::hitl::ConfirmationPolicy::default())
        .with_permission_policy(permission_policy.clone())
        .with_tool_timeout(RESEARCH_TOOL_EXEC_TIMEOUT_MS)
        .with_duplicate_tool_call_threshold(RESEARCH_DUPLICATE_TOOL_CALL_THRESHOLD)
        .with_file_memory(memory_dir)
        // DeepResearch invokes only host-owned tools. Keep one manual `task`
        // slot for the optional local-workspace retrieval step; never expose
        // automatic delegation, parallel fan-out, or parent continuations.
        .with_continuation(false)
        .with_max_parallel_tasks(1)
        .with_auto_delegation_enabled(false)
        .with_auto_parallel_delegation(false)
        .with_manual_delegation_enabled(true);
    let llm_client = resolve_llm_client(&code_config, &opts, &session_id)
        .map_err(|error| anyhow::anyhow!("failed to resolve DeepResearch model: {error}"))?;
    let opts = opts.with_llm_client(llm_client);
    let agent = Agent::from_config(code_config)
        .await
        .map_err(|e| anyhow::anyhow!("failed to load DeepResearch agent: {e}"))?;
    let session = agent
        .session_async(workspace.to_string(), Some(opts))
        .await?;
    session.register_dynamic_workflow_runtime()?;
    Ok(Arc::new(session))
}

fn deep_research_execution_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("research-{nanos:016x}-{:x}", std::process::id())
}

async fn run_deepresearch_inquiry(
    session: Arc<AgentSession>,
    args: serde_json::Value,
) -> Result<ToolCallResult, String> {
    let timeout_ms = crate::tui::DEEP_RESEARCH_EVIDENCE_FIRST_HOST_TIMEOUT_MS;
    let (mut progress_rx, workflow_join) =
        crate::tui::spawn_deep_research_evidence_first(session, args);
    let workflow_abort = workflow_join.abort_handle();
    let progress_drain = tokio::spawn(async move { while progress_rx.recv().await.is_some() {} });
    let result = match tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        workflow_join,
    )
    .await
    {
        Ok(Ok(result)) => result.map_err(|err| err.to_string()),
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => {
            workflow_abort.abort();
            Err(format!(
                "DeepResearch timed out after {timeout_ms} ms while acquiring sources and publishing its Host-owned report"
            ))
        }
    };
    progress_drain.abort();
    result.map(|mut result| {
        result.output = crate::tui::deep_research_cli_canonical_workflow_output(
            &result.output,
            result.metadata.as_ref(),
        );
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    // Frozen replay tests stay isolated from production control flow.
    #[path = "baseline.rs"]
    mod baseline;
    #[path = "cli.rs"]
    mod cli;
    #[path = "workflow.rs"]
    mod workflow;
    use a3s_code_core::llm::{
        ContentBlock, LlmClient, LlmResponse, Message, StreamEvent, TokenUsage, ToolDefinition,
    };
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    struct ScriptedLlmClient {
        responses: Mutex<VecDeque<LlmResponse>>,
    }

    #[async_trait]
    impl LlmClient for ScriptedLlmClient {
        fn model_generation_concurrency(&self) -> a3s_code_core::llm::ModelGenerationConcurrency {
            a3s_code_core::llm::ModelGenerationConcurrency::bounded(
                std::num::NonZeroUsize::new(1).expect("scripted test concurrency is non-zero"),
            )
        }

        async fn complete(
            &self,
            messages: &[Message],
            system: Option<&str>,
            tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            Ok(self.response_for_messages(messages, system, tools))
        }

        async fn complete_streaming(
            &self,
            messages: &[Message],
            system: Option<&str>,
            tools: &[ToolDefinition],
            _cancel_token: CancellationToken,
        ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
            let response = self.response_for_messages(messages, system, tools);
            let (tx, rx) = mpsc::channel(1);
            tokio::spawn(async move {
                let _ = tx.send(StreamEvent::Done(response)).await;
            });
            Ok(rx)
        }

        fn native_structured_support(
            &self,
        ) -> a3s_code_core::llm::structured::NativeStructuredSupport {
            a3s_code_core::llm::structured::NativeStructuredSupport::ForcedTool
        }
    }

    impl ScriptedLlmClient {
        fn new(responses: Vec<LlmResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
            }
        }

        fn response_for_messages(
            &self,
            _messages: &[Message],
            _system: Option<&str>,
            _tools: &[ToolDefinition],
        ) -> LlmResponse {
            self.next_response()
        }

        fn next_response(&self) -> LlmResponse {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| text_response("DONE"))
        }
    }

    fn text_response(text: impl Into<String>) -> LlmResponse {
        LlmResponse {
            message: Message {
                role: "assistant".into(),
                content: vec![ContentBlock::Text { text: text.into() }],
                reasoning_content: None,
            },
            usage: TokenUsage::default(),
            stop_reason: Some("stop".into()),
            token_logprobs: Vec::new(),
            meta: None,
        }
    }
}
