//! `/relay` picker: resume A3S Code sessions or continue external-agent tasks.

use super::super::*;
use a3s_tui::components::{TabbedMenuItem, TabbedMenuPanel, TabbedMenuPanelMsg, TabbedMenuTab};

pub(crate) use super::relay_scan::RelaySession;
use super::relay_scan::{scan_relay_sessions, RelayAgent};

const RELAY_MAX_VISIBLE_ROWS: usize = 12;

pub(crate) struct RelayPanel {
    request_id: u64,
    sessions: Vec<RelaySession>,
    tab: usize,
    selected: usize,
    loading: bool,
    error: Option<String>,
}

impl RelayPanel {
    fn loading(request_id: u64) -> Self {
        Self {
            request_id,
            sessions: Vec::new(),
            tab: 0,
            selected: 0,
            loading: true,
            error: None,
        }
    }

    fn active_agent(&self) -> RelayAgent {
        RelayAgent::ALL[self.tab.min(RelayAgent::ALL.len() - 1)]
    }

    fn active_indices(&self) -> Vec<usize> {
        let agent = self.active_agent();
        self.sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| (session.agent == agent).then_some(index))
            .collect()
    }

    fn clamp_selection(&mut self) {
        self.tab = self.tab.min(RelayAgent::ALL.len() - 1);
        self.selected = self
            .selected
            .min(self.active_indices().len().saturating_sub(1));
    }
}

fn relay_agent_color(agent: RelayAgent) -> Color {
    match agent {
        RelayAgent::A3sCode => ACCENT,
        RelayAgent::ClaudeCode => TN_ORANGE,
        RelayAgent::Codex => TN_CYAN,
        RelayAgent::WorkBuddy => TN_PURPLE,
    }
}

fn relay_max_rows(height: usize) -> usize {
    height.saturating_sub(8).clamp(3, RELAY_MAX_VISIBLE_ROWS)
}

fn relay_panel_height(panel: &RelayPanel, max_items: usize) -> usize {
    let item_count = panel.active_indices().len().max(1).min(max_items);
    // Title + tab strip + hint + rows + footer spacing.
    4 + item_count
}

fn relay_menu_panel(panel: &RelayPanel, max_items: usize) -> TabbedMenuPanel {
    let empty_text = if panel.loading {
        "(scanning sessions…)".to_string()
    } else if let Some(error) = &panel.error {
        format!("(scan failed: {})", truncate(error, 72))
    } else {
        "(no sessions for this workspace)".to_string()
    };
    let tabs = RelayAgent::ALL
        .into_iter()
        .map(|agent| {
            let items = panel
                .sessions
                .iter()
                .filter(|session| session.agent == agent)
                .map(|session| TabbedMenuItem::new(session.label.clone()).prefix("⮌"))
                .collect::<Vec<_>>();
            TabbedMenuTab::new(agent.label(), relay_agent_color(agent))
                .items(items)
                .empty_text(empty_text.clone())
        })
        .collect::<Vec<_>>();

    TabbedMenuPanel::new(tabs)
        .title("Resume or relay session")
        .hint("↑/↓ session · ←/→ agent · Enter continue · Esc")
        .active_tab(panel.tab)
        .selected(panel.selected)
        .max_items(max_items)
        .indent(2)
        .hint_color(TN_GRAY)
        .text_color(TN_FG)
        .muted_color(TN_GRAY)
        .selected_colors(TN_FG, SURFACE_SELECTED)
}

fn relay_menu_lines(panel: &RelayPanel, width: usize, max_items: usize) -> Vec<String> {
    relay_menu_panel(panel, max_items)
        .view(
            width.min(u16::MAX as usize) as u16,
            relay_panel_height(panel, max_items),
        )
        .lines()
        .map(str::to_string)
        .collect()
}

fn relay_overlay_y_offset(screen_height: usize, row_count: usize, rows_below: usize) -> u16 {
    screen_height
        .saturating_sub(rows_below)
        .saturating_sub(row_count)
        .min(u16::MAX as usize) as u16
}

