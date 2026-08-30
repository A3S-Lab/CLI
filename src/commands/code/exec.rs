use std::io::IsTerminal;
use std::sync::Arc;

use a3s_code_core::{Agent, AgentEvent, ManifestWorkspaceBackend};
use anyhow::{bail, Context};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use crate::cli::args::{
    CodeCapabilityRuntime, CodeExecArgs, CodeToolPolicy, CodeWebSearch, OutputMode,
};
use crate::cli::context::InvocationContext;
use crate::cli::output::{render_value, write_jsonl, CliError, ExitClass};
use crate::workspace_retrieval::SessionOptionsWorkspaceRetrievalExt;

mod scoped_runtime;

const MAX_PROMPT_BYTES: u64 = 16 * 1024 * 1024;

pub(super) async fn run(args: CodeExecArgs, context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    let CodeExecArgs {
        prompt,
        prompt_file,
        images,
        mode,
        tool_policy,
        web_search,
        capability_runtime,
        model,
    } = args;
    super::exec_policy::validate_tool_policy(mode, tool_policy)?;
    let runtime_configuration =
        crate::commands::config::resolve_code_runtime_configuration(context)?;
    let active_config_path = runtime_configuration.config_path;
    let code_config = runtime_configuration.config;
    let workspace_retrieval = crate::workspace_retrieval::build_workspace_retrieval_options(
        &runtime_configuration.workspace_retrieval,
        &runtime_configuration.trusted_host_config,
        Some(&context.component_paths.data_root),
        context.network.allow_first_use_install,
    )
    .await?;
    let scheduled_policy = if tool_policy == CodeToolPolicy::ScheduledReport {
        let loop_id = context
            .environment
            .utf8(crate::code_schedule::SCHEDULE_LOOP_ENV)?
            .filter(|value| !value.trim().is_empty())
            .context(
                "scheduled-report is an internal policy and requires a scheduled loop identity",
            )?;
        Some(crate::code_schedule::scheduled_execution_policy(
            &context.directory,
            &loop_id,
            &active_config_path,
        )?)
    } else {
        None
    };
    let prompt_file = prompt_file.map(|path| context.resolve_path(path));
    let prompt = apply_web_search_preference(
        read_prompt(prompt, prompt_file.as_deref(), !images.is_empty()).await?,
        web_search,
    );
    if !images.is_empty() {
        crate::image_input::ensure_model_supports_images(&code_config, model.as_deref())?;
    }
    let image_paths = images
        .into_iter()
        .map(|path| context.resolve_path(path))
        .collect::<Vec<_>>();
    let attachments = crate::image_input::load_image_attachments(&image_paths)?;
    let image_count = attachments.len();
    let sandbox = if matches!(
        tool_policy,
        CodeToolPolicy::Standard | CodeToolPolicy::LocalWorkspace
    ) {
        resolve_exec_sandbox(context, output).await
    } else {
        None
    };
    let agent = Agent::from_config(code_config.clone())
        .await
        .map_err(|error| anyhow::anyhow!("failed to load A3S Code: {error}"))?;
    let session_id = execution_id();
    let workspace = &context.directory;
    let workspace_backend = ManifestWorkspaceBackend::new_with_access_policy(
        workspace,
        a3s_code_core::workspace::LocalWorkspaceAccessPolicy::CredentialBoundary,
    );
    let workspace_services = crate::workspace_retrieval::workspace_services_for_host(
        workspace_backend,
        workspace_retrieval.as_ref(),
    )?;
    let hook_executor = crate::code_hooks::CommandHookExecutor::discover(
        workspace,
        context.home.as_deref(),
        context
            .component_paths
            .state_root
            .join("code/hooks-trust.json"),
    )?;
    let mut options =
        super::exec_policy::session_options_with_sandbox_and_schedule_and_workspace_services_and_web_search(
            mode,
            tool_policy,
            web_search,
            workspace,
            &session_id,
            sandbox,
            scheduled_policy,
            workspace_services,
        )
        .with_hook_executor(hook_executor);
    if let Some(model) = model {
        options = options.with_model(model);
    }
    options = options.with_optional_workspace_retrieval(workspace_retrieval.as_ref());
    let client =
        crate::session_llm::resolve_session_llm_client(&code_config, &options, &session_id)
            .map_err(anyhow::Error::msg)?;
    options = options.with_llm_client(Arc::clone(&client));
    let session = Arc::new(
        agent
            .session_builder(workspace.to_string_lossy().to_string())
            .options(options)
            .build()
            .await?,
    );

    let capability_runtime_policy = match capability_runtime {
        Some(CodeCapabilityRuntime::ScopedV1) => scoped_runtime::PreparationPolicy::Required,
        None => scoped_runtime::PreparationPolicy::InstalledOnly,
    };
    let capability_runtime_preparation = match scoped_runtime::prepare(
        context,
        &active_config_path,
        Arc::clone(&session),
        capability_runtime_policy,
    )
    .await
    {
        Ok(evidence) => evidence,
        Err(error) => {
            session.close().await;
            return Err(error);
        }
    };
    if context.cancellation.is_cancelled() {
        session.close().await;
        let _ = capability_runtime_preparation.shutdown().await;
        return Err(scoped_runtime::cancelled_error().into());
    }
    if let Some(warning) = capability_runtime_preparation.warning.as_deref() {
        if output == OutputMode::Human {
            eprintln!("warning: {warning}");
        } else {
            tracing::warn!(%warning, "scoped capability runtime warning");
        }
    }
    let capability_runtime_evidence = capability_runtime_preparation.evidence.clone();

    let execution = async {
        let (mut receiver, worker) = if attachments.is_empty() {
            session.stream(&prompt, None).await?
        } else {
            session
                .stream_with_attachments(&prompt, &attachments, None)
                .await?
        };
        let mut execution = StreamExecution::default();
        loop {
            let event = tokio::select! {
                event = receiver.recv() => event,
                _ = context.cancellation.cancelled() => {
                    let _ = session.cancel().await;
                    execution.cancelled = true;
                    None
                }
            };
            let Some(event) = event else {
                break;
            };
            match &event {
                AgentEvent::TextDelta { text } => {
                    execution.streamed.push_str(text);
                    if output == OutputMode::Human {
                        print!("{text}");
                    }
                }
                AgentEvent::ToolStart { name, .. } if output == OutputMode::Human => {
                    eprintln!("tool: {name}");
                }
                AgentEvent::ConfirmationRequired {
                    tool_id, tool_name, ..
                } => {
                    execution.approval_required = Some(tool_name.clone());
                    let _ = session
                        .confirm_tool_use(
                            tool_id,
                            false,
                            Some(
                                "non-interactive execution cannot request hidden approval"
                                    .to_string(),
                            ),
                        )
                        .await;
                    let _ = session.cancel().await;
                    if output == OutputMode::Human {
                        eprintln!("denied approval-required tool: {tool_name}");
                    }
                }
                AgentEvent::End {
                    text,
                    usage: event_usage,
                    ..
                } => {
                    execution.final_text = text.clone();
                    execution.usage = serde_json::to_value(event_usage)?;
                    execution.completed = true;
                }
                AgentEvent::Error { message } => {
                    execution.runtime_error = Some(message.clone());
                    if output == OutputMode::Human {
                        eprintln!("error: {message}");
                    }
                }
                _ => {}
            }
            if output == OutputMode::Jsonl {
                let event = public_event_value(&event)?;
                write_jsonl(&json!({
                    "schemaVersion": 1,
                    "command": "code.exec",
                    "type": "event",
                    "sequence": execution.sequence,
                    "event": event,
                }))?;
                execution.sequence += 1;
            }
        }
        execution.worker_error = worker.await.err().map(|error| format!("{error:#}"));
        Ok::<_, anyhow::Error>(execution)
    }
    .await;
    let workspace_retrieval_status = session.workspace_retrieval_status();
    session.close().await;
    let runtime_shutdown = capability_runtime_preparation.shutdown().await;
    let mut execution = match execution {
        Ok(execution) => {
            runtime_shutdown?;
            execution
        }
        Err(error) => {
            if let Err(cleanup) = runtime_shutdown {
                tracing::error!(
                    error = %cleanup,
                    "scoped capability Runtime cleanup also failed after execution"
                );
            }
            return Err(error);
        }
    };
    if execution.cancelled {
        return Err(CliError::new(
            "operation.cancelled",
            "code execution cancelled",
            ExitClass::Cancelled,
        )
        .with_jsonl_sequence(execution.sequence)
        .into());
    }
    if let Some(failure) = terminal_failure(
        execution.approval_required,
        execution.runtime_error,
        execution.worker_error,
        execution.completed,
    ) {
        return Err(failure.into_cli_error(execution.sequence).into());
    }
    let emitted_stream = !execution.streamed.is_empty();
    if execution.final_text.is_empty() {
        execution.final_text = execution.streamed.clone();
    }
    if output == OutputMode::Human {
        if !execution.final_text.is_empty() && !emitted_stream {
            println!("{}", execution.final_text);
        } else if emitted_stream && !execution.streamed.ends_with('\n') {
            println!();
        }
        return Ok(());
    }
    let data = json!({
        "text": execution.final_text,
        "usage": execution.usage,
        "sessionId": session_id,
        "imageCount": image_count,
        "toolPolicy": tool_policy_name(tool_policy),
        "webSearch": web_search_name(web_search),
        "workspaceRetrieval": workspace_retrieval_status,
        "capabilityRuntime": capability_runtime_evidence,
    });
    if output == OutputMode::Jsonl {
        write_jsonl(&json!({
            "schemaVersion": 1,
            "command": "code.exec",
            "type": "result",
            "sequence": execution.sequence,
            "ok": true,
            "data": data,
        }))?;
        return Ok(());
    }
    render_value(output, "code.exec", data, || {})
}

