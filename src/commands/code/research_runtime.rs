//! Non-interactive DeepResearch execution and report synthesis.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use a3s_code_core::config::CodeConfig;
use a3s_code_core::{Agent, AgentSession, SessionOptions, ToolCallResult};

mod evidence;
mod prompts;
mod report;
mod workflow_source;

pub(crate) use evidence::*;
pub(crate) use prompts::*;
pub(crate) use report::*;

use crate::budget::{
    budget_plan_for_effort_index, BudgetPlan, BudgetWorkload, DEFAULT_TUI_EFFORT_INDEX,
};

const RESEARCH_TOOL_EXEC_TIMEOUT_MS: u64 = 30 * 60 * 1000;
const RESEARCH_DUPLICATE_TOOL_CALL_THRESHOLD: u32 = 12;
pub(crate) const DEEP_RESEARCH_SYNTHESIS_TIMEOUT_MS: u64 = 8 * 60 * 1000;
pub(crate) const DEEP_RESEARCH_REPAIR_TIMEOUT_MS: u64 = 3 * 60 * 1000;

pub(crate) fn deep_research_default_budget() -> BudgetPlan {
    budget_plan_for_effort_index(DEFAULT_TUI_EFFORT_INDEX, None, BudgetWorkload::DeepResearch)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeepResearchRuntimeMode {
    Auto,
    Local,
    Os,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeepResearchCliOptions {
    query: String,
    runtime_mode: DeepResearchRuntimeMode,
}

fn parse_deepresearch_args(args: &[String]) -> anyhow::Result<DeepResearchCliOptions> {
    let mut runtime_mode = DeepResearchRuntimeMode::Auto;
    let mut query_parts = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--local" => runtime_mode = DeepResearchRuntimeMode::Local,
            "--os" => runtime_mode = DeepResearchRuntimeMode::Os,
            "-h" | "--help" | "help" => {
                anyhow::bail!("usage: a3s code deepresearch [--local|--os] <query>");
            }
            value if value.starts_with('-') => {
                anyhow::bail!("unknown a3s code deepresearch option `{value}`")
            }
            value => query_parts.push(value.to_string()),
        }
    }
    let query = query_parts.join(" ").trim().to_string();
    if query.is_empty() {
        anyhow::bail!("usage: a3s code deepresearch [--local|--os] <query>");
    }
    Ok(DeepResearchCliOptions {
        query,
        runtime_mode,
    })
}

