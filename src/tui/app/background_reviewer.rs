//! ReplyVerifierLane + GitReviewLane: async side-path drain + isolated execution.
//!
//! Hard contract:
//! - Sticky claim↔record goes through `ReplyVerifierLane` → Gate + executor
//!   (never `AgentStyle::CodeReview`, never the main AgentEvent pump).
//! - Explicit `/review` goes through `GitReviewLane` → CodeReview side-session.
//! - At most one reviewer job is in flight across both lanes (shared serial
//!   gate); git is preferred when both are queued.
//! - Finish returns only via `Msg::Reviewer`.

use super::*;
use a3s_code_core::hitl::TimeoutAction;
use app_reply_verifier::{reply_verifier_fail_closed_chrome, run_reply_verifier};
#[cfg(test)]
use app_reply_verifier::sticky_reply_verifier_identity;

impl App {
    fn reviewer_busy(&self) -> bool {
        self.reply_verifier_lane.is_inflight() || self.git_review_lane.is_inflight()
    }

    fn reviewer_depth(&self) -> usize {
        self.reply_verifier_lane.pending()
            + self.git_review_lane.pending()
            + usize::from(self.reviewer_busy())
    }

    /// Prefer git `/review`, then sticky reply verifier, when the serial gate is free.
    pub(super) fn drain_reviewer_lanes(&mut self) -> Option<Cmd<Msg>> {
        if self.reviewer_busy() {
            return None;
        }
        if let Some(cmd) = self.drain_git_review_lane() {
            return Some(cmd);
        }
        self.drain_reply_verifier_lane()
    }

    pub(super) fn enqueue_git_review_job(&mut self, job: GitReviewJob) -> Option<Cmd<Msg>> {
        let _sequence = self.git_review_lane.enqueue(job.clone());
        self.push_line(&Style::new().fg(TN_GRAY).render(
            &panels::review::reviewer_lane_enqueued_line("review", &job.display, self.reviewer_depth()),
        ));
        self.drain_reviewer_lanes()
    }

    pub(super) fn enqueue_reply_verifier_job(
        &mut self,
        job: ReplyVerifierJob,
    ) -> Option<Cmd<Msg>> {
        let _sequence = self.reply_verifier_lane.enqueue(job.clone());
        self.push_line(&Style::new().fg(TN_GRAY).render(
            &panels::review::reviewer_lane_enqueued_line("sticky", &job.display, self.reviewer_depth()),
        ));
        self.drain_reviewer_lanes()
    }