struct StreamExecution {
    sequence: u64,
    streamed: String,
    final_text: String,
    usage: Value,
    approval_required: Option<String>,
    runtime_error: Option<String>,
    worker_error: Option<String>,
    completed: bool,
    cancelled: bool,
}

impl Default for StreamExecution {
    fn default() -> Self {
        Self {
            sequence: 1,
            streamed: String::new(),
            final_text: String::new(),
            usage: Value::Null,
            approval_required: None,
            runtime_error: None,
            worker_error: None,
            completed: false,
            cancelled: false,
        }
    }
}

fn public_event_value(event: &AgentEvent) -> anyhow::Result<Value> {
    let mut value = serde_json::to_value(event)?;
    if let Some(meta) = value.get_mut("meta").and_then(Value::as_object_mut) {
        // The provider endpoint is operational configuration. Keep useful
        // response metadata without copying that endpoint into public JSONL.
        meta.remove("request_url");
    }
    Ok(value)
}

async fn resolve_exec_sandbox(
    context: &InvocationContext,
    output: OutputMode,
) -> Option<Arc<dyn a3s_code_core::sandbox::BashSandbox>> {
    let resolution = a3s::components::resolve_managed_srt(
        &context.component_paths,
        &context.directory,
        context.network.allow_first_use_install,
        context.network.offline,
        context.output.progress,
    )
    .await;
    let resolved = match resolution.runtime {
        Some(runtime) => match runtime.build_and_probe_sandbox(&context.directory).await {
            Ok(sandbox) => {
                return Some(Arc::new(sandbox) as Arc<dyn a3s_code_core::sandbox::BashSandbox>)
            }
            Err(error) => Some(format!(
                "local command sandbox failed its OS capability probe: {error:#}"
            )),
        },
        None => resolution.warning,
    };
    if let Some(warning) = resolved {
        if output == OutputMode::Human {
            eprintln!("warning: {warning}");
        } else {
            tracing::warn!(%warning, "code exec local command sandbox is unavailable");
        }
    }
    None
}

