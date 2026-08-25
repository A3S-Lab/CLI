//! TUI session construction, resume, and terminal launch flow.

use super::*;
use crate::cli::args::ColorMode;
use crate::cli::context::InvocationContext;
use crate::tui::app_startup::{FirstFrameGate, StartupLoadingIndicator, StartupTrace};
use anyhow::Context as _;
use tokio_util::sync::CancellationToken;

const CODE_INTELLIGENCE_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const CODE_INTELLIGENCE_SHUTDOWN_SETTLE: Duration = Duration::from_secs(1);
const CODE_INTELLIGENCE_ABORT_SETTLE: Duration = Duration::from_millis(250);
const USE_SETUP_STOP_GRACE: Duration = Duration::from_millis(250);
const USE_REGISTRY_SHUTDOWN_SETTLE: Duration = Duration::from_secs(1);
const USE_SMOKE_PROJECTION_SETTLE: Duration = Duration::from_secs(30);
const DEFAULT_TUI_TERMINAL_SIZE: (u16, u16) = (80, 24);

fn usable_terminal_size(size: Option<(u16, u16)>) -> (u16, u16) {
    match size {
        Some((width, height)) if width > 0 && height > 0 => (width, height),
        _ => DEFAULT_TUI_TERMINAL_SIZE,
    }
}

fn ensure_tui_lane_queue(code_config: &mut CodeConfig) {
    // The TUI owns user-turn admission and interruption. Core's a3s-lane
    // queue independently prioritizes admitted tool work (query before
    // execute). Preserve an explicit ACL queue block verbatim.
    code_config
        .queue
        .get_or_insert_with(a3s_code_core::queue::SessionQueueConfig::default);
}

fn with_tui_prompt_context(
    options: SessionOptions,
    instructions: Option<&str>,
    os_address: Option<&str>,
    ctx_ready: bool,
    learned_preferences: Option<&str>,
    effort_guideline: Option<&str>,
) -> SessionOptions {
    let mut parts = Vec::new();
    if let Some(instructions) = instructions {
        parts.push(instructions.to_string());
    }
    if let Some(address) = os_address {
        parts.push(os_platform_guide(address));
    }
    if ctx_ready {
        parts.push(panels::ctx::ctx_history_guide());
    }
    if let Some(preferences) = learned_preferences {
        parts.push(preferences.to_string());
    }
    if parts.is_empty() && effort_guideline.is_none() {
        options
    } else {
        let mut slots = SystemPromptSlots::default();
        if !parts.is_empty() {
            slots = slots.with_extra(parts.join("\n\n"));
        }
        if let Some(guideline) = effort_guideline {
            slots = slots.with_guidelines(guideline);
        }
        options.with_prompt_slots(slots)
    }
}

struct CodeUseSetupGuard {
    registry: crate::use_registry::UseRegistrySlot,
    cancellation: CancellationToken,
    settled: bool,
}

impl CodeUseSetupGuard {
    fn new(
        registry: crate::use_registry::UseRegistrySlot,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            registry,
            cancellation,
            settled: false,
        }
    }

    fn ready(&mut self, handle: crate::use_registry::UseRegistryHandle, warning: Option<String>) {
        self.registry.set_ready(handle, warning);
        self.settled = true;
    }

    fn unavailable(&mut self, reason: impl Into<String>) {
        self.cancellation.cancel();
        self.registry.set_unavailable(reason);
        self.settled = true;
    }
}

impl Drop for CodeUseSetupGuard {
    fn drop(&mut self) {
        if !self.settled {
            self.cancellation.cancel();
            self.registry
                .set_unavailable("background setup stopped before completion");
        }
    }
}

struct CodeWebviewResolution {
    executable: Option<PathBuf>,
    warning: Option<String>,
}

fn spawn_code_use_setup(
    context: &InvocationContext,
    active_session: SharedActiveSession,
    registry: crate::use_registry::UseRegistrySlot,
    plugin_policy_handoff: Option<a3s::plugin_manager::PluginPolicyHandoff>,
    runtime_tasks: Option<Arc<dyn crate::use_registry::RuntimeTaskInvoker>>,
    mcp_runtime: Option<Arc<dyn crate::use_registry::McpRuntimeResolver>>,
    first_frame: FirstFrameGate,
) -> (CancellationToken, tokio::task::JoinHandle<()>) {
    let component_paths = context.component_paths.clone();
    let directory = context.directory.clone();
    let allow_first_use_install = context.network.allow_first_use_install;
    let offline = context.network.offline;
    let cancellation = context.cancellation.child_token();
    let task_cancellation = cancellation.clone();
    let plugin_management = (|| -> anyhow::Result<_> {
        let handoff = plugin_policy_handoff.ok_or_else(|| {
            anyhow::anyhow!("the host plugin authorization policy was not available")
        })?;
        Ok(crate::use_registry::PluginManagementMcpLaunch::new(
            std::env::current_exe().context("failed to resolve the A3S executable")?,
            crate::commands::config::active_config_path(context)?,
            offline,
            handoff.source().map(Path::to_path_buf),
            handoff.policy_digest().to_string(),
        ))
    })();
    let (plugin_management, plugin_warning) = match plugin_management {
        Ok(launch) => (Some(launch), None),
        Err(error) => (
            None,
            Some(format!("Plugin Manager MCP is unavailable: {error}")),
        ),
    };

    let task = tokio::spawn(async move {
        let mut setup = CodeUseSetupGuard::new(registry, task_cancellation.clone());
        tokio::select! {
            biased;
            _ = task_cancellation.cancelled() => {
                setup.unavailable("background setup was cancelled before the first frame");
                return;
            }
            _ = first_frame.wait() => {}
        }
        first_frame.record_deferred_operation("a3s_use_setup");
        let knowledge_paths = a3s_use_extension::ExtensionPaths::new(
            component_paths.data_root.join("use"),
            component_paths.state_root.join("use"),
        );
        let resolution = crate::code_use_host::resolve_with(
            allow_first_use_install,
            offline,
            || a3s::components::find_ready_executable_with("use", &component_paths),
            || {
                a3s::components::resolve_or_install_with(
                    "use",
                    &component_paths,
                    allow_first_use_install,
                    false,
                )
            },
        )
        .await;
        let Some(executable) = resolution.executable else {
            setup.unavailable(resolution.warning.unwrap_or_else(|| {
                "A3S Use is unavailable; run /use repair for recovery guidance".to_string()
            }));
            return;
        };
        if task_cancellation.is_cancelled() {
            setup.unavailable("background setup was cancelled");
            return;
        }

        let initial_session = match active_session.lock() {
            Ok(session) => Arc::clone(&session),
            Err(_) => {
                setup.unavailable("active Code session lock was poisoned");
                return;
            }
        };
        let (handle, registry_warning) = crate::use_registry::start(
            executable,
            directory,
            knowledge_paths,
            task_cancellation.clone(),
            Arc::clone(&initial_session),
            crate::use_registry::ProjectionHost::new(plugin_management, runtime_tasks, mcp_runtime),
        )
        .await;
        if task_cancellation.is_cancelled() {
            handle.shutdown().await;
            setup.unavailable("background setup was cancelled");
            return;
        }
        let warning = [resolution.warning, plugin_warning, registry_warning]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let warning = (!warning.is_empty()).then(|| warning.join("; "));

        // Session replacement and registry publication share the active-session
        // lock. A rebuild therefore either sees the ready handle and reattaches
        // itself, or completes first and is the session attached here.
        let attached = match active_session.lock() {
            Ok(session) => {
                if !Arc::ptr_eq(&session, &initial_session) {
                    handle.replace_session(Arc::clone(&session));
                }
                setup.ready(handle.clone(), warning);
                true
            }
            Err(_) => false,
        };
        if !attached {
            setup.unavailable("active Code session lock was poisoned");
            handle.shutdown().await;
        }
    });
    (cancellation, task)
}

async fn stop_code_use_setup(
    cancellation: &CancellationToken,
    task: &mut tokio::task::JoinHandle<()>,
) {
    cancellation.cancel();
    if tokio::time::timeout(USE_SETUP_STOP_GRACE, &mut *task)
        .await
        .is_ok()
    {
        return;
    }
    task.abort();
    let _ = tokio::time::timeout(CODE_INTELLIGENCE_ABORT_SETTLE, &mut *task).await;
}

fn spawn_code_use_shutdown(
    registry: &crate::use_registry::UseRegistrySlot,
) -> Option<tokio::task::JoinHandle<()>> {
    registry.ready_handle().map(|registry| {
        tokio::spawn(async move {
            registry.shutdown().await;
        })
    })
}

async fn settle_code_use_shutdown(shutdown: Option<tokio::task::JoinHandle<()>>) {
    let Some(mut shutdown) = shutdown else {
        return;
    };
    match tokio::time::timeout(USE_REGISTRY_SHUTDOWN_SETTLE, &mut shutdown).await {
        Ok(Ok(())) => return,
        Ok(Err(error)) => {
            tracing::warn!(%error, "A3S Use registry shutdown task failed");
            return;
        }
        Err(_) => {}
    }
    tracing::warn!(
        timeout = ?USE_REGISTRY_SHUTDOWN_SETTLE,
        "A3S Use registry cleanup did not settle after the Code session closed"
    );
    shutdown.abort();
    let _ = tokio::time::timeout(CODE_INTELLIGENCE_ABORT_SETTLE, &mut shutdown).await;
}