impl App {
    pub(crate) fn open_relay_panel(&mut self) -> Option<Cmd<Msg>> {
        self.textarea.clear();
        if self.goal_run.is_some() {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  stop the active goal with /goal clear before switching sessions"),
            );
            return None;
        }
        self.relay_scan_seq = self.relay_scan_seq.wrapping_add(1).max(1);
        let request_id = self.relay_scan_seq;
        self.relay_panel = Some(RelayPanel::loading(request_id));
        let store = Arc::clone(&self.store);
        let workspace = PathBuf::from(&self.cwd);
        Some(cmd::cmd(move || async move {
            Msg::RelayData {
                request_id,
                result: scan_relay_sessions(store, workspace).await,
            }
        }))
    }

    pub(crate) fn apply_relay_scan(
        &mut self,
        request_id: u64,
        result: Result<Vec<RelaySession>, String>,
    ) {
        let Some(panel) = self.relay_panel.as_mut() else {
            return;
        };
        if panel.request_id != request_id {
            return;
        }
        panel.loading = false;
        match result {
            Ok(sessions) => {
                panel.sessions = sessions;
                panel.error = None;
            }
            Err(error) => {
                panel.sessions.clear();
                panel.error = Some(error);
            }
        }
        panel.clamp_selection();
    }

    pub(crate) fn handle_relay_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let panel = self.relay_panel.as_mut()?;
        let last = panel.active_indices().len().saturating_sub(1);
        match key.code {
            KeyCode::Up => panel.selected = panel.selected.saturating_sub(1),
            KeyCode::Down => panel.selected = (panel.selected + 1).min(last),
            KeyCode::Left => {
                panel.tab = panel.tab.saturating_sub(1);
                panel.selected = 0;
            }
            KeyCode::Right | KeyCode::Tab => {
                panel.tab = (panel.tab + 1).min(RelayAgent::ALL.len() - 1);
                panel.selected = 0;
            }
            KeyCode::Enter => {
                let session = panel
                    .active_indices()
                    .get(panel.selected.min(last))
                    .and_then(|index| panel.sessions.get(*index))
                    .cloned();
                return session.and_then(|session| self.activate_relay_session(session));
            }
            KeyCode::Esc => self.relay_panel = None,
            _ => {}
        }
        None
    }

    pub(crate) fn handle_relay_mouse(&mut self, mouse: &MouseEvent) -> Option<Cmd<Msg>> {
        let panel = self.relay_panel.as_ref()?;
        let max_items = relay_max_rows(self.height as usize);
        let height = relay_panel_height(panel, max_items);
        let mut menu = relay_menu_panel(panel, max_items);
        let row_count = menu.view(self.width, height).lines().count();
        if row_count == 0 {
            return None;
        }
        menu.set_y_offset(relay_overlay_y_offset(
            self.height as usize,
            row_count,
            self.overlay_rows_below(),
        ));
        match menu.handle_mouse(mouse) {
            Some(TabbedMenuPanelMsg::TabChanged(tab)) => {
                if let Some(panel) = self.relay_panel.as_mut() {
                    panel.tab = tab.min(RelayAgent::ALL.len() - 1);
                    panel.selected = 0;
                }
                None
            }
            Some(TabbedMenuPanelMsg::Selected { tab, item }) => {
                let session = self.relay_panel.as_ref().and_then(|panel| {
                    let agent = RelayAgent::ALL[tab.min(RelayAgent::ALL.len() - 1)];
                    panel
                        .sessions
                        .iter()
                        .filter(|session| session.agent == agent)
                        .nth(item)
                        .cloned()
                });
                session.and_then(|session| self.activate_relay_session(session))
            }
            Some(TabbedMenuPanelMsg::Cancelled) | None => None,
        }
    }

    fn activate_relay_session(&mut self, target: RelaySession) -> Option<Cmd<Msg>> {
        self.relay_panel = None;
        if let Some(session_id) = target.native_id.as_deref() {
            if session_id == self.session_id {
                self.push_line(
                    &Style::new()
                        .fg(TN_GRAY)
                        .render("  this A3S Code session is already active"),
                );
                return None;
            }
            if let Err(error) = self.persist_tui_session_state() {
                self.push_line(&Style::new().fg(TN_RED).render(&format!(
                    "  could not save current session settings before relay: {error}"
                )));
                return None;
            }
            let (profile, restore) = match self.relay_restore_profile(&target) {
                Ok(result) => result,
                Err(error) => {
                    self.push_line(
                        &Style::new()
                            .fg(TN_RED)
                            .render(&format!("  could not resume relay session: {error}")),
                    );
                    return None;
                }
            };
            return self.start_session_rebuild(profile, SessionRebuildAction::Relay { restore });
        }

        let Some(seed) = target.seed else {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  the selected transcript has no user task to relay"),
            );
            return None;
        };
        let source = target.agent.label();
        self.messages.push(TranscriptEntry::preformatted(gutter(
            relay_agent_color(target.agent),
            &format!("⮌ relaying from {source}: {}", truncate(&seed, 72)),
        )));
        let prompt = format!(
            "The following unfinished task comes from a {source} session in this workspace. \
             Inspect the current workspace state, determine what remains unfinished, then continue \
             and complete the task without assuming that earlier edits were applied:\n\n{seed}"
        );
        self.start_stream_inner(
            prompt,
            format!("⮌ {source}: {}", truncate(&seed, 56)),
            true,
            false,
            false,
        )
    }

    fn relay_restore_profile(
        &self,
        target: &RelaySession,
    ) -> Result<(SessionRebuildProfile, RelayRestoreState), String> {
        let session_id = target
            .native_id
            .clone()
            .ok_or_else(|| "native session id is missing".to_string())?;
        let state = load_tui_session_state(Path::new(&self.cwd), &session_id)
            .map_err(|error| format!("could not load its TUI settings: {error}"))?;
        let effort = state
            .as_ref()
            .and_then(TuiSessionState::effort_index)
            .or_else(load_tui_effort_preference)
            .unwrap_or(DEFAULT_TUI_EFFORT_INDEX)
            .min(EFFORT_LEVELS.len().saturating_sub(1));
        let configured =
            app_launch::configured_model_preference(target.persisted_model.clone(), &self.models);
        let global = load_model_selection_preference().filter(|preference| {
            target.persisted_model.as_deref().is_none_or(|model| {
                app_launch::preference_matches_persisted_model(preference, model)
            })
        });
        let preference = state
            .as_ref()
            .and_then(|state| state.model.clone())
            .or(configured)
            .or(global);
        let restored = preference.as_ref().and_then(|preference| {
            restore_model_selection(
                preference,
                &self.models,
                self.os_session.as_ref(),
                &session_id,
                effort,
            )
        });
        let model_source = restored
            .as_ref()
            .and(preference.as_ref())
            .map(|preference| preference.source)
            .unwrap_or(ModelSelectionSource::Config);
        let model = restored
            .as_ref()
            .map(|(model, _)| model.clone())
            .or_else(|| self.code_config.default_model.clone())
            .or_else(|| self.models.first().cloned());
        let llm_override = restored.and_then(|(_, client)| client);
        let context_limit = model
            .as_deref()
            .map(|model| ctx_limit_for_model(&self.model_ctx, model))
            .unwrap_or_else(|| resolve_ctx_limit(None));
        let restore = RelayRestoreState {
            session_id: session_id.clone(),
            model: model.clone(),
            model_source,
            effort,
            mode: state
                .as_ref()
                .map(TuiSessionState::mode)
                .unwrap_or(Mode::Default),
            context_limit,
            llm_override: llm_override.clone(),
            theme: state.as_ref().and_then(TuiSessionState::theme_index),
            paused_goal: state.and_then(|state| state.paused_goal),
        };
        let profile = SessionRebuildProfile {
            session_id,
            model,
            effort,
            context_limit,
            llm_override,
            compact_summary: None,
        };
        Ok((profile, restore))
    }

    pub(crate) fn commit_relay_session(
        &mut self,
        session: AgentSession,
        restore: RelayRestoreState,
    ) {
        let resumed = session.history();
        let entries = app_launch::resumed_transcript_entries(&resumed);
        let history = resumed
            .iter()
            .filter(|message| message.role == "user")
            .map(|message| message.text().trim().to_string())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>();
        let auto_review_revision = u64::try_from(history.len()).unwrap_or(u64::MAX);

        self.restore_autonomy();
        self.session_id = restore.session_id;
        self.model = restore.model;
        self.model_source = restore.model_source;
        self.effort = restore.effort;
        self.active_turn_mode = None;
        self.set_composer_mode(restore.mode);
        self.context_limit = restore.context_limit;
        self.llm_override = restore.llm_override;
        if let Some(theme) = restore.theme {
            SYNTAX_THEME.store(
                theme.min(THEMES.len().saturating_sub(1)),
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        self.replace_session(session);
        self.messages = Transcript::from_entries(entries);
        self.history = history;
        self.history_pos = None;
        self.history_draft = None;
        self.auto_review = AutoReviewTracker::new(auto_review_revision);
        self.compact_summary = None;
        self.output_tokens = 0;
        self.last_prompt_tokens = 0;
        self.ctx_warned_tier = 0;
        self.completed = 0;
        self.plan.clear();
        self.runtime.clear_turn_entities();
        self.runtime.clear_subagent_entities();
        self.background_subagent_watches.clear();
        self.invalidate_subagent_snapshots();
        self.queue.clear();
        self.queued_turn_modes.clear();
        self.queued_plan_drafts.clear();
        self.active_queued_turn = None;
        self.active_queued_turn_token = None;
        self.active_turn_mode = None;
        self.active_plan_draft = None;
        self.pending_plan_review = None;
        self.plan_review = None;
        self.queue_retry_generation = self.queue_retry_generation.wrapping_add(1);
        self.queue_retry_attempt = 0;
        self.running_task = None;
        self.restore_current_approval_feedback();
        self.pending_tools.clear();
        self.permission_rule_write_inflight = None;
        self.pending_images.clear();
        self.pending_ctx = None;
        self.ctx_hits.clear();
        self.review = None;
        self.review_open = false;
        self.review_pending = false;
        self.sleep_pending = false;
        self.loop_remaining = 0;
        self.loop_panel = None;
        self.goal = None;
        self.goal_since = None;
        self.goal_run = None;
        self.pending_goal_failure = None;
        self.paused_goal = restore.paused_goal;
        self.goal_resume_prompt = self.paused_goal.as_ref().map(|_| 0);
        self.agent_dev = None;
        self.pending_agent_subcommand = None;
        self.mcp_dev = None;
        self.pending_mcp_subcommand = None;
        self.skill_dev = None;
        self.pending_skill_subcommand = None;
        self.okf_dev = None;
        self.pending_okf_subcommand = None;
        self.streaming.clear();
        self.thinking.clear();
        self.turn_text.clear();
        self.transcript_view = None;
        self.push_line(
            &Style::new()
                .fg(TN_GREEN)
                .render(&format!("  ⮌ resumed A3S Code session {}", self.session_id)),
        );
        self.relayout();
        self.rebuild_viewport();
    }

    pub(crate) fn overlay_relay_menu(&self, composed: String) -> String {
        let Some(panel) = self.relay_panel.as_ref() else {
            return composed;
        };
        let menu = relay_menu_lines(
            panel,
            self.width as usize,
            relay_max_rows(self.height as usize),
        );
        self.overlay_list(composed, &menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_tui::style::visible_len;

    #[test]
    fn relay_tabs_include_workbuddy() {
        assert_eq!(
            RelayAgent::ALL.map(RelayAgent::label),
            ["A3S Code", "Claude Code", "Codex", "WorkBuddy"]
        );
    }

    #[test]
    fn relay_menu_rows_are_bounded_to_the_terminal_width() {
        let panel = RelayPanel {
            request_id: 1,
            sessions: vec![RelaySession {
                agent: RelayAgent::WorkBuddy,
                native_id: None,
                seed: Some("task".to_string()),
                label: "a very long WorkBuddy task label that must be clipped safely".to_string(),
                modified: UNIX_EPOCH,
                persisted_model: None,
            }],
            tab: 3,
            selected: 0,
            loading: false,
            error: None,
        };

        for line in relay_menu_lines(&panel, 32, 12) {
            assert!(visible_len(&line) <= 32, "{line:?}");
        }
    }
}
