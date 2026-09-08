//! Reviewer lane: async side-path drain + isolated side-session execution.
//!
//! Hard contract:
//! - Every reviewer task goes through `ReviewerLane`'s `a3s_lane::PriorityQueue`
//!   (manual `/review` priority 0, sticky reply review priority 1).
//! - Execution is asynchronous (`cmd::cmd`) on a dedicated session id and
//!   returns only via `Msg::Reviewer` — never the main AgentEvent pump.
//! - At most one reviewer side-session is in flight; finish drains the next
//!   queued job without touching the primary turn queue.

use super::*;
use a3s_code_core::hitl::TimeoutAction;

impl App {
    /// Enqueue onto the reviewer lane and kick the isolated drain loop.
    pub(super) fn enqueue_reviewer_job(
        &mut self,
        priority: a3s_lane::Priority,
        job: ReviewerJob,
    ) -> Option<Cmd<Msg>> {
        let _sequence = self.reviewer_lane.enqueue(priority, job.clone());
        let depth = self.reviewer_lane.pending() + usize::from(self.reviewer_lane.is_inflight());
        let origin = match job.origin {
            ReviewerOrigin::Manual => "review",
            ReviewerOrigin::Sticky => "sticky",
        };
        self.push_line(&Style::new().fg(TN_GRAY).render(
            &panels::review::reviewer_lane_enqueued_line(origin, &job.display, depth),
        ));
        self.drain_reviewer_lane()
    }

    /// Admit at most one reviewer side-session from the dedicated queue.
    pub(super) fn drain_reviewer_lane(&mut self) -> Option<Cmd<Msg>> {
        let (ticket, job) = self.reviewer_lane.claim_next()?;
        self.review_pending = true;
        self.review_pending_kind = Some(match job.origin {
            ReviewerOrigin::Sticky => panels::review::ReviewReportKind::Reply,
            ReviewerOrigin::Manual => panels::review::ReviewReportKind::Code,
        });
        self.push_line(&Style::new().fg(TN_GRAY).render(
            &panels::review::reviewer_started_line(&job.display),
        ));

        let agent = self.agent.clone();
        let workspace = self.cwd.clone();
        let model = self.model.clone();
        let llm_override = self.llm_override.clone();
        let effort = EFFORT_LEVELS
            .get(self.effort.min(EFFORT_LEVELS.len().saturating_sub(1)))
            .map(|level| level.id)
            .unwrap_or("medium");
        let code_config = Arc::clone(&self.code_config);
        let sandbox = self.execution_policy.sandbox_handle();
        let sandbox_available = self.execution_policy.sandbox_available();
        let bg_session_id = format!("bg-review-{}", ticket.id);
        let prompt = job.prompt;
        let origin = job.origin;

        Some(cmd::cmd(move || async move {
            let conf = a3s_code_core::hitl::ConfirmationPolicy::enabled()
                .with_timeout(BACKGROUND_CONFIRM_TIMEOUT_MS, TimeoutAction::Reject);
            let execution_policy = TuiExecutionPolicy::for_workspace_with_sandbox(
                Mode::Reviewer,
                PathBuf::from(&workspace),
                sandbox,
            );
            execution_policy.set_sandbox_available(sandbox_available);
            let opts = apply_launch_model_options(
                tui_session_options_with_gate_grants_and_execution(
                    conf,
                    DeepResearchReportToolGate::default(),
                    TuiPermissionGrants::default(),
                    execution_policy,
                )
                .with_prompt_slots(background_reviewer_prompt_slots(origin))
                .with_auto_compact(false)
                .with_session_id(bg_session_id.as_str()),
                model.as_deref(),
                llm_override.as_ref(),
                effort,
                code_config.as_ref(),
                &bg_session_id,
            );

            let text = match agent.session_async(workspace, Some(opts)).await {
                Ok(sess) => match sess.stream(&prompt, None).await {
                    Ok((mut rx, _join)) => {
                        let mut answer = String::new();
                        while let Some(ev) = rx.recv().await {
                            match ev {
                                AgentEvent::TextDelta { text } => answer.push_str(&text),
                                AgentEvent::End { text, .. } => {
                                    if answer.trim().is_empty() {
                                        answer = text;
                                    }
                                    break;
                                }
                                AgentEvent::Error { message } => {
                                    answer = format!("reviewer failed: {message}");
                                    break;
                                }
                                _ => {}
                            }
                        }
                        answer
                    }
                    Err(error) => format!("reviewer failed: {error}"),
                },
                Err(error) => format!("reviewer failed: {error}"),
            };
            Msg::Reviewer(ReviewerMsg::Finished { ticket, text })
        }))
    }

    pub(super) fn spawn_background_reviewer(
        &mut self,
        prompt: String,
        display: impl AsRef<str>,
    ) -> Option<Cmd<Msg>> {
        self.enqueue_reviewer_job(
            REVIEWER_MANUAL_PRIORITY,
            ReviewerJob {
                prompt,
                display: display.as_ref().to_string(),
                origin: ReviewerOrigin::Manual,
            },
        )
    }

    pub(super) fn maybe_spawn_sticky_background_reviewer(&mut self) -> Option<Cmd<Msg>> {
        let bundle = panels::workspace_review::latest_turn_evidence(&self.messages);
        if !panels::workspace_review::sticky_reply_reviewer_should_arm(
            self.mode == Mode::Reviewer,
            self.deep_research_loop.is_some(),
            self.sleep_pending,
            self.goal_run.is_some(),
            bundle.is_some(),
        ) {
            return None;
        }
        let bundle = bundle?;
        let (prompt, display) = panels::workspace_review::sticky_reply_review_prompt_and_display(
            Path::new(&self.cwd),
            &bundle,
        );
        self.enqueue_reviewer_job(
            REVIEWER_STICKY_PRIORITY,
            ReviewerJob {
                prompt,
                display,
                origin: ReviewerOrigin::Sticky,
            },
        )
    }

