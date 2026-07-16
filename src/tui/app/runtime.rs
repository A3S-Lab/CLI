//! Agent stream lifecycle, queue draining, and autonomous-run settlement.

use super::*;

fn take_matching_queue_claim<T>(
    active: &mut Option<PriorityItem<T>>,
    active_token: &mut Option<u64>,
    token: u64,
) -> Option<PriorityItem<T>> {
    if *active_token != Some(token) {
        return None;
    }
    *active_token = None;
    active.take()
}

impl App {
    pub(super) fn start_stream_inner(
        &mut self,
        prompt: String,
        display_task: String,
        clear_turn_artifacts: bool,
        include_attachments: bool,
        synthesis: bool,
    ) -> Option<Cmd<Msg>> {
        self.start_stream_inner_with_runtime(
            prompt,
            display_task,
            clear_turn_artifacts,
            include_attachments,
            synthesis,
            None,
        )
    }

    pub(super) fn start_stream_inner_with_runtime(
        &mut self,
        prompt: String,
        display_task: String,
        clear_turn_artifacts: bool,
        include_attachments: bool,
        synthesis: bool,
        runtime_expectation: Option<RuntimeExpectation>,
    ) -> Option<Cmd<Msg>> {
        let submitted_images = if include_attachments {
            std::mem::take(&mut self.pending_images)
        } else {
            Vec::new()
        };
        self.start_stream_inner_with_runtime_and_images(
            prompt,
            display_task,
            clear_turn_artifacts,
            synthesis,
            runtime_expectation,
            submitted_images,
        )
    }

