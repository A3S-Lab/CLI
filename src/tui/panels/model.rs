//! `/model` picker (with account tabs) + `/effort` rebuild logic + overlays.

use super::super::*;
use super::login::{claude_models, has_local_login, AuthProvider};

/// A tab in the `/model` picker: config models, or a signed-in account's models.
struct ModelTab {
    label: &'static str,
    color: Color,
    models: Vec<String>,
    provider: Option<AuthProvider>, // None = config.acl
}

// Per-source accents, tuned to the Tokyo Night palette (blue / orange / teal).
const A3S_COLOR: Color = ACCENT;
const CLAUDE_COLOR: Color = TN_ORANGE;
const CODEX_COLOR: Color = Color::Rgb(115, 218, 202); // tokyo teal

/// Ultracode system-prompt steer: express the whole task as ONE generated
/// `program` workflow script that fans out via `parallel_task` inside it — a
/// hard rule (no top-level delegation) plus a copy-paste template, because a
/// soft "prefer the script" steer let the model just call parallel_task directly
/// (the PTC fans out child agents on the multi-threaded runtime since 4.2.6).
const ULTRACODE_GUIDELINES: &str = "\
[ultracode] Dynamic-workflow mode. Express ALL of your work as ONE generated, \
executable workflow SCRIPT. Do NOT call `parallel_task` or `task` directly at \
the top level — the script IS the workflow.\n\
1. PLAN. Decompose the task into numbered steps; mark independent (concurrent) \
vs dependent (sequential).\n\
2. WRITE + RUN THE SCRIPT by calling the `program` tool with a JavaScript \
`source` of this shape:\n\
     async function run(ctx, inputs) {\n\
       const results = await ctx.tool(\"parallel_task\", { tasks: [\n\
         { description: \"step A\", prompt: \"...\" },\n\
         { description: \"step B\", prompt: \"...\" }\n\
       ] });\n\
       return results;\n\
     }\n\
   Put EVERY task/parallel_task call INSIDE the script; add further ctx.tool(...) \
calls for dependent steps and aggregate their outputs.\n\
3. parallel_task inside the script fans out concurrent subagents on the \
multi-threaded runtime. After it returns, synthesize the results into your \
final answer.\n\
4. Be exhaustive: pursue every thread to completion.";

impl App {
    /// Tabs: a3s-code always; Claude Code / Codex appear when that local login
    /// is detected.
    fn model_tabs(&self) -> Vec<ModelTab> {
        let mut tabs = vec![ModelTab {
            label: "a3s-code",
            color: A3S_COLOR,
            models: self.models.clone(),
            provider: None,
        }];
        if has_local_login(AuthProvider::Claude) {
            tabs.push(ModelTab {
                label: "Claude Code",
                color: CLAUDE_COLOR,
                models: claude_models(), // from ~/.claude.json
                provider: Some(AuthProvider::Claude),
            });
        }
        if has_local_login(AuthProvider::Codex) {
            tabs.push(ModelTab {
                label: "Codex",
                color: CODEX_COLOR,
                models: crate::codex::codex_models(), // from ~/.codex/models_cache.json
                provider: Some(AuthProvider::Codex),
            });
        }
        tabs
    }

    /// Open the /model picker on the current model + matching tab.
    pub(crate) fn open_model_menu(&mut self) {
        let tabs = self.model_tabs();
        if tabs.iter().all(|t| t.models.is_empty()) {
            self.push_line(
                &Style::new()
                    .fg(Color::Red)
                    .render("  no models configured in config.acl"),
            );
            return;
        }
        // The active model is always a config model (account tabs are
        // informational), so open on the config tab at the current model.
        self.model_tab = 0;
        let cur = self.model.as_deref();
        let idx = tabs[0]
            .models
            .iter()
            .position(|m| Some(m.as_str()) == cur)
            .unwrap_or(0);
        self.model_menu = Some(idx);
    }