async fn resolve_code_webview_with<D, F, Fut>(
    allow_first_use_install: bool,
    offline: bool,
    discover: D,
    install: F,
) -> CodeWebviewResolution
where
    D: FnOnce() -> anyhow::Result<Option<PathBuf>>,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<PathBuf>>,
{
    match discover() {
        Ok(Some(executable)) => CodeWebviewResolution {
            executable: Some(executable),
            warning: None,
        },
        Ok(None) if allow_first_use_install => match install().await {
            Ok(executable) => CodeWebviewResolution {
                executable: Some(executable),
                warning: None,
            },
            Err(error) => CodeWebviewResolution {
                executable: None,
                warning: Some(format!(
                    "A3S WebView first-use setup failed; Code will continue without native RemoteUI and Agent Island windows: {error}. Run `a3s doctor webview` and `a3s install webview` for recovery"
                )),
            },
        },
        Ok(None) => CodeWebviewResolution {
            executable: None,
            warning: Some(if offline {
                "A3S WebView is not ready and first-use setup is disabled in offline mode; run `a3s install webview` after going online"
                    .to_string()
            } else {
                "A3S WebView is not ready and first-use setup is disabled by A3S_NO_AUTO_INSTALL; run `a3s install webview` for explicit setup"
                    .to_string()
            }),
        },
        Err(error) => CodeWebviewResolution {
            executable: None,
            warning: Some(format!(
                "A3S WebView discovery failed; Code will continue without native RemoteUI and Agent Island windows: {error}. Run `a3s doctor webview` for recovery"
            )),
        },
    }
}

async fn resolve_code_webview(context: &InvocationContext) -> CodeWebviewResolution {
    resolve_code_webview_with(
        context.network.allow_first_use_install,
        context.network.offline,
        || a3s::components::find_ready_executable_with("webview", &context.component_paths),
        || {
            a3s::components::resolve_or_install_with(
                "webview",
                &context.component_paths,
                context.network.allow_first_use_install,
                context.output.progress,
            )
        },
    )
    .await
}

fn tui_manifest_backend(workspace: &Path) -> Arc<ManifestWorkspaceBackend> {
    ManifestWorkspaceBackend::new_deferred_with_access_policy(
        workspace,
        a3s_code_core::workspace::LocalWorkspaceAccessPolicy::CredentialBoundary,
    )
}

pub(crate) fn resolve_tui_session_store_dir(workspace: &Path) -> PathBuf {
    let tui_dir = workspace.join(".a3s/tui");
    let canonical = tui_dir.join("sessions");
    let legacy = workspace.join(".a3s/tui-sessions");
    if !canonical.exists() && legacy.exists() {
        // Same-filesystem rename preserves all session IDs atomically. If it
        // fails, keep using the legacy store so existing history remains visible.
        let _ = std::fs::create_dir_all(&tui_dir);
        if std::fs::rename(&legacy, &canonical).is_err() {
            return legacy;
        }
    }
    canonical
}

fn sort_saved_sessions_by_recency(saved: &mut [(String, i64)]) {
    saved.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| right.0.cmp(&left.0)));
}

async fn saved_sessions_by_recency(
    store: &dyn a3s_code_core::store::SessionStore,
) -> anyhow::Result<Vec<(String, i64)>> {
    let mut saved = Vec::new();
    for id in store
        .list()
        .await
        .map_err(|error| anyhow::anyhow!("failed to list saved sessions: {error}"))?
    {
        match store.load(&id).await {
            Ok(Some(session)) => saved.push((id, session.updated_at)),
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, %id, "skipping unreadable saved session"),
        }
    }
    sort_saved_sessions_by_recency(&mut saved);
    Ok(saved)
}

#[derive(Debug, PartialEq, Eq)]
enum ResumeSessionSelection {
    Selected(String),
    Missing {
        requested: Option<String>,
        available: Vec<String>,
    },
}

async fn select_resume_session(
    store: &dyn a3s_code_core::store::SessionStore,
    explicit_id: Option<&str>,
) -> anyhow::Result<ResumeSessionSelection> {
    if let Some(id) = explicit_id {
        if store
            .exists(id)
            .await
            .map_err(|error| anyhow::anyhow!("failed to inspect session {id}: {error}"))?
        {
            return Ok(ResumeSessionSelection::Selected(id.to_string()));
        }

        let available = saved_sessions_by_recency(store)
            .await?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        return Ok(ResumeSessionSelection::Missing {
            requested: Some(id.to_string()),
            available,
        });
    }

    let saved = saved_sessions_by_recency(store).await?;
    match saved.first() {
        Some((id, _)) => Ok(ResumeSessionSelection::Selected(id.clone())),
        None => Ok(ResumeSessionSelection::Missing {
            requested: None,
            available: Vec::new(),
        }),
    }
}

fn configured_model_preference_from_session(
    session: &a3s_code_core::store::SessionData,
    configured_models: &[String],
) -> Option<ModelSelectionPreference> {
    configured_model_preference(persisted_model_from_session(session), configured_models)
}

pub(super) fn persisted_model_from_session(
    session: &a3s_code_core::store::SessionData,
) -> Option<String> {
    session
        .llm_config
        .as_ref()
        .map(|config| format!("{}/{}", config.provider, config.model))
        .or_else(|| session.model_name.clone())
}

pub(super) fn configured_model_preference(
    model: Option<String>,
    configured_models: &[String],
) -> Option<ModelSelectionPreference> {
    let model = model?;
    configured_models
        .iter()
        .any(|configured| configured == &model)
        .then_some(ModelSelectionPreference {
            source: ModelSelectionSource::Config,
            model,
        })
}

pub(super) fn preference_matches_persisted_model(
    preference: &ModelSelectionPreference,
    persisted_model: &str,
) -> bool {
    let selected_model = preference
        .source
        .account_provider()
        .map(|provider| provider.canonical_model(&preference.model))
        .unwrap_or_else(|| preference.model.clone());
    selected_model == persisted_model
}

fn render_resume_command(session_id: &str, color: bool) -> String {
    let command = format!("a3s code resume {session_id}");
    if color {
        Style::new().fg(ACCENT).bold().render(&command)
    } else {
        command
    }
}

fn render_resume_hint(session_id: &str, color: bool) -> String {
    let command = render_resume_command(session_id, color);
    format!("\n  session saved · resume it with:  {command}\n")
}

fn stdout_color_enabled(context: &InvocationContext) -> bool {
    match context.output.color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => context.terminal.stdout,
    }
}

fn stderr_color_enabled(context: &InvocationContext) -> bool {
    match context.output.color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => context.terminal.stderr,
    }
}

async fn shutdown_code_intelligence(provider: Arc<LocalCodeIntelligence>) -> bool {
    // Keep polling one owned shutdown future across both bounds. Recreating the
    // future after a timeout is not cancellation-safe because shutdown may
    // already have taken registry entries or marked a runtime as stopping.
    let mut shutdown = tokio::spawn(async move {
        provider.shutdown().await;
    });
    match tokio::time::timeout(CODE_INTELLIGENCE_SHUTDOWN_GRACE, &mut shutdown).await {
        Ok(Ok(())) => return true,
        Ok(Err(error)) => {
            tracing::warn!(%error, "Code Intelligence shutdown task failed");
            return false;
        }
        Err(_) => {}
    }

    tracing::warn!(
        timeout = ?CODE_INTELLIGENCE_SHUTDOWN_GRACE,
        "Code Intelligence graceful shutdown timed out; waiting for cleanup to settle"
    );
    match tokio::time::timeout(CODE_INTELLIGENCE_SHUTDOWN_SETTLE, &mut shutdown).await {
        Ok(Ok(())) => return true,
        Ok(Err(error)) => {
            tracing::warn!(%error, "Code Intelligence shutdown task failed while settling");
            return false;
        }
        Err(_) => {}
    }

    tracing::warn!(
        timeout = ?CODE_INTELLIGENCE_SHUTDOWN_SETTLE,
        "Code Intelligence cleanup did not settle before host exit; aborting the shutdown task"
    );
    shutdown.abort();
    if tokio::time::timeout(CODE_INTELLIGENCE_ABORT_SETTLE, &mut shutdown)
        .await
        .is_err()
    {
        tracing::warn!(
            timeout = ?CODE_INTELLIGENCE_ABORT_SETTLE,
            "Code Intelligence shutdown task did not acknowledge abort before host exit"
        );
    }
    false
}

fn push_resumed_text_entry(transcript: &mut Transcript, role: &str, pending: &mut String) {
    if pending.trim().is_empty() {
        pending.clear();
        return;
    }
    let text = std::mem::take(pending);
    match role {
        "user" => transcript.push(TranscriptEntry::user(text.trim().to_string())),
        "assistant" => transcript.push(TranscriptEntry::assistant_markdown(text)),
        _ => {}
    }
}