    fn start_stream_inner_with_runtime_and_images(
        &mut self,
        prompt: String,
        display_task: String,
        clear_turn_artifacts: bool,
        synthesis: bool,
        runtime_expectation: Option<RuntimeExpectation>,
        submitted_images: Vec<PendingImage>,
    ) -> Option<Cmd<Msg>> {
        let attachments = match submitted_images
            .iter()
            .map(PendingImage::attachment)
            .collect::<std::io::Result<Vec<_>>>()
        {
            Ok(attachments) => attachments,
            Err(error) => {
                restore_submitted_images(&mut self.pending_images, submitted_images);
                self.push_notice(
                    NoticeKind::Error,
                    format!("Image attachment could not be read: {error}"),
                );
                self.relayout();
                return None;
            }
        };
        if clear_turn_artifacts && !synthesis {
            self.auto_review.on_user_turn();
            self.last_activity = Instant::now();
        }
        self.streaming.clear();
        self.llm_turn_checkpoint = None;
        self.got_delta = false; // track if this turn streamed any text deltas
        self.turn_text.clear();
        self.turn_had_agent_activity = false;
        self.turn_text_after_activity = false;
        if let Some(expectation) = runtime_expectation {
            self.runtime_expectation = Some(expectation);
        }
        self.stream_join_settling = false;
        self.stream_settle_abort = None;
        self.ultracode_synthesis_inflight = synthesis;
        if !synthesis {
            self.ultracode_synthesis_used = false;
        }
        self.last_paint = None; // first delta of the turn paints immediately
        self.viewport.set_auto_scroll(true); // sending a message jumps to latest
        if clear_turn_artifacts {
            self.plan.clear(); // fresh plan per user turn; planning events refill it

            // Keep completed agents visible until the next user turn; a fresh
            // user turn starts a fresh runtime-entity projection.
            self.runtime.clear_turn_entities();
            self.runtime.set_subagent_task(display_task.clone());
        } else {
            self.runtime.clear_live_tools();
        }
        self.running_task = Some(display_task);
        self.state = State::Streaming;
        self.relayout();
        self.stream_started = Some(Instant::now());
        self.spinner.start();
        self.rebuild_viewport();
        let session = self.session.clone();
        // Keep the agent aligned with the standing goal (display stays clean).
        // Internal synthesis keeps its marker first so Core can reliably
        // suppress runtime auto-delegation for that final-answer-only turn.
        let prompt = match (&self.goal, synthesis) {
            (_, true) => prompt,
            (Some(g), false) => format!("[Ongoing goal: {g}]\n\n{prompt}"),
            (None, false) => prompt,
        };
        self.stream_start_token = self.stream_start_token.wrapping_add(1);
        let stream_start_token = self.stream_start_token;
        let deep_research_timeout = if let Some(loop_state) = self.deep_research_loop.as_mut() {
            if !self.host_progress_inflight {
                let now = Instant::now();
                loop_state.phase_started_at = Some(now);
                self.deep_research_stream_timeout_token =
                    self.deep_research_stream_timeout_token.wrapping_add(1);
                let token = self.deep_research_stream_timeout_token;
                let timeout_ms = if self.deep_research_report_repair_used {
                    DEEP_RESEARCH_REPAIR_TIMEOUT_MS
                } else {
                    deep_research_planned_synthesis_timeout_ms(
                        self.deep_research_workflow.output.as_deref(),
                    )
                    .unwrap_or(DEEP_RESEARCH_SYNTHESIS_TIMEOUT_MS)
                };
                let delay = deep_research_synthesis_timeout_delay(
                    loop_state.started_at,
                    now,
                    now,
                    Duration::from_millis(timeout_ms),
                    0,
                    true,
                )
                .unwrap_or(Duration::ZERO);
                Some((delay, token))
            } else {
                None
            }
        } else {
            None
        };
        // (A `/ctx <n>` staged transcript window is attached upstream, only to a
        // genuine typed user message — see on_submit — never to a `/loop`,
        // asset review, `?`, or synthesis continuation.)
        // ultracode no longer rewrites the user turn. Whether a turn plans and
        // fans out is decided by the core's message-gated planning
        // (PlanningMode::Auto) plus the `parallel_task` tool description — not an
        // unconditional per-turn imperative, which made even "hi" trigger a plan
        // and workspace exploration.
        let mut commands = vec![
            cmd::cmd(move || async move {
                let res =
                    tokio::time::timeout(Duration::from_millis(STREAM_START_TIMEOUT_MS), async {
                        if attachments.is_empty() {
                            session.stream(prompt.as_str(), None).await
                        } else {
                            session
                                .stream_with_attachments(prompt.as_str(), &attachments, None)
                                .await
                        }
                    })
                    .await;
                match res {
                    Ok(Ok((rx, join))) => Msg::StreamStarted {
                        token: stream_start_token,
                        session: Arc::clone(&session),
                        rx: Arc::new(Mutex::new(rx)),
                        join,
                        submitted_images,
                    },
                    Ok(Err(error)) => {
                        let retryable_admission = matches!(&error, CodeError::SessionBusy { .. });
                        Msg::StreamError {
                            token: stream_start_token,
                            error: error.to_string(),
                            retryable_admission,
                            submitted_images,
                        }
                    }
                    Err(_) => Msg::StreamError {
                        token: stream_start_token,
                        error: format!(
                            "model stream admission timed out after {STREAM_START_TIMEOUT_MS} ms"
                        ),
                        retryable_admission: true,
                        submitted_images,
                    },
                }
            }),
            spinner_tick(),
            stream_commit_tick(),
        ];
        if let Some((delay, token)) = deep_research_timeout {
            commands.push(cmd::cmd(move || async move {
                tokio::time::sleep(delay).await;
                Msg::DeepResearchSynthesisTimedOut { token }
            }));
        }
        Some(cmd::batch(commands))
    }

    /// Pop exactly one queued user turn. Once user input is exhausted, resume
    /// a DeepResearch synthesis that was deliberately deferred behind it.
    pub(super) fn drain_queue(&mut self) -> Option<Cmd<Msg>> {
        // A queued successor must not race a session rebuild or an interrupted
        // stream admission that still has a chance to acquire the core's
        // single-flight lease. The corresponding completion handler calls this
        // method again as soon as the barrier clears.
        if self.state != State::Idle
            || self.session_rebuild_pending.is_some()
            || self.interrupted_stream_start_token.is_some()
            || self.active_queued_turn.is_some()
        {
            return None;
        }
        if let Some(next) = self.queue.pop() {
            let queued = next.value().clone();
            if let Some((query, os_runtime, evidence_scope)) = queued.deep_research {
                let command = self.start_deep_research_workflow(
                    query,
                    os_runtime,
                    evidence_scope,
                    queued.runtime_expectation,
                );
                if command.is_none() {
                    self.queue.restore(next);
                    self.relayout();
                }
                return command;
            }
            let command = self.start_stream_inner_with_runtime_and_images(
                queued.text,
                queued.display,
                true,
                false,
                queued.runtime_expectation,
                queued.images,
            );
            if command.is_some() {
                self.active_queued_turn_token = Some(self.stream_start_token);
                self.active_queued_turn = Some(next);
            } else {
                self.queue.restore(next);
                self.relayout();
            }
            return command;
        }

        let (prompt, display) = self.pending_deep_research_synthesis.take()?;
        self.start_deep_research_report_generation(
            prompt,
            display,
            DeepResearchReportGenerationPhase::Synthesis,
        )
    }