pub(crate) async fn execute_deepresearch_in(
    args: &[String],
    workspace: &Path,
    code_config: CodeConfig,
    memory_dir: PathBuf,
) -> anyhow::Result<DeepResearchReportSynthesis> {
    let opts = parse_deepresearch_args(args)?;
    if opts.runtime_mode == DeepResearchRuntimeMode::Os {
        anyhow::bail!(
            "--os is temporarily disabled for DeepResearch; OS Runtime support should use Function-as-a-Service instead of remote tool-call fan-out"
        );
    }
    let workspace_text = workspace.to_string_lossy().to_string();
    let (session, report_tool_gate) =
        build_deepresearch_session(&workspace_text, code_config, memory_dir).await?;
    let os_runtime = match opts.runtime_mode {
        DeepResearchRuntimeMode::Local => false,
        DeepResearchRuntimeMode::Os => false,
        DeepResearchRuntimeMode::Auto => false,
    };

    eprintln!(
        "deepresearch: gathering evidence via {} workflow…",
        if os_runtime { "OS Runtime" } else { "local" }
    );
    let workflow_args = deep_research_workflow_args(&opts.query, os_runtime);
    let workflow = run_deepresearch_workflow(&session, workflow_args.clone()).await;
    let (workflow_output, exit_code, metadata) = match workflow {
        Ok(result) => (result.output, result.exit_code, result.metadata),
        Err(error) => (error, 1, None),
    };

    synthesize_deepresearch_report(
        &session,
        workspace,
        &opts.query,
        os_runtime,
        &workflow_output,
        exit_code,
        metadata.as_ref(),
        &report_tool_gate,
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeepResearchReportStatus {
    Completed,
    FallbackDraft,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResearchReportArtifacts {
    pub(crate) markdown: PathBuf,
    pub(crate) html: PathBuf,
}

#[derive(Clone, Default)]
pub(crate) struct DeepResearchReportToolGate {
    report_only: Arc<AtomicBool>,
}

impl DeepResearchReportToolGate {
    pub(crate) fn set_report_only(&self, enabled: bool) {
        self.report_only.store(enabled, Ordering::SeqCst);
    }

    pub(crate) fn report_only(&self) -> bool {
        self.report_only.load(Ordering::SeqCst)
    }
}

#[derive(Debug)]
pub(crate) struct DeepResearchReportSynthesis {
    pub(crate) text: String,
    pub(crate) artifacts: ResearchReportArtifacts,
    pub(crate) status: DeepResearchReportStatus,
}

#[allow(clippy::too_many_arguments)]
async fn synthesize_deepresearch_report(
    session: &AgentSession,
    workspace: &Path,
    query: &str,
    os_runtime: bool,
    workflow_output: &str,
    exit_code: i32,
    metadata: Option<&serde_json::Value>,
    report_tool_gate: &DeepResearchReportToolGate,
) -> anyhow::Result<DeepResearchReportSynthesis> {
    eprintln!("deepresearch: synthesizing report artifacts…");
    if exit_code != 0 && !deep_research_has_source_evidence(workflow_output, metadata) {
        report_tool_gate.set_report_only(false);
        let artifacts = materialize_deep_research_fallback_draft(
            workspace,
            query,
            "DeepResearch evidence collection failed before source-backed evidence was available.",
            workflow_output,
        )
        .map_err(anyhow::Error::msg)?;
        return Ok(DeepResearchReportSynthesis {
            text: format!(
                "DeepResearch fallback draft written at {}\n",
                artifacts.html.display()
            ),
            artifacts,
            status: DeepResearchReportStatus::FallbackDraft,
        });
    }
    let prompt = if exit_code == 0 {
        deep_research_synthesis_prompt(query, os_runtime, workflow_output, metadata)
    } else {
        deep_research_recovery_prompt(query, os_runtime, workflow_output, metadata)
    };
    report_tool_gate.set_report_only(true);
    let (mut final_text, synthesis_completed) = send_deepresearch_text(
        session,
        &prompt,
        DEEP_RESEARCH_SYNTHESIS_TIMEOUT_MS,
        "synthesis",
    )
    .await;
    let mut artifacts = deep_research_report_artifacts_from_output_for_query(
        &final_text,
        workspace,
        query,
        workflow_output,
        metadata,
    );
    let mut status = DeepResearchReportStatus::Completed;

    if deep_research_output_has_internal_leak(&final_text) {
        if let Some(artifacts) = artifacts.as_ref() {
            if let Some(clean_text) =
                clean_deep_research_final_text_from_artifacts(artifacts, workspace)
            {
                final_text = clean_text;
            }
        }
    }
    if artifacts.is_none() {
        artifacts = materialize_deep_research_completed_report_from_markdown(
            workspace,
            query,
            workflow_output,
            metadata,
        );
        if let Some(artifacts) = artifacts.as_ref() {
            if let Some(clean_text) =
                clean_deep_research_final_text_from_artifacts(artifacts, workspace)
            {
                final_text = clean_text;
            }
        }
    }

    if (artifacts.is_none() || deep_research_output_has_internal_leak(&final_text))
        && synthesis_completed
    {
        eprintln!("deepresearch: report marker/artifacts missing, running focused repair pass…");
        let repair =
            deep_research_repair_prompt(query, os_runtime, workflow_output, metadata, &final_text);
        let (repair_text, repair_completed) =
            send_deepresearch_text(session, &repair, DEEP_RESEARCH_REPAIR_TIMEOUT_MS, "repair")
                .await;
        final_text = repair_text;
        artifacts = deep_research_report_artifacts_from_output_for_query(
            &final_text,
            workspace,
            query,
            workflow_output,
            metadata,
        );
        if deep_research_output_has_internal_leak(&final_text) {
            if let Some(artifacts) = artifacts.as_ref() {
                if let Some(clean_text) =
                    clean_deep_research_final_text_from_artifacts(artifacts, workspace)
                {
                    final_text = clean_text;
                }
            }
        }
        if artifacts.is_none() {
            artifacts = materialize_deep_research_completed_report_from_markdown(
                workspace,
                query,
                workflow_output,
                metadata,
            );
            if let Some(artifacts) = artifacts.as_ref() {
                if let Some(clean_text) =
                    clean_deep_research_final_text_from_artifacts(artifacts, workspace)
                {
                    final_text = clean_text;
                }
            }
        }
        if !repair_completed {
            eprintln!("deepresearch: repair pass did not complete, using host fallback…");
        }
    }

    if artifacts.is_none() || deep_research_output_has_internal_leak(&final_text) {
        eprintln!(
            "deepresearch: report artifacts still missing, materializing host fallback draft…"
        );
        report_tool_gate.set_report_only(false);
        let fallback_artifacts = materialize_deep_research_fallback_draft(
            workspace,
            query,
            &final_text,
            workflow_output,
        )
        .map_err(anyhow::Error::msg)?;
        if deep_research_output_has_internal_leak(&final_text) {
            final_text.clear();
        } else if !final_text.ends_with('\n') {
            final_text.push('\n');
        }
        final_text.push_str(&format!(
            "DeepResearch fallback draft written at {}\n",
            fallback_artifacts.html.display()
        ));
        artifacts = Some(fallback_artifacts);
        status = DeepResearchReportStatus::FallbackDraft;
    }

    let artifacts = artifacts.ok_or_else(|| {
        anyhow::anyhow!(
            "DeepResearch did not produce the required report artifacts: expected `A3S_RESEARCH_VIEW: .a3s/research/<slug>/index.html`, plus sibling report.md"
        )
    })?;
    report_tool_gate.set_report_only(false);
    Ok(DeepResearchReportSynthesis {
        text: final_text,
        artifacts,
        status,
    })
}

async fn send_deepresearch_text(
    session: &AgentSession,
    prompt: &str,
    timeout_ms: u64,
    phase: &str,
) -> (String, bool) {
    match tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        session.send(prompt, Some(&[])),
    )
    .await
    {
        Ok(Ok(result)) => (result.text, true),
        Ok(Err(error)) => (
            format!("DeepResearch {phase} model call failed: {error}"),
            false,
        ),
        Err(_) => (
            format!("DeepResearch {phase} model call timed out after {timeout_ms} ms."),
            false,
        ),
    }
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
            "Write(.a3s/research/**)",
            "Edit(.a3s/research/**)",
            "write(.a3s/research/**)",
            "edit(.a3s/research/**)",
        ]);
    policy.default_decision = a3s_code_core::permissions::PermissionDecision::Deny;
    policy
}