/// Rebuild semantic transcript cells from persisted LLM messages. Tool uses
/// and their paired results are retained by call id, so resume preserves call
/// order and Ctrl+T/main-history behavior instead of showing text only.
pub(super) fn resumed_transcript_entries(history: &[Message]) -> Vec<TranscriptEntry> {
    let mut transcript = Transcript::default();
    let mut calls = HashMap::<String, (String, serde_json::Value)>::new();

    for message in history {
        match message.role.as_str() {
            "assistant" => {
                if let Some(reasoning) = message
                    .reasoning_content
                    .as_deref()
                    .filter(|reasoning| !reasoning.trim().is_empty())
                {
                    transcript.push(TranscriptEntry::reasoning(reasoning));
                }
                let mut pending = String::new();
                for block in &message.content {
                    match block {
                        ContentBlock::Text { text } => pending.push_str(text),
                        ContentBlock::ToolUse { id, name, input } => {
                            push_resumed_text_entry(&mut transcript, "assistant", &mut pending);
                            transcript.restore_tool_execution(
                                id.clone(),
                                name.clone(),
                                input.clone(),
                                true,
                            );
                            calls.insert(id.clone(), (name.clone(), input.clone()));
                        }
                        ContentBlock::Image { .. } | ContentBlock::ToolResult { .. } => {}
                    }
                }
                push_resumed_text_entry(&mut transcript, "assistant", &mut pending);
            }
            "user" => {
                let mut pending = String::new();
                for block in &message.content {
                    match block {
                        ContentBlock::Text { text } => pending.push_str(text),
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } => {
                            push_resumed_text_entry(&mut transcript, "user", &mut pending);
                            let (name, args) =
                                calls.get(tool_use_id).cloned().unwrap_or_else(|| {
                                    (
                                        "tool".to_string(),
                                        serde_json::Value::Object(Default::default()),
                                    )
                                });
                            let failed = is_error.unwrap_or(false);
                            transcript.finish_tool_with_state(
                                tool_use_id,
                                name,
                                Some(args),
                                content.as_text(),
                                i32::from(failed),
                                None,
                                if failed {
                                    ToolCallState::Failed
                                } else {
                                    ToolCallState::Succeeded
                                },
                                true,
                            );
                        }
                        ContentBlock::Image { .. } | ContentBlock::ToolUse { .. } => {}
                    }
                }
                push_resumed_text_entry(&mut transcript, "user", &mut pending);
            }
            _ => {}
        }
    }
    transcript.interrupt_unfinished_tools();
    transcript.into_entries()
}

fn deferred_ui_metadata_command(workspace: String) -> Cmd<Msg> {
    cmd::cmd(move || async move {
        let result = tokio::task::spawn_blocking(move || StartupUiMetadata {
            branch: git_branch(&workspace),
            disabled_skills: load_disabled_skills(),
            codex_account_models: crate::account_providers::codex::cached_codex_models(),
        })
        .await
        .map_err(|error| format!("deferred TUI metadata loader failed: {error}"));
        Msg::StartupUiMetadataLoaded(result)
    })
}

fn deferred_interrupted_research_recovery_command(
    session: Arc<AgentSession>,
    workspace: PathBuf,
) -> Cmd<Msg> {
    cmd::cmd(move || async move {
        let result = async {
            let running_tracker_children = session
                .pending_subagent_tasks()
                .await
                .into_iter()
                .map(|snapshot| snapshot.task_id)
                .collect::<HashSet<_>>();
            let recovery =
                reconcile_interrupted_latest_run(&workspace, &running_tracker_children).await?;
            let Some(recovery) = recovery else {
                return Ok::<_, anyhow::Error>(None);
            };
            for task_id in &recovery.cancel_children {
                let _ = session.cancel_subagent_task(task_id).await;
            }
            let disposition = match &recovery.disposition {
                ResearchRecoveryDisposition::PublicationPreserved { artifacts, outcome } => {
                    format!(
                        "preserved the exact receipt-backed publication at {} with {:?} outcome",
                        artifacts.html.display(),
                        outcome
                    )
                }
                ResearchRecoveryDisposition::AcquisitionPreserved { artifacts } => format!(
                    "preserved the completed acquisition checkpoint as an audit-only report at {}",
                    artifacts.html.display()
                ),
                ResearchRecoveryDisposition::FailedWithoutRecoverableAcquisition => {
                    "no completed acquisition checkpoint was available".to_string()
                }
            };
            Ok(Some(format!(
                "⚠ recovered interrupted DeepResearch run {} · cancelled {} live child{} · reconciled {} orphan{} · {}",
                recovery.run_id,
                recovery.cancel_children.len(),
                if recovery.cancel_children.len() == 1 { "" } else { "ren" },
                recovery.orphaned_children.len(),
                if recovery.orphaned_children.len() == 1 { "" } else { "s" },
                disposition,
            )))
        }
        .await
        .map_err(|error| error.to_string());
        Msg::InterruptedResearchStartupRecovered(result)
    })
}