    /// Commit the queue claim once Core has admitted its stream. Before this
    /// point the exact a3s-lane entry remains restorable on `SessionBusy` or an
    /// admission timeout.
    pub(super) fn commit_active_queued_turn(&mut self, token: u64) -> bool {
        if take_matching_queue_claim(
            &mut self.active_queued_turn,
            &mut self.active_queued_turn_token,
            token,
        )
        .is_none()
        {
            return false;
        }
        self.queue_retry_generation = self.queue_retry_generation.wrapping_add(1);
        self.queue_retry_attempt = 0;
        true
    }

    /// Return a turn whose stream was never admitted to its original priority
    /// and FIFO position. Its images are Arc-backed, so dropping the attempted
    /// admission copy cannot invalidate the retained queue payload.
    pub(super) fn restore_active_queued_turn(&mut self, token: u64) -> bool {
        if let Some(item) = take_matching_queue_claim(
            &mut self.active_queued_turn,
            &mut self.active_queued_turn_token,
            token,
        ) {
            self.queue.restore(item);
            self.relayout();
            return true;
        }
        false
    }

    pub(super) fn discard_active_queued_turn(&mut self, token: u64) {
        let _ = take_matching_queue_claim(
            &mut self.active_queued_turn,
            &mut self.active_queued_turn_token,
            token,
        );
    }

