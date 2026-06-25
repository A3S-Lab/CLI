//! `/relay` panel: resumable/relayable sessions across coding agents.

use super::super::*;

impl App {
    /// The agent tabs, always shown (even when a tab has no sessions) so the
    /// user can switch between them and see each agent's history.
    pub(crate) fn relay_tabs(&self) -> Vec<&'static str> {
        vec!["a3s-code", "claude code", "codex"]
    }

    /// Indices into `self.relay` for the sessions under the active tab.
    pub(crate) fn relay_tab_indices(&self) -> Vec<usize> {
        let tabs = self.relay_tabs();
        let Some(agent) = tabs.get(self.relay_tab).copied() else {
            return Vec::new();
        };
        self.relay
            .iter()
            .enumerate()
            .filter(|(_, s)| s.agent == agent)
            .map(|(i, _)| i)
            .collect()
    }

    pub(crate) fn handle_relay_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let sel = self.relay_menu?;
        let tabs = self.relay_tabs();
        let last = self.relay_tab_indices().len().saturating_sub(1);
        match key.code {
            // ←/→ switch agent tab, resetting the row selection.
            KeyCode::Left => {
                self.relay_tab = self.relay_tab.saturating_sub(1);
                self.relay_menu = Some(0);
                Some(None)
            }
            KeyCode::Right => {
                self.relay_tab = (self.relay_tab + 1).min(tabs.len().saturating_sub(1));
                self.relay_menu = Some(0);
                Some(None)
            }
            KeyCode::Up => {
                self.relay_menu = Some(sel.saturating_sub(1));
                Some(None)
            }
            KeyCode::Down => {
                self.relay_menu = Some((sel + 1).min(last));
                Some(None)
            }
            KeyCode::Enter => {
                let idxs = self.relay_tab_indices();
                self.relay_menu = None;
                idxs.get(sel.min(last)).map(|&i| self.relay_select(i))
            }
            KeyCode::Esc => {
                self.relay_menu = None;
                Some(None)
            }
            _ => None,
        }
    }

    /// Resume a native a3s-code session, or continue a foreign agent's task here.
    pub(crate) fn relay_select(&mut self, idx: usize) -> Option<Cmd<Msg>> {
        let (native_id, seed, agent) = {
            let s = self.relay.get(idx)?;
            (s.native_id.clone(), s.seed.clone(), s.agent)
        };
        if let Some(id) = native_id {
            let mut opts = SessionOptions::new()
                .with_session_store(self.store.clone())
                .with_session_id(id.as_str())
                .with_confirmation_policy(self.confirmation.clone())
                .with_auto_save(true);
            // Resume under the CURRENT model, not whatever the saved session used
            // (e.g. a smoke-test's gpt-4o that this config doesn't have).
            if let Some(m) = self.model.clone().or_else(|| self.models.first().cloned()) {
                opts = opts.with_model(&m);
            }
            match self.agent.resume_session(id.as_str(), opts) {
                Ok(sess) => {
                    self.session = Arc::new(sess);
                    self.session_id = id.clone();
                    self.messages.clear();
                    let w = (self.width as usize).saturating_sub(PAD + 2);
                    for m in self.session.history() {
                        let text = m.text();
                        if text.trim().is_empty() {
                            continue;
                        }
                        match m.role.as_str() {
                            "user" => self.messages.push(gutter(ACCENT, text.trim())),
                            "assistant" => {
                                let mut md = StreamingMarkdown::new(w);
                                md.push(&text);
                                self.messages.push(gutter(Color::Green, &md.view()));
                            }
                            _ => {}
                        }
                    }
                    self.push_line(
                        &Style::new()
                            .fg(Color::Green)
                            .render(&format!("  ⮌ resumed a3s-code session {id}")),
                    );
                }
                Err(e) => self.push_line(
                    &Style::new()
                        .fg(Color::Red)
                        .render(&format!("  failed to resume: {e}")),
                ),
            }
            None
        } else if let Some(seed) = seed {
            if self.state != State::Idle {
                self.push_line(
                    &Style::new()
                        .fg(Color::Yellow)
                        .render("  finish the current turn before relaying"),
                );
                return None;
            }
            self.messages.push(gutter(
                Color::Magenta,
                &format!("⮌ relaying from {agent}: {}", truncate(&seed, 60)),
            ));
            self.start_stream(format!(
                "The following task was last being worked on in {agent}. Analyze where it \
                 left off, then continue and finish the unfinished work:\n\n{seed}"
            ))
        } else {
            None
        }
    }

    pub(crate) fn overlay_relay_menu(&self, composed: String) -> String {
        let Some(sel) = self.relay_menu else {
            return composed;
        };
        let tabs = self.relay_tabs();
        if tabs.is_empty() {
            return composed;
        }
        let width = self.width as usize;
        let active = tabs.get(self.relay_tab).copied().unwrap_or("");

        // Tab strip: each agent in its theme colour; the active one boxed.
        let mut strip = String::from("  ");
        for t in &tabs {
            let c = agent_color(t);
            if *t == active {
                strip.push_str(
                    &Style::new()
                        .fg(Color::Black)
                        .bg(c)
                        .bold()
                        .render(&format!(" {t} ")),
                );
            } else {
                strip.push_str(&Style::new().fg(c).render(&format!(" {t} ")));
            }
            strip.push(' ');
        }
        let mut menu = vec![
            pad_to(&strip, width),
            pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render("  ←/→ agent · ↑/↓ session · Enter continue · Esc"),
                width,
            ),
        ];

        let idxs = self.relay_tab_indices();
        let color = agent_color(active);
        if idxs.is_empty() {
            menu.push(pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render(&format!("    (no {active} sessions for this directory)")),
                width,
            ));
        }
        for (row, &gi) in idxs.iter().enumerate().take(12) {
            let s = &self.relay[gi];
            let raw = pad_to(
                &format!("  {}", truncate(&s.label, width.saturating_sub(4))),
                width,
            );
            menu.push(if row == sel.min(idxs.len().saturating_sub(1)) {
                Style::new().fg(Color::Black).bg(color).render(&raw)
            } else {
                Style::new().fg(color).render(&raw)
            });
        }
        self.overlay_list(composed, &menu)
    }
}