/// Launch Code using the directory, configuration, and platform paths resolved
/// once at the typed CLI boundary. This function never changes process CWD.
pub(crate) async fn run_in(
    args: Vec<String>,
    workspace: &Path,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    let mut startup_trace = StartupTrace::from_env();
    startup_trace.checkpoint("process_entry");
    let first_frame = startup_trace.first_frame_gate();
    let smoke_mode = std::env::var_os("A3S_CODE_TUI_SMOKE").is_some();
    let mut loading_indicator = StartupLoadingIndicator::begin(!smoke_mode);
    if smoke_mode {
        first_frame.activate_headless();
    }
    // `a3s code resume [id]` continues a saved session (newest if no id given);
    // otherwise a fresh id. Existence is verified against the store below.
    let resuming = args.first().map(String::as_str) == Some("resume");
    let explicit_id = if resuming { args.get(1).cloned() } else { None };
    let mut session_id = explicit_id.clone().unwrap_or_else(new_session_id);
    // First launch creates a user starter only when no explicit, workspace, or
    // user ACL layer exists.
    let created_config = if context.explicit_config.is_none()
        && crate::commands::config_resolver::workspace_config_path(workspace).is_none()
        && context
            .user_config_path()
            .is_none_or(|path| !path.is_file())
    {
        let path = context
            .user_config_path()
            .ok_or_else(|| anyhow::anyhow!("no user home found for ~/.a3s/config.acl"))?;
        write_template_config(&path)
            .map_err(|error| anyhow::anyhow!("failed to write starter config {path:?}: {error}"))?;
        true
    } else {
        false
    };
    let runtime_configuration =
        crate::commands::config::resolve_code_runtime_configuration(context)?;
    let config_path = runtime_configuration.config_path;
    let mut code_config = runtime_configuration.config;
    let workspace_retrieval_options =
        crate::workspace_retrieval::build_deferred_workspace_retrieval_options(
            &runtime_configuration.workspace_retrieval,
            &runtime_configuration.trusted_host_config,
            Some(&context.component_paths.data_root),
            context.network.allow_first_use_install,
        )?;
    ensure_tui_lane_queue(&mut code_config);
    let asset_directories = runtime_configuration.asset_directories;
    let memory_dir = runtime_configuration.memory_dir;
    let hook_executor = crate::code_hooks::CommandHookExecutor::discover(
        workspace,
        context.home.as_deref(),
        context
            .component_paths
            .state_root
            .join("code/hooks-trust.json"),
    )?;
    // Compose the Use-owned Plugin Manager service once over Code's immutable
    // host policy and provider boundary. The local manager remains available
    // only as the Runtime Task invoker; `/packages` owns no parallel plan or
    // mutation path. Initialization remains fail-closed without blocking Code.
    let (
        plugin_runtime_manager,
        plugin_manager_service,
        plugin_authorization,
        plugin_manager_error,
    ) = match crate::commands::plugin::load_host_authorization_context(context).await {
        Ok(authorization) => {
            match a3s::plugin_manager::PluginManager::from_host_with_policy_and_runtime_config(
                &config_path,
                workspace,
                authorization.handoff().source(),
                a3s::plugin_manager::PluginManagerPolicy {
                    offline: context.network.offline,
                    authorization: authorization.policy().clone(),
                },
            )
            .await
            {
                Ok(manager) => {
                    let manager = Arc::new(manager);
                    match manager.shared_service() {
                        Ok(service) => (
                            Some(manager),
                            Some(Arc::new(service)),
                            Some(authorization),
                            None,
                        ),
                        Err(error) => (
                            Some(manager),
                            None,
                            Some(authorization),
                            Some(format!(
                                "the Use-owned Plugin Manager service could not be initialized: {error}"
                            )),
                        ),
                    }
                }
                Err(error) => (
                    None,
                    None,
                    Some(authorization),
                    Some(format!(
                        "the Code Plugin Manager host could not be initialized: {error}"
                    )),
                ),
            }
        }
        Err(error) => (
            None,
            None,
            None,
            Some(format!(
                "the host plugin authorization policy could not be loaded: {error}"
            )),
        ),
    };
    startup_trace.checkpoint("configuration_and_policy");
    let configured_mcp_servers = code_config.mcp_servers.clone();
    let mut bootstrap_code_config = code_config.clone();
    bootstrap_code_config.mcp_servers.clear();
    let agent = Arc::new(
        Agent::from_config(bootstrap_code_config)
            .await
            .map_err(|error| anyhow::anyhow!("failed to load effective agent config: {error}"))?,
    );
    let workspace = workspace.to_string_lossy().into_owned();
    let evolution = crate::evolution::WorkspaceEvolution::new(&workspace);
    // The existing Evolution catalog is enough for the first prompt. Scanning
    // the complete memory store is maintenance work and starts after the first
    // frame so a large store cannot delay terminal handoff.
    let learned_preferences = match evolution.session_preference_prompt() {
        Ok(preferences) => preferences,
        Err(error) => {
            tracing::warn!(%error, "could not load learned preferences before TUI session startup");
            None
        }
    };
    let evolution_observer = crate::evolution::EvolutionMemoryObserver::new(evolution.clone());
    startup_trace.checkpoint("agent_and_evolution_catalog");

    // Configured "provider/model" ids (+ context windows) + the default model.
    let mut models: Vec<String> = Vec::new();
    let mut model_ctx: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for (p, m) in code_config.list_models() {
        let id = format!("{}/{}", p.name, m.id);
        model_ctx.insert(id.clone(), m.limit.context);
        models.push(id);
    }
    let default_model = code_config.default_model.clone();
    let os_config = code_config.os.clone();

    // Persistent, resumable session: stored under <cwd>/.a3s/tui/sessions.
    let store_dir = resolve_tui_session_store_dir(std::path::Path::new(&workspace));
    // keyed by a fixed id, so relaunching in the same directory continues the
    // conversation. Falls back to a fresh session when none exists yet.

    // Resolve `resume`: verify the id exists (else show what's available), or
    // pick the most recent session when no id was given.
    let store: Arc<dyn a3s_code_core::store::SessionStore> = Arc::new(
        a3s_code_core::store::FileSessionStore::new(&store_dir)
            .await
            .map_err(|error| {
                anyhow::anyhow!("failed to open session store {store_dir:?}: {error}")
            })?,
    );
    if resuming {
        match select_resume_session(store.as_ref(), explicit_id.as_deref()).await? {
            ResumeSessionSelection::Selected(selected) => session_id = selected,
            ResumeSessionSelection::Missing {
                requested: Some(id),
                available,
            } => {
                loading_indicator.clear();
                eprintln!("a3s: session '{id}' not found in {}", store_dir.display());
                if available.is_empty() {
                    eprintln!("  (no saved sessions in this directory)");
                } else {
                    eprintln!("  available sessions (newest first):");
                    for session_id in available.iter().take(10) {
                        eprintln!("    a3s code resume {session_id}");
                    }
                }
                return Ok(());
            }
            ResumeSessionSelection::Missing {
                requested: None, ..
            } => {
                loading_indicator.clear();
                eprintln!(
                    "a3s: no saved sessions to resume in {}",
                    store_dir.display()
                );
                return Ok(());
            }
        }
    }
    startup_trace.checkpoint("session_store");

    let tui_session_state = match load_tui_session_state(Path::new(&workspace), &session_id) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(
                %error,
                %session_id,
                "ignoring unreadable per-session TUI state"
            );
            None
        }
    };
    if let Some(theme) = tui_session_state
        .as_ref()
        .and_then(TuiSessionState::theme_index)
    {
        SYNTAX_THEME.store(theme, std::sync::atomic::Ordering::Relaxed);
    }

    // Enable HITL confirmation so file-modifying tools (write/edit/patch) can
    // run — they require a confirmation manager, otherwise they fail with
    // "requires confirmation but no HITL confirmation manager is configured".
    // The TUI is that manager (approve/deny modal, or /auto). Keep the human
    // confirmation wait separate from the tool execution timeout: reading and
    // deciding must not consume the tool's runtime budget.
    let confirmation = a3s_code_core::hitl::ConfirmationPolicy::enabled()
        .with_timeout(HITL_CONFIRM_TIMEOUT_MS, TimeoutAction::Reject);
    // Claude Code compatibility: load Claude/plugin SKILL.md skills alongside
    // a3s's own (they share the markdown + YAML-frontmatter format).
    let mut claude_dirs = agent_skill_dirs_with_configured(&workspace, &asset_directories.skill);
    // Restore the persisted OS login *before* building the session, so its
    // login-gated built-in `a3s-os-capabilities` skill is materialized and
    // loaded from the first turn (only when signed in).
    let os_session = os_config.as_ref().and_then(crate::a3s_os::current_session);
    if let Some(s) = &os_session {
        // Export endpoint + token so the agent's shell uses $A3S_OS_* directly
        // instead of re-reading ~/.a3s/os-auth.json every call.
        crate::a3s_os::export_os_env(s);
        if let Some(dir) = os_config
            .as_ref()
            .and_then(crate::a3s_os::ensure_capability_skill_dir)
        {
            claude_dirs.push(dir);
        }
    }
    let initial_effort = tui_session_state
        .as_ref()
        .and_then(TuiSessionState::effort_index)
        .or_else(load_tui_effort_preference)
        .unwrap_or(DEFAULT_TUI_EFFORT_INDEX);
    let initial_mode = tui_session_state
        .as_ref()
        .map(TuiSessionState::mode)
        .unwrap_or(Mode::Default);
    let sidecar_model_preference = tui_session_state
        .as_ref()
        .and_then(|state| state.model.clone());
    // Legacy sessions predate the TUI sidecar. Their Core snapshot can still
    // identify a config.acl model, or guard an account-backed global fallback
    // by requiring the model identity to match this exact session.
    let persisted_session = if resuming && sidecar_model_preference.is_none() {
        match store.load(&session_id).await {
            Ok(session) => session,
            Err(error) => {
                tracing::warn!(
                    %error,
                    %session_id,
                    "could not inspect the persisted model while restoring TUI settings"
                );
                None
            }
        }
    } else {
        None
    };
    let persisted_model = persisted_session
        .as_ref()
        .and_then(persisted_model_from_session);
    let persisted_config_model_preference = persisted_session
        .as_ref()
        .and_then(|session| configured_model_preference_from_session(session, &models));
    let global_model_preference = load_model_selection_preference().filter(|preference| {
        persisted_model.as_deref().is_none_or(|persisted_model| {
            preference_matches_persisted_model(preference, persisted_model)
        })
    });
    let model_preference = sidecar_model_preference
        .or(persisted_config_model_preference)
        .or(global_model_preference);
    let restored_model_selection = model_preference.as_ref().and_then(|preference| {
        restore_model_selection(
            preference,
            &models,
            os_session.as_ref(),
            session_id.as_str(),
            initial_effort,
        )
    });
    let launch_model_source = restored_model_selection
        .as_ref()
        .and(model_preference.as_ref())
        .map(|preference| preference.source)
        .unwrap_or(ModelSelectionSource::Config);
    let launch_model = restored_model_selection
        .as_ref()
        .map(|(model, _)| model.clone())
        .or_else(|| default_model.clone());
    let launch_llm_override = restored_model_selection
        .as_ref()
        .and_then(|(_, client)| client.clone());
    let context_limit = launch_model
        .as_ref()
        .map(|m| ctx_limit_for_model(&model_ctx, m))
        .unwrap_or_else(|| resolve_ctx_limit(None));
    let initial_budget = budget_plan_for_effort_index(
        initial_effort,
        Some(context_limit),
        BudgetWorkload::Interactive,
    );
    let has_exact_codex_effort = launch_llm_override
        .as_ref()
        .and_then(|client| client.codex_effort_status(EFFORT_LEVELS[initial_effort].id))
        .is_some_and(|status| !status.capped);
    let launch_effort_guideline =
        panels::model::prompt_guideline_for_effort(initial_effort, has_exact_codex_effort);
    startup_trace.checkpoint("session_profile");
    let initial_auto_delegation = effort_uses_automatic_delegation(initial_effort);
    let deep_research_report_tool_gate = DeepResearchReportToolGate::default();
    deep_research_report_tool_gate.set_workspace(Path::new(&workspace));
    let project_permission_rules_path = project_permission_rules_path(Path::new(&workspace));
    let permission_rules_to_load = project_permission_rules_path.clone();
    let project_permission_load = tokio::task::spawn_blocking(move || {
        load_project_permission_grants(&permission_rules_to_load)
    })
    .await
    .map_err(|error| format!("permission rule loader failed: {error}"))
    .and_then(|result| result);
    let (project_permission_grants, project_permission_load_error) = match project_permission_load {
        Ok(grants) => (grants, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let permission_grants = TuiPermissionGrants::with_project(project_permission_grants);
    startup_trace.checkpoint("permissions");
    let deferred_sandbox = Arc::new(DeferredBashSandbox::new());
    let sandbox_handle: Arc<dyn a3s_code_core::sandbox::BashSandbox> = deferred_sandbox.clone();
    let execution_policy = TuiExecutionPolicy::for_workspace_with_deferred_sandbox(
        initial_mode,
        PathBuf::from(&workspace),
        sandbox_handle,
    );
    startup_trace.checkpoint("sandbox_proxy");
    // Claude Code compatibility: inject CLAUDE.md (AGENTS.md is auto-loaded by
    // the core) into the system prompt via prompt slots.
    let instructions = project_instructions(&workspace);
    // When a persisted login is restored on launch, inject the OS-platform
    // directive too (mirrors effort_session_opts) so the very first turn already
    // routes OS questions through the progressive-API skill.
    let os_address = os_session.as_ref().map(|s| s.address.clone());
    // Past-session recall: when the ctx CLI is installed, teach the agent to
    // search local agent history before re-deriving prior work.
    let ctx_ready = panels::ctx::ctx_available();
    let with_instr = |o: SessionOptions| {
        with_tui_prompt_context(
            o,
            instructions.as_deref(),
            os_address.as_deref(),
            ctx_ready,
            learned_preferences.as_deref(),
            launch_effort_guideline,
        )
    };
    let manifest_backend = tui_manifest_backend(Path::new(&workspace));
    let workspace_manifest = manifest_backend.manifest();
    debug_assert!(!workspace_manifest.is_active());
    let initial_files = Vec::new();
    let workspace_manifest_rx = Arc::new(Mutex::new(workspace_manifest.subscribe()));
    let code_intelligence_file_system: Arc<dyn a3s_code_core::workspace::WorkspaceFileSystem> =
        manifest_backend.clone();
    let code_intelligence = LocalCodeIntelligence::start(
        "a3s-code-tui",
        Arc::clone(&workspace_manifest),
        code_intelligence_file_system,
    )
    .await
    .map_err(|error| anyhow::anyhow!("failed to start Code Intelligence: {error}"))?;
    startup_trace.checkpoint("workspace_services");
    let provider: Arc<dyn WorkspaceCodeIntelligence> = code_intelligence.clone();
    let workspace_services = crate::workspace_retrieval::workspace_services_for_host(
        manifest_backend,
        workspace_retrieval_options.as_ref(),
    )?
    .with_code_intelligence(provider);
    let session_memory: Arc<dyn a3s_memory::MemoryStore> = Arc::new(
        super::lazy_memory_store::LazyFileMemoryStore::new(memory_dir.clone()),
    );
    let auto_compact_threshold = auto_compact_threshold_for_path(&config_path);
    let build_session_options = |thinking: bool| {
        let mut options = apply_launch_model_options(
            with_instr(with_recent_workspace_context(
                tui_session_options_with_gate_grants_and_execution(
                    confirmation.clone(),
                    deep_research_report_tool_gate.clone(),
                    permission_grants.clone(),
                    execution_policy.clone(),
                )
                .with_session_store(store.clone())
                .with_hook_executor(hook_executor.clone())
                .with_session_id(session_id.as_str())
                .with_workspace_backend(workspace_services.clone())
                .with_skill_dirs(claude_dirs.clone())
                .with_auto_save(true)
                .with_auto_compact(true)
                .with_max_context_tokens(context_limit as usize)
                .with_auto_compact_threshold(auto_compact_threshold as f32)
                .with_memory(Arc::clone(&session_memory))
                .with_memory_observer(evolution_observer.clone())
                .with_max_parallel_tasks(initial_budget.max_parallel_tasks)
                .with_max_tool_rounds(initial_budget.max_tool_rounds)
                .with_max_continuation_turns(initial_budget.max_continuation_turns)
                .with_auto_delegation_enabled(initial_auto_delegation)
                .with_auto_parallel_delegation(initial_auto_delegation)
                .with_manual_delegation_enabled(true),
                &workspace_manifest,
            )),
            launch_model.as_deref(),
            launch_llm_override.as_ref(),
            EFFORT_LEVELS[initial_effort].id,
            &code_config,
            session_id.as_str(),
        );
        if thinking {
            options = options.with_thinking_budget(initial_budget.thinking_budget);
        }
        if initial_effort == ULTRACODE {
            options = options
                .with_planning_mode(a3s_code_core::PlanningMode::Auto)
                .with_goal_tracking(true);
        }
        options.with_optional_workspace_retrieval(workspace_retrieval_options.as_ref())
    };
    let session_mode = if resuming {
        SessionRebuildMode::ResumeExisting
    } else {
        SessionRebuildMode::CreateFresh
    };
    let session_result = panels::model::rebuild_agent_session(
        Arc::clone(&agent),
        workspace.clone(),
        session_id.clone(),
        build_session_options(true),
        build_session_options(false),
        session_mode,
    )
    .await;
    let (session, _thinking_dropped) = session_result.map_err(|error| {
        if resuming {
            anyhow::anyhow!(
                "failed to resume session {session_id}; refusing to replace its persisted history with an empty session: {error}"
            )
        } else {
            anyhow::anyhow!("failed to create session {session_id}: {error}")
        }
    })?;
    startup_trace.checkpoint("session");
    let _ = session
        .memory()
        .ok_or_else(|| anyhow::anyhow!("session memory was not initialized"))?;

    // DynamicWorkflowRuntime is always available in the TUI because built-in
    // `?` deep research and ultracode dynamic workflows both route through it.
    let _ = session.register_dynamic_workflow_runtime();

    // A3S Runtime offload tool: registered only when signed in to OS, so the
    // model sees `runtime` after login and not before. Auth changes re-sync it via
    // `refresh_after_auth` → `sync_runtime_tool`.
    if let Some(os) = os_session.as_ref() {
        let _ = session.register_dynamic_tool(std::sync::Arc::new(
            crate::runtime_tool::RuntimeTool::new(os),
        ));
    }

    // Some PTY hosts transiently report a successful 0x0 size. Treat that as
    // unavailable so the first frame still contains the loading state and an
    // input surface instead of being rendered as an empty screen.
    let (width, height) = usable_terminal_size(a3s_tui::terminal::Terminal::size().ok());

    // Seed the transcript with the complete resumed conversation, including
    // semantic tool calls paired with their persisted results.
    let resumed = session.history();
    let mut initial_messages = resumed_transcript_entries(&resumed);
    // Seed ↑/↓ input recall with the user's prior prompts so resuming a session
    // keeps its command history (tool-result `user` messages carry no text block,
    // so the non-empty filter excludes them).
    let history_seed: Vec<String> = resumed
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.text().trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let initial_auto_review_revision = u64::try_from(history_seed.len()).unwrap_or(u64::MAX);

    // Quiet confirmation that the persisted login was restored. Only when
    // RESUMING an existing conversation — on a fresh start, leaving the transcript
    // empty lets the welcome banner show (it notes the signed-in account itself);
    // inserting this line here is what was suppressing the banner after OS login.
    if let Some(s) = &os_session {
        if !initial_messages.is_empty() {
            initial_messages.insert(
                0,
                TranscriptEntry::preformatted(Style::new().fg(TN_GRAY).render(&format!(
                    "  ✓ signed in to OS as {} · capabilities skill active · /logout to sign out",
                    s.display_label()
                ))),
            );
        }
    }

    let session = Arc::new(session);
    let active_session = Arc::new(std::sync::Mutex::new(Arc::clone(&session)));
    let (configured_mcp, mut configured_mcp_task) =
        ConfiguredMcpRuntime::start(configured_mcp_servers, Arc::clone(&session));

    // A3S Use is a first-use component. Discovery, verified installation, and
    // initial MCP/Skill projection run behind a shared slot so terminal takeover
    // never waits for the network or a provider process. Once ready, the
    // registry is attached to whichever session is active at that instant.
    // Offline/A3S_NO_AUTO_INSTALL remain strict no-mutation policies.
    let use_registry = crate::use_registry::UseRegistrySlot::preparing();
    let (use_setup_cancellation, mut use_setup_task) = spawn_code_use_setup(
        context,
        Arc::clone(&active_session),
        use_registry.clone(),
        plugin_authorization
            .as_ref()
            .map(|authorization| authorization.handoff().clone()),
        plugin_runtime_manager
            .as_ref()
            .map(|manager| Arc::clone(manager) as Arc<dyn crate::use_registry::RuntimeTaskInvoker>),
        plugin_runtime_manager
            .as_ref()
            .map(|manager| Arc::clone(manager) as Arc<dyn crate::use_registry::McpRuntimeResolver>),
        first_frame.clone(),
    );

    // Headless smoke mode exercises the same Use and WebView first-use
    // preparation that the interactive TUI receives, without taking over the
    // terminal.
    let deferred_sandbox_setup = if smoke_mode {
        if let Some(warning) = prepare_deferred_sandbox(
            context,
            Path::new(&workspace),
            Arc::clone(&deferred_sandbox),
            execution_policy.clone(),
        )
        .await
        {
            eprintln!("[smoke] {warning}");
        }
        None
    } else {
        Some(deferred_sandbox_setup_command(
            context.clone(),
            PathBuf::from(&workspace),
            Arc::clone(&deferred_sandbox),
            execution_policy.clone(),
        ))
    };
    let deferred_webview_setup = if smoke_mode {
        let webview_resolution = resolve_code_webview(context).await;
        if let Some(warning) = webview_resolution.warning {
            initial_messages.push(TranscriptEntry::preformatted(
                Style::new().fg(TN_YELLOW).render(&format!("  ⚠ {warning}")),
            ));
        }
        None
    } else {
        let webview_context = context.clone();
        Some(cmd::cmd(move || async move {
            let resolution = resolve_code_webview(&webview_context).await;
            Msg::CodeWebviewReady {
                executable: resolution.executable,
                warning: resolution.warning,
            }
        }))
    };
    startup_trace.checkpoint("session_runtime");

    if smoke_mode {
        // Headless smoke has no rendered frame. Its explicit headless gate was
        // opened at process entry, so activate the same workspace discovery
        // that an interactive session starts after its first flushed frame.
        workspace_manifest.activate();
        configured_mcp.activate();
        configured_mcp.wait_for_initial_projection().await;
        if let Some(retrieval) = &workspace_retrieval_options {
            // Headless smoke has no terminal frame to open the normal startup
            // gate. It explicitly opts into the same post-frame work before
            // issuing a model or direct-tool request.
            retrieval.activate_background_indexing();
        }
        if std::env::var_os("A3S_CODE_TUI_SMOKE_WAIT_USE").is_some() {
            // Capability E2E tests opt into the old first-turn projection
            // contract. Setup may return after its normal five-second
            // projection budget while a cold provider continues converging,
            // so give the attached session one additional bounded window.
            // A plain startup smoke must remain bounded even when a provider
            // process ignores cancellation or never produces a registry
            // snapshot.
            let _ = (&mut use_setup_task).await;
            let projection_visible = match use_registry.ready_handle() {
                Some(handle) => {
                    handle
                        .wait_until_projection_visible(
                            session.as_ref(),
                            USE_SMOKE_PROJECTION_SETTLE,
                        )
                        .await
                }
                None => false,
            };
            if !projection_visible {
                eprintln!(
                    "[smoke] A3S Use projection did not settle:\n{}",
                    use_registry.status_text(Arc::clone(&session), false).await
                );
            }
        } else {
            stop_code_use_setup(&use_setup_cancellation, &mut use_setup_task).await;
        }
        let result = run_smoke(
            Arc::clone(&session),
            Path::new(&workspace),
            deep_research_report_tool_gate,
        )
        .await;
        // Begin registry cancellation before closing the session. Core then
        // observes session cancellation inside any in-flight MCP connection and
        // completes its rollback instead of making host shutdown wait for the
        // provider's full connection timeout.
        let use_registry_shutdown = spawn_code_use_shutdown(&use_registry);
        stop_configured_mcp_runtime(&configured_mcp, &mut configured_mcp_task).await;
        let _ = settle_session_close_for_quit(
            async move {
                session.close().await;
            },
            Duration::from_millis(GRACEFUL_QUIT_SESSION_CLOSE_GRACE_MS),
        )
        .await;
        deferred_sandbox.close().await;
        settle_code_use_shutdown(use_registry_shutdown).await;
        return result;
    }

    let deferred_research_recovery = Some(deferred_interrupted_research_recovery_command(
        Arc::clone(&session),
        PathBuf::from(&workspace),
    ));
    startup_trace.checkpoint("research_recovery_deferred");

    let keymap = Keymap::new()
        .bind(
            KeyBinding::new(KeyCode::PageUp),
            Action::ScrollUp,
            "Scroll up",
        )
        .bind(
            KeyBinding::new(KeyCode::PageDown),
            Action::ScrollDown,
            "Scroll down",
        )
        // NB: Ctrl+U / Ctrl+D are intentionally NOT bound to scroll — they shadow
        // readline line-editing (Ctrl+U = delete-to-start) in the input. PageUp/Down
        // and Ctrl+Home/End cover scrolling.
        .bind(
            KeyBinding::ctrl(KeyCode::Home),
            Action::ScrollTop,
            "Scroll to top",
        )
        .bind(
            KeyBinding::ctrl(KeyCode::End),
            Action::ScrollBottom,
            "Scroll to bottom",
        );

    let initial_paused_goal = tui_session_state
        .as_ref()
        .and_then(|state| state.paused_goal.clone());
    let initial_goal_resume_prompt = initial_paused_goal.as_ref().map(|_| 0);
    startup_trace.checkpoint("app_prelude");

    let agent_presence = agent_presence::AgentPresenceRuntime::new(None);
    startup_trace.checkpoint("agent_presence");
    let messages = Transcript::from_entries(initial_messages);
    startup_trace.checkpoint("transcript");
    let deferred_ui_metadata = Some(deferred_ui_metadata_command(workspace.clone()));
    let codex_account_models = Vec::new();
    let branch = None;
    let skill_count = 0;
    let skills = Vec::new();
    let disabled_skills = HashSet::new();
    startup_trace.checkpoint("ui_metadata_deferred");
    let viewport = Viewport::new(width, height.saturating_sub(7));
    let textarea = Textarea::new()
        .with_height(1)
        .with_auto_grow(8) // box grows with Shift+Enter newlines (no scroll)
        .with_width(textarea_width_for(width)) // prompt prefix is outside the textarea
        .with_submit_on_enter(true);
    let spinner = Spinner::new().with_title("");
    let streaming = StreamingMarkdown::new(transcript_markdown_width_for(width));
    let workspace_retrieval_status = session.workspace_retrieval_status();
    startup_trace.checkpoint("ui_primitives");

    let mut app = App {
        session,
        active_session: Arc::clone(&active_session),
        first_frame,
        startup_loading: StartupLoadingState::waiting_for_first_frame(),
        use_registry: use_registry.clone(),
        configured_mcp: configured_mcp.clone(),
        deferred_sandbox_setup,
        deferred_webview_setup,
        deferred_ui_metadata,
        deferred_research_recovery,
        plugin_manager_service,
        plugin_manager_error,
        agent: agent.clone(),
        store: store.clone(),
        confirmation,
        deep_research_report_tool_gate,
        session_id: session_id.clone(),
        model_source: launch_model_source,
        session_rebuild_seq: 0,
        session_rebuild_pending: None,
        models,
        model_ctx,
        context_limit,
        last_prompt_tokens: 0,
        compact_summary: None,
        ctx_warned_tier: 0,
        model_menu: None,
        model_tab: 0,
        relay_panel: None,
        relay_scan_seq: 0,
        task_panel: None,
        task_panel_seq: 0,
        permission_panel: None,
        codex_account_models,
        codex_models_loading: false,
        codex_models_refreshed_at: None,
        account_models: HashMap::new(),
        account_models_loading: HashSet::new(),
        account_model_errors: HashMap::new(),
        llm_override: launch_llm_override,
        code_config: Arc::new(code_config),
        workspace_retrieval_options,
        workspace_retrieval_status,
        asset_directories,
        config_path: config_path.clone(),
        hook_executor,
        memory_dir,
        memory_store: Arc::clone(&session_memory),
        auto_compact_threshold,
        os_config,
        os_session,
        os_refreshing: false,
        os_gateway_models: None,
        os_gateway_models_loading: false,
        os_gateway_error: None,
        last_view: None,
        pending_deep_research_report_view: None,
        deep_research_loop: None,
        deep_research_workflow: DeepResearchWorkflowSnapshot::default(),
        deep_research_handle: None,
        deep_research_events: None,
        deep_research_outcome: DeepResearchRunOutcome::Active,
        deep_research_stream_timeout_token: 0,
        stream_start_token: 0,
        interrupted_stream_start_token: None,
        pending_interrupted_continuation: None,
        runtime_expectation: None,
        effort: initial_effort,
        effort_panel: None,
        theme_panel: None,
        quit_armed: None,
        quitting: false,
        last_activity: Instant::now(),
        auto_review: AutoReviewTracker::new(initial_auto_review_revision),
        shell_mode: false,
        research_mode: false,
        review_pending: false,
        sleep_pending: false,
        review: None,
        review_open: false,
        flow: None,
        pending_flow_subcommand: None,
        agent_picker: None,
        pending_agent_subcommand: None,
        agent_dev: None,
        mcp_picker: None,
        pending_mcp_subcommand: None,
        mcp_dev: None,
        skill_picker: None,
        pending_skill_subcommand: None,
        skill_dev: None,
        okf_picker: None,
        pending_okf_subcommand: None,
        okf_dev: None,
        autonomy_restore: None,
        ctx_ready,
        ctx_hits: Vec::new(),
        pending_ctx: None,
        loop_continuation: false,
        turn_text: String::new(),
        llm_turn_checkpoint: None,
        selection: None,
        last_workflow: None,
        pending_images: Vec::new(),
        goal: None,
        goal_since: None,
        goal_run: None,
        paused_goal: initial_paused_goal,
        goal_resume_prompt: initial_goal_resume_prompt,
        goal_generation: 0,
        pending_goal_failure: None,
        deep_research_goal_restore: None,
        loop_remaining: 0,
        runtime: RuntimeProjection::default(),
        core_run_status: CoreRunStatus::default(),
        agent_presence,
        background_subagent_watches: HashSet::new(),
        subagent_snapshot_request_id: 0,
        deep_research_subagent_settlement_inflight: false,
        deep_research_journal_finalization_inflight: false,
        deep_research_terminal_artifacts: None,
        deep_research_agent_event_sequence: 0,
        deep_research_projection: None,
        turn_had_agent_activity: false,
        turn_text_after_activity: false,
        ultracode_synthesis_inflight: false,
        ultracode_synthesis_used: false,
        instructions,
        workspace_manifest: Arc::clone(&workspace_manifest),
        workspace_manifest_rx,
        workspace_services,
        gradient_until: None,
        gradient_frame: 0,
        ultracode_animation_epoch: 0,
        effort_anim: None,
        transcript_view: None,
        viewport,
        textarea,
        spinner,
        streaming,
        got_delta: false,
        compacting: None,
        updating: None,
        checkup_inflight: false,
        last_paint: None,
        thinking: String::new(),
        state: State::Idle,
        messages,
        startup_transcript_bounded: false,
        rx: None,
        stream_join: None,
        stream_join_settling: false,
        stream_settle_abort: None,
        host_tool_abort: None,
        host_progress_inflight: false,
        host_tool_call_id: None,
        interrupting: false,
        pending_tools: VecDeque::new(),
        permission_grants,
        execution_policy,
        project_permission_rules_path,
        permission_rule_write_inflight: None,
        project_permission_revoke_seq: 0,
        project_permission_revoke_inflight: None,
        approval_feedback: None,
        approval_sel: 0,
        history: history_seed,
        history_panel: None,
        history_pos: None,
        history_draft: None,
        model: launch_model,
        output_tokens: 0,
        stream_started: None,
        blink_tick: 0,
        anim: 0,
        mode: initial_mode,
        queue: PriorityQueue::new(),
        queued_turn_modes: HashMap::new(),
        queued_plan_drafts: HashMap::new(),
        send_now_queued_sequence: None,
        queue_panel: None,
        active_rewind_checkpoint: None,
        rewind_checkpoints: VecDeque::new(),
        next_rewind_checkpoint_id: 0,
        rewind_finalization_pending: None,
        active_queued_turn: None,
        active_queued_turn_token: None,
        active_turn_mode: None,
        active_plan_draft: None,
        queue_retry_generation: 0,
        queue_retry_attempt: 0,
        running_task: None,
        plan: PlanProjection::default(),
        pending_plan_review: None,
        plan_review: None,
        ide: None,
        memory: None,
        evolution: None,
        asset_list: None,
        runtime_activity: None,
        kb: None,
        loop_panel: None,
        help_open: false,
        help_scroll: 0,
        completed: 0,
        branch,
        slash_sel: 0,
        slash_menu_dismissed_for: None,
        files: initial_files,
        at_expanded: std::collections::HashSet::new(),
        file_sel: 0,
        skill_count,
        skills,
        disabled_skills,
        plugins_panel: None,
        package_panel: None,
        package_panel_seq: 0,
        update_available: None,
        cwd: workspace.clone(),
        width,
        height,
        keymap,
    };
    startup_trace.checkpoint("app_constructed");

    // Model::init owns the initial viewport build. Append startup feedback as
    // semantic entries here without rebuilding the complete resumed history
    // once per notice.
    if let Some(error) = project_permission_load_error {
        app.messages.push(TranscriptEntry::notice(
            NoticeKind::Warning,
            format!("Project permission rules were ignored: {error}"),
        ));
    }
    // First launch: drop the user straight into the editor on the new config.
    if created_config {
        app.messages.push(TranscriptEntry::preformatted(gutter(
            ACCENT,
            "Welcome to a3s code! Generated a starter ~/.a3s/config.acl — fill in your \
             provider apiKey/baseUrl + model, Ctrl+S to save, Esc to close, then restart \
             `a3s code` to load it.",
        )));
        app.open_config_in_ide(&config_path);
        app.rebuild_viewport();
    }
    startup_trace.checkpoint("first_launch_editor");

    // Launch constructed the complete effort profile already. Do not resume
    // and rebuild the same session a second time before terminal takeover.
    startup_trace.checkpoint("terminal_handoff");
    loading_indicator.clear();
    let program_result = ProgramBuilder::new(app)
        .with_alt_screen()
        // Capture mouse input so wheel/trackpad scrolling works in the alternate
        // screen. Drag-copy is app-owned: on release we write the selected text to
        // the clipboard, so scroll and copy can coexist.
        .with_mouse_support()
        .with_fps(120)
        .run()
        .await;

    // Stop repository discovery before waiting on any other background
    // service. A manifest rescan can own a cancellable Git process; opening
    // its cancellation boundary first prevents that process from extending
    // terminal shutdown or retaining workspace-service pipes.
    workspace_manifest.shutdown();
    stop_code_use_setup(&use_setup_cancellation, &mut use_setup_task).await;
    stop_configured_mcp_runtime(&configured_mcp, &mut configured_mcp_task).await;
    let use_registry_shutdown = spawn_code_use_shutdown(&use_registry);

    let final_session = active_session
        .lock()
        .map(|session| Arc::clone(&session))
        .map_err(|_| anyhow::anyhow!("active session lock was poisoned"));
    if let Ok(session) = &final_session {
        let session = Arc::clone(session);
        let _ = settle_session_close_for_quit(
            async move {
                session.close().await;
            },
            Duration::from_millis(GRACEFUL_QUIT_SESSION_CLOSE_GRACE_MS),
        )
        .await;
    }
    deferred_sandbox.close().await;
    settle_code_use_shutdown(use_registry_shutdown).await;
    if let Some(manager) = &plugin_runtime_manager {
        manager.shutdown().await;
    }
    let code_intelligence_shutdown_complete =
        shutdown_code_intelligence(Arc::clone(&code_intelligence)).await;
    program_result?;

    let final_session = final_session?;
    let session_id = final_session.session_id().to_string();
    if let Err(error) = final_session.save().await {
        eprintln!("⚠  could not save session {session_id}: {error}");
    }

    // `/update` found a newer version → upgrade via Homebrew in the (now
    // restored) shell so brew's own download progress shows, then re-exec the
    // freshly-installed binary. Use PATH `a3s` (brew repointed its symlink to
    // the new version); current_exe() is the OLD version's path.
    if UPGRADE_ON_EXIT.load(std::sync::atomic::Ordering::Relaxed) {
        let resume_command = render_resume_command(&session_id, stderr_color_enabled(context));
        let latest = LATEST
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_default();
        match crate::update::perform_upgrade(&latest) {
            Ok(bin) => {
                let restart_args = ["code", "resume", session_id.as_str()];
                if !code_intelligence_shutdown_complete {
                    eprintln!(
                        "\n✓ updated to a3s {latest}; automatic restart was skipped because \
                         background cleanup did not settle. Resume manually with: {resume_command}\n"
                    );
                    return Ok(());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt;
                    // exec replaces this process; only returns on failure → fall back.
                    let err = std::process::Command::new(&bin).args(restart_args).exec();
                    eprintln!(
                        "\n⚠  updated, but restart via {} failed: {err}",
                        bin.display()
                    );
                    if let Ok(exe) = std::env::current_exe() {
                        let err = std::process::Command::new(&exe).args(restart_args).exec();
                        eprintln!("⚠  fallback restart via {} failed: {err}", exe.display());
                    }
                    eprintln!(
                        "✓ updated to a3s {latest}; resume manually with: {resume_command}\n"
                    );
                }
                #[cfg(not(unix))]
                {
                    match std::process::Command::new(&bin).args(restart_args).status() {
                        Ok(status) if status.success() => {}
                        Ok(status) => eprintln!(
                            "\n⚠  updated, but restart exited with status {status}; resume manually with: {resume_command}\n"
                        ),
                        Err(err) => eprintln!(
                            "\n⚠  updated, but restart failed: {err}; resume manually with: {resume_command}\n"
                        ),
                    }
                }
            }
            Err(error) => {
                eprintln!("\n✗ upgrade failed: {error}");
                eprintln!("get the latest from https://github.com/A3S-Lab/Cli/releases/latest\n");
            }
        }
        return Ok(());
    }

    // Session is auto-saved under this directory; show how to come back.
    print!(
        "{}",
        render_resume_hint(&session_id, stdout_color_enabled(context))
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_code_core::workspace::{
        WorkspaceFileSystem, WorkspaceGrepRequest, WorkspacePath, WorkspacePathResolver,
        WorkspaceSearch,
    };
    use a3s_tui::style::strip_ansi;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const TEST_MANIFEST_READY_TIMEOUT: Duration = Duration::from_secs(30);

    #[test]
    fn zero_sized_terminal_uses_a_visible_first_frame() {
        assert_eq!(usable_terminal_size(Some((0, 0))), (80, 24));
        assert_eq!(usable_terminal_size(Some((120, 0))), (80, 24));
        assert_eq!(usable_terminal_size(None), (80, 24));
        assert_eq!(usable_terminal_size(Some((120, 40))), (120, 40));
    }

    #[tokio::test]
    async fn tui_manifest_discovery_is_dormant_until_post_frame_activation() {
        let workspace = tempfile::tempdir().expect("temporary workspace");
        std::fs::write(workspace.path().join("main.rs"), "fn main() {}\n")
            .expect("workspace fixture");
        let backend = tui_manifest_backend(workspace.path());
        let manifest = backend.manifest();
        let mut snapshots = manifest.subscribe();

        assert!(!manifest.is_active());
        assert_eq!(manifest.snapshot().version, 0);
        tokio::task::yield_now().await;
        assert!(snapshots.try_recv().is_err());

        assert!(manifest.activate());
        let ready = tokio::time::timeout(TEST_MANIFEST_READY_TIMEOUT, snapshots.recv())
            .await
            .expect("manifest activation timeout")
            .expect("manifest snapshot stream");
        assert!(ready.files.iter().any(|file| file.path == "main.rs"));
        manifest.shutdown();
    }

    struct ExplicitResumeStore {
        list_calls: AtomicUsize,
        load_calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl a3s_code_core::store::SessionStore for ExplicitResumeStore {
        async fn save(&self, _session: &a3s_code_core::store::SessionData) -> anyhow::Result<()> {
            unreachable!("selection must not save a session")
        }

        async fn load(
            &self,
            _id: &str,
        ) -> anyhow::Result<Option<a3s_code_core::store::SessionData>> {
            self.load_calls.fetch_add(1, Ordering::Relaxed);
            Ok(None)
        }

        async fn delete(&self, _id: &str) -> anyhow::Result<()> {
            unreachable!("selection must not delete a session")
        }

        async fn list(&self) -> anyhow::Result<Vec<String>> {
            self.list_calls.fetch_add(1, Ordering::Relaxed);
            Ok(Vec::new())
        }

        async fn exists(&self, id: &str) -> anyhow::Result<bool> {
            Ok(id == "selected-session")
        }
    }

    #[test]
    fn tui_core_lane_queue_defaults_are_enabled() {
        let mut config = CodeConfig::default();

        ensure_tui_lane_queue(&mut config);

        let queue = config.queue.expect("TUI should enable the Core lane queue");
        assert_eq!(queue.control_max_concurrency, 2);
        assert_eq!(queue.query_max_concurrency, 4);
        assert_eq!(queue.execute_max_concurrency, 2);
        assert_eq!(queue.generate_max_concurrency, 1);
        assert!(queue.retry_policy.is_none());
    }

    #[test]
    fn tui_core_lane_queue_preserves_explicit_configuration() {
        let mut config = CodeConfig {
            queue: Some(a3s_code_core::queue::SessionQueueConfig {
                query_max_concurrency: 9,
                pressure_threshold: Some(17),
                ..Default::default()
            }),
            ..Default::default()
        };

        ensure_tui_lane_queue(&mut config);

        let queue = config.queue.unwrap();
        assert_eq!(queue.query_max_concurrency, 9);
        assert_eq!(queue.pressure_threshold, Some(17));
    }

    #[test]
    fn resume_hint_highlights_the_complete_command_when_color_is_enabled() {
        let rendered = render_resume_hint("session-42", true);

        assert!(rendered.contains("\x1b["));
        assert!(strip_ansi(&rendered).contains("a3s code resume session-42"));
    }

    #[test]
    fn resume_hint_is_plain_when_color_is_disabled() {
        let rendered = render_resume_hint("session-42", false);

        assert!(!rendered.contains("\x1b["));
        assert!(rendered.contains("a3s code resume session-42"));
    }

    #[tokio::test]
    async fn initial_tui_options_inject_and_remove_materialized_preferences() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        let evolution = crate::evolution::WorkspaceEvolution::new(&workspace);
        let item = a3s_memory::MemoryItem::new(
            "Keep completion claims concise and backed by current evidence.",
        )
        .with_type(a3s_memory::MemoryType::Semantic)
        .with_importance(0.94)
        .with_metadata("source", "preference")
        .with_metadata("scope", "workspace")
        .with_metadata("workspace", workspace.display().to_string())
        .with_metadata("session_id", "session-one")
        .with_metadata("confidence", "0.97")
        .with_metadata("evolution_schema", "a3s.evolution.signal.v1")
        .with_metadata("evolution_kind", "preference")
        .with_metadata("evolution_pattern", "preference.response.concise-evidence")
        .with_metadata("evolution_title", "Concise evidence-backed completion")
        .with_metadata(
            "evolution_summary",
            "Keep completion claims concise while retaining current supporting evidence.",
        )
        .with_metadata(
            "evolution_instructions",
            r#"["Keep completion claims concise.","Retain concrete current evidence."]"#,
        );
        let observation = a3s_code_core::memory::MemoryObservation {
            incoming: item.clone(),
            stored: item,
            merged: false,
        };
        evolution.observe(observation).await.unwrap();
        let candidate_id = evolution.overview().await.unwrap().candidates[0].id.clone();
        evolution.materialize(&candidate_id, false).await.unwrap();

        let learned = evolution.session_preference_prompt().unwrap().unwrap();
        let options = with_tui_prompt_context(
            SessionOptions::new(),
            None,
            None,
            false,
            Some(&learned),
            None,
        );
        let extra = options
            .prompt_slots
            .as_ref()
            .and_then(|slots| slots.extra.as_deref())
            .unwrap();
        assert!(extra.contains("# Learned Local Preferences"));
        assert!(extra.contains("Keep completion claims concise."));
        assert!(!extra.contains("Keep completion claims concise and backed"));

        evolution.rollback(&candidate_id, Some(0)).await.unwrap();
        let options = with_tui_prompt_context(
            SessionOptions::new(),
            None,
            None,
            false,
            evolution.session_preference_prompt().unwrap().as_deref(),
            None,
        );
        assert!(options.prompt_slots.is_none());
    }

    #[test]
    fn initial_tui_options_keep_effort_guidance_without_extra_context() {
        let options = with_tui_prompt_context(
            SessionOptions::new(),
            None,
            None,
            false,
            None,
            Some("Use deliberate verification depth."),
        );
        let slots = options.prompt_slots.as_ref().unwrap();

        assert!(slots.extra.is_none());
        assert_eq!(
            slots.guidelines.as_deref(),
            Some("Use deliberate verification depth.")
        );
    }

    #[test]
    fn saved_sessions_are_sorted_newest_first_with_a_stable_tie_breaker() {
        let mut saved = vec![
            ("older".to_string(), 10),
            ("same-a".to_string(), 20),
            ("newest".to_string(), 30),
            ("same-b".to_string(), 20),
        ];

        sort_saved_sessions_by_recency(&mut saved);

        assert_eq!(
            saved.into_iter().map(|(id, _)| id).collect::<Vec<_>>(),
            ["newest", "same-b", "same-a", "older"]
        );
    }

    #[tokio::test]
    async fn explicit_resume_does_not_load_unrelated_sessions() {
        let store = ExplicitResumeStore {
            list_calls: AtomicUsize::new(0),
            load_calls: AtomicUsize::new(0),
        };

        let selected = select_resume_session(&store, Some("selected-session"))
            .await
            .unwrap();

        assert_eq!(
            selected,
            ResumeSessionSelection::Selected("selected-session".to_string())
        );
        assert_eq!(store.list_calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.load_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn tui_workspace_backend_enforces_the_direct_credential_boundary() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join(".env"), "TUI_BOUNDARY_TOKEN=secret\n").unwrap();
        std::fs::write(
            workspace.path().join("README.md"),
            "TUI_BOUNDARY_TOKEN is supplied externally\n",
        )
        .unwrap();
        let backend = tui_manifest_backend(workspace.path());

        let secret = backend.normalize(".env").unwrap();
        let read_error = backend
            .read_text(&secret)
            .await
            .expect_err("the TUI backend must deny direct credential reads");
        assert!(read_error.to_string().contains("credential boundary"));

        let manifest = backend.manifest();
        let mut snapshots = manifest.subscribe();
        assert!(manifest.activate());
        tokio::time::timeout(TEST_MANIFEST_READY_TIMEOUT, snapshots.recv())
            .await
            .unwrap()
            .unwrap();
        let grep = backend
            .grep(WorkspaceGrepRequest {
                base: WorkspacePath::root(),
                pattern: "TUI_BOUNDARY_TOKEN".to_string(),
                glob: None,
                context_lines: 0,
                case_insensitive: false,
                max_output_size: 1024,
            })
            .await
            .unwrap();

        assert_eq!(grep.match_count, 1);
        assert!(grep.output.contains("README.md"));
        assert!(!grep.output.contains("secret"));
        assert!(!grep.output.contains(".env"));
        manifest.shutdown();
    }

    #[test]
    fn legacy_session_config_model_beats_an_unrelated_global_choice() {
        let configured = vec!["openai/session-model".to_string()];
        let preference =
            configured_model_preference(Some("openai/session-model".to_string()), &configured)
                .expect("configured session model");

        assert_eq!(preference.source, ModelSelectionSource::Config);
        assert_eq!(preference.model, "openai/session-model");
        assert!(
            configured_model_preference(Some("codex/other".to_string()), &configured).is_none()
        );
    }

    #[test]
    fn legacy_account_preference_must_match_the_sessions_persisted_model() {
        let preference = ModelSelectionPreference {
            source: ModelSelectionSource::Codex,
            model: "gpt-session".to_string(),
        };

        assert!(preference_matches_persisted_model(
            &preference,
            "gpt-session"
        ));
        assert!(!preference_matches_persisted_model(
            &preference,
            "gpt-another-session"
        ));
    }

    #[tokio::test]
    async fn code_use_resolution_installs_once_when_the_component_is_missing() {
        let installed = PathBuf::from("/managed/a3s-use");
        let called = AtomicBool::new(false);

        let resolution = crate::code_use_host::resolve_with(
            true,
            false,
            || Ok(None),
            || async {
                called.store(true, Ordering::SeqCst);
                Ok(installed.clone())
            },
        )
        .await;

        assert!(called.load(Ordering::SeqCst));
        assert_eq!(resolution.executable.as_deref(), Some(installed.as_path()));
        assert!(resolution.warning.is_none());
    }

    #[tokio::test]
    async fn aborted_code_use_setup_settles_the_shared_slot() {
        let registry = crate::use_registry::UseRegistrySlot::preparing();
        let cancellation = CancellationToken::new();
        let setup = CodeUseSetupGuard::new(registry.clone(), cancellation.clone());

        drop(setup);

        tokio::time::timeout(Duration::from_secs(1), registry.wait_until_settled())
            .await
            .expect("an aborted setup must wake /use repair");
        assert!(cancellation.is_cancelled());
        assert!(registry.ready_handle().is_none());
    }

    #[tokio::test]
    async fn code_use_resolution_honors_the_no_auto_install_boundary() {
        let called = AtomicBool::new(false);

        let resolution = crate::code_use_host::resolve_with(
            false,
            false,
            || Ok(None),
            || async {
                called.store(true, Ordering::SeqCst);
                anyhow::bail!("installer must not run")
            },
        )
        .await;

        assert!(!called.load(Ordering::SeqCst));
        assert!(resolution.executable.is_none());
        assert!(resolution
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("A3S_NO_AUTO_INSTALL")));
    }

    #[tokio::test]
    async fn code_use_resolution_keeps_install_failure_non_fatal_and_actionable() {
        let resolution = crate::code_use_host::resolve_with(
            true,
            false,
            || Ok(None),
            || async { anyhow::bail!("release unavailable") },
        )
        .await;

        assert!(resolution.executable.is_none());
        let warning = resolution.warning.unwrap();
        assert!(warning.contains("release unavailable"), "{warning}");
        assert!(warning.contains("/use repair"), "{warning}");
    }
}