    pub(super) fn retry_queued_turn_after_admission_failure(&mut self) -> Cmd<Msg> {
        self.queue_retry_generation = self.queue_retry_generation.wrapping_add(1);
        self.queue_retry_attempt = self.queue_retry_attempt.saturating_add(1);
        let generation = self.queue_retry_generation;
        let shift = u32::from(self.queue_retry_attempt.saturating_sub(1).min(4));
        let delay_ms = QUEUE_ADMISSION_RETRY_BASE_MS
            .saturating_mul(1_u64 << shift)
            .min(QUEUE_ADMISSION_RETRY_MAX_MS);
        cmd::cmd(move || async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            Msg::QueueRetry { generation }
        })
    }

    pub(super) fn interrupted_continuation(
        goal_cancelled: bool,
        deep_research_interrupted: bool,
    ) -> InterruptedContinuation {
        if deep_research_interrupted {
            InterruptedContinuation::SettleDeepResearch
        } else if goal_cancelled {
            InterruptedContinuation::RestoreGoalMode
        } else {
            InterruptedContinuation::DrainQueue
        }
    }

    pub(super) fn defer_or_continue_after_interrupt(
        &mut self,
        continuation: InterruptedContinuation,
    ) -> Option<Cmd<Msg>> {
        if self.interrupted_stream_start_token.is_some() {
            self.pending_interrupted_continuation = Some(continuation);
            // Keep submissions queue-only until the stale admission has been
            // cancelled. This closes the race where its cleanup could cancel a
            // newly started queue head on the same session.
            self.state = State::Streaming;
            self.relayout();
            return None;
        }
        self.continue_after_interrupt(continuation)
    }

    pub(super) fn on_interrupted_stream_start_settled(&mut self, token: u64) -> Option<Cmd<Msg>> {
        if self.interrupted_stream_start_token != Some(token) {
            return None;
        }
        self.interrupted_stream_start_token = None;
        let Some(continuation) = self.pending_interrupted_continuation.take() else {
            // The cancellation command has not produced `Msg::Interrupted`
            // yet. That handler will observe the cleared barrier and continue.
            return None;
        };
        self.state = State::Idle;
        self.relayout();
        self.continue_after_interrupt(continuation)
    }

    fn continue_after_interrupt(
        &mut self,
        continuation: InterruptedContinuation,
    ) -> Option<Cmd<Msg>> {
        match continuation {
            InterruptedContinuation::SettleDeepResearch => {
                self.settle_or_finalize_deep_research(DeepResearchSettlementExit::Interrupted)
            }
            InterruptedContinuation::RestoreGoalMode => {
                self.restore_autonomy();
                self.restore_goal_planning_mode()
            }
            InterruptedContinuation::DrainQueue => {
                self.restore_autonomy();
                self.drain_queue()
            }
        }
    }

    pub(super) fn has_queued_turn(&self) -> bool {
        !self.queue.is_empty() || self.pending_deep_research_synthesis.is_some()
    }

    pub(super) fn wait_for_completed_stream_join(
        &mut self,
        stream_join: StreamJoin,
        synthesis: Option<(String, String)>,
    ) -> Cmd<Msg> {
        self.stream_settle_abort = Some(stream_join.abort_handle());
        self.stream_join_settling = true;
        self.state = State::Streaming;
        self.relayout();
        wait_for_stream_join(
            Arc::clone(&self.session),
            stream_join,
            self.stream_start_token,
            synthesis,
        )
    }

    /// Shared turn-completion: count the turn, wait for the stream lifecycle to
    /// settle, consume queued user input, then continue autonomous work.
    /// Called from BOTH the normal `AgentEvent::End` arm (the happy path, which
    /// returns without re-pumping so `StreamEnded` never fires) and the
    /// `StreamEnded` channel-closed arm — previously this lived only in
    /// `StreamEnded`, so on success the queue never drained and `/loop` ran once.
    pub(super) fn complete_turn(&mut self) -> Option<Cmd<Msg>> {
        if self.deep_research_loop.is_some() {
            self.deep_research_stream_timeout_token =
                self.deep_research_stream_timeout_token.wrapping_add(1);
        }
        if self.deep_research_loop.as_ref().is_some_and(|state| {
            state.started_at.elapsed() >= Duration::from_millis(DEEP_RESEARCH_RUN_HARD_TIMEOUT_MS)
        }) {
            self.loop_remaining = 0;
            self.pending_deep_research_report_repair_prompt = None;
        }
        let degraded_deep_research = self.deep_research_loop.is_some()
            && matches!(self.deep_research_outcome, DeepResearchRunOutcome::Degraded);
        if self.state == State::Streaming && !degraded_deep_research {
            self.completed += 1;
        }
        self.warn_missing_runtime_evidence();
        // Goal iterations are evaluated by Core before End. A generic hidden
        // synthesis turn must not replace that event gate or consume the turn
        // as if it were complete.
        let synthesis = if degraded_deep_research || self.goal_run.is_some() {
            None
        } else {
            self.prepare_ultracode_synthesis()
        };
        let completed_stream_join = self.stream_join.take();
        self.finish();
        if let Some(completed_stream_join) = completed_stream_join {
            // Keep input queue-only until the worker has completed persistence,
            // cleanup, and release of core's single-flight admission lease.
            return Some(self.wait_for_completed_stream_join(completed_stream_join, synthesis));
        }
        self.continue_after_stream_settled(synthesis)
    }

    pub(super) fn continue_after_stream_settled(
        &mut self,
        synthesis: Option<(String, String)>,
    ) -> Option<Cmd<Msg>> {
        if self.goal_run.as_ref().is_some_and(|run| run.achieved) {
            self.pending_goal_failure = None;
            return self.finish_achieved_goal();
        }
        if self.goal_run.is_some() {
            // User messages queued while the iteration ran still execute under
            // the forced-planning goal session. Otherwise schedule the next
            // host-owned iteration without a fixed cap.
            if !self.queue.is_empty() {
                return self.drain_queue();
            }
            let failure = self.pending_goal_failure.take();
            return self.continue_goal_run(failure);
        }
        self.pending_goal_failure = None;
        // A real user follow-up supersedes the hidden answer-only synthesis for
        // the previous turn. It must become the next model turn, not sit behind
        // another autonomous request.
        if self.has_queued_turn() {
            return self.drain_queue();
        }
        if let Some((prompt, display_task)) = synthesis {
            return self.start_ultracode_synthesis(prompt, display_task);
        }
        self.continue_completed_turn()
    }

    /// Select the next turn only after the prior stream lifecycle has fully
    /// settled. Keeping this selection deferred is important: queued
    /// DeepResearch starts `tool_with_events` synchronously, while normal model
    /// streams start when their returned command is polled.
    pub(super) fn continue_completed_turn(&mut self) -> Option<Cmd<Msg>> {
        // Codex-style follow-ups consume one turn at a time before any hidden
        // synthesis or autonomous loop continuation. The same rule applies to
        // DeepResearch so a user steer cannot wait for the whole research run.
        if self.has_queued_turn() {
            return self.drain_queue();
        }
        if self.loop_remaining > 0 {
            if let Some(prompt) = self.pending_deep_research_report_repair_prompt.take() {
                self.loop_remaining -= 1;
                let n = self.loop_remaining;
                self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                    "  ↻ deep research report repair ({n} left · Esc to stop)"
                )));
                self.loop_continuation = true;
                let query = self
                    .deep_research_loop
                    .as_ref()
                    .map(|state| state.query.clone())
                    .unwrap_or_else(|| "report".to_string());
                return self.start_deep_research_report_generation(
                    prompt,
                    format!("✦\u{200A}repair report {query}"),
                    DeepResearchReportGenerationPhase::Repair,
                );
            }
        }
        // Required runtime evidence is a deliverable, not just a warning. In
        // autonomous runs, spend the next loop turn on a targeted correction
        // before falling back to the generic "Continue" prompt.
        if self.loop_remaining > 0 {
            if let Some(prompt) = self
                .runtime_expectation
                .as_ref()
                .and_then(RuntimeExpectation::corrective_prompt)
            {
                self.loop_remaining -= 1;
                let n = self.loop_remaining;
                self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                    "  ↻ runtime evidence retry ({n} left · Esc to stop)"
                )));
                self.loop_continuation = true;
                return Some(cmd::msg(Msg::Submit(prompt)));
            }
        }
        // /loop: auto-continue until the agent says DONE, the cap is hit, or Esc.
        // Queued user messages take priority.
        if self.loop_remaining > 0 {
            self.loop_remaining -= 1;
            let n = self.loop_remaining;
            let (label, prompt) = if let Some(deep_research) = &self.deep_research_loop {
                let layer = deep_research.total_layers.saturating_sub(n);
                (
                    "deep research verification",
                    deep_research.verification_prompt(layer.max(1)),
                )
            } else {
                (
                    "loop",
                    "Continue. If the task is fully complete, reply DONE and stop.".to_string(),
                )
            };
            self.push_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render(&format!("  ↻ {label} ({n} left · Esc to stop)")),
            );
            // Mark the continuation as machine-driven so on_submit doesn't
            // attach a staged `/ctx` window to it.
            self.loop_continuation = true;
            return Some(cmd::msg(Msg::Submit(prompt)));
        }
        // The loop is drained (or was never armed): an autonomous run that
        // auto-switched to auto mode is over — restore the user's mode.
        if self.loop_remaining == 0 {
            if self.deep_research_loop.is_some() {
                self.invalidate_subagent_snapshots();
                return self
                    .settle_or_finalize_deep_research(DeepResearchSettlementExit::ReportReady);
            }
            self.open_pending_deep_research_report_view();
            self.restore_autonomy();
        }
        // Run the next queued message (submitted while busy), if any.
        self.drain_queue()
    }

    pub(super) fn settle_or_finalize_deep_research(
        &mut self,
        exit: DeepResearchSettlementExit,
    ) -> Option<Cmd<Msg>> {
        match self.begin_deep_research_subagent_settlement(exit) {
            Some(settlement) => Some(settlement),
            None => self.finalize_deep_research_settlement(exit),
        }
    }

    pub(super) fn begin_deep_research_subagent_settlement(
        &mut self,
        exit: DeepResearchSettlementExit,
    ) -> Option<Cmd<Msg>> {
        if self.deep_research_loop.is_none() || self.deep_research_subagent_settlement_inflight {
            return None;
        }
        let mut task_ids = self.runtime.subagent_ids();
        if task_ids.is_empty() {
            return None;
        }
        task_ids.sort();
        self.deep_research_subagent_settlement_inflight = true;
        self.state = State::Streaming;
        self.spinner.start();
        self.relayout();
        Some(settle_deep_research_subagents(
            Arc::clone(&self.session),
            self.session_id.clone(),
            self.session_rebuild_seq,
            task_ids,
            exit,
        ))
    }

    pub(super) fn finalize_deep_research_settlement(
        &mut self,
        exit: DeepResearchSettlementExit,
    ) -> Option<Cmd<Msg>> {
        if self.deep_research_journal_finalization_inflight {
            return None;
        }
        let run_id = self
            .deep_research_workflow
            .args
            .as_ref()
            .and_then(|args| args.get("run_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let outcome = match self.deep_research_outcome {
            DeepResearchRunOutcome::Active if exit == DeepResearchSettlementExit::Interrupted => {
                Some(ResearchOutcome::Failed)
            }
            DeepResearchRunOutcome::Active => None,
            DeepResearchRunOutcome::Completed => Some(ResearchOutcome::Completed),
            DeepResearchRunOutcome::Qualified => Some(ResearchOutcome::Qualified),
            DeepResearchRunOutcome::Degraded => Some(ResearchOutcome::Degraded),
        };
        if let (Some(run_id), Some(outcome)) = (run_id, outcome) {
            self.deep_research_journal_finalization_inflight = true;
            let workspace = PathBuf::from(&self.cwd);
            let artifacts = self.deep_research_terminal_artifacts.clone();
            return Some(cmd::cmd(move || async move {
                let result = record_deep_research_run_terminal(
                    &workspace,
                    &run_id,
                    outcome,
                    artifacts.as_ref(),
                )
                .await
                .map_err(|error| error.to_string());
                Msg::DeepResearchJournalFinalized {
                    run_id,
                    exit,
                    result,
                }
            }));
        }
        self.complete_deep_research_settlement(exit)
    }

    pub(super) fn complete_deep_research_settlement(
        &mut self,
        exit: DeepResearchSettlementExit,
    ) -> Option<Cmd<Msg>> {
        if exit.opens_report() && self.deep_research_outcome.report_ready() {
            self.open_pending_deep_research_report_view();
        } else {
            self.pending_deep_research_report_view = None;
        }
        self.restore_autonomy();
        self.drain_queue()
    }

    pub(super) fn resume_after_pending_confirmation(&self) -> Cmd<Msg> {
        resume_after_pending_confirmation_cmd(self.rx.clone())
    }

    /// An autonomous directive run is starting (/sleep, asset reviews,
    /// asset run/deploy, /flow drafts, /loop): switch to auto-approve so tool
    /// prompts can't stall it, and arm the loop budget that re-prompts until
    /// the deliverable lands. The prior mode is restored when the run ends
    /// (loop drained, interrupt, error, or /clear). A user already in auto
    /// mode keeps it — nothing is remembered or restored.
    pub(super) fn engage_autonomy(&mut self, budget: usize) {
        self.loop_remaining = self.loop_remaining.max(budget);
        if self.mode != Mode::Auto {
            self.autonomy_restore = Some(self.mode);
            self.mode = Mode::Auto;
            self.push_line(&Style::new().fg(TN_GRAY).render(
                "  ⏵⏵ auto mode engaged for this task — restores when it completes (Esc stops)",
            ));
        }
    }

    pub(super) fn engage_single_turn_autonomy(&mut self) {
        if self.mode != Mode::Auto {
            self.autonomy_restore = Some(self.mode);
            self.mode = Mode::Auto;
            self.push_line(&Style::new().fg(TN_GRAY).render(
                "  ⏵⏵ auto mode engaged for this task — restores when it completes (Esc stops)",
            ));
        }
    }

    pub(super) fn record_runtime_tool_evidence(&mut self, name: &str) {
        if let Some(expectation) = &mut self.runtime_expectation {
            expectation.record_tool(name);
        }
    }

    pub(super) fn record_runtime_parallel_evidence(&mut self) {
        if let Some(expectation) = &mut self.runtime_expectation {
            expectation.record_parallel_work();
        }
    }

    pub(super) fn record_runtime_view_evidence(&mut self) {
        if let Some(expectation) = &mut self.runtime_expectation {
            expectation.record_remote_view();
        }
    }

    pub(super) fn warn_missing_runtime_evidence(&mut self) {
        let warning = self
            .runtime_expectation
            .as_mut()
            .and_then(RuntimeExpectation::missing_warning);
        if let Some(warning) = warning {
            self.push_line(&Style::new().fg(TN_YELLOW).render(&warning));
        }
    }

    /// Restore the pre-autonomy mode (no-op when nothing was auto-switched).
    pub(super) fn restore_autonomy(&mut self) {
        self.runtime_expectation = None;
        self.deep_research_loop = None;
        if let Some((goal, goal_since)) = self.deep_research_goal_restore.take() {
            self.goal = goal;
            self.goal_since = goal_since;
        }
        self.deep_research_report_repair_used = false;
        self.deep_research_workflow.clear();
        self.deep_research_outcome = DeepResearchRunOutcome::Active;
        self.deep_research_journal_finalization_inflight = false;
        self.deep_research_terminal_artifacts = None;
        self.deep_research_agent_event_sequence = 0;
        self.deep_research_projection = None;
        self.pending_deep_research_report_repair_prompt = None;
        self.pending_deep_research_synthesis = None;
        self.pending_deep_research_report_view = None;
        self.deep_research_report_tools.clear();
        self.deep_research_report_tool_gate.set_report_only(false);
        self.deep_research_subagent_settlement_inflight = false;
        if let Some(prev) = self.autonomy_restore.take() {
            self.mode = prev;
            self.push_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render("  ⏵ autonomous task ended — auto mode restored to your previous mode"),
            );
        }
    }

    pub(super) fn should_delay_deep_research_report_tool(&self) -> bool {
        should_delay_deep_research_report_tool(
            self.deep_research_loop.is_some(),
            &self.deep_research_report_tool_gate,
        )
    }

    pub(super) fn record_deep_research_child_event_cmd(
        &mut self,
        task_id: String,
        started: bool,
        payload: serde_json::Value,
    ) -> Option<Cmd<Msg>> {
        let run_id = self
            .deep_research_workflow
            .args
            .as_ref()?
            .get("run_id")?
            .as_str()?
            .to_string();
        self.deep_research_agent_event_sequence =
            self.deep_research_agent_event_sequence.saturating_add(1);
        let sequence = self.deep_research_agent_event_sequence;
        let workspace = PathBuf::from(&self.cwd);
        Some(cmd::cmd(move || async move {
            let result = record_deep_research_child_event(
                &workspace, &run_id, sequence, &task_id, started, payload,
            )
            .await
            .map_err(|error| error.to_string());
            Msg::DeepResearchJournalEventRecorded { run_id, result }
        }))
    }
}

