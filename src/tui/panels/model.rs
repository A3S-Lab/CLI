//! `/model` picker (with account tabs) + `/effort` rebuild logic + overlays.

use super::super::*;
use super::login::{detect_local, save_creds, AuthProvider};

// Account-backend model menus (the ChatGPT / Anthropic account backends don't
// expose a list, so these are sensible defaults — pick what your plan supports).
const CLAUDE_MODELS: &[&str] = &[
    "claude-opus-4-20250514",
    "claude-sonnet-4-20250514",
    "claude-3-5-haiku-20241022",
];
const GPT_MODELS: &[&str] = &["gpt-5-codex", "gpt-5", "o4-mini"];

/// A tab in the `/model` picker: config models, or a signed-in account's models.
struct ModelTab {
    label: &'static str,
    models: Vec<String>,
    provider: Option<AuthProvider>, // None = config.acl
}

impl App {
    /// Tabs: config always; Claude / GPT appear when that local login exists.
    fn model_tabs(&self) -> Vec<ModelTab> {
        let mut tabs = vec![ModelTab {
            label: "Config",
            models: self.models.clone(),
            provider: None,
        }];
        if detect_local(AuthProvider::Claude).is_some() {
            tabs.push(ModelTab {
                label: "Claude",
                models: CLAUDE_MODELS.iter().map(|s| s.to_string()).collect(),
                provider: Some(AuthProvider::Claude),
            });
        }
        if detect_local(AuthProvider::Codex).is_some() {
            tabs.push(ModelTab {
                label: "GPT",
                models: GPT_MODELS.iter().map(|s| s.to_string()).collect(),
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
        let active = self.auth.as_ref().map(|(p, _)| *p);
        self.model_tab = tabs.iter().position(|t| t.provider == active).unwrap_or(0);
        let cur = self.model.as_deref();
        let idx = tabs[self.model_tab]
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
                if let Some(model) = model {
                    match provider {
                        None => {
                            self.auth = None; // config.acl credentials
                            self.switch_model(&model);
                        }
                        Some(p) => self.switch_account_model(p, &model),
                    }
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

    /// Sign in with a detected local account and switch to one of its models.
    fn switch_account_model(&mut self, provider: AuthProvider, model: &str) {
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(Color::Yellow)
                    .render("  finish the current turn before switching models"),
            );
            return;
        }
        let Some(token) = detect_local(provider) else {
            self.push_line(
                &Style::new()
                    .fg(Color::Red)
                    .render("  no local login found for that account"),
            );
            return;
        };
        save_creds(provider, &token); // remember the account choice across restarts
        self.auth = Some((provider, token));
        self.model = Some(model.to_string()); // before rebuild → auth_client uses it
        match self.rebuild_session(Some(model)) {
            Ok((s, _)) => {
                self.session = Arc::new(s);
                let who = if provider == AuthProvider::Codex {
                    "Codex"
                } else {
                    "Claude (experimental)"
                };
                self.push_line(
                    &Style::new()
                        .fg(Color::Green)
                        .render(&format!("  ⇄ {who} · {model}")),
                );
            }
            Err(e) => self.push_line(
                &Style::new()
                    .fg(Color::Red)
                    .render(&format!("  failed to switch: {e}")),
            ),
        }
    }

    /// Switch the active model by resuming the session under it (history kept).
    /// Base session options carrying the current effort. `ultracode` turns on
    /// planning + parallel subagent delegation (a3s-code PTC), so a turn plans a
    /// dynamic workflow and fans tasks out to multiple subagents.
    pub(crate) fn effort_session_opts(&self, thinking: bool) -> SessionOptions {
        let mut opts = SessionOptions::new()
            .with_session_store(self.store.clone())
            .with_session_id(self.session_id.as_str())
            .with_confirmation_policy(self.confirmation.clone())
            .with_skill_dirs(claude_skill_dirs(&self.cwd))
            .with_auto_save(true)
            // Auto-compact the context when it nears the window (Claude-style).
            .with_auto_compact(true)
            .with_auto_compact_threshold(0.85)
            .with_file_memory(memory_dir())
            // Parallel fan-out available in every mode (not just ultracode).
            .with_max_parallel_tasks(8)
            .with_auto_delegation_enabled(true)
            .with_auto_parallel_delegation(true);
        // Keep project instructions (CLAUDE.md) + any /compact summary across
        // model/effort/compact rebuilds, injected into the system prompt.
        let extra = match (&self.instructions, &self.compact_summary) {
            (Some(i), Some(s)) => Some(format!("{i}\n\n# Earlier conversation (compacted)\n\n{s}")),
            (Some(i), None) => Some(i.clone()),
            (None, Some(s)) => Some(format!("# Earlier conversation (compacted)\n\n{s}")),
            (None, None) => None,
        };
        if let Some(e) = extra {
            opts = opts.with_prompt_slots(SystemPromptSlots::default().with_extra(e));
        }
        // Extended thinking is Anthropic-only; only request it when asked.
        if thinking {
            opts = opts.with_thinking_budget(EFFORT_LEVELS[self.effort].1);
        }
        if self.effort == ULTRACODE {
            opts = opts
                .with_planning_mode(a3s_code_core::PlanningMode::Enabled)
                .with_goal_tracking(true)
                .with_max_tool_rounds(40);
        }
        // Signed in via /login → route the model through the account token.
        if let Some(client) = self.auth_client() {
            opts = opts.with_llm_client(client);
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
        let hint = if tabs.len() > 1 {
            "  Select model — ↑/↓ · ←/→ account · Enter · Esc"
        } else {
            "  Select model — ↑/↓ · Enter · Esc · /login not needed (use ←/→ once signed in)"
        };
        let mut menu = vec![pad_to(&Style::new().fg(ACCENT).bold().render(hint), width)];
        // Tab bar (only worth showing when there's more than the config tab).
        if tabs.len() > 1 {
            let mut bar = String::from("  ");
            for (i, tab) in tabs.iter().enumerate() {
                let chip = format!(" {} ", tab.label);
                bar.push_str(&if i == t {
                    Style::new()
                        .fg(Color::BrightWhite)
                        .bg(ACCENT)
                        .bold()
                        .render(&chip)
                } else {
                    Style::new().fg(Color::BrightBlack).render(&chip)
                });
                bar.push(' ');
            }
            menu.push(pad_to(&bar, width));
        }
        let active = self.auth.as_ref().map(|(p, _)| *p);
        let models = &tabs[t].models;
        let last = models.len().saturating_sub(1);
        for (i, m) in models.iter().enumerate().take(12) {
            let cur = Some(m.as_str()) == self.model.as_deref() && tabs[t].provider == active;
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