    pub(super) fn on_reviewer_msg(&mut self, msg: ReviewerMsg) -> Option<Cmd<Msg>> {
        match msg {
            ReviewerMsg::Finished { ticket, text } => {
                self.on_background_review_finished(ticket, text)
            }
        }
    }

    pub(super) fn on_background_review_finished(
        &mut self,
        ticket: BackgroundReviewTicket,
        text: String,
    ) -> Option<Cmd<Msg>> {
        if !self.reviewer_lane.accept(&ticket) {
            return None;
        }
        let trimmed = text.trim();
        match panels::review::classify_background_review_finish(trimmed) {
            panels::review::BackgroundReviewFinishKind::Empty => {
                self.review_pending = false;
                self.review_pending_kind = None;
                self.push_line(
                    &Style::new()
                        .fg(TN_YELLOW)
                        .render(panels::review::reviewer_empty_finish_line()),
                );
            }
            panels::review::BackgroundReviewFinishKind::Failed => {
                self.review_pending = false;
                self.review_pending_kind = None;
                self.push_line(&Style::new().fg(TN_RED).render(&format!(
                    "{}{trimmed}",
                    panels::review::reviewer_failed_finish_prefix()
                )));
            }
            panels::review::BackgroundReviewFinishKind::Capture => {
                self.capture_review(trimmed);
                if self.review_pending {
                    self.review_pending = false;
                    self.review_pending_kind = None;
                    let dim = |s: &str| format!("  {}", Style::new().fg(TN_GRAY).render(s));
                    let mut lines = vec![dim("⚖ reviewer bus")];
                    lines.extend(trimmed.lines().take(40).map(dim));
                    self.push_line(&lines.join("\n"));
                }
            }
        }
        // Keep the reviewer lane moving independently of the main stream.
        self.drain_reviewer_lane()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewer_lane_queues_while_inflight() {
        let mut lane = ReviewerLane::default();
        let (ticket, first) = {
            lane.enqueue(
                REVIEWER_MANUAL_PRIORITY,
                ReviewerJob {
                    prompt: "a".into(),
                    display: "first".into(),
                    origin: ReviewerOrigin::Manual,
                },
            );
            lane.claim_next().expect("claim first")
        };
        assert_eq!(first.display, "first");
        assert!(lane.is_inflight());
        lane.enqueue(
            REVIEWER_MANUAL_PRIORITY,
            ReviewerJob {
                prompt: "b".into(),
                display: "second".into(),
                origin: ReviewerOrigin::Manual,
            },
        );
        assert_eq!(lane.pending(), 1);
        assert!(lane.claim_next().is_none());
        assert!(lane.accept(&ticket));
        let (_, second) = lane.claim_next().expect("claim second");
        assert_eq!(second.display, "second");
    }

    #[test]
    fn sticky_reviews_coalesce_in_the_lane() {
        let mut lane = ReviewerLane::default();
        lane.enqueue(
            REVIEWER_STICKY_PRIORITY,
            ReviewerJob {
                prompt: "old".into(),
                display: "sticky-old".into(),
                origin: ReviewerOrigin::Sticky,
            },
        );
        lane.enqueue(
            REVIEWER_MANUAL_PRIORITY,
            ReviewerJob {
                prompt: "manual".into(),
                display: "manual".into(),
                origin: ReviewerOrigin::Manual,
            },
        );
        lane.enqueue(
            REVIEWER_STICKY_PRIORITY,
            ReviewerJob {
                prompt: "new".into(),
                display: "sticky-new".into(),
                origin: ReviewerOrigin::Sticky,
            },
        );
        // Manual outranks sticky; only one sticky remains.
        let (ticket, first) = lane.claim_next().unwrap();
        assert_eq!(first.display, "manual");
        assert_eq!(lane.pending(), 1);
        assert!(lane.accept(&ticket));
        let (_, sticky) = lane.claim_next().unwrap();
        assert_eq!(sticky.display, "sticky-new");
        assert_eq!(sticky.prompt, "new");
    }

    #[test]
    fn reviewer_priority_queue_admits_manual_before_sticky() {
        let mut lane = ReviewerLane::default();
        lane.enqueue(
            REVIEWER_STICKY_PRIORITY,
            ReviewerJob {
                prompt: "sticky".into(),
                display: "sticky".into(),
                origin: ReviewerOrigin::Sticky,
            },
        );
        lane.enqueue(
            REVIEWER_MANUAL_PRIORITY,
            ReviewerJob {
                prompt: "manual".into(),
                display: "manual".into(),
                origin: ReviewerOrigin::Manual,
            },
        );
        let (ticket, first) = lane.claim_next().expect("manual first");
        assert_eq!(first.origin, ReviewerOrigin::Manual);
        assert_eq!(first.display, "manual");
        assert!(lane.is_inflight());
        assert_eq!(lane.pending(), 1);
        // While in flight, further claims are blocked — async side-path serializes
        // execution even though the priority queue still holds sticky work.
        assert!(lane.claim_next().is_none());
        assert!(lane.accept(&ticket));
        let (_, sticky) = lane.claim_next().expect("sticky next");
        assert_eq!(sticky.origin, ReviewerOrigin::Sticky);
    }

    #[test]
    fn reviewer_main_stream_stays_default() {
        assert_eq!(Mode::Reviewer.main_stream_mode(), Mode::Default);
        assert_eq!(Mode::Plan.main_stream_mode(), Mode::Plan);
        assert!(Mode::Reviewer.agent_style().is_none());
        assert!(!Mode::Reviewer.is_readonly_specialty());
    }
}