    fn drain_git_review_lane(&mut self) -> Option<Cmd<Msg>> {
        let (ticket, job) = self.git_review_lane.claim_next()?;
        self.review_pending = true;
        self.review_pending_kind = Some(panels::review::ReviewReportKind::Code);
        self.push_line(
            &Style::new()
                .fg(TN_GRAY)
                .render(&panels::review::reviewer_started_line(&job.display)),
        );

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
                .with_prompt_slots(git_review_side_session_prompt_slots())
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
            Msg::Reviewer(ReviewerMsg::Finished {
                lane: ReviewerLaneKind::Git,
                ticket,
                text,
            })
        }))
    }

    fn drain_reply_verifier_lane(&mut self) -> Option<Cmd<Msg>> {
        let (ticket, job) = self.reply_verifier_lane.claim_next()?;
        self.review_pending = true;
        self.review_pending_kind = Some(panels::review::ReviewReportKind::Reply);
        self.push_line(
            &Style::new()
                .fg(TN_GRAY)
                .render(&panels::review::reviewer_started_line(&job.display)),
        );

        let cwd = PathBuf::from(&self.cwd);
        let bundle = job.bundle;
        Some(cmd::cmd(move || async move {
            let text = run_reply_verifier(&cwd, bundle).await;
            Msg::Reviewer(ReviewerMsg::Finished {
                lane: ReviewerLaneKind::Reply,
                ticket,
                text,
            })
        }))
    }

    pub(super) fn spawn_background_reviewer(
        &mut self,
        prompt: String,
        display: impl AsRef<str>,
    ) -> Option<Cmd<Msg>> {
        self.enqueue_git_review_job(GitReviewJob {
            prompt,
            display: display.as_ref().to_string(),
        })
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
        let display = bundle.display_label();
        self.enqueue_reply_verifier_job(ReplyVerifierJob { bundle, display })
    }

    pub(super) fn on_reviewer_msg(&mut self, msg: ReviewerMsg) -> Option<Cmd<Msg>> {
        match msg {
            ReviewerMsg::Finished {
                lane,
                ticket,
                text,
            } => self.on_background_review_finished(lane, ticket, text),
        }
    }

    pub(super) fn on_background_review_finished(
        &mut self,
        lane: ReviewerLaneKind,
        ticket: BackgroundReviewTicket,
        text: String,
    ) -> Option<Cmd<Msg>> {
        let accepted = match lane {
            ReviewerLaneKind::Reply => self.reply_verifier_lane.accept(&ticket),
            ReviewerLaneKind::Git => self.git_review_lane.accept(&ticket),
        };
        if !accepted {
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
            panels::review::BackgroundReviewFinishKind::FailClosed => {
                // Do not capture a clean report; do not clear open sticky findings.
                self.review_pending = false;
                self.review_pending_kind = None;
                self.push_line(
                    &Style::new()
                        .fg(TN_YELLOW)
                        .render(reply_verifier_fail_closed_chrome()),
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
        self.drain_reviewer_lanes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::panels::workspace_review::{TurnEvidenceBundle, TurnEvidenceTool};

    fn sample_bundle(label: &str) -> TurnEvidenceBundle {
        TurnEvidenceBundle {
            user: format!("user-{label}"),
            assistant: format!("assistant-{label}"),
            tools: vec![TurnEvidenceTool {
                index: 1,
                name: "bash".into(),
                state: "succeeded".into(),
                args: "{}".into(),
                output: "ok".into(),
                exit_code: Some(0),
                truncated: false,
            }],
            complete: true,
        }
    }

    #[test]
    fn git_review_lane_queues_while_inflight() {
        let mut lane = GitReviewLane::default();
        let (ticket, first) = {
            lane.enqueue(GitReviewJob {
                prompt: "a".into(),
                display: "first".into(),
            });
            lane.claim_next().expect("claim first")
        };
        assert_eq!(first.display, "first");
        assert!(lane.is_inflight());
        lane.enqueue(GitReviewJob {
            prompt: "b".into(),
            display: "second".into(),
        });
        assert_eq!(lane.pending(), 1);
        assert!(lane.claim_next().is_none());
        assert!(lane.accept(&ticket));
        let (_, second) = lane.claim_next().expect("claim second");
        assert_eq!(second.display, "second");
    }

    #[test]
    fn sticky_reviews_coalesce_in_reply_lane() {
        let mut reply = ReplyVerifierLane::default();
        reply.enqueue(ReplyVerifierJob {
            bundle: sample_bundle("old"),
            display: "sticky-old".into(),
        });
        reply.enqueue(ReplyVerifierJob {
            bundle: sample_bundle("new"),
            display: "sticky-new".into(),
        });
        assert_eq!(reply.pending(), 1);
        let (_, sticky) = reply.claim_next().unwrap();
        assert_eq!(sticky.display, "sticky-new");
        assert_eq!(sticky.bundle.assistant, "assistant-new");
    }

    #[test]
    fn reply_and_git_lanes_do_not_share_queued_jobs() {
        let mut reply = ReplyVerifierLane::default();
        let mut git = GitReviewLane::default();
        reply.enqueue(ReplyVerifierJob {
            bundle: sample_bundle("sticky"),
            display: "sticky".into(),
        });
        git.enqueue(GitReviewJob {
            prompt: "manual".into(),
            display: "manual".into(),
        });
        reply.enqueue(ReplyVerifierJob {
            bundle: sample_bundle("sticky2"),
            display: "sticky2".into(),
        });
        assert_eq!(reply.pending(), 1);
        assert_eq!(git.pending(), 1);
        let (_, sticky) = reply.claim_next().unwrap();
        assert_eq!(sticky.display, "sticky2");
        let (_, manual) = git.claim_next().unwrap();
        assert_eq!(manual.display, "manual");
    }

    #[test]
    fn reviewer_main_stream_stays_default() {
        assert_eq!(Mode::Reviewer.main_stream_mode(), Mode::Default);
        assert_eq!(Mode::Plan.main_stream_mode(), Mode::Plan);
        assert!(Mode::Reviewer.agent_style().is_none());
        assert!(!Mode::Reviewer.is_readonly_specialty());
    }

    #[test]
    fn sticky_reply_verifier_does_not_use_code_review_style() {
        let (purpose, session_id) = sticky_reply_verifier_identity();
        assert_eq!(purpose, "cli.reply-verifier");
        assert_eq!(session_id, "cli-reply-verifier");
        assert!(!session_id.starts_with("bg-review-"));
        // Git `/review` alone owns CodeReview style on the side-session.
        let git_slots = git_review_side_session_prompt_slots();
        assert_eq!(
            git_slots.style,
            Some(a3s_code_core::AgentStyle::CodeReview)
        );
        // Sticky main-stream mode must not install CodeReview either.
        assert!(Mode::Reviewer.agent_style().is_none());
        let sticky_slots = background_reviewer_prompt_slots();
        assert!(sticky_slots.style.is_none());
    }
}