fn tool_policy_name(policy: crate::cli::args::CodeToolPolicy) -> &'static str {
    use crate::cli::args::CodeToolPolicy;

    match policy {
        CodeToolPolicy::Standard => "standard",
        CodeToolPolicy::ReadOnly => "read-only",
        CodeToolPolicy::WorkspaceWrite => "workspace-write",
        CodeToolPolicy::LocalWorkspace => "local-workspace",
        CodeToolPolicy::ScheduledReport => "scheduled-report",
    }
}

fn web_search_name(preference: CodeWebSearch) -> &'static str {
    match preference {
        CodeWebSearch::Auto => "auto",
        CodeWebSearch::Enabled => "enabled",
        CodeWebSearch::Disabled => "disabled",
    }
}

fn apply_web_search_preference(prompt: String, preference: CodeWebSearch) -> String {
    if preference != CodeWebSearch::Enabled {
        return prompt;
    }
    format!(
        "A3S Code execution preference:\n- The user explicitly enabled web search for this task. Use web_search and web_fetch when external evidence would improve the result.\n\n{prompt}"
    )
}

#[derive(Debug, Eq, PartialEq)]
enum TerminalFailure {
    ApprovalRequired { tool_name: String },
    Runtime { message: String },
    Worker { message: String },
    Incomplete,
}

