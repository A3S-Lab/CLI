//! Main Code TUI controller actions and agent-event projection.

use super::*;

impl App {
    pub(super) fn show_session_status(&mut self) {
        self.refresh_workspace_retrieval_status();
        let active_mode = self.active_turn_mode.unwrap_or(self.mode);
        let activity = match self.state {
            State::Idle => "idle".to_string(),
            State::Streaming => self
                .running_task
                .as_deref()
                .map(|task| format!("running {}", truncate(task, 48)))
                .unwrap_or_else(|| "running".to_string()),
            State::Awaiting => "awaiting permission decision".to_string(),
            State::Rebuilding => "rebuilding session".to_string(),
        };
        let os_account = match (&self.os_config, &self.os_session) {
            (None, _) => "not configured".to_string(),
            (Some(_), Some(session)) => format!("signed in as {}", session.display_label()),
            (Some(_), None) => "signed out".to_string(),
        };
        let mut active = Vec::new();
        if let Some(goal) = self.goal.as_deref() {
            active.push(format!("goal:{}", truncate(goal, 48)));
        }
        if self.loop_remaining > 0 {
            active.push(format!("loop:{} turns left", self.loop_remaining));
        }

        let report = SessionStatusReport {
            session_id: self.session_id.clone(),
            workspace: self.cwd.clone(),
            branch: self.branch.clone(),
            model: self
                .model
                .clone()
                .unwrap_or_else(|| "resolving provider model".to_string()),
            effort: EFFORT_LEVELS[self.effort.min(EFFORT_LEVELS.len().saturating_sub(1))]
                .label
                .to_string(),
            active_mode,
            next_mode: self.mode,
            context_limit: self.context_limit as usize,
            prompt_tokens: self.last_prompt_tokens,
            output_tokens: self.output_tokens,
            activity,
            queued_turns: self.queue.ordered().len(),
            os_account,
            workspace_retrieval: crate::workspace_retrieval::workspace_retrieval_status_report(
                &self.workspace_retrieval_status,
            ),
            active_scope: if active.is_empty() {
                "none".to_string()
            } else {
                active.join(" · ")
            },
        };
        self.textarea.clear();
        self.push_line(&render_session_status_report(
            &report,
            self.viewport_content_width(),
        ));
    }

    pub(super) fn show_sandbox_status(&mut self) {
        let report = SandboxStatusReport {
            handle_attached: self.execution_policy.sandbox_handle().is_some(),
            verified: self.execution_policy.sandbox_available(),
            mode: self.active_turn_mode.unwrap_or(self.mode),
        };
        self.textarea.clear();
        self.push_line(&render_sandbox_status_report(
            &report,
            self.viewport_content_width(),
        ));
    }

    pub(super) fn session_status_line(&self, width: usize) -> String {
        let base = render_session_status_line(
            &self.cwd,
            self.branch.as_deref(),
            self.model.as_deref(),
            self.context_limit,
            self.last_prompt_tokens,
            self.output_tokens,
            self.session_status_chips(),
            width,
        );
        merge_status_line_extension(&base, self.status_line_extension.as_deref(), width)
    }

    /// Transient work meter folded into the status line (replaces the
    /// old dedicated activity row above the composer).
    pub(super) fn working_meter_chip(&self) -> Option<SessionStatusChip> {
        if self.updating.is_some() {
            return Some(
                SessionStatusChip::new("⬇", "checking for updates…").color(COMPOSER_CHROME.success),
            );
        }
        if self.compacting.is_some() {
            return Some(SessionStatusChip::new("⟳", "compacting…").color(COMPOSER_CHROME.active));
        }
        // Deferred startup work is silent once the first frame is up.
        // Live turn progress renders on the ephemeral working line above the
        // PromptBar. Keep the quiet footer free of Working… noise.
        match self.state {
            State::Streaming | State::Rebuilding => None,
            State::Awaiting | State::Idle => None,
        }
    }

    /// working label for the ephemeral line above the composer.
    pub(super) fn ephemeral_working_label(&self) -> Option<String> {
        if self.updating.is_some() {
            return Some("checking for updates…".to_string());
        }
        if self.compacting.is_some() {
            return Some("compacting…".to_string());
        }
        // Do not surface deferred startup as "starting…" after first paint.
        match self.state {
            State::Streaming => {
                let tool = self.runtime.latest_active_tool();
                let verb = tool.map(|tool| tool_running_verb(&tool.name));
                let detail = tool.and_then(|tool| {
                    tool.args()
                        .as_ref()
                        .and_then(|args| arg_summary_for_tool(&tool.name, args))
                });
                let elapsed = self.stream_started.map(|t0| t0.elapsed());
                Some(compose_working_label(
                    &self.thinking,
                    verb,
                    detail.as_deref(),
                    &self.core_run_status.activity_label(),
                    elapsed,
                ))
            }
            State::Rebuilding => Some("Updating session…".to_string()),
            State::Awaiting | State::Idle => None,
        }
    }

