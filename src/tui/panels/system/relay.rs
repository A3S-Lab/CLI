//! `/relay` panel: resumable/relayable sessions across coding agents.

use super::super::*;

fn relay_line(rendered: &str, width: usize) -> String {
    pad_to(&truncate(rendered, width), width)
}

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
            let mut opts = with_recent_workspace_context(
                tui_session_options(self.confirmation.clone())
                    .with_session_store(self.store.clone())
                    .with_session_id(id.as_str())
                    .with_workspace_backend(self.workspace_services.clone())
                    .with_auto_save(true),
                &self.workspace_manifest,
            );
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
                                self.messages.push(gutter(TN_GREEN, &md.view()));
                            }
                            _ => {}
                        }
                    }
                    self.push_line(
                        &Style::new()
                            .fg(TN_GREEN)
                            .render(&format!("  ⮌ resumed a3s-code session {id}")),
                    );
                }
                Err(e) => self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  failed to resume: {e}")),
                ),
            }
            None
        } else if let Some(seed) = seed {
            if self.state != State::Idle {
                self.push_line(
                    &Style::new()
                        .fg(TN_YELLOW)
                        .render("  finish the current turn before relaying"),
                );
                return None;
            }
            self.messages.push(gutter(
                TN_PURPLE,
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
            relay_line(&strip, width),
            relay_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render("  ←/→ agent · ↑/↓ session · Enter continue · Esc"),
                width,
            ),
        ];

        let idxs = self.relay_tab_indices();
        let color = agent_color(active);
        if idxs.is_empty() {
            menu.push(relay_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render(&format!("    (no {active} sessions for this directory)")),
                width,
            ));
        }
        // Scroll a window around the selection so a session past row 12 stays
        // visible and reachable (the list used to render a fixed first-12 only).
        let total = idxs.len();
        let sel = sel.min(total.saturating_sub(1));
        let max_rows = (self.height as usize).saturating_sub(8).clamp(3, 12);
        let start = if sel < max_rows {
            0
        } else {
            sel + 1 - max_rows
        };
        let end = (start + max_rows).min(total);
        for (row, &gi) in idxs.iter().enumerate().take(end).skip(start) {
            let s = &self.relay[gi];
            let raw = relay_line(&format!("  {}", s.label), width);
            menu.push(if row == sel {
                Style::new().fg(Color::Black).bg(color).render(&raw)
            } else {
                Style::new().fg(color).render(&raw)
            });
        }
        if total > max_rows {
            menu.push(relay_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render(&format!("  {}/{total}", sel + 1)),
                width,
            ));
        }
        self.overlay_list(composed, &menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_lines_are_width_bounded_with_styles() {
        let line = relay_line(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render("  claude code codex a3s-code with an extra long tab strip and help text"),
            34,
        );

        assert!(
            a3s_tui::style::visible_len(&line) <= 34,
            "{}",
            a3s_tui::style::strip_ansi(&line)
        );
    }
}