impl TerminalFailure {
    fn code(&self) -> &'static str {
        match self {
            Self::ApprovalRequired { .. } => "approval.required",
            Self::Runtime { .. } | Self::Worker { .. } => "code.exec.failed",
            Self::Incomplete => "code.exec.incomplete",
        }
    }

    fn into_cli_error(self, sequence: u64) -> CliError {
        let code = self.code();
        match self {
            Self::ApprovalRequired { tool_name } => CliError::new(
                code,
                format!(
                    "tool `{tool_name}` requires approval that cannot be requested in non-interactive execution"
                ),
                ExitClass::Failure,
            )
            .with_suggestion(
                "Run the task interactively with `a3s code` or adjust the configured permission policy.",
            )
            .with_details(json!({"tool": tool_name}))
            .with_jsonl_sequence(sequence),
            Self::Runtime { message } | Self::Worker { message } => CliError::new(
                code,
                format!("code execution failed: {message}"),
                ExitClass::Failure,
            )
            .with_jsonl_sequence(sequence),
            Self::Incomplete => CliError::new(
                code,
                "code execution ended without a terminal completion event",
                ExitClass::Failure,
            )
            .with_jsonl_sequence(sequence),
        }
    }
}

fn terminal_failure(
    approval_required: Option<String>,
    runtime_error: Option<String>,
    worker_error: Option<String>,
    completed: bool,
) -> Option<TerminalFailure> {
    if let Some(tool_name) = approval_required {
        return Some(TerminalFailure::ApprovalRequired { tool_name });
    }
    if let Some(message) = runtime_error {
        return Some(TerminalFailure::Runtime { message });
    }
    if !completed {
        return Some(TerminalFailure::Incomplete);
    }
    if let Some(message) = worker_error {
        return Some(TerminalFailure::Worker { message });
    }
    None
}

async fn read_prompt(
    prompt: Option<String>,
    prompt_file: Option<&std::path::Path>,
    allow_empty: bool,
) -> anyhow::Result<String> {
    let prompt = if let Some(prompt) = prompt {
        prompt
    } else if let Some(path) = prompt_file {
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("could not inspect prompt file {}", path.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_PROMPT_BYTES {
            bail!("prompt file must be a regular UTF-8 file no larger than 16 MiB");
        }
        std::fs::read_to_string(path)
            .with_context(|| format!("could not read prompt file {}", path.display()))?
    } else {
        if std::io::stdin().is_terminal() {
            if allow_empty {
                return Ok(String::new());
            }
            bail!("a prompt, --prompt-file, piped stdin, or --image is required");
        }
        let mut bytes = Vec::new();
        tokio::io::stdin()
            .take(MAX_PROMPT_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .context("could not read prompt from stdin")?;
        if bytes.len() as u64 > MAX_PROMPT_BYTES {
            bail!("piped prompt exceeds 16 MiB");
        }
        String::from_utf8(bytes).context("piped prompt must be UTF-8")?
    };
    if prompt.trim().is_empty() && !allow_empty {
        bail!("prompt is empty");
    }
    Ok(prompt)
}

fn execution_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("exec-{nanos:016x}-{:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_failure_precedes_worker_cancellation() {
        let failure = terminal_failure(
            Some("bash".to_string()),
            None,
            Some("task was cancelled".to_string()),
            false,
        )
        .expect("approval must be terminal");

        assert_eq!(failure.code(), "approval.required");
        assert_eq!(
            failure,
            TerminalFailure::ApprovalRequired {
                tool_name: "bash".to_string()
            }
        );
    }

    #[test]
    fn agent_error_is_a_failed_execution_even_if_the_worker_completes() {
        let failure = terminal_failure(None, Some("provider failed".to_string()), None, false)
            .expect("AgentEvent::Error must be terminal");

        assert_eq!(failure.code(), "code.exec.failed");
        assert_eq!(
            failure,
            TerminalFailure::Runtime {
                message: "provider failed".to_string()
            }
        );
    }

    #[test]
    fn closed_stream_without_end_is_incomplete() {
        let failure = terminal_failure(
            None,
            None,
            Some("worker stopped before completion".to_string()),
            false,
        )
        .expect("a stream without AgentEvent::End must not succeed");

        assert_eq!(failure.code(), "code.exec.incomplete");
        assert_eq!(failure, TerminalFailure::Incomplete);
    }

    #[test]
    fn worker_failure_after_end_is_a_failed_execution() {
        let failure = terminal_failure(
            None,
            None,
            Some("worker failed to settle".to_string()),
            true,
        )
        .expect("a failed worker must not report success");

        assert_eq!(failure.code(), "code.exec.failed");
        assert_eq!(
            failure,
            TerminalFailure::Worker {
                message: "worker failed to settle".to_string()
            }
        );
    }

    #[test]
    fn terminal_completion_without_errors_can_succeed() {
        assert_eq!(terminal_failure(None, None, None, true), None);
    }
}
