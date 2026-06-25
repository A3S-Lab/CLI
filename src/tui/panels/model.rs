//! `/model` picker + `/effort` model-rebuild logic and the model overlay.

use super::super::*;

impl App {
    /// Open the /model picker on the current model (no-op if none configured).
    pub(crate) fn open_model_menu(&mut self) {
        if self.models.is_empty() {
            self.push_line(
                &Style::new()
                    .fg(Color::Red)
                    .render("  no models configured in config.acl"),
            );
            return;
        }
        let cur = self.model.as_deref();
        let idx = self
            .models
            .iter()
            .position(|m| Some(m.as_str()) == cur)
            .unwrap_or(0);
        self.model_menu = Some(idx);
    }

    /// Keys while the /model panel is open: ↑/↓ select, Enter switch, Esc close.
    pub(crate) fn handle_model_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let sel = self.model_menu?;
        let last = self.models.len().saturating_sub(1);
        match key.code {
            KeyCode::Up => {
                self.model_menu = Some(sel.saturating_sub(1));
                Some(None)
            }
            KeyCode::Down => {
                self.model_menu = Some((sel + 1).min(last));
                Some(None)
            }
            KeyCode::Enter => {
                let model = self.models[sel.min(last)].clone();
                self.model_menu = None;
                self.switch_model(&model);
                Some(None)
            }
            KeyCode::Esc => {
                self.model_menu = None;
                Some(None)
            }
            _ => None,
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
            .with_auto_save(true)
            // Auto-compact the context when it nears the window (Claude-style).
            .with_auto_compact(true)
            .with_auto_compact_threshold(0.85);
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
                .with_max_parallel_tasks(8)
                .with_auto_delegation_enabled(true)
                .with_auto_parallel_delegation(true)
                .with_max_tool_rounds(40);
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
        if self.models.is_empty() {
            return composed;
        }
        let width = self.width as usize;
        let mut menu = vec![pad_to(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render("  Select model — ↑/↓ · Enter · Esc"),
            width,
        )];
        for (i, m) in self.models.iter().enumerate().take(12) {
            let cur = Some(m.as_str()) == self.model.as_deref();
            let raw = pad_to(&format!("  {} {m}", if cur { "●" } else { " " }), width);
            menu.push(if i == sel.min(self.models.len() - 1) {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
            } else {
                Style::new().fg(Color::BrightBlack).render(&raw)
            });
        }
        self.overlay_list(composed, &menu)
    }
}