#[derive(Clone)]
struct DeepResearchPermissionChecker {
    base: a3s_code_core::permissions::PermissionPolicy,
    report_tool_gate: DeepResearchReportToolGate,
}

impl a3s_code_core::permissions::PermissionChecker for DeepResearchPermissionChecker {
    fn check(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> a3s_code_core::permissions::PermissionDecision {
        if self.report_tool_gate.report_only() {
            deep_research_report_phase_tool_permission(tool_name, args)
        } else {
            self.base.check(tool_name, args)
        }
    }
}

pub(crate) fn deep_research_report_phase_tool_permission(
    tool_name: &str,
    args: &serde_json::Value,
) -> a3s_code_core::permissions::PermissionDecision {
    match tool_name.to_ascii_lowercase().as_str() {
        "write" | "edit" if report_artifact_write_args(args) => {
            a3s_code_core::permissions::PermissionDecision::Allow
        }
        "read" | "ls" | "glob" | "grep" if report_artifact_read_args(args) => {
            a3s_code_core::permissions::PermissionDecision::Allow
        }
        _ => a3s_code_core::permissions::PermissionDecision::Deny,
    }
}

fn report_artifact_write_args(args: &serde_json::Value) -> bool {
    ["file_path", "path"]
        .iter()
        .filter_map(|key| args.get(*key).and_then(serde_json::Value::as_str))
        .any(is_report_artifact_path)
}

fn report_artifact_read_args(args: &serde_json::Value) -> bool {
    [
        "file_path",
        "path",
        "dir",
        "directory",
        "root",
        "pattern",
        "glob",
        "include",
    ]
    .iter()
    .filter_map(|key| args.get(*key).and_then(serde_json::Value::as_str))
    .any(is_report_artifact_path)
}

fn is_report_artifact_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized.starts_with(".a3s/research/") || normalized.contains("/.a3s/research/")
}

