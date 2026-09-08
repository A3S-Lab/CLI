//! Stream interruption controls used by keyboard and composer actions.

use super::*;

impl App {
    pub(in crate::tui) fn begin_stream_interrupt(&mut self, reason: &str) -> Option<Cmd<Msg>> {
        self.begin_stream_interrupt_with_goal_policy(reason, true)
    }

    pub(in crate::tui) fn begin_send_now_interrupt(&mut self) -> Option<Cmd<Msg>> {
        self.begin_stream_interrupt_with_goal_policy("superseded by Send now", false)
    }

    fn begin_stream_interrupt_with_goal_policy(
        &mut self,
        reason: &str,
        cancel_goal: bool,
    ) -> Option<Cmd<Msg>> {
        if !self.stream_stop_available() {
            return None;
        }
        let goal_cancelled = cancel_goal && self.cancel_goal_state(reason);
        self.interrupting = true;
        if self.stream_join.is_none() && self.rx.is_none() && !self.host_progress_inflight {
            self.interrupted_stream_start_token = Some(self.stream_start_token);
        }
        self.stream_start_token = self.stream_start_token.wrapping_add(1);
        self.deep_research_stream_timeout_token =
            self.deep_research_stream_timeout_token.wrapping_add(1);
        let status_entry =
            self.push_tracked_line(&Style::new().fg(TN_YELLOW).render("  ⎋ interrupting…"));
        let session = self.session.clone();
        let join = self.stream_join.take();
        let host_abort = self.host_tool_abort.take();
        let deep_research = self.deep_research_loop.is_some();
        let deep_research_handle = self.deep_research_handle.take();
        self.deep_research_events = None;
        self.host_progress_inflight = false;
        Some(cmd::cmd(move || async move {
            if let Some(host_abort) = host_abort {
                host_abort.abort();
            }
            if let Some(handle) = deep_research_handle {
                let _ = handle.cancel_and_settle().await;
            } else if !deep_research {
                let _ = session
                    .cancel_and_settle(
                        Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                        Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                    )
                    .await;
            }
            if let Some(join) = join {
                let _ = settle_stream_join_for_quit(
                    join,
                    Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                )
                .await;
            }
            Msg::Interrupted {
                goal_cancelled,
                status_entry,
            }
        }))
    }

    pub(in crate::tui) fn stream_stop_available(&self) -> bool {
        self.state == State::Streaming
            && !self.interrupting
            && !self.stream_join_settling
            && !self.deep_research_subagent_settlement_inflight
    }
}