#[cfg(test)]
mod queue_claim_tests {
    use super::*;

    #[test]
    fn admission_failure_restores_the_original_lane_position() {
        let mut queue = PriorityQueue::new();
        queue.push(USER_TURN_PRIORITY, "first");
        queue.push(USER_TURN_PRIORITY, "second");
        queue.push(SYNTHETIC_TURN_PRIORITY, "continuation");

        let mut active = queue.pop();
        let mut active_token = Some(9);
        let claimed = take_matching_queue_claim(&mut active, &mut active_token, 9)
            .expect("matching admission claim");
        queue.restore(claimed);

        let ordered = queue
            .ordered()
            .into_iter()
            .map(|item| *item.value())
            .collect::<Vec<_>>();
        assert_eq!(ordered, ["first", "second", "continuation"]);
    }

    #[test]
    fn stale_stream_token_cannot_consume_the_current_lane_claim() {
        let mut queue = PriorityQueue::new();
        queue.push(USER_TURN_PRIORITY, "current");
        let mut active = queue.pop();
        let original_sequence = active.as_ref().expect("active claim").sequence();
        let mut active_token = Some(12);

        assert!(take_matching_queue_claim(&mut active, &mut active_token, 11).is_none());
        assert_eq!(active_token, Some(12));
        assert_eq!(
            active.as_ref().expect("claim must remain").sequence(),
            original_sequence
        );
    }

    #[test]
    fn matching_lane_claim_is_consumed_at_most_once() {
        let mut queue = PriorityQueue::new();
        queue.push(USER_TURN_PRIORITY, "once");
        let mut active = queue.pop();
        let mut active_token = Some(21);

        assert!(take_matching_queue_claim(&mut active, &mut active_token, 21).is_some());
        assert!(take_matching_queue_claim(&mut active, &mut active_token, 21).is_none());
        assert!(active.is_none());
        assert_eq!(active_token, None);
    }
}