    /// Keys while the /model panel is open: ↑/↓ select, ←/→/Tab switch tab,
    /// Enter activate (config model, or sign in with the tab's account), Esc.
    pub(crate) fn handle_model_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let sel = self.model_menu?;
        let tabs = self.model_tabs();
        let tab_count = tabs.len().max(1);
        let t = self.model_tab.min(tab_count - 1);
        let last = tabs[t].models.len().saturating_sub(1);
        match key.code {
            KeyCode::Up => {
                self.model_menu = Some(sel.saturating_sub(1));
                Some(None)
            }
            KeyCode::Down => {
                self.model_menu = Some((sel + 1).min(last));
                Some(None)
            }
            KeyCode::Left => {
                self.model_tab = t.saturating_sub(1);
                self.model_menu = Some(0);
                Some(None)
            }
            KeyCode::Right | KeyCode::Tab => {
                self.model_tab = (t + 1).min(tab_count - 1);
                self.model_menu = Some(0);
                Some(None)
            }
            KeyCode::Enter => {
                let model = tabs[t].models.get(sel.min(last)).cloned();
                let provider = tabs[t].provider;
                self.model_menu = None;
                match provider {
                    None => {
                        self.llm_override = None; // config.acl credentials
                        if let Some(model) = model {
                            self.switch_model(&model);
                        }
                    }
                    Some(AuthProvider::Codex) => {
                        if let Some(model) = model {
                            self.sign_in_codex(&model);
                        }
                    }
                    Some(p) => self.account_model_note(p), // Claude: still experimental
                }
                Some(None)
            }
            KeyCode::Esc => {
                self.model_menu = None;
                Some(None)
            }
            _ => None,
        }
    }

    /// Sign in with the local Codex login and switch to one of its models by
    /// injecting the custom Codex client (talks to the ChatGPT backend).
    fn sign_in_codex(&mut self, model: &str) {
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(Color::Yellow)
                    .render("  finish the current turn before switching models"),
            );
            return;
        }
        match crate::codex::CodexClient::from_codex_login(model, &self.session_id) {
            Ok(client) => {
                self.llm_override = Some(Arc::new(client));
                self.model = Some(model.to_string()); // before rebuild
                match self.rebuild_session(Some(model)) {
                    Ok((s, _)) => {
                        self.session = Arc::new(s);
                        self.push_line(
                            &Style::new()
                                .fg(TN_GREEN)
                                .render(&format!("  ⇄ Codex · {model}")),
                        );
                    }
                    Err(e) => {
                        self.llm_override = None;
                        self.push_line(
                            &Style::new()
                                .fg(TN_RED)
                                .render(&format!("  failed to switch: {e}")),
                        );
                    }
                }
            }
            Err(e) => self.push_line(
                &Style::new()
                    .fg(TN_RED)
                    .render(&format!("  Codex sign-in failed: {e}")),
            ),
        }
    }

    /// Claude account tab is informational for now: a3s-code can't drive the
    /// Anthropic account API, so point the user at an API key in config.acl.
    fn account_model_note(&mut self, provider: AuthProvider) {
        self.push_line(&Style::new().fg(Color::Yellow).render(&format!(
            "  {} login detected, but a3s can't use it yet — add an API key in \
             config.acl and pick it from the a3s-code tab (/config to edit)",
            provider.label()
        )));
    }

    /// Switch the active model by resuming the session under it (history kept).
    /// Base session options carrying the current effort. `ultracode` adds a
    /// system-prompt steer + goal tracking + a wider tool-round budget so a turn
    /// plans, then fans independent work out to parallel subagents via direct
    /// `parallel_task` calls.
    pub(crate) fn effort_session_opts(&self, thinking: bool) -> SessionOptions {
        let mut opts = SessionOptions::new()
            .with_session_store(self.store.clone())
            .with_session_id(self.session_id.as_str())
            .with_confirmation_policy(self.confirmation.clone())
            .with_skill_dirs(agent_skill_dirs(&self.cwd))
            .with_auto_save(true)
            // Auto-compact the context when it nears the window (Claude-style).
            .with_auto_compact(true)
            .with_auto_compact_threshold(0.85)
            .with_file_memory(memory_dir())
            // Parallel fan-out available in every mode (not just ultracode).
            .with_max_parallel_tasks(8)
            .with_auto_delegation_enabled(true)
            .with_auto_parallel_delegation(true)
            // Pin manual delegation on so `parallel_task`/`task` stay registered
            // even if config.acl disables them — else ultracode's fan-out calls
            // an unregistered tool ("Unknown tool: parallel_task").
            .with_manual_delegation_enabled(true);
        // Keep project instructions (CLAUDE.md) + any /compact summary across
        // model/effort/compact rebuilds, injected into the system prompt.
        let extra = match (&self.instructions, &self.compact_summary) {
            (Some(i), Some(s)) => Some(format!("{i}\n\n# Earlier conversation (compacted)\n\n{s}")),
            (Some(i), None) => Some(i.clone()),
            (None, Some(s)) => Some(format!("# Earlier conversation (compacted)\n\n{s}")),
            (None, None) => None,
        };
        let ultra = self.effort == ULTRACODE;
        if extra.is_some() || ultra {
            let mut slots = SystemPromptSlots::default();
            if let Some(e) = extra {
                slots = slots.with_extra(e);
            }
            if ultra {
                slots = slots.with_guidelines(ULTRACODE_GUIDELINES);
            }
            opts = opts.with_prompt_slots(slots);
        }
        // Extended thinking is Anthropic-only; only request it when asked.
        if thinking {
            opts = opts.with_thinking_budget(EFFORT_LEVELS[self.effort].1);
        }
        if ultra {
            // Dynamic-workflow mode: the model generates a `program` script that
            // fans out via `parallel_task` (PTC dispatches on the multi-threaded
            // runtime since 4.2.6). Not planning mode (mutually exclusive with
            // auto-parallel fan-out). Steering is in the system prompt; here we
            // just track the goal + widen the budget.
            opts = opts.with_goal_tracking(true).with_max_tool_rounds(40);
        }
        // Signed in via the /model Codex tab → route through the account client.
        if let Some(client) = &self.llm_override {
            opts = opts.with_llm_client(client.clone());
        }
        opts
    }

    /// Rebuild the session under the current effort. Tries with the thinking
    /// budget, then falls back without it (so models that don't support extended
    /// thinking don't error). Returns (session, thinking_dropped).
    pub(crate) fn rebuild_session(
        &self,
        model: Option<&str>,
    ) -> Result<(AgentSession, bool), String> {
        let build = |thinking: bool| {
            let o = self.effort_session_opts(thinking);
            match model {
                Some(m) => o.with_model(m),
                None => o,
            }
        };
        // Resume keeps history if the session was saved. Before the first turn
        // it isn't in the store ("Session not found"), so fall back to a fresh
        // session with the same id (no turns yet = no history to lose). Each is
        // also retried without the thinking budget for non-Anthropic models.
        for thinking in [true, false] {
            if let Ok(s) = self
                .agent
                .resume_session(self.session_id.as_str(), build(thinking))
            {
                return Ok((s, !thinking));
            }
            if let Ok(s) = self.agent.session(self.cwd.clone(), Some(build(thinking))) {
                return Ok((s, !thinking));
            }
        }
        Err("could not rebuild the session".into())
    }

    pub(crate) fn switch_model(&mut self, model: &str) {
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(Color::Yellow)
                    .render("  finish the current turn before switching models"),
            );
            return;
        }
        match self.rebuild_session(Some(model)) {
            Ok((s, _)) => {
                self.session = Arc::new(s);
                self.model = Some(model.to_string());
                self.context_limit = self.model_ctx.get(model).copied().unwrap_or(0);
                self.push_line(
                    &Style::new()
                        .fg(Color::Green)
                        .render(&format!("  ⇄ switched to {model}")),
                );
            }
            Err(e) => self.push_line(
                &Style::new()
                    .fg(Color::Red)
                    .render(&format!("  failed to switch model: {e}")),
            ),
        }
    }

    /// Apply the selected effort by rebuilding the session (keeps model + history).
    pub(crate) fn apply_effort(&mut self) {
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(Color::Yellow)
                    .render("  finish the current turn before changing effort"),
            );
            return;
        }
        let model = self.model.clone();
        match self.rebuild_session(model.as_deref()) {
            Ok((s, dropped)) => {
                self.session = Arc::new(s);
                if self.effort == ULTRACODE {
                    // Unattended fan-out: auto-approve so subagents run freely.
                    self.mode = Mode::Auto;
                    self.rainbow_until = Some(Instant::now()); // rainbow flourish
                    self.rainbow_frame = 0;
                    self.push_line(&Style::new().fg(ACCENT).bold().render(
                        "  ◆ ultracode — planning a dynamic workflow + parallel subagents (auto-approve on)",
                    ));
                } else if dropped {
                    self.push_line(&Style::new().fg(Color::BrightBlack).render(&format!(
                        "  ◇ effort: {} (this model uses its default depth)",
                        EFFORT_LEVELS[self.effort].0
                    )));
                } else {
                    self.push_line(
                        &Style::new()
                            .fg(Color::Green)
                            .render(&format!("  ◇ effort: {}", EFFORT_LEVELS[self.effort].0)),
                    );
                }
            }
            Err(e) => self.push_line(
                &Style::new()
                    .fg(Color::Red)
                    .render(&format!("  failed to set effort: {e}")),
            ),
        }
    }

    pub(crate) fn overlay_model_menu(&self, composed: String) -> String {
        let Some(sel) = self.model_menu else {
            return composed;
        };
        let tabs = self.model_tabs();
        if tabs.is_empty() {
            return composed;
        }
        let t = self.model_tab.min(tabs.len() - 1);
        let width = self.width as usize;
        let mut menu = vec![pad_to(
            &Style::new()
                .fg(tabs[t].color)
                .bold()
                .render("  Select model — ↑/↓ · ←/→ account · Enter · Esc"),
            width,
        )];
        // Tab strip (relay-style): each source in its brand colour, active boxed.
        if tabs.len() > 1 {
            let mut bar = String::from("  ");
            for (i, tab) in tabs.iter().enumerate() {
                let chip = format!(" {} ", tab.label);
                bar.push_str(&if i == t {
                    Style::new()
                        .fg(Color::Black)
                        .bg(tab.color)
                        .bold()
                        .render(&chip)
                } else {
                    Style::new().fg(tab.color).render(&chip)
                });
                bar.push(' ');
            }
            menu.push(pad_to(&bar, width));
        }
        let models = &tabs[t].models;
        let last = models.len().saturating_sub(1);
        for (i, m) in models.iter().enumerate().take(12) {
            // Only config-tab models can be the active model (account tabs are
            // informational until a3s can drive those APIs).
            let cur = Some(m.as_str()) == self.model.as_deref() && tabs[t].provider.is_none();
            let raw = pad_to(&format!("  {} {m}", if cur { "●" } else { " " }), width);
            menu.push(if i == sel.min(last) {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
            } else {
                Style::new().fg(Color::BrightBlack).render(&raw)
            });
        }
        self.overlay_list(composed, &menu)
    }
}
