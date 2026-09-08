//! Prompt submission, slash-command dispatch, and attachment handling.

use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

static DIRECT_SHELL_CALL_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn direct_shell_tool_args(command: &str) -> serde_json::Value {
    serde_json::json!({
        "command": command,
        // Preserve the TUI's long-running command budget while still routing
        // the actual deadline, process-tree cancellation, output budget, and
        // cross-platform shell choice through Core's `bash` implementation.
        "timeout": TOOL_EXEC_TIMEOUT_MS,
    })
}

pub(super) fn next_direct_shell_call_id() -> String {
    let sequence = DIRECT_SHELL_CALL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("host-bash-{}-{sequence}", std::process::id())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubmissionIntent {
    Queue,
    SendNow,
}

pub(super) fn parse_use_status_command(rest: &str) -> Result<bool, &'static str> {
    match parse_use_hub_command(rest)? {
        UseHubCommand::Status { repair } => Ok(repair),
        _ => Err(USE_HUB_USAGE),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UseHubCommand {
    Status { repair: bool },
    Plugin,
    Packages,
    Reload,
}

pub(super) const USE_HUB_USAGE: &str = "usage: /use [status|repair|plugin|packages|reload]";

pub(super) fn parse_use_hub_command(rest: &str) -> Result<UseHubCommand, &'static str> {
    match rest.trim() {
        "" | "status" => Ok(UseHubCommand::Status { repair: false }),
        "repair" => Ok(UseHubCommand::Status { repair: true }),
        "plugin" | "plugins" => Ok(UseHubCommand::Plugin),
        "package" | "packages" => Ok(UseHubCommand::Packages),
        "reload" => Ok(UseHubCommand::Reload),
        _ => Err(USE_HUB_USAGE),
    }
}

pub(super) fn expand_skill_mentions(
    prompt: &str,
    skills: &[(String, String)],
    disabled_skills: &std::collections::HashSet<String>,
    sticky_skill: Option<&str>,
) -> String {
    let available = skills
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| !disabled_skills.contains(*name))
        .collect::<std::collections::HashSet<_>>();
    let mut selected = Vec::new();
    // Sticky is an explicit session attach (Alt/Option+Enter). It outranks the
    // disabled-skill set for this session so the footer chip stays truthful.
    if let Some(name) = sticky_skill {
        if !name.is_empty() {
            selected.push(name);
        }
    }
    for token in prompt.split_whitespace() {
        let token = token.trim_start_matches(&['(', '[', '{', '"', '\'', '（', '【', '“'][..]);
        let Some(name) = token.strip_prefix('$') else {
            continue;
        };
        let name = name.trim_end_matches(
            &[
                '.', ',', '!', '?', ';', ':', ')', ']', '}', '"', '\'', '。', '，', '！', '？',
                '；', '：', '）', '】', '”',
            ][..],
        );
        if !name.is_empty() && available.contains(name) && !selected.contains(&name) {
            selected.push(name);
        }
    }
    if selected.is_empty() {
        return prompt.to_string();
    }
    format!(
        "[Selected skills]\n{}\n[/Selected skills]\n\n{prompt}",
        selected
            .iter()
            .map(|name| format!("- Use your `{name}` skill."))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// Alt (Option on macOS) or Meta+Enter attaches a sticky skill; plain Enter is one-shot.
pub(crate) fn skill_enter_attaches_sticky(modifiers: KeyModifiers) -> bool {
    modifiers.contains(KeyModifiers::ALT) || modifiers.contains(KeyModifiers::META)
}

pub(crate) fn sticky_skill_name_from_mention(cmd: &str) -> Option<&str> {
    cmd.strip_prefix('$').filter(|name| !name.is_empty())
}

/// After sticky Alt/Option+Enter, the composer must be empty so Esc can clear
/// sticky immediately (Cursor custom-mode exit). Plain Enter keeps the mention.
pub(crate) fn composer_value_after_skill_menu_enter(
    completed_mention: &str,
    sticky: bool,
) -> String {
    if sticky {
        String::new()
    } else {
        completed_mention.to_string()
    }
}

/// Esc clears sticky skill only when Idle, composer empty, and no slash menu.
pub(crate) fn should_clear_sticky_on_esc(
    key: &KeyEvent,
    idle: bool,
    composer_empty: bool,
    has_sticky: bool,
    slash_menu_open: bool,
) -> bool {
    key.code == KeyCode::Esc && idle && composer_empty && has_sticky && !slash_menu_open
}

impl App {
    pub(super) fn on_submit(&mut self, text: String) -> Option<Cmd<Msg>> {
        self.on_submit_with_intent(text, SubmissionIntent::Queue)
    }

    pub(super) fn on_submit_now(&mut self, text: String) -> Option<Cmd<Msg>> {
        let trimmed = text.trim();
        if self.shell_mode
            || self.research_mode
            || matches!(trimmed.chars().next(), Some('/' | '!' | '?'))
        {
            self.textarea.set_value(&text);
            self.push_notice(
                NoticeKind::Warning,
                "Send now accepts an agent prompt, not a shell, research, or slash command",
            );
            return None;
        }
        self.on_submit_with_intent(text, SubmissionIntent::SendNow)
    }

    /// Busy-path submissions must not append a user bubble while Thought /
    /// interrupt markers from the active turn are still unsettled.
    fn should_defer_user_transcript(&self) -> bool {
        matches!(
            self.state,
            State::Streaming | State::Awaiting | State::Rebuilding
        ) || self.interrupting
            || self.host_progress_inflight
            || self.interrupted_stream_start_token.is_some()
    }

    fn on_submit_with_intent(
        &mut self,
        text: String,
        intent: SubmissionIntent,
    ) -> Option<Cmd<Msg>> {
        let trimmed = text.trim();
        if trimmed.is_empty() && self.pending_images.is_empty() && self.pending_pastes.is_empty() {
            return None;
        }
        // No input while compacting or upgrading.
        if self.compacting.is_some() || self.updating.is_some() {
            self.textarea.clear();
            return None;
        }
        // `/checkup` owns a short host-side inspection before it admits the
        // strict Plan turn. Keep any asynchronously-routed prompt as a draft
        // instead of letting an ordinary turn race that immutable snapshot.
        if self.checkup_inflight {
            self.textarea.set_value(&text);
            return None;
        }
        if self.session_rebuild_pending.is_some() {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  wait for the current session change to finish"),
            );
            return None;
        }
        // Shell mode (`!`) is explicit host intent, so it bypasses the model but
        // still uses Core's host-direct tool runtime. That keeps workspace
        // binding, Windows shell selection, process-tree cancellation, output
        // limits, timeouts, hooks, budgets, and typed failures aligned with the
        // same `bash` implementation used by agent turns.
        if self.shell_mode {
            self.shell_mode = false;
            let command = trimmed.trim_start_matches('!').trim().to_string();
            if command.is_empty() {
                return None;
            }
            self.history.push(format!("! {command}"));
            self.history_pos = None;
            self.history_draft = None;
            self.textarea.clear();

            let call_id = next_direct_shell_call_id();
            let args = direct_shell_tool_args(&command);
            self.messages.start_tool_execution(
                call_id.clone(),
                "bash".to_string(),
                args.clone(),
                true,
            );
            self.runtime
                .start_execution(call_id.clone(), "bash".to_string(), args.clone());
            self.rebuild_viewport();

            let session = Arc::clone(&self.session);
            return Some(cmd::cmd(move || async move {
                let result = session
                    .tool("bash", args.clone())
                    .await
                    .map_err(|error| error.to_string());
                Msg::ShellOutput {
                    call_id,
                    args,
                    result,
                }
            }));
        }
        // Deep research: `/research <query>` is the primary entry. Leading `?`
        // (and legacy sticky research mode) remain as shortcuts and tip the hub.
        if self.research_mode || trimmed.starts_with('?') {
            self.research_mode = false;
            let raw_query = trimmed.trim_start_matches('?').trim();
            return self.start_deep_research(raw_query, true);
        }
        // `/goal clear` is intentionally available during a running goal. It
        // invalidates delayed retries immediately, then cancels and joins the
        // active stream before restoring normal Ultracode planning.
        if trimmed == "/goal clear" {
            return self.clear_goal_command();
        }
        // Block session-mutating commands while a turn is streaming.
        if self.state != State::Idle {
            let cmd0 = trimmed.split_whitespace().next().unwrap_or("");
            if IDLE_ONLY.contains(&cmd0) {
                self.textarea.clear();
                self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                    "  {cmd0} is unavailable while a turn is running — press Esc to stop first"
                )));
                return None;
            }
        }
        if let Some(rest) = slash_tail(trimmed, "/use") {
            match parse_use_hub_command(rest) {
                Ok(UseHubCommand::Status {
                    repair: include_repair_guidance,
                }) => {
                    self.textarea.clear();
                    let status_entry = self.push_tracked_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render("  inspecting A3S Use capabilities…"),
                    );
                    let registry = self.use_registry.clone();
                    let session = Arc::clone(&self.session);
                    return Some(cmd::cmd(move || async move {
                        if include_repair_guidance {
                            registry.wait_until_settled().await;
                        }
                        let text = registry.status_text(session, include_repair_guidance).await;
                        Msg::UseStatus { status_entry, text }
                    }));
                }
                Ok(UseHubCommand::Plugin) => {
                    self.textarea.clear();
                    return self.open_plugins_panel(false);
                }
                Ok(UseHubCommand::Packages) => {
                    if self.state != State::Idle {
                        self.textarea.clear();
                        self.push_line(&Style::new().fg(TN_YELLOW).render(
                            "  /use packages is unavailable while a turn is running — press Esc to stop first",
                        ));
                        return None;
                    }
                    self.textarea.clear();
                    return self.open_package_panel();
                }
                Ok(UseHubCommand::Reload) => {
                    if self.state != State::Idle {
                        self.textarea.clear();
                        self.push_line(&Style::new().fg(TN_YELLOW).render(
                            "  /use reload is unavailable while a turn is running — press Esc to stop first",
                        ));
                        return None;
                    }
                    self.textarea.clear();
                    return self.reload_skills_and_plugins();
                }
                Err(usage) => {
                    self.textarea.clear();
                    self.push_line(&Style::new().fg(TN_YELLOW).render(&format!("  {usage}")));
                    return None;
                }
            }
        }
        if let Some(rest) = slash_tail(trimmed, "/review") {
            self.textarea.clear();
            let target = match panels::workspace_review::parse_workspace_review_target(rest) {
                Ok(target) => target,
                Err(usage) => {
                    self.push_line(&Style::new().fg(TN_YELLOW).render(&format!("  {usage}")));
                    return None;
                }
            };
            let label = format!("code review: {}", target.label());
            let submitted = rest.trim();
            let command = if submitted.is_empty() {
                "/review".to_string()
            } else {
                format!("/review {submitted}")
            };
            self.messages.push(TranscriptEntry::user(command));
            let prompt = panels::workspace_review::workspace_review_prompt(
                std::path::Path::new(&self.cwd),
                &target,
            );
            // Never enqueue onto the main turn queue — reviewer is a side-session.
            return self.spawn_background_reviewer(prompt, label);
        }
        if let Some(rest) = slash_tail(trimmed, "/login") {
            self.textarea.clear();
            let Some(os_config) = self.os_config.clone() else {
                self.push_line(&format!(
                    "{}\n{}\n{}\n{}",
                    Style::new()
                        .fg(TN_YELLOW)
                        .render("  /login needs an OS endpoint, but none is configured."),
                    Style::new().fg(TN_GRAY).render(
                        "  Add it to ~/.a3s/config.acl (or your project's .a3s/config.acl):"
                    ),
                    Style::new()
                        .fg(TN_CYAN)
                        .render("      os = \"https://your-os-host.example.com\""),
                    Style::new()
                        .fg(TN_GRAY)
                        .render("  then restart a3s code and run /login again."),
                ));
                return None;
            };
            let token = rest.trim();
            if !token.is_empty() {
                let token = token.to_string();
                let status_entry =
                    self.push_tracked_line(&Style::new().fg(TN_GRAY).render("  signing in to OS…"));
                return Some(cmd::cmd(move || async move {
                    let result = crate::a3s_os::login_with_token(&os_config, &token)
                        .await
                        .map(|session| session.display_label())
                        .map_err(|error| error.to_string());
                    Msg::OsLogin {
                        status_entry,
                        result,
                    }
                }));
            }

            // Already signed in (restored from a previous run) → no need to
            // re-authenticate; tell the user how to switch instead.
            if let Some(s) = &self.os_session {
                self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                    "  already signed in to OS as {} · /logout to switch accounts",
                    s.display_label()
                )));
                return None;
            }

            let status_entry = self.push_tracked_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render("  opening OS login in your browser…"),
            );
            return Some(cmd::cmd(move || async move {
                let result = crate::a3s_os::login_via_browser(os_config)
                    .await
                    .map(|session| session.display_label())
                    .map_err(|error| error.to_string());
                Msg::OsLogin {
                    status_entry,
                    result,
                }
            }));
        }
        if trimmed == "/logout" {
            self.textarea.clear();
            let Some(os_config) = self.os_config.clone() else {
                self.push_line(&Style::new().fg(TN_YELLOW).render(
                    "  configure `os = \"https://...\"` in .a3s/config.acl to enable /logout",
                ));
                return None;
            };
            match crate::a3s_os::logout(&os_config) {
                Ok(true) => {
                    self.os_session = None;
                    crate::a3s_os::remove_capability_skill_dir();
                    crate::a3s_os::clear_os_env();
                    let rebuild = self.refresh_after_auth();
                    self.push_line(
                        &Style::new()
                            .fg(TN_GREEN)
                            .render("  ✓ signed out from OS · capabilities skill removed"),
                    );
                    return rebuild;
                }
                Ok(false) => {
                    self.os_session = None;
                    crate::a3s_os::remove_capability_skill_dir();
                    crate::a3s_os::clear_os_env();
                    let rebuild = self.refresh_after_auth();
                    self.push_line(&Style::new().fg(TN_GRAY).render("  no OS login was stored"));
                    return rebuild;
                }
                Err(error) => self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  logout failed: {error}")),
                ),
            }
            return None;
        }
        // `/kb` opens the local personal knowledge-base panel. Notes/imports/search are
        // explicit subcommands so a mistyped path no longer becomes a note.
        // `/ctx <query>` searches past agent sessions; `/ctx <n>` stages hit n
        // as context for the next message (ctx CLI, local SQLite index).
        if let Some(rest) = slash_tail(trimmed, "/research") {
            let rest = rest.trim();
            if rest.is_empty() {
                self.textarea.clear();
                // Default diagnostic when no query / action is provided.
                let active_run_id = self
                    .deep_research_workflow
                    .args
                    .as_ref()
                    .and_then(|args| args.get("run_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let workspace = PathBuf::from(&self.cwd);
                return Some(cmd::cmd(move || async move {
                    Msg::ResearchDiagnostic(
                        research_diagnostic(
                            &workspace,
                            active_run_id.as_deref(),
                            ResearchDiagnosticKind::Status,
                        )
                        .await
                        .map_err(|error| error.to_string()),
                    )
                }));
            }
            let mut parts = rest.split_whitespace();
            let action = parts.next().unwrap_or("status");
            if action == "diff" {
                self.textarea.clear();
                let left = parts.next().map(str::to_string);
                let right = parts.next().map(str::to_string);
                if left.is_none() || right.is_none() || parts.next().is_some() {
                    self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render("  usage: /research diff <left-run-id> <right-run-id>"),
                    );
                    return None;
                }
                let (Some(left), Some(right)) = (left, right) else {
                    return None;
                };
                let workspace = PathBuf::from(&self.cwd);
                return Some(cmd::cmd(move || async move {
                    Msg::ResearchDiagnostic(
                        research_diff(&workspace, &left, &right)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                }));
            }
            if matches!(action, "status" | "explain" | "replay") {
                self.textarea.clear();
                let explicit_run_id = parts.next().map(str::to_string);
                if parts.next().is_some() {
                    self.push_line(&Style::new().fg(TN_GRAY).render(
                        "  usage: /research <query> · /research [status|explain|replay] [run-id] · /research diff <left> <right>",
                    ));
                    return None;
                }
                let kind = match action {
                    "status" => ResearchDiagnosticKind::Status,
                    "explain" => ResearchDiagnosticKind::Explain,
                    "replay" => ResearchDiagnosticKind::Replay,
                    _ => unreachable!(),
                };
                let active_run_id = self
                    .deep_research_workflow
                    .args
                    .as_ref()
                    .and_then(|args| args.get("run_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let run_id = explicit_run_id.or(active_run_id);
                let workspace = PathBuf::from(&self.cwd);
                return Some(cmd::cmd(move || async move {
                    Msg::ResearchDiagnostic(
                        research_diagnostic(&workspace, run_id.as_deref(), kind)
                            .await
                            .map_err(|error| error.to_string()),
                    )
                }));
            }
            // Any other tail is a deep-research query (primary entry).
            return self.start_deep_research(rest, false);
        }
        if let Some(rest) = slash_tail(trimmed, "/ctx") {
            return self.handle_ctx_command(rest);
        }
        if let Some(rest) = slash_tail(trimmed, "/kb") {
            self.push_prefer_hub_tip("/ctx kb");
            return self.handle_kb_command(rest);
        }
        // `/goal [text|resume|clear]` — a persistent goal prepended to every prompt.
        if let Some(rest) = slash_tail(trimmed, "/goal") {
            let g = rest.trim();
            self.textarea.clear();
            if g.is_empty() {
                match &self.goal {
                    Some(cur) => self.push_line(&gutter(
                        TN_CYAN,
                        &format!("◎\u{200A}goal: {cur}   (/goal clear to remove)"),
                    )),
                    None => match &self.paused_goal {
                        Some(paused) => self.push_line(&gutter(
                            TN_YELLOW,
                            &format!(
                                "◎\u{200A}goal paused: {}   (/goal resume or /goal clear)",
                                paused.goal
                            ),
                        )),
                        None => self.push_line(
                            &Style::new()
                                .fg(TN_GRAY)
                                .render("  usage: /goal <what you're working toward>"),
                        ),
                    },
                }
            } else if g == "clear" {
                return self.clear_goal_command();
            } else if g == "resume" {
                if self.paused_goal.is_none() {
                    self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render("  no paused goal to resume"),
                    );
                    return None;
                }
                return self.resume_paused_goal();
            } else {
                return self.start_goal_run(g);
            }
            return None;
        }
        // `/loop` — engineered loop dashboard + subcommands; unknown tails keep
        // the quick-loop contract (`/loop <task>`).
        if let Some(rest) = slash_tail(trimmed, "/loop") {
            return self.handle_loop_command(rest);
        }
        // `/sleep [focus]` — end-of-day consolidation: the `/loop` mechanism
        // drives the agent through reviewing today's work (cross-session via
        // `ctx` when installed) until a turn ends with the machine-readable
        // ```a3s-sleep report, which capture_sleep persists into long-term
        // memory (experience · preferences · knowledge). Idle-only.
        if let Some(rest) = slash_tail(trimmed, "/sleep") {
            return self.start_sleep_command(rest, true);
        }
        if let Some(rest) = slash_tail(trimmed, "/fork") {
            return self.submit_fork_command(rest);
        }
        if let Some(rest) = slash_tail(trimmed, "/worktree") {
            return self.submit_worktree_lifecycle_command(rest);
        }
        if let Some(rest) = slash_tail(trimmed, "/hooks") {
            self.textarea.clear();
            match self.hook_executor.manage(rest.trim()) {
                Ok(status) => {
                    for line in status.lines() {
                        self.push_line(&Style::new().fg(TN_GRAY).render(&format!("  {line}")));
                    }
                }
                Err(error) => self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  /hooks: {error:#}")),
                ),
            }
            self.relayout();
            return None;
        }
        if let Some(rest) = slash_tail(trimmed, "/statusline") {
            self.textarea.clear();
            match rest.trim() {
                "clear" | "off" | "none" => {
                    self.status_line_extension = None;
                    self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render("  statusline · decorator cleared"),
                    );
                }
                "" => match self.status_line_extension.as_deref() {
                    Some(extension) => self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                        "  statusline · decorator active · {extension} · /statusline clear"
                    ))),
                    None => self.push_line(&Style::new().fg(TN_GRAY).render(
                        "  statusline · no external decorator · /display for density · /statusline clear",
                    )),
                },
                value => {
                    self.status_line_extension = Some(value.to_string());
                    self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                        "  statusline · decorator set · {value}"
                    )));
                }
            }
            return None;
        }
        if let Some(rest) = slash_tail(trimmed, "/copy") {
            return self.submit_copy_command(rest);
        }
        if let Some(rest) = slash_tail(trimmed, "/export") {
            return self.submit_export_command(rest);
        }
        // Slash commands run inline in any state.
        match trimmed {
            "/exit" => return self.begin_graceful_quit(),
            "/rewind" => return self.submit_rewind_command(),
            "/clear" => {
                self.textarea.clear();
                self.sticky_skill = None;
                self.cancel_goal_state("cleared by /clear");
                self.clear_paused_goal("cleared by /clear");
                self.goal = None;
                self.goal_since = None;
                // Actually reset the conversation, not just the screen: swap in a
                // fresh session (new id, no history, no carried compact summary)
                // and zero the token/ctx counters. All visible state is committed
                // only by SessionRebuilt after construction succeeds, so a failed
                // clear leaves the current transcript and active modes intact.
                let session_id = new_session_id();
                let mut profile = self.session_rebuild_profile();
                profile.session_id = session_id.clone();
                profile.compact_summary = None;
                return self
                    .start_session_rebuild(profile, SessionRebuildAction::Clear { session_id });
            }
            "/unstick" => {
                self.textarea.clear();
                match self.sticky_skill.take() {
                    Some(name) => self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render(&format!("  sticky skill · ${name} cleared")),
                    ),
                    None => self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render("  no sticky skill attached"),
                    ),
                }
                return None;
            }
            "/init" => {
                // Agent-driven: analyze the workspace and write AGENTS.md (auto-loaded
                // by the core, like CLAUDE.md). Guarded idle by IDLE_ONLY above.
                self.textarea.clear();
                self.messages
                    .push(TranscriptEntry::user("/init — generate AGENTS.md"));
                self.rebuild_viewport();
                return self.start_stream(
                    "Analyze this codebase and create (or update) an AGENTS.md file at the \
                     project root. Include: a concise project overview, the exact build / test / \
                     lint / run commands, the high-level architecture and key directories, and \
                     the conventions an AI coding agent should follow. Base everything on what's \
                     actually in the workspace, and write the file with your file-writing tool."
                        .to_string(),
                );
            }
            "/compact" => {
                self.textarea.clear();
                if self.state != State::Idle {
                    self.push_line(
                        &Style::new()
                            .fg(TN_YELLOW)
                            .render("  finish the current turn before compacting"),
                    );
                    return None;
                }
                let history = self.session.history();
                if history.is_empty() {
                    self.push_line(&Style::new().fg(TN_GRAY).render("  nothing to compact yet"));
                    return None;
                }
                let llm_client = match crate::session_llm::resolve_session_llm_client(
                    &self.code_config,
                    &self.effort_session_opts(false),
                    &self.session_id,
                ) {
                    Ok(client) => client,
                    Err(error) => {
                        self.push_line(
                            &Style::new()
                                .fg(TN_RED)
                                .render(&format!("  could not prepare compaction: {error}")),
                        );
                        return None;
                    }
                };
                self.compacting = Some(Instant::now()); // progress bar + input lock
                let previous_summary = self.compact_summary.clone();
                let hook_executor = self.hook_executor.clone();
                let session_id = self.session_id.clone();
                let message_count = history.len();
                let used_tokens = self.last_prompt_tokens;
                let max_tokens = self.context_limit as usize;
                return Some(cmd::cmd(move || async move {
                    let pre_event = a3s_code_core::hooks::HookEvent::PreCompact(
                        a3s_code_core::hooks::PreCompactEvent {
                            session_id: session_id.clone(),
                            message_count,
                            used_tokens,
                            max_tokens,
                        },
                    );
                    match a3s_code_core::hooks::HookExecutor::fire_outcome(
                        hook_executor.as_ref(),
                        &pre_event,
                    )
                    .await
                    {
                        a3s_code_core::hooks::HookOutcome::Block { reason }
                        | a3s_code_core::hooks::HookOutcome::Retry { reason, .. }
                        | a3s_code_core::hooks::HookOutcome::Escalate { reason, .. } => {
                            return Msg::Compacted(Err(format!(
                                "compaction blocked by lifecycle hook: {reason}"
                            )));
                        }
                        a3s_code_core::hooks::HookOutcome::Continue(_)
                        | a3s_code_core::hooks::HookOutcome::Skip => {}
                        _ => {
                            return Msg::Compacted(Err(
                                "compaction blocked by an unsupported lifecycle hook decision"
                                    .to_string(),
                            ));
                        }
                    }
                    let result = crate::compact::compact_history(
                        llm_client,
                        &history,
                        previous_summary.as_deref(),
                    )
                    .await;
                    if let Ok(summary) = &result {
                        let post_event = a3s_code_core::hooks::HookEvent::PostCompact(
                            a3s_code_core::hooks::PostCompactEvent {
                                session_id,
                                message_count_before: message_count,
                                message_count_after: usize::from(summary.is_some()),
                                summary_generated: summary.is_some(),
                            },
                        );
                        let _ = a3s_code_core::hooks::HookExecutor::fire(
                            hook_executor.as_ref(),
                            &post_event,
                        )
                        .await;
                    }
                    Msg::Compacted(result)
                }));
            }
            "/help" => {
                self.textarea.clear();
                self.help_open = true;
                self.help_scroll = 0;
                return None;
            }
            "/status" => {
                self.show_session_status();
                return None;
            }
            "/terminal" => {
                self.textarea.clear();
                let diagnostic =
                    panels::terminal::current_terminal_diagnostic(self.viewport_content_width());
                self.push_line(&diagnostic);
                return None;
            }
            "/checkup" => return self.submit_checkup_command(),
            "/queue" => {
                self.textarea.clear();
                self.open_queue_panel();
                return None;
            }
            "/history" => {
                self.textarea.clear();
                self.open_history_panel("");
                return None;
            }
            "/permissions" => {
                self.textarea.clear();
                self.open_permission_panel();
                return None;
            }
            "/sandbox" => {
                self.show_sandbox_status();
                return None;
            }
            "/display" => {
                self.display_profile = self.display_profile.next();
                self.textarea.clear();
                self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                    "  display · {} · {}",
                    self.display_profile.name(),
                    self.display_profile.summary()
                )));
                return None;
            }
            "/auto" => {
                self.set_composer_mode(Mode::Auto);
                self.textarea.clear();
                self.push_line(&Style::new().fg(TN_GRAY).render(
                    "  auto · same as Shift+Tab → auto · future turns stay non-interactive",
                ));
                self.rebuild_viewport();
                return None;
            }
            "/ask" | "/plan" => {
                self.set_composer_mode(Mode::Plan);
                self.textarea.clear();
                let notice = if trimmed == "/ask" {
                    "  ask · read-only explore (maps to plan) · no file edits · Shift+Tab → plan"
                } else {
                    "  plan · same as Shift+Tab → plan · read-only discovery until you leave plan"
                };
                self.push_line(&Style::new().fg(TN_GRAY).render(notice));
                self.rebuild_viewport();
                return None;
            }
            "/reviewer" => {
                let next = if self.mode == Mode::Reviewer {
                    Mode::Default
                } else {
                    Mode::Reviewer
                };
                self.set_composer_mode(next);
                self.textarea.clear();
                let notice = if next == Mode::Reviewer {
                    panels::review::reviewer_mode_on_notice()
                } else {
                    panels::review::reviewer_mode_off_notice()
                };
                self.push_line(&Style::new().fg(TN_GRAY).render(notice));
                self.rebuild_viewport();
                return None;
            }
            "/yolo" => {
                self.set_composer_mode(Mode::Yolo);
                self.textarea.clear();
                self.push_line(&Style::new().fg(COMPOSER_CHROME.error).render(
                    "  ⚡ yolo · same as Shift+Tab → yolo · high-risk auto-allowed; critical denials remain",
                ));
                self.rebuild_viewport();
                return None;
            }
            "/config" => {
                self.textarea.clear();
                let path = self.config_path.clone();
                self.open_config_in_ide(&path);
                return None;
            }
            "/model" => {
                self.textarea.clear();
                self.open_model_menu();
                let mut commands = Vec::new();
                if let Some(command) = self.maybe_refresh_codex_models() {
                    commands.push(command);
                }
                if let Some(command) = self.maybe_fetch_active_model_models() {
                    commands.push(command);
                }
                return match commands.len() {
                    0 => None,
                    1 => commands.pop(),
                    _ => Some(cmd::batch(commands)),
                };
            }
            "/effort" => {
                self.textarea.clear();
                self.effort_panel = Some(self.effort);
                return None;
            }
            "/ide" => {
                self.textarea.clear();
                self.push_line(&Style::new().fg(TN_GRAY).render(
                    "  advanced · prefer an external editor for large files · /config keeps minimal in-TUI editing",
                ));
                let entries = ide_children(std::path::Path::new(&self.cwd), 0);
                self.ide = Some(Ide::workspace(entries));
                return None;
            }
            "/plugin" => {
                self.textarea.clear();
                return self.open_plugins_panel(true);
            }
            "/packages" => {
                self.textarea.clear();
                self.push_prefer_hub_tip("/use packages");
                return self.open_package_panel();
            }
            "/theme" => {
                self.textarea.clear();
                let cur = SYNTAX_THEME.load(std::sync::atomic::Ordering::Relaxed);
                self.theme_panel = Some(cur.min(THEMES.len() - 1));
                return None;
            }
            "/reload" => {
                self.textarea.clear();
                self.push_prefer_hub_tip("/use reload");
                return self.reload_skills_and_plugins();
            }
            "/update" => {
                self.textarea.clear();
                self.updating = Some(Instant::now()); // "checking…" + input lock
                self.relayout();
                return Some(cmd::cmd(|| async {
                    // Quick version check only; the actual upgrade runs in the
                    // shell after the TUI exits (run()), so brew's/curl's own
                    // progress shows and the restart picks up the new binary.
                    let latest = crate::update::fetch_latest_async().await;
                    Msg::UpdatePlan(latest)
                }));
            }
            "/tasks" => {
                self.textarea.clear();
                return self.open_task_panel();
            }
            "/relay" => return self.open_relay_panel(),
            "/memory" => {
                self.textarea.clear();
                return self.open_memory_panel(true);
            }
            "/evolution" => {
                self.textarea.clear();
                return self.open_evolution_panel(true);
            }
            _ => {}
        }

        if trimmed.starts_with('/') {
            self.textarea.clear();
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render(&format!("  {}", unknown_slash_command_message(trimmed))),
            );
            return None;
        }

        let pastes = std::mem::take(&mut self.pending_pastes);
        if !trimmed.is_empty() || !pastes.is_empty() {
            let history_entry = merge_paste_bodies(&pastes, trimmed);
            if !history_entry.is_empty() {
                self.history.push(history_entry);
            }
        }
        self.history_pos = None;
        self.history_draft = None;
        // Composer chips disappear on submit, while compact textual references
        // remain in the user bubble just like Codex's `[Image #n]` markers.
        let paste_references = paste_reference_line(&pastes);
        let image_references = attachment_reference_line(&self.pending_images);
        let chip_references = match (paste_references.is_empty(), image_references.is_empty()) {
            (true, true) => String::new(),
            (false, true) => paste_references.clone(),
            (true, false) => image_references.clone(),
            (false, false) => format!("{paste_references}\n{image_references}"),
        };
        let user_display = match (chip_references.is_empty(), trimmed.is_empty()) {
            (true, _) => trimmed.to_string(),
            (false, true) => chip_references.clone(),
            (false, false) => format!("{chip_references}\n{trimmed}"),
        };
        // While a turn is still streaming, keep the next user bubble out of the
        // transcript until admission. Otherwise live Thought / interrupt
        // markers from the prior turn render under the new prompt.
        let defer_user_transcript = self.should_defer_user_transcript();
        if !defer_user_transcript {
            let transcript_images = self
                .pending_images
                .iter()
                .map(PendingImage::transcript_image)
                .collect::<Vec<_>>();
            self.messages.push(TranscriptEntry::user_with_images(
                user_display,
                transcript_images,
            ));
        }
        self.textarea.clear();
        // One-shot `/ctx <n>` context: attach the staged transcript to THIS
        // genuine typed message only (never a `/loop` "Continue." re-entry),
        // invisibly — the display bubble above stays clean. Travels with the
        // message whether it runs now or is queued.
        let loop_cont = std::mem::take(&mut self.loop_continuation);
        if !loop_cont {
            if let Some(run) = self.goal_run.as_mut() {
                run.pause_achievement_for_user_turn();
            }
        }
        let typed_prompt = if trimmed.is_empty() && pastes.is_empty() {
            "Please inspect the attached image or images.".to_string()
        } else {
            merge_paste_bodies(&pastes, trimmed)
        };
        let typed_prompt = expand_skill_mentions(
            &typed_prompt,
            &self.skills,
            &self.disabled_skills,
            self.sticky_skill.as_deref(),
        );
        let task_label = if trimmed.is_empty() {
            if chip_references.is_empty() {
                image_references
            } else {
                chip_references
            }
        } else {
            trimmed.to_string()
        };
        let prompt = match (loop_cont, self.pending_ctx.take()) {
            (false, Some(c)) => format!("{c}\n\n{typed_prompt}"),
            _ => typed_prompt,
        };
        let prompt = panels::workspace_review::with_open_reply_findings_prefix(
            prompt,
            loop_cont,
            &self.open_reply_findings,
        );
        let display = task_label;
        let send_now = intent == SubmissionIntent::SendNow && self.state == State::Streaming;
        let priority = if send_now {
            PLAN_REVIEW_PRIORITY
        } else if loop_cont {
            SYNTHETIC_TURN_PRIORITY
        } else {
            USER_TURN_PRIORITY
        };
        let execution_mode = self.mode.main_stream_mode();
        let images = std::mem::take(&mut self.pending_images);
        let sequence = if execution_mode == Mode::Plan && !loop_cont {
            let request = PlanDraftRequest::initial(prompt, display.clone());
            self.enqueue_plan_turn(
                priority,
                Queued {
                    text: request.planning_prompt(),
                    display,
                    images,
                    pastes: Vec::new(),
                    runtime_expectation: None,
                    deep_research: None,
                    transcript_posted: !defer_user_transcript,
                },
                request,
            )
        } else {
            self.enqueue_turn(
                priority,
                Queued {
                    text: prompt,
                    display,
                    images,
                    pastes: Vec::new(),
                    runtime_expectation: None,
                    deep_research: None,
                    transcript_posted: !defer_user_transcript,
                },
                execution_mode,
            )
        };
        if send_now {
            self.send_now_queued_sequence = Some(sequence);
            return self.begin_send_now_interrupt();
        }
        if self.state == State::Idle {
            self.drain_queue()
        } else {
            // Keep this transient state out of the durable transcript. The
            // queue panel disappears as soon as drain_queue claims the turn.
            self.relayout();
            None
        }
    }

    pub(crate) fn start_deep_research(
        &mut self,
        raw_query: &str,
        from_question_shortcut: bool,
    ) -> Option<Cmd<Msg>> {
        // One LLM call defines the semantic research contract and the exact
        // provider queries for one bounded retrieval pass. Rust never routes
        // free-form query text through lexical rules.
        let (query, evidence_scope) = parse_deep_research_tui_query(raw_query);
        if query.is_empty() {
            self.textarea.clear();
            if !from_question_shortcut {
                self.push_line(&Style::new().fg(TN_GRAY).render(
                    "  usage: /research <query> · /research [status|explain|replay] [run-id] · /research diff <left> <right>",
                ));
            }
            return None;
        }
        self.history.push(format!("? {query}"));
        self.history_pos = None;
        self.history_draft = None;
        self.textarea.clear();
        if from_question_shortcut {
            self.push_prefer_hub_tip("/research <query>");
        }
        self.messages.push(TranscriptEntry::preformatted(gutter(
            TN_CYAN,
            &Style::new()
                .bold()
                .render(&format!("✦\u{200A}deep research: {query}")),
        )));
        let evidence_scope_label = evidence_scope.label();
        let runtime_hint = if self.os_session.is_some() {
            format!(
                "  ◎\u{200A}goal set · semantic plan · one evidence pass · {evidence_scope_label} · closed-evidence review · local HTML opens in RemoteUI (Esc stops)"
            )
        } else {
            format!(
                "  ◎\u{200A}goal set · semantic plan · one evidence pass · {evidence_scope_label} · closed-evidence review · report + HTML opens in RemoteUI (Esc stops)"
            )
        };
        self.push_line(&Style::new().fg(TN_GRAY).render(&runtime_hint));
        let display = format!("✦\u{200A}{query}");
        let runtime_expectation = Some(RuntimeExpectation::required("deep research"));
        let execution_mode = self.mode;
        self.enqueue_turn(
            USER_TURN_PRIORITY,
            Queued {
                text: format!("? {query}"),
                display,
                images: Vec::new(),
                pastes: Vec::new(),
                runtime_expectation,
                deep_research: Some((query, evidence_scope)),
                transcript_posted: true,
            },
            execution_mode,
        );
        if self.state == State::Idle {
            return self.drain_queue();
        }
        // The bottom queue projection is the only owner of pending-turn
        // status. A transcript entry would outlive the queue item after it
        // is claimed and make an already-running turn look pending.
        self.relayout();
        None
    }

    pub(crate) fn push_prefer_hub_tip(&mut self, preferred: &str) {
        self.push_line(
            &Style::new()
                .fg(TN_GRAY)
                .render(&panels::review::prefer_hub_tip_line(preferred)),
        );
    }

    pub(crate) fn open_plugins_panel(&mut self, tip_redirect: bool) -> Option<Cmd<Msg>> {
        if tip_redirect {
            self.push_prefer_hub_tip("/use plugin");
        }
        if self.skills.is_empty() {
            self.push_line(&Style::new().fg(TN_GRAY).render(
                "  no skills/plugins found (~/.claude/skills, ~/.codex/skills, ~/.claude/plugins)",
            ));
        } else {
            self.plugins_panel = Some(0);
        }
        None
    }

    pub(crate) fn reload_skills_and_plugins(&mut self) -> Option<Cmd<Msg>> {
        // Hot-reload: re-discover skill dirs, refresh the UI catalog,
        // and rebuild the session so the core skill registry and
        // next Claude/system prompt see the same skills.
        let dirs = agent_skill_dirs_with_configured(&self.cwd, &self.asset_directories.skill);
        self.skills = load_skills(&dirs);
        self.skill_count = count_skill_files(&dirs);
        let profile = self.session_rebuild_profile();
        self.start_session_rebuild(
            profile,
            SessionRebuildAction::Reload {
                skill_count: self.skills.len(),
            },
        )
    }

    pub(crate) fn open_memory_panel(&mut self, tip_redirect: bool) -> Option<Cmd<Msg>> {
        if tip_redirect {
            self.push_prefer_hub_tip("/ctx memory");
        }
        // Open immediately ("loading…"); load the file snapshot off the
        // UI thread, with live session memory as a fallback.
        let dir = self.memory_dir.clone();
        self.memory = Some(MemPanel {
            entries: Vec::new(),
            sel: 0,
            details: std::collections::BTreeMap::new(),
            graph: MemoryGraph::default(),
            loaded_from_session: false,
            detail: memutil::MemDetail::default(),
            detail_scroll: 0,
            dir: dir.clone(),
            note: panels::review::memory_panel_loading_note().into(),
        });
        Some(self.load_memory_panel(dir))
    }

    pub(crate) fn open_evolution_panel(&mut self, tip_redirect: bool) -> Option<Cmd<Msg>> {
        if tip_redirect {
            self.push_prefer_hub_tip("/ctx evolution");
        }
        self.evolution = Some(panels::evolution::EvolutionPanel::loading());
        Some(self.load_evolution_panel())
    }

    pub(crate) fn start_sleep_command(
        &mut self,
        rest: &str,
        tip_redirect: bool,
    ) -> Option<Cmd<Msg>> {
        let focus = rest.trim().to_string();
        self.textarea.clear();
        if tip_redirect {
            self.push_prefer_hub_tip("/ctx sleep");
        }
        self.sleep_pending = true;
        self.engage_autonomy(8);
        self.push_line(
            &Style::new()
                .fg(TN_GRAY)
                .render("  ☾ sleep — consolidating today's work into memory… (Esc stops)"),
        );
        let directive =
            panels::sleep::sleep_directive(&focus, self.ctx_ready, &panels::sleep::sleep_today());
        let display = if focus.is_empty() {
            "☾ sleep".to_string()
        } else {
            format!("☾ sleep · {focus}")
        };
        self.start_stream_inner(directive, display, true, true, false)
    }

    /// Grab a clipboard image and add an interactive chip to the composer.
    /// This method only stages the image; Enter remains the sole send action.
    pub(super) fn paste_clipboard_image(&mut self) {
        if let Err(error) = self.ensure_image_can_be_staged() {
            self.push_notice(
                NoticeKind::Warning,
                format!("Image attachment was not added: {error}"),
            );
            return;
        }
        match PendingImage::from_clipboard() {
            Ok(image) => {
                if let Err(error) = self.stage_pending_image(image) {
                    self.push_notice(
                        NoticeKind::Warning,
                        format!("Image attachment was not added: {error}"),
                    );
                }
            }
            Err(error) => self.push_notice(
                NoticeKind::Warning,
                format!("Clipboard image unavailable: {error}"),
            ),
        }
    }

    pub(super) fn start_stream(&mut self, prompt: String) -> Option<Cmd<Msg>> {
        self.start_stream_inner(prompt.clone(), prompt, true, true, false)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        composer_value_after_skill_menu_enter, expand_skill_mentions, should_clear_sticky_on_esc,
        skill_enter_attaches_sticky, sticky_skill_name_from_mention, KeyCode, KeyEvent,
        KeyModifiers,
    };
    use std::collections::HashSet;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers }
    }

    #[test]
    fn dollar_mentions_select_enabled_skills_without_rewriting_the_visible_prompt() {
        let skills = vec![
            ("review".to_string(), "Review code".to_string()),
            ("a3s-office".to_string(), "Edit Office files".to_string()),
        ];
        let expanded = expand_skill_mentions(
            "$review 请检查，然后用 $a3s-office。",
            &skills,
            &HashSet::new(),
            None,
        );

        assert!(expanded.contains("- Use your `review` skill."));
        assert!(expanded.contains("- Use your `a3s-office` skill."));
        assert!(expanded.ends_with("$review 请检查，然后用 $a3s-office。"));
    }

    #[test]
    fn sticky_skill_is_selected_even_without_dollar_mention() {
        let skills = vec![("review".to_string(), "Review code".to_string())];
        let expanded = expand_skill_mentions(
            "please check auth",
            &skills,
            &HashSet::new(),
            Some("review"),
        );
        assert!(expanded.contains("- Use your `review` skill."));
        assert!(expanded.contains("please check auth"));
    }

    #[test]
    fn sticky_skill_outranks_disabled_skill_list() {
        let skills = vec![("review".to_string(), "Review code".to_string())];
        let disabled = HashSet::from(["review".to_string()]);
        let expanded =
            expand_skill_mentions("please check auth", &skills, &disabled, Some("review"));
        assert!(
            expanded.contains("- Use your `review` skill."),
            "sticky attach must keep injecting even when the skill is disabled: {expanded}"
        );
        // Explicit $mention of a disabled skill still stays plain.
        assert_eq!(
            expand_skill_mentions("$review costs $5", &skills, &disabled, None),
            "$review costs $5"
        );
    }

    #[test]
    fn unknown_and_disabled_dollar_tokens_remain_plain_prompt_text() {
        let skills = vec![("review".to_string(), "Review code".to_string())];
        let disabled = HashSet::from(["review".to_string()]);
        let prompt = "$review costs $5 and $unknown";

        assert_eq!(
            expand_skill_mentions(prompt, &skills, &disabled, None),
            prompt
        );
    }

    #[test]
    fn alt_or_meta_enter_attaches_sticky_skill_plain_enter_does_not() {
        assert!(!skill_enter_attaches_sticky(KeyModifiers::NONE));
        assert!(skill_enter_attaches_sticky(KeyModifiers::ALT));
        assert!(skill_enter_attaches_sticky(KeyModifiers::META));
        assert_eq!(sticky_skill_name_from_mention("$review"), Some("review"));
        assert_eq!(sticky_skill_name_from_mention("review"), None);
    }

    #[test]
    fn sticky_skill_menu_enter_clears_composer_so_esc_can_unstick() {
        let completed = "$review ";
        assert!(composer_value_after_skill_menu_enter(completed, true).is_empty());
        assert_eq!(
            composer_value_after_skill_menu_enter(completed, false),
            completed
        );
        // Empty composer + sticky + Esc clears (matches menu sticky attach path).
        let esc = key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(should_clear_sticky_on_esc(
            &esc,
            true,
            composer_value_after_skill_menu_enter(completed, true)
                .trim()
                .is_empty(),
            true,
            false
        ));
    }

    #[test]
    fn sticky_clears_only_on_esc_when_idle_empty_and_menu_closed() {
        let esc = key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(should_clear_sticky_on_esc(&esc, true, true, true, false));
        assert!(!should_clear_sticky_on_esc(&esc, false, true, true, false));
        assert!(!should_clear_sticky_on_esc(&esc, true, false, true, false));
        assert!(!should_clear_sticky_on_esc(&esc, true, true, false, false));
        assert!(!should_clear_sticky_on_esc(&esc, true, true, true, true));
        assert!(!should_clear_sticky_on_esc(
            &key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            true,
            true,
            true,
            false
        ));
    }
}