    pub(super) fn session_status_chips(&self) -> Vec<SessionStatusChip> {
        let effective_mode = self.active_turn_mode.unwrap_or(self.mode);
        let mut chips = Vec::new();
        if let Some(working) = self.working_meter_chip() {
            chips.push(working);
        }
        chips.push(mode_status_chip(effective_mode));
        if self
            .active_turn_mode
            .is_some_and(|active| active != self.mode)
        {
            chips.push(
                SessionStatusChip::new("↪", format!("next:{}", self.mode.name()))
                    .color(COMPOSER_CHROME.secondary),
            );
        }
        // Sticky skill is mode-like session state: keep it visible even in Zen.
        if let Some(skill) = self.sticky_skill.as_deref() {
            chips.push(sticky_skill_status_chip(skill));
        }
        if self.display_profile == DisplayProfile::Zen {
            return chips;
        }
        if self.plan_review.is_some() {
            chips.push(SessionStatusChip::new("◆", "plan review").color(COMPOSER_CHROME.active));
        }

        let include_retrieval = self.display_profile == DisplayProfile::Default;
        let goal_chip = self
            .goal
            .as_ref()
            .map(|_| goal_status_chip(self.goal_since));
        if include_retrieval {
            append_retrieval_and_goal_chips(
                &mut chips,
                &self.workspace_retrieval_status,
                goal_chip,
            );
        } else if let Some(goal) = goal_chip {
            chips.push(goal);
        }
        if self.display_profile == DisplayProfile::Compact {
            if let Some(version) = self.update_available.as_deref() {
                chips.push(SessionStatusChip::new("⬆", version).color(COMPOSER_CHROME.warning));
            }
            return chips;
        }
        if self.loop_remaining > 0 {
            chips.push(
                SessionStatusChip::new("↻", self.loop_remaining.to_string())
                    .color(COMPOSER_CHROME.secondary),
            );
        }
        if let Some(version) = self.update_available.as_deref() {
            chips.push(SessionStatusChip::new("⬆", version).color(COMPOSER_CHROME.warning));
        }

        chips
    }

    pub(super) fn refresh_workspace_retrieval_status(&mut self) {
        self.workspace_retrieval_status = self.session.workspace_retrieval_status();
    }
}

pub(super) fn goal_status_chip(since: Option<Instant>) -> SessionStatusChip {
    let label = since
        .map(|started| format!("goal · {}", fmt_elapsed(started.elapsed())))
        .unwrap_or_else(|| "goal".to_string());
    SessionStatusChip::new("◎", label).color(COMPOSER_CHROME.active)
}

/// Footer chip for sticky `$skill` (Cursor custom-mode spirit).
pub(super) fn sticky_skill_status_chip(skill: &str) -> SessionStatusChip {
    SessionStatusChip::new("$", format!("sticky:{skill}")).color(COMPOSER_CHROME.active)
}

pub(super) fn workspace_retrieval_status_chip(
    status: &WorkspaceRetrievalStatus,
) -> Option<SessionStatusChip> {
    let (label, color) = match status.phase {
        WorkspaceRetrievalPhase::Disabled => return None,
        WorkspaceRetrievalPhase::Building if status.indexed_chunks > 0 => (
            format!("retrieval {}%", status.coverage_bps.min(10_000) / 100),
            COMPOSER_CHROME.active,
        ),
        WorkspaceRetrievalPhase::Building => {
            ("retrieval building".to_string(), COMPOSER_CHROME.active)
        }
        WorkspaceRetrievalPhase::Ready => ("retrieval ready".to_string(), COMPOSER_CHROME.success),
        WorkspaceRetrievalPhase::Degraded => {
            ("retrieval degraded".to_string(), COMPOSER_CHROME.warning)
        }
        WorkspaceRetrievalPhase::Closed => ("retrieval closed".to_string(), COMPOSER_CHROME.error),
    };
    Some(SessionStatusChip::new("⌕", label).color(color))
}

pub(super) fn append_retrieval_and_goal_chips(
    chips: &mut Vec<SessionStatusChip>,
    status: &WorkspaceRetrievalStatus,
    goal: Option<SessionStatusChip>,
) {
    let retrieval = workspace_retrieval_status_chip(status);
    if status.phase == WorkspaceRetrievalPhase::Ready {
        chips.extend(goal);
        chips.extend(retrieval);
    } else {
        chips.extend(retrieval);
        chips.extend(goal);
    }
}

#[cfg(test)]
mod tests {
    use super::sticky_skill_status_chip;

    #[test]
    fn sticky_skill_chip_label_matches_footer_contract() {
        let chip = sticky_skill_status_chip("review");
        assert_eq!(chip.glyph(), "$");
        assert_eq!(chip.label(), "sticky:review");
    }
}
