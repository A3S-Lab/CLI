//! DeepResearch launch, synthesis, timeout, and recovery controller actions.

use super::*;

impl App {
    fn arm_deep_research_report_repair_after_model_failure(
        &mut self,
        workflow_output: &str,
        workflow_metadata: Option<&serde_json::Value>,
        prior_text: &str,
        failure: &str,
    ) -> bool {
        if self.deep_research_report_repair_used
            || deep_research_workflow_needs_recovery_report_with_metadata(
                workflow_output,
                workflow_metadata,
            )
        {
            return false;
        }
        let prior_is_safe =
            !prior_text.trim().is_empty() && !deep_research_output_has_internal_leak(prior_text);
        if prior_is_safe {
            self.deep_research_workflow.last_synthesis_text = Some(prior_text.to_string());
        }
        if !arm_deep_research_report_repair(
            &mut self.loop_remaining,
            &mut self.deep_research_report_repair_used,
        ) {
            return false;
        }
        let repair_context = if prior_is_safe {
            format!(
                "Validation feedback (must be corrected): {failure}\n\nPrevious synthesis:\n{prior_text}"
            )
        } else {
            format!(
                "Validation feedback (must be corrected): {failure}\n\nThe previous synthesis was omitted because it was empty or contained internal output."
            )
        };
        self.pending_deep_research_report_repair_prompt =
            deep_research_report_repair_prompt_from_state(
                self.deep_research_loop.as_ref(),
                workflow_output,
                workflow_metadata,
                &repair_context,
            );
        if self.pending_deep_research_report_repair_prompt.is_none() {
            return false;
        }
        self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
            "  ⚠ {failure} Running one focused, evidence-closed report repair pass…"
        )));
        true
    }

    pub(super) fn start_ultracode_synthesis(
        &mut self,
        prompt: String,
        display_task: String,
    ) -> Option<Cmd<Msg>> {
        self.ultracode_synthesis_used = true;
        self.push_line(&Style::new().fg(TN_GRAY).render("  ⇉ synthesizing results…"));
        self.start_stream_inner(prompt, display_task, false, false, true)
    }

    pub(super) fn start_deep_research_report_generation(
        &mut self,
        prompt: String,
        display_task: String,
        phase: DeepResearchReportGenerationPhase,
    ) -> Option<Cmd<Msg>> {
        let query = self.deep_research_loop.as_ref()?.query.clone();
        self.deep_research_report_tool_gate.set_synthesis_only();
        self.streaming.clear();
        self.got_delta = false;
        self.turn_text.clear();
        self.turn_had_agent_activity = false;
        self.turn_text_after_activity = false;
        self.deep_research_report_tools.clear();
        self.running_task = Some(display_task);
        self.state = State::Streaming;
        self.host_progress_inflight = true;
        self.stream_started = Some(Instant::now());
        self.spinner.start();
        self.relayout();
        self.rebuild_viewport();

        self.deep_research_stream_timeout_token =
            self.deep_research_stream_timeout_token.wrapping_add(1);
        let token = self.deep_research_stream_timeout_token;
        let timeout_ms = if phase.is_repair() {
            DEEP_RESEARCH_REPAIR_TIMEOUT_MS
        } else {
            deep_research_planned_synthesis_timeout_ms(
                self.deep_research_workflow.output.as_deref(),
            )
            .unwrap_or(DEEP_RESEARCH_SYNTHESIS_TIMEOUT_MS)
        };
        let args = deep_research_report_generation_args(&prompt, timeout_ms);
        let session = Arc::clone(&self.session);
        let workflow_output = self
            .deep_research_workflow
            .output
            .clone()
            .unwrap_or_default();
        let workflow_metadata = self.deep_research_workflow.metadata.clone();
        let run_id = self
            .deep_research_workflow
            .args
            .as_ref()
            .and_then(|args| args.get("run_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("deepresearch-report")
            .to_string();
        let use_sectioned_report = !phase.is_repair()
            && sectioned_report_available(&workflow_output, workflow_metadata.as_ref());

        Some(cmd::batch(vec![
            cmd::cmd(move || async move {
                let result = if use_sectioned_report {
                    generate_sectioned_report(
                        &session,
                        &query,
                        &workflow_output,
                        workflow_metadata.as_ref(),
                        &run_id,
                    )
                    .await
                } else {
                    let timeout = Duration::from_millis(timeout_ms);
                    match tokio::time::timeout(timeout, session.tool("generate_object", args)).await
                    {
                        Ok(Ok(result)) => Ok(result),
                        Ok(Err(error)) => Err(error.to_string()),
                        Err(_) => {
                            let _ = session
                                .cancel_and_settle(
                                    Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                                    Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                                )
                                .await;
                            Err(format!(
                                "DeepResearch {} model call timed out after {timeout_ms} ms",
                                if phase.is_repair() {
                                    "repair"
                                } else {
                                    "synthesis"
                                }
                            ))
                        }
                    }
                };
                Msg::DeepResearchReportGenerated {
                    token,
                    query,
                    phase,
                    result,
                }
            }),
            spinner_tick(),
            stream_commit_tick(),
        ]))
    }

    pub(super) fn on_deep_research_report_generated(
        &mut self,
        token: u64,
        query: String,
        phase: DeepResearchReportGenerationPhase,
        result: Result<ToolCallResult, String>,
    ) -> Option<Cmd<Msg>> {
        if token != self.deep_research_stream_timeout_token
            || self.state != State::Streaming
            || self.interrupting
            || self
                .deep_research_loop
                .as_ref()
                .map(|state| state.query.as_str())
                != Some(query.as_str())
        {
            return None;
        }
        self.host_progress_inflight = false;

        let mut projection_merge_error = None;
        if let Ok(generated) = result.as_ref() {
            if let Some(workflow_output) = self.deep_research_workflow.output.as_mut() {
                if let Err(error) = merge_sectioned_inquiry_projection(
                    workflow_output,
                    self.deep_research_workflow.metadata.as_mut(),
                    generated.metadata.as_ref(),
                ) {
                    projection_merge_error = Some(format!(
                        "DeepResearch sectioned report projection merge failed: {error}"
                    ));
                }
            }
        }

        let generated = match projection_merge_error {
            Some(error) => Err(error),
            None => result.and_then(|result| {
                deep_research_report_from_generation(&result.output, result.exit_code)
            }),
        };
        let (generated_report, mut generation_error) = match generated {
            Ok(report) => (Some(report), None),
            Err(error) => (None, Some(error)),
        };
        let report_text = generated_report
            .as_ref()
            .map(|report| report.markdown.clone())
            .unwrap_or_default();
        if !phase.is_repair() && !report_text.trim().is_empty() {
            self.deep_research_workflow.last_synthesis_text = Some(report_text.clone());
        }

        let workflow_output = self
            .deep_research_workflow
            .output
            .clone()
            .unwrap_or_default();
        let workflow_metadata = self.deep_research_workflow.metadata.clone();
        let evidence_scope = self
            .deep_research_workflow
            .args
            .as_ref()
            .map(|args| deep_research_evidence_scope_from_args(args, &query))
            .unwrap_or_default();
        let report_outcome = deep_research_report_outcome_for_workflow(
            &query,
            evidence_scope,
            &workflow_output,
            workflow_metadata.as_ref(),
        );
        let workspace = PathBuf::from(&self.cwd);
        let artifacts = generated_report.as_ref().and_then(|report| {
            match materialize_deep_research_completed_report_from_generation(
                &workspace,
                &query,
                report,
                &workflow_output,
                workflow_metadata.as_ref(),
            ) {
                Ok(artifacts) => Some(artifacts),
                Err(error) => {
                    generation_error = Some(error);
                    None
                }
            }
        });

        if let Some(artifacts) = artifacts {
            let final_text = clean_deep_research_final_text_from_artifacts(&artifacts, &workspace)
                .unwrap_or(report_text);
            self.loop_remaining = 0;
            self.stage_deep_research_report(&artifacts, report_outcome);
            self.streaming.push(&final_text);
            self.turn_text.clear();
            self.turn_text.push_str(&final_text);
            self.mark_assistant_text(&final_text);
            self.finalize_streaming();
            self.push_line(&Style::new().fg(TN_GREEN).render(&format!(
                "  ✓ DeepResearch report validated and rendered at {}",
                artifacts.html.display()
            )));
            return self.complete_turn();
        }

        let diagnostic = generation_error.unwrap_or_else(|| {
            deep_research_report_rejection_diagnostic_from_answer_text(
                &query,
                &report_text,
                &workflow_output,
                workflow_metadata.as_ref(),
            )
            .unwrap_or_else(|| {
                "report artifacts were not accepted for an unknown reason".to_string()
            })
        });
        self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
            "  ⚠ DeepResearch {} report rejected: {diagnostic}",
            if phase.is_repair() {
                "repair"
            } else {
                "synthesis"
            }
        )));

        if !phase.is_repair()
            && self.arm_deep_research_report_repair_after_model_failure(
                &workflow_output,
                workflow_metadata.as_ref(),
                &report_text,
                &format!("The initial structured synthesis was rejected: {diagnostic}."),
            )
        {
            let repair_prompt = self.pending_deep_research_report_repair_prompt.take()?;
            self.loop_remaining = self.loop_remaining.saturating_sub(1);
            return self.start_deep_research_report_generation(
                repair_prompt,
                format!("✦\u{200A}repair report {query}"),
                DeepResearchReportGenerationPhase::Repair,
            );
        }

        let recovery_text = if report_text.trim().is_empty() {
            self.deep_research_workflow
                .last_synthesis_text
                .as_deref()
                .unwrap_or(&diagnostic)
        } else {
            report_text.as_str()
        };
        match materialize_deep_research_recovery_report(
            &workspace,
            &query,
            recovery_text,
            &workflow_output,
            workflow_metadata.as_ref(),
        ) {
            Ok(artifacts) => {
                let final_text =
                    clean_deep_research_final_text_from_artifacts(&artifacts, &workspace)
                        .unwrap_or_else(|| diagnostic.clone());
                self.stage_deep_research_report(&artifacts, DeepResearchRunOutcome::Degraded);
                self.streaming.push(&final_text);
                self.turn_text.clear();
                self.turn_text.push_str(&final_text);
                self.mark_assistant_text(&final_text);
                self.finalize_streaming();
                self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                    "  ⚠ DeepResearch could not validate a completed report; wrote a degraded recovery report at {}",
                    artifacts.html.display()
                )));
            }
            Err(error) => {
                self.push_line(&Style::new().fg(TN_RED).render(&format!(
                    "  error: DeepResearch report recovery failed: {error}"
                )));
            }
        }
        self.loop_remaining = 0;
        self.deep_research_report_repair_used = true;
        self.complete_turn()
    }

    pub(super) fn on_deep_research_synthesis_timed_out(&mut self, token: u64) -> Option<Cmd<Msg>> {
        if token != self.deep_research_stream_timeout_token
            || self.state != State::Streaming
            || self.host_progress_inflight
            || self.deep_research_loop.is_none()
        {
            return None;
        }

        let repair_phase = self.deep_research_report_repair_used;
        let timeout_ms = if repair_phase {
            DEEP_RESEARCH_REPAIR_TIMEOUT_MS
        } else {
            deep_research_planned_synthesis_timeout_ms(
                self.deep_research_workflow.output.as_deref(),
            )
            .unwrap_or(DEEP_RESEARCH_SYNTHESIS_TIMEOUT_MS)
        };
        let now = Instant::now();
        let loop_state = self.deep_research_loop.as_ref()?;
        let phase_started_at = loop_state.phase_started_at.unwrap_or(loop_state.started_at);
        if let Some(delay) = deep_research_synthesis_timeout_delay(
            loop_state.started_at,
            phase_started_at,
            now,
            Duration::from_millis(timeout_ms),
            self.runtime.active_tool_count(),
            self.deep_research_report_tools.is_empty(),
        ) {
            return Some(cmd::cmd(move || async move {
                tokio::time::sleep(delay).await;
                Msg::DeepResearchSynthesisTimedOut { token }
            }));
        }
        let phase = if repair_phase { "repair" } else { "synthesis" };
        let status = format!("DeepResearch {phase} model call timed out after {timeout_ms} ms.");

        let session = Arc::clone(&self.session);
        let join = self.stream_join.take();
        self.rx = None;
        let streamed_text = self.turn_text.clone();
        self.interrupting = true;
        self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
            "  ⚠ {status} Cancelling the timed-out DeepResearch run before writing recovery artifacts…"
        )));

        Some(cmd::cmd(move || async move {
            let _ = session
                .cancel_and_settle(
                    Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                    Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                )
                .await;
            if let Some(join) = join {
                let _ = settle_stream_join_for_quit(
                    join,
                    Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                )
                .await;
            }
            Msg::DeepResearchSynthesisTimedOutAfterCancel {
                token,
                status,
                streamed_text,
                report_completed: false,
            }
        }))
    }

    pub(super) fn stop_deep_research_synthesis_if_report_ready(&mut self) -> Option<Cmd<Msg>> {
        if self.interrupting
            || self.state != State::Streaming
            || self.deep_research_loop.is_none()
            || !self.deep_research_report_tool_gate.report_only()
        {
            return None;
        }
        let query = &self.deep_research_loop.as_ref()?.query;
        let marker = format!(
            "{RESEARCH_VIEW_MARKER} .a3s/research/{}/index.html",
            deep_research_report_slug(query)
        );
        let baseline = self.deep_research_workflow.report_baseline.as_ref()?;
        deep_research_report_artifacts_from_output_for_current_run(
            &marker,
            Path::new(&self.cwd),
            query,
            self.deep_research_workflow
                .output
                .as_deref()
                .unwrap_or_default(),
            self.deep_research_workflow.metadata.as_ref(),
            baseline,
        )?;

        let token = self.deep_research_stream_timeout_token;
        let session = Arc::clone(&self.session);
        let join = self.stream_join.take();
        self.rx = None;
        self.interrupting = true;
        Some(cmd::cmd(move || async move {
            let _ = session
                .cancel_and_settle(
                    Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                    Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                )
                .await;
            if let Some(join) = join {
                let _ = settle_stream_join_for_quit(
                    join,
                    Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                )
                .await;
            }
            Msg::DeepResearchSynthesisTimedOutAfterCancel {
                token,
                status: "DeepResearch report artifacts completed".to_string(),
                streamed_text: marker,
                report_completed: true,
            }
        }))
    }

    pub(super) fn on_deep_research_synthesis_timed_out_after_cancel(
        &mut self,
        token: u64,
        status: String,
        streamed_text: String,
        report_completed: bool,
    ) -> Option<Cmd<Msg>> {
        if token != self.deep_research_stream_timeout_token || self.deep_research_loop.is_none() {
            return None;
        }

        self.finalize_streaming();
        self.preserve_interrupted_tools();
        if report_completed {
            self.push_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render("  ✓ report artifacts validated; synthesis stream stopped"),
            );
        } else {
            self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                "  ⚠ {status} Checking for a completed report before writing recovery artifacts."
            )));
        }

        let workspace = PathBuf::from(&self.cwd);
        let repair_phase = self.deep_research_report_repair_used;
        let query = self
            .deep_research_loop
            .as_ref()
            .map(|state| state.query.clone());
        let workflow_output = self
            .deep_research_workflow
            .output
            .clone()
            .unwrap_or_default();
        let workflow_metadata = self.deep_research_workflow.metadata.clone();
        let workflow_args = self.deep_research_workflow.args.clone();
        let report_outcome = query.as_deref().map(|query| {
            let evidence_scope = workflow_args
                .as_ref()
                .map(|args| deep_research_evidence_scope_from_args(args, query))
                .unwrap_or_default();
            deep_research_report_outcome_for_workflow(
                query,
                evidence_scope,
                &workflow_output,
                workflow_metadata.as_ref(),
            )
        });
        let validated_view = query.as_deref().and_then(|query| {
            let baseline = self.deep_research_workflow.report_baseline.as_ref()?;
            deep_research_report_view_spec_for_current_run(
                &streamed_text,
                &workspace,
                query,
                &workflow_output,
                workflow_metadata.as_ref(),
                baseline,
            )
        });
        if let Some(spec) = validated_view {
            self.deep_research_outcome =
                report_outcome.unwrap_or(DeepResearchRunOutcome::Completed);
            self.pending_deep_research_report_view = Some(spec);
            self.push_line(&Style::new().fg(TN_YELLOW).render(
                "  ⚠ DeepResearch timed out after writing a validated current-query report; preserving its RemoteUI view.",
            ));
        } else {
            let prior_synthesis_text = repair_phase
                .then_some(self.deep_research_workflow.last_synthesis_text.as_deref())
                .flatten()
                .map(str::to_string);
            match query {
                Some(query) => {
                    let (workflow_output, workflow_metadata) =
                        recover_deep_research_workflow_state_for_report_timeout(
                            &workspace,
                            &query,
                            workflow_args.as_ref(),
                            workflow_output,
                            workflow_metadata,
                        );
                    self.deep_research_workflow.output = Some(workflow_output.clone());
                    self.deep_research_workflow.metadata = workflow_metadata.clone();
                    let marker = format!(
                        "{RESEARCH_VIEW_MARKER} .a3s/research/{}/index.html",
                        deep_research_report_slug(&query)
                    );
                    let current_run_artifacts = self
                        .deep_research_workflow
                        .report_baseline
                        .as_ref()
                        .and_then(|baseline| {
                            deep_research_report_artifacts_from_output_for_current_run(
                                &marker,
                                &workspace,
                                &query,
                                &workflow_output,
                                workflow_metadata.as_ref(),
                                baseline,
                            )
                        });
                    let completed_artifacts = current_run_artifacts.or_else(|| {
                        materialize_deep_research_timeout_completed_report(
                            &workspace,
                            &query,
                            &streamed_text,
                            prior_synthesis_text.as_deref(),
                            &workflow_output,
                            workflow_metadata.as_ref(),
                        )
                    });
                    if let Some(artifacts) = completed_artifacts {
                        let evidence_scope = workflow_args
                            .as_ref()
                            .map(|args| deep_research_evidence_scope_from_args(args, &query))
                            .unwrap_or_default();
                        let report_outcome = deep_research_report_outcome_for_workflow(
                            &query,
                            evidence_scope,
                            &workflow_output,
                            workflow_metadata.as_ref(),
                        );
                        self.stage_deep_research_report(&artifacts, report_outcome);
                        self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                            "  ⚠ DeepResearch timed out, but a completed report was recovered into {}",
                            artifacts.html.display()
                        )));
                    } else if !repair_phase
                        && self.arm_deep_research_report_repair_after_model_failure(
                            &workflow_output,
                            workflow_metadata.as_ref(),
                            &streamed_text,
                            "The initial synthesis timed out before producing a valid report.",
                        )
                    {
                        return self.complete_turn();
                    } else {
                        let recovery_text = [
                            prior_synthesis_text.as_deref(),
                            Some(streamed_text.as_str()),
                        ]
                        .into_iter()
                        .flatten()
                        .find(|text| {
                            !text.trim().is_empty() && !deep_research_output_has_internal_leak(text)
                        })
                        .unwrap_or(status.as_str());
                        match materialize_deep_research_recovery_report(
                            &workspace,
                            &query,
                            recovery_text,
                            &workflow_output,
                            workflow_metadata.as_ref(),
                        ) {
                            Ok(artifacts) => {
                                self.stage_deep_research_report(
                                    &artifacts,
                                    DeepResearchRunOutcome::Degraded,
                                );
                                self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                                    "  ⚠ DeepResearch recovery report written at {}",
                                    artifacts.html.display()
                                )));
                            }
                            Err(error) => self.push_line(&Style::new().fg(TN_RED).render(
                                &format!("  error: DeepResearch recovery report failed: {error}"),
                            )),
                        }
                    }
                }
                None => self.push_line(&Style::new().fg(TN_RED).render(
                    "  error: DeepResearch timed out but the original query is unavailable",
                )),
            }
        }

        self.loop_remaining = 0;
        self.deep_research_report_repair_used = true;
        self.complete_turn()
    }

    pub(super) fn recover_deep_research_report_after_model_error(&mut self, message: &str) -> bool {
        let Some(query) = self
            .deep_research_loop
            .as_ref()
            .map(|state| state.query.clone())
        else {
            return false;
        };

        self.finalize_streaming();
        let workspace = PathBuf::from(&self.cwd);
        let workflow_output = self
            .deep_research_workflow
            .output
            .clone()
            .unwrap_or_default();
        let workflow_metadata = self.deep_research_workflow.metadata.clone();
        let evidence_scope = self
            .deep_research_workflow
            .args
            .as_ref()
            .map(|args| deep_research_evidence_scope_from_args(args, &query))
            .unwrap_or_default();
        let report_outcome = deep_research_report_outcome_for_workflow(
            &query,
            evidence_scope,
            &workflow_output,
            workflow_metadata.as_ref(),
        );
        let partial_text = self.turn_text.clone();
        if let Some(artifacts) = materialize_deep_research_timeout_completed_report(
            &workspace,
            &query,
            &partial_text,
            self.deep_research_workflow.last_synthesis_text.as_deref(),
            &workflow_output,
            workflow_metadata.as_ref(),
        ) {
            self.stage_deep_research_report(&artifacts, report_outcome);
            self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                "  ⚠ DeepResearch synthesis failed; preserved a completed source-backed report at {}",
                artifacts.html.display()
            )));
            self.loop_remaining = 0;
            self.deep_research_report_repair_used = true;
            return true;
        }
        if self.arm_deep_research_report_repair_after_model_failure(
            &workflow_output,
            workflow_metadata.as_ref(),
            &partial_text,
            "The report model call failed.",
        ) {
            return true;
        }
        let artifacts = materialize_deep_research_recovery_report(
            &workspace,
            &query,
            message,
            &workflow_output,
            workflow_metadata.as_ref(),
        );
        match artifacts {
            Ok(artifacts) => {
                self.stage_deep_research_report(&artifacts, DeepResearchRunOutcome::Degraded);
                self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                    "  ⚠ DeepResearch synthesis and repair failed; wrote an explicit low-confidence recovery report at {}",
                    artifacts.html.display()
                )));
            }
            Err(error) => self.push_line(&Style::new().fg(TN_RED).render(&format!(
                "  error: DeepResearch report recovery failed: {error}"
            ))),
        }
        self.loop_remaining = 0;
        self.deep_research_report_repair_used = true;
        true
    }

    pub(super) fn backfill_parallel_subagents_from_workflow_metadata(
        &mut self,
        metadata: Option<&serde_json::Value>,
    ) {
        let backfills = metadata
            .map(workflow_parallel_subagent_backfills)
            .unwrap_or_default();
        if backfills.is_empty() {
            return;
        }
        let now = Instant::now();
        for backfill in backfills {
            self.runtime.start_subagent(
                backfill.task_id.clone(),
                backfill.agent.clone(),
                backfill.description,
                now,
            );
            self.runtime.end_subagent(
                backfill.task_id,
                backfill.agent,
                String::new(),
                backfill.success,
                now,
            );
        }
        self.relayout();
    }
}