async fn build_deepresearch_session(
    workspace: &str,
    code_config: CodeConfig,
    memory_dir: PathBuf,
) -> anyhow::Result<(AgentSession, DeepResearchReportToolGate)> {
    let agent = Agent::from_config(code_config)
        .await
        .map_err(|e| anyhow::anyhow!("failed to load DeepResearch agent: {e}"))?;
    let budget = deep_research_default_budget();
    let permission_policy = deepresearch_cli_permission_policy();
    let report_tool_gate = DeepResearchReportToolGate::default();
    let opts = SessionOptions::new()
        .with_confirmation_policy(a3s_code_core::hitl::ConfirmationPolicy::default())
        .with_permission_policy(permission_policy.clone())
        .with_permission_checker(Arc::new(DeepResearchPermissionChecker {
            base: permission_policy,
            report_tool_gate: report_tool_gate.clone(),
        }))
        .with_tool_timeout(RESEARCH_TOOL_EXEC_TIMEOUT_MS)
        .with_duplicate_tool_call_threshold(RESEARCH_DUPLICATE_TOOL_CALL_THRESHOLD)
        .with_file_memory(memory_dir)
        .with_max_parallel_tasks(budget.max_parallel_tasks)
        .with_max_tool_rounds(budget.max_tool_rounds)
        .with_max_continuation_turns(budget.max_continuation_turns)
        .with_auto_delegation_enabled(true)
        .with_auto_parallel_delegation(true)
        .with_manual_delegation_enabled(true);
    let session = agent
        .session_async(workspace.to_string(), Some(opts))
        .await?;
    session.register_dynamic_workflow_runtime()?;
    Ok((session, report_tool_gate))
}

