//! TUI session construction, resume, and terminal launch flow.

use super::*;
use crate::cli::context::InvocationContext;

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

/// Launch Code using the directory, configuration, and platform paths resolved
/// once at the typed CLI boundary. This function never changes process CWD.
pub(crate) async fn run_in(
    args: Vec<String>,
    workspace: &Path,
    context: &InvocationContext,
) -> anyhow::Result<()> {
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
            .ok_or_else(|| anyhow::anyhow!("no HOME directory found for ~/.a3s/config.acl"))?;
        write_template_config(&path)
            .map_err(|error| anyhow::anyhow!("failed to write starter config {path:?}: {error}"))?;
        true
    } else {
        false
    };
    let runtime_configuration =
        crate::commands::config::resolve_code_runtime_configuration(context)?;
    let config_path = runtime_configuration.config_path;
    let code_config = runtime_configuration.config;
    let asset_directories = runtime_configuration.asset_directories;
    let memory_dir = runtime_configuration.memory_dir;
    let agent = Arc::new(
        Agent::from_config(code_config.clone())
            .await
            .map_err(|error| anyhow::anyhow!("failed to load effective agent config: {error}"))?,
    );
    let workspace = workspace.to_string_lossy().into_owned();

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

    // Persistent, resumable session: stored under <cwd>/.a3s/tui/sessions and
    let tui_dir = std::path::Path::new(&workspace).join(".a3s/tui");
    let mut store_dir = tui_dir.join("sessions");
    let legacy_store_dir = std::path::Path::new(&workspace).join(".a3s/tui-sessions");
    if !store_dir.exists() && legacy_store_dir.exists() {
        // Same-filesystem rename preserves all session IDs atomically. If it
        // fails, keep using the legacy store so existing history remains visible.
        let _ = std::fs::create_dir_all(&tui_dir);
        if std::fs::rename(&legacy_store_dir, &store_dir).is_err() {
            store_dir = legacy_store_dir;
        }
    }
    // keyed by a fixed id, so relaunching in the same directory continues the
    // conversation. Falls back to a fresh session when none exists yet.

    // Resolve `resume`: verify the id exists (else show what's available), or
    // pick the most recent session when no id was given.
    if resuming {
        let mut saved: Vec<(String, std::time::SystemTime)> = std::fs::read_dir(&store_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("json") {
                    return None;
                }
                let id = p.file_stem()?.to_str()?.to_string();
                let mtime = e.metadata().ok()?.modified().ok()?;
                Some((id, mtime))
            })
            .collect();
        saved.sort_by_key(|e| std::cmp::Reverse(e.1)); // newest first
        match &explicit_id {
            Some(id) if !saved.iter().any(|(s, _)| s == id) => {
                eprintln!("a3s: session '{id}' not found in {}", store_dir.display());
                if saved.is_empty() {
                    eprintln!("  (no saved sessions in this directory)");
                } else {
                    eprintln!("  available sessions (newest first):");
                    for (s, _) in saved.iter().take(10) {
                        eprintln!("    a3s code resume {s}");
                    }
                }
                return Ok(());
            }
            None => match saved.first() {
                Some((s, _)) => session_id = s.clone(),
                None => {
                    eprintln!(
                        "a3s: no saved sessions to resume in {}",
                        store_dir.display()
                    );
                    return Ok(());
                }
            },
            _ => {}
        }
    }

    let store: Arc<dyn a3s_code_core::store::SessionStore> = Arc::new(
        a3s_code_core::store::FileSessionStore::new(&store_dir)
            .await
            .map_err(|e| anyhow::anyhow!("failed to open session store {store_dir:?}: {e}"))?,
    );
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
    let restored_model_selection =
        restore_model_selection(&models, os_session.as_ref(), session_id.as_str());
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
    let initial_effort = load_tui_effort_preference().unwrap_or(DEFAULT_TUI_EFFORT_INDEX);
    let initial_budget = budget_plan_for_effort_index(
        initial_effort,
        Some(context_limit),
        BudgetWorkload::Interactive,
    );
    let initial_auto_delegation = effort_uses_automatic_delegation(initial_effort);
    let deep_research_report_tool_gate = DeepResearchReportToolGate::default();
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
        let mut parts: Vec<String> = Vec::new();
        if let Some(i) = &instructions {
            parts.push(i.clone());
        }
        if let Some(addr) = &os_address {
            parts.push(os_platform_guide(addr));
        }
        if ctx_ready {
            parts.push(panels::ctx::ctx_history_guide());
        }
        if parts.is_empty() {
            o
        } else {
            o.with_prompt_slots(SystemPromptSlots::default().with_extra(parts.join("\n\n")))
        }
    };
    let manifest_backend = ManifestWorkspaceBackend::new(std::path::PathBuf::from(&workspace));
    let workspace_manifest = manifest_backend.manifest();
    let initial_manifest = workspace_manifest.snapshot();
    let initial_files = initial_manifest.file_paths();
    let workspace_manifest_rx = Arc::new(Mutex::new(workspace_manifest.subscribe()));
    let workspace_services = WorkspaceServices::local_with_manifest_backend(manifest_backend);
    let auto_compact_threshold = auto_compact_threshold_for_path(&config_path);
    let session = match agent
        .resume_session_async(
            session_id.as_str(),
            apply_launch_model_options(
                with_instr(with_recent_workspace_context(
                    tui_session_options_with_gate(
                        confirmation.clone(),
                        deep_research_report_tool_gate.clone(),
                    )
                    .with_session_store(store.clone())
                    .with_workspace_backend(workspace_services.clone())
                    .with_skill_dirs(claude_dirs.clone())
                    .with_auto_save(true)
                    .with_auto_compact(true)
                    .with_max_context_tokens(context_limit as usize)
                    .with_auto_compact_threshold(auto_compact_threshold as f32)
                    .with_file_memory(memory_dir.clone())
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
            ),
        )
        .await
    {
        Ok(s) => s,
        Err(error) if resuming => {
            return Err(anyhow::anyhow!(
                "failed to resume session {session_id}; refusing to replace its persisted history with an empty session: {error}"
            ));
        }
        Err(_) => {
            agent
                .session_async(
                    workspace.clone(),
                    Some(apply_launch_model_options(
                        with_instr(with_recent_workspace_context(
                            tui_session_options_with_gate(
                                confirmation.clone(),
                                deep_research_report_tool_gate.clone(),
                            )
                            .with_session_store(store.clone())
                            .with_session_id(session_id.as_str())
                            .with_workspace_backend(workspace_services.clone())
                            .with_skill_dirs(claude_dirs.clone())
                            .with_auto_save(true)
                            .with_auto_compact(true)
                            .with_max_context_tokens(context_limit as usize)
                            .with_auto_compact_threshold(auto_compact_threshold as f32)
                            .with_file_memory(memory_dir.clone())
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
                    )),
                )
                .await?
        }
    };
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

    let (width, height) = a3s_tui::terminal::Terminal::size().unwrap_or((80, 24));

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

    // Headless smoke mode: exercise the agent-stream integration (the hard part
    // the TUI depends on) without taking over the terminal. Useful for CI/probes
    // and for validating a model/config end-to-end.
    if std::env::var_os("A3S_CODE_TUI_SMOKE").is_some() {
        return run_smoke(
            session,
            Path::new(&workspace),
            os_session.is_some(),
            deep_research_report_tool_gate,
        )
        .await;
    }

    let running_tracker_children = session
        .pending_subagent_tasks()
        .await
        .into_iter()
        .map(|snapshot| snapshot.task_id)
        .collect::<HashSet<_>>();
    let interrupted_research_recovery =
        reconcile_interrupted_latest_run(Path::new(&workspace), &running_tracker_children).await;
    if let Ok(Some(recovery)) = interrupted_research_recovery.as_ref() {
        for task_id in &recovery.cancel_children {
            let _ = session.cancel_subagent_task(task_id).await;
        }
    }

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

    remote_ui::prime_webview_lookup();

    let mut app = App {
        session,
        active_session: Arc::clone(&active_session),
        agent: agent.clone(),
        store: store.clone(),
        confirmation,
        deep_research_report_tool_gate,
        session_id: session_id.clone(),
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
        codex_account_models: crate::account_providers::codex::cached_codex_models(),
        codex_models_loading: false,
        codex_models_refreshed_at: None,
        account_models: HashMap::new(),
        account_models_loading: HashSet::new(),
        account_model_errors: HashMap::new(),
        llm_override: launch_llm_override,
        code_config: Arc::new(code_config),
        asset_directories,
        config_path: config_path.clone(),
        memory_dir,
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
        deep_research_report_repair_used: false,
        deep_research_workflow: DeepResearchWorkflowSnapshot::default(),
        deep_research_outcome: DeepResearchRunOutcome::Active,
        pending_deep_research_report_repair_prompt: None,
        deep_research_stream_timeout_token: 0,
        stream_start_token: 0,
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
        selection: None,
        last_workflow: None,
        pending_images: Vec::new(),
        goal: None,
        goal_since: None,
        goal_run: None,
        goal_generation: 0,
        pending_goal_failure: None,
        deep_research_goal_restore: None,
        loop_remaining: 0,
        runtime: RuntimeProjection::default(),
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
        workspace_manifest,
        workspace_manifest_rx,
        workspace_services,
        gradient_until: None,
        gradient_frame: 0,
        ultracode_animation_epoch: 0,
        effort_anim: None,
        transcript_view: None,
        viewport: Viewport::new(width, height.saturating_sub(7)),
        textarea: Textarea::new()
            .with_height(1)
            .with_auto_grow(8) // box grows with Shift+Enter newlines (no scroll)
            .with_width(textarea_width_for(width)) // prompt prefix is outside the textarea
            .with_submit_on_enter(true),
        spinner: Spinner::new().with_title(""),
        streaming: StreamingMarkdown::new(transcript_markdown_width_for(width)),
        deep_research_report_tools: ReportPhaseToolBuffer::default(),
        got_delta: false,
        compacting: None,
        updating: None,
        last_paint: None,
        thinking: String::new(),
        state: State::Idle,
        messages: Transcript::from_entries(initial_messages),
        rx: None,
        stream_join: None,
        stream_join_settling: false,
        host_tool_abort: None,
        host_progress_inflight: false,
        host_tool_call_id: None,
        interrupting: false,
        pending_tools: VecDeque::new(),
        approval_sel: 0,
        history: history_seed,
        history_pos: None,
        history_draft: None,
        model: launch_model,
        output_tokens: 0,
        stream_started: None,
        blink_tick: 0,
        anim: 0,
        mode: Mode::Default,
        queue: BinaryHeap::new(),
        seq: 0,
        running_task: None,
        plan: PlanProjection::default(),
        ide: None,
        memory: None,
        asset_list: None,
        runtime_activity: None,
        kb: None,
        loop_panel: None,
        help_open: false,
        help_scroll: 0,
        completed: 0,
        branch: git_branch(&workspace),
        slash_sel: 0,
        slash_menu_dismissed_for: None,
        files: initial_files,
        at_expanded: std::collections::HashSet::new(),
        file_sel: 0,
        skill_count: count_skill_files(&claude_dirs),
        skills: load_skills(&claude_dirs),
        disabled_skills: load_disabled_skills(),
        plugins_panel: None,
        update_available: None,
        cwd: workspace.clone(),
        width,
        height,
        keymap,
    };

    match interrupted_research_recovery {
        Ok(Some(recovery)) => {
            app.messages.push(TranscriptEntry::preformatted(gutter(
                TN_YELLOW,
                &format!(
                    "⚠ recovered interrupted DeepResearch run {} · cancelled {} live child{} · reconciled {} orphan{}",
                    recovery.run_id,
                    recovery.cancel_children.len(),
                    if recovery.cancel_children.len() == 1 { "" } else { "ren" },
                    recovery.orphaned_children.len(),
                    if recovery.orphaned_children.len() == 1 { "" } else { "s" },
                ),
            )));
            app.rebuild_viewport();
        }
        Ok(None) => {}
        Err(error) => {
            app.messages.push(TranscriptEntry::preformatted(gutter(
                TN_YELLOW,
                &format!("⚠ DeepResearch recovery audit failed: {error}"),
            )));
            app.rebuild_viewport();
        }
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

    // Apply the complete current profile (default `high`) before the first turn.
    // The launch session already has host budgets and a native Codex effort, but
    // effort_session_opts also applies provider-appropriate prompt guidance and
    // ultracode orchestration. Best-effort: keep the launch session if it cannot
    // rebuild. (Resumes the same id, so transcript history is preserved.)
    let with_thinking = app.effort_session_opts(true);
    let without_thinking = app.effort_session_opts(false);
    if let Ok((s, _)) = panels::model::rebuild_agent_session(
        Arc::clone(&app.agent),
        app.cwd.clone(),
        app.session_id.clone(),
        with_thinking,
        without_thinking,
        SessionRebuildMode::ResumeExisting,
    )
    .await
    {
        app.replace_session(s);
    }

    ProgramBuilder::new(app)
        .with_alt_screen()
        // Capture mouse input so wheel/trackpad scrolling works in the alternate
        // screen. Drag-copy is app-owned: on release we write the selected text to
        // the clipboard, so scroll and copy can coexist.
        .with_mouse_support()
        .with_fps(120)
        .run()
        .await?;

    let final_session = active_session
        .lock()
        .map(|session| Arc::clone(&session))
        .map_err(|_| anyhow::anyhow!("active session lock was poisoned"))?;
    let session_id = final_session.session_id().to_string();
    if let Err(error) = final_session.save().await {
        eprintln!("⚠  could not save session {session_id}: {error}");
    }

    // `/update` found a newer version → upgrade via Homebrew in the (now
    // restored) shell so brew's own download progress shows, then re-exec the
    // freshly-installed binary. Use PATH `a3s` (brew repointed its symlink to
    // the new version); current_exe() is the OLD version's path.
    if UPGRADE_ON_EXIT.load(std::sync::atomic::Ordering::Relaxed) {
        let latest = LATEST
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_default();
        match crate::update::perform_upgrade(&latest) {
            Ok(bin) => {
                let restart_args = ["code", "resume", session_id.as_str()];
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
                        "✓ updated to a3s {latest}; resume manually with: a3s code resume {session_id}\n"
                    );
                }
                #[cfg(not(unix))]
                {
                    match std::process::Command::new(&bin).args(restart_args).status() {
                        Ok(status) if status.success() => {}
                        Ok(status) => eprintln!(
                            "\n⚠  updated, but restart exited with status {status}; resume manually with: a3s code resume {session_id}\n"
                        ),
                        Err(err) => eprintln!(
                            "\n⚠  updated, but restart failed: {err}; resume manually with: a3s code resume {session_id}\n"
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
    println!("\n  session saved · resume it with:  a3s code resume {session_id}\n");
    Ok(())
}