async fn run_deepresearch_workflow(
    session: &AgentSession,
    args: serde_json::Value,
) -> Result<ToolCallResult, String> {
    let timeout_ms = deep_research_workflow_host_timeout_ms(&args);
    let (mut progress_rx, workflow_join) = session.tool_with_events("dynamic_workflow", args);
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
                "dynamic_workflow timed out after {timeout_ms} ms while gathering DeepResearch evidence"
            ))
        }
    };
    progress_drain.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[path = "cli.rs"]
    mod cli;
    #[path = "workflow.rs"]
    mod workflow;
    use a3s_code_core::llm::{
        ContentBlock, LlmClient, LlmResponse, Message, StreamEvent, TokenUsage, ToolDefinition,
    };
    use a3s_code_core::tools::{Tool, ToolContext, ToolOutput};
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
            messages: &[Message],
            system: Option<&str>,
            tools: &[ToolDefinition],
        ) -> LlmResponse {
            if tools.iter().any(|tool| tool.name == "emit_step_output") {
                return tool_call_response(
                    "toolu_emit_step_output",
                    "emit_step_output",
                    serde_json::json!({
                        "summary": "Structured DeepResearch track evidence confirms local fan-out completed before synthesis.",
                        "sources": [{
                            "title": "Example research source",
                            "url_or_path": "https://example.com/research",
                            "date": "2026-07-08",
                            "quote_or_fact": "Local DeepResearch fan-out completed before synthesis.",
                            "reliability": "deterministic test evidence"
                        }],
                        "key_evidence": [
                            "Local parallel_task fan-out produced deterministic evidence."
                        ],
                        "contradictions": [],
                        "confidence": "high for deterministic test evidence",
                        "gaps": []
                    }),
                );
            }
            let last = message_text(messages.last());
            if system.is_some_and(|system| system.contains("pre-analysis assistant"))
                || last.contains("ONLY the JSON object")
            {
                return text_response(
                    r#"{"intent":"GeneralPurpose","requires_planning":false,"goal":{"description":"DeepResearch child task","success_criteria":["evidence returned"]},"execution_plan":{"complexity":"Simple","steps":[],"required_tools":[]},"optimized_input":"DeepResearch child task"}"#,
                );
            }
            let trimmed = last.trim_start();
            let lower = trimmed.to_ascii_lowercase();
            if lower.contains("deep-research evidence track for:")
                && !lower.contains("dynamicworkflowruntime output:")
                && !lower.contains("dynamicworkflowruntime metadata:")
                && !lower.contains("complete only the missing report work")
                && !last.contains("DeepResearch verification layer")
            {
                return text_response(
                    "Track evidence: https://example.com/research confirms the local \
                     DeepResearch fan-out completed before synthesis.",
                );
            }
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

    struct StructuredCoercionFailsLlmClient;

    #[async_trait]
    impl LlmClient for StructuredCoercionFailsLlmClient {
        async fn complete(
            &self,
            messages: &[Message],
            system: Option<&str>,
            tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            Ok(structured_failure_response(messages, system, tools))
        }

        async fn complete_streaming(
            &self,
            messages: &[Message],
            system: Option<&str>,
            tools: &[ToolDefinition],
            _cancel_token: CancellationToken,
        ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
            let response = structured_failure_response(messages, system, tools);
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

    fn structured_failure_response(
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
    ) -> LlmResponse {
        if tools.iter().any(|tool| tool.name == "emit_step_output") {
            return text_response("I found evidence, but I am not emitting the schema tool.");
        }
        let last = message_text(messages.last());
        if system.is_some_and(|system| system.contains("pre-analysis assistant"))
            || last.contains("ONLY the JSON object")
        {
            return text_response(
                r#"{"intent":"GeneralPurpose","requires_planning":false,"goal":{"description":"DeepResearch child task","success_criteria":["evidence returned"]},"execution_plan":{"complexity":"Simple","steps":[],"required_tools":[]},"optimized_input":"DeepResearch child task"}"#,
            );
        }
        if last
            .to_ascii_lowercase()
            .contains("deep-research evidence track for:")
        {
            return text_response(
                "## Summary\n\nThe latest stable Rust version is 1.96.1, released on 2026-06-30.\n\n## Sources\n\n- Official Rust Blog: https://blog.rust-lang.org/2026/06/30/Rust-1.96.1/ confirms Rust 1.96.1.\n- Rust stable manifest: https://static.rust-lang.org/dist/channel-rust-stable.toml confirms pkg.rust.version 1.96.1.\n\n## Confidence\n\nHigh because two official Rust sources agree.",
            );
        }
        text_response("DONE")
    }

    fn message_text(message: Option<&Message>) -> String {
        message
            .map(|message| {
                message
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
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

    fn tool_call_response(id: &str, name: &str, input: serde_json::Value) -> LlmResponse {
        LlmResponse {
            message: Message {
                role: "assistant".into(),
                content: vec![ContentBlock::ToolUse {
                    id: id.into(),
                    name: name.into(),
                    input,
                }],
                reasoning_content: None,
            },
            usage: TokenUsage::default(),
            stop_reason: Some("tool_use".into()),
            token_logprobs: Vec::new(),
            meta: None,
        }
    }

    struct StructuredRuntimeTool {
        seen_args: std::sync::Arc<Mutex<Vec<serde_json::Value>>>,
    }

    #[async_trait]
    impl Tool for StructuredRuntimeTool {
        fn name(&self) -> &str {
            "runtime"
        }

        fn description(&self) -> &str {
            "Returns completed structured runtime output for DeepResearch tests."
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }

        async fn execute(
            &self,
            args: &serde_json::Value,
            _ctx: &ToolContext,
        ) -> anyhow::Result<ToolOutput> {
            self.seen_args.lock().unwrap().push(args.clone());
            let structured = serde_json::json!({
                "summary": "Runtime structured evidence confirms OS fan-out completed before synthesis.",
                "sources": [{
                    "title": "Runtime Evidence",
                    "url_or_path": "https://example.com/runtime-evidence",
                    "date": "2026-07-08",
                    "quote_or_fact": "OS Runtime returned a schema-shaped evidence object.",
                    "reliability": "deterministic test fixture"
                }],
                "key_evidence": ["OS Runtime results are normalized into structured evidence."],
                "contradictions": [],
                "confidence": "high",
                "gaps": []
            });
            Ok(ToolOutput::success(
                serde_json::json!({
                    "batchId": "batch-structured",
                    "results": [{
                        "invocationId": "inv-1",
                        "state": "completed",
                        "output": structured.to_string(),
                        "error": null
                    }]
                })
                .to_string(),
            ))
        }
    }

    fn test_config(path: &std::path::Path) {
        std::fs::write(
            path,
            "default_model = \"openai/x\"\n\
             providers \"openai\" {\n  apiKey = \"x\"\n  baseUrl = \"http://127.0.0.1:1\"\n  \
             models \"x\" { name = \"x\" }\n}\n\
             memory {\n  llmExtraction = false\n}\n",
        )
        .unwrap();
    }
}
