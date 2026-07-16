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
            || deep_research_workflow_needs_recovery_report(workflow_output)
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

        Some(cmd::batch(vec![
            cmd::cmd(move || async move {
                let timeout = Duration::from_millis(timeout_ms);
                let result = match tokio::time::timeout(
                    timeout,
                    session.tool("generate_object", args),
                )
                .await
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

        let generated = result.and_then(|result| {
            deep_research_report_from_generation(&result.output, result.exit_code)
        });
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
            self.stage_deep_research_report(&artifacts, DeepResearchRunOutcome::Completed);
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

    pub(super) fn start_deep_research_workflow(
        &mut self,
        query: String,
        _os_runtime: bool,
        evidence_scope: DeepResearchEvidenceScope,
        runtime_expectation: Option<RuntimeExpectation>,
    ) -> Option<Cmd<Msg>> {
        self.auto_review.on_user_turn();
        self.last_activity = Instant::now();
        let os_runtime = false;
        self.streaming.clear();
        self.got_delta = false;
        self.turn_text.clear();
        self.turn_had_agent_activity = false;
        self.turn_text_after_activity = false;
        if self.deep_research_goal_restore.is_none() {
            self.deep_research_goal_restore = Some((self.goal.clone(), self.goal_since));
        }
        self.goal = Some(deep_research_goal(&query));
        self.goal_since = Some(Instant::now());
        self.engage_single_turn_autonomy();
        let run_started_at = Instant::now();
        self.deep_research_loop = Some(DeepResearchLoop {
            query: query.clone(),
            total_layers: 1,
            os_runtime,
            evidence_scope,
            started_at: run_started_at,
            phase_started_at: None,
        });
        self.deep_research_report_repair_used = false;
        self.deep_research_workflow
            .reset_for_run(snapshot_deep_research_report_artifacts(
                Path::new(&self.cwd),
                &query,
            ));
        self.deep_research_outcome = DeepResearchRunOutcome::Active;
        self.deep_research_subagent_settlement_inflight = false;
        self.deep_research_journal_finalization_inflight = false;
        self.deep_research_terminal_artifacts = None;
        self.deep_research_agent_event_sequence = 0;
        self.deep_research_projection = None;
        self.pending_deep_research_report_repair_prompt = None;
        self.pending_deep_research_synthesis = None;
        self.pending_deep_research_report_view = None;
        self.deep_research_report_tools.clear();
        self.deep_research_report_tool_gate
            .set_report_target(Path::new(&self.cwd), &query);
        self.deep_research_report_tool_gate
            .set_evidence_scope(evidence_scope);
        if let Some(expectation) = runtime_expectation {
            self.runtime_expectation = Some(expectation);
        }
        self.ultracode_synthesis_inflight = false;
        self.ultracode_synthesis_used = false;
        self.last_paint = None;
        self.viewport.set_auto_scroll(true);
        self.plan.clear();
        self.runtime.clear_turn_entities();
        let display_task = format!("✦\u{200A}{query}");
        self.runtime.set_subagent_task(display_task.clone());
        self.running_task = Some(display_task);
        self.state = State::Streaming;
        self.relayout();
        self.stream_started = Some(Instant::now());
        self.spinner.start();
        self.push_line(
            &Style::new()
                .fg(TN_GRAY)
                .render("  ⇉ gathering evidence with bounded recursive DynamicWorkflowRuntime…"),
        );
        self.rebuild_viewport();

        let budget = deep_research_budget_for_effort_index(self.effort, self.context_limit);
        let mut args =
            deep_research_workflow_args_for_budget(&query, os_runtime, evidence_scope, budget);
        ensure_deep_research_workflow_run_id(&mut args);
        self.deep_research_workflow.args = Some(args.clone());
        let (progress_rx, workflow_join) = self
            .session
            .tool_with_events("dynamic_workflow", args.clone());
        let progress_rx = Arc::new(Mutex::new(progress_rx));
        self.rx = Some(progress_rx.clone());
        self.stream_join = None;
        self.host_tool_abort = Some(workflow_join.abort_handle());
        self.host_progress_inflight = true;
        self.host_tool_call_id = None;
        self.interrupting = false;
        let workflow_abort = workflow_join.abort_handle();
        let configured_timeout_ms = deep_research_workflow_host_timeout_ms(&args);
        let timeout = Duration::from_millis(configured_timeout_ms).min(
            Duration::from_millis(DEEP_RESEARCH_RUN_HARD_TIMEOUT_MS)
                .saturating_sub(run_started_at.elapsed()),
        );
        let timeout_ms = timeout.as_millis().min(u128::from(u64::MAX)) as u64;
        let workflow_workspace = PathBuf::from(&self.cwd);
        let args_for_timeout = args.clone();
        let journal_run_id = args
            .get("run_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let journal_spec = ResearchSpec {
            query: query.clone(),
            current_date: chrono::Local::now().date_naive().to_string(),
            evidence_scope: evidence_scope.label().to_string(),
            required_claims: Vec::new(),
            total_budget_ms: timeout_ms,
            finalization_reserve_ms: timeout_ms.saturating_mul(15) / 100,
            host_pid: std::process::id(),
        };
        let finalization_reserve_ms = journal_spec.finalization_reserve_ms;
        Some(cmd::batch(vec![
            cmd::cmd(move || async move {
                if let Some(run_id) = journal_run_id.as_deref() {
                    let _ = record_deep_research_workflow_started(
                        &workflow_workspace,
                        run_id,
                        journal_spec,
                    )
                    .await;
                }
                let mut workflow_join = workflow_join;
                let result = match tokio::time::timeout(timeout, &mut workflow_join).await {
                    Ok(Ok(result)) => result.map_err(|err| err.to_string()),
                    Ok(Err(err)) => Err(err.to_string()),
                    Err(_) => {
                        workflow_abort.abort();
                        let _ = tokio::time::timeout(
                            Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                            &mut workflow_join,
                        )
                        .await;
                        let message = format!(
                            "dynamic_workflow timed out after {timeout_ms} ms while gathering DeepResearch evidence"
                        );
                        deep_research_workflow_timeout_tool_result(
                            &workflow_workspace,
                            &args_for_timeout,
                            message,
                        )
                    }
                };
                let (workflow_output, workflow_metadata) = match &result {
                    Ok(result) => (result.output.as_str(), result.metadata.as_ref()),
                    Err(error) => (error.as_str(), None),
                };
                let convergence = evaluate_convergence(deep_research_convergence_input(
                    DeepResearchConvergenceContext {
                        query: &query,
                        evidence_scope,
                        workflow_output,
                        workflow_metadata,
                        args: &args,
                        elapsed: run_started_at.elapsed(),
                        total_budget_ms: timeout_ms,
                        finalization_reserve_ms,
                    },
                ));
                let accepted_evidence =
                    accepted_evidence_ledger(workflow_output, workflow_metadata);
                if let Some(run_id) = journal_run_id.as_deref() {
                    let _ = record_deep_research_workflow_completed(
                        &workflow_workspace,
                        run_id,
                        result.is_ok(),
                    )
                    .await;
                    let contradictory_evidence = accepted_evidence
                        .iter()
                        .filter(|item| !item.contradictions.is_empty())
                        .cloned()
                        .collect::<Vec<_>>();
                    if !contradictory_evidence.is_empty() {
                        let _ = fork_current_for_contradiction_review(
                            &workflow_workspace,
                            run_id,
                            &contradictory_evidence,
                        )
                        .await;
                    }
                    let _ = record_deep_research_evidence_ledger(
                        &workflow_workspace,
                        run_id,
                        &accepted_evidence,
                    )
                    .await;
                    let _ =
                        record_deep_research_convergence(&workflow_workspace, run_id, &convergence)
                            .await;
                }
                Msg::DeepResearchWorkflowCompleted {
                    query,
                    os_runtime,
                    args,
                    result,
                    convergence,
                    accepted_evidence,
                }
            }),
            pump(progress_rx),
            spinner_tick(),
            stream_commit_tick(),
        ]))
    }

    pub(super) fn on_deep_research_workflow_completed(
        &mut self,
        query: String,
        os_runtime: bool,
        args: serde_json::Value,
        result: Result<ToolCallResult, String>,
        convergence: ConvergenceDecision,
        accepted_evidence: Vec<AcceptedEvidence>,
    ) -> Option<Cmd<Msg>> {
        let current_run_id = self
            .deep_research_workflow
            .args
            .as_ref()
            .and_then(|value| value.get("run_id"))
            .and_then(serde_json::Value::as_str);
        let completed_run_id = args.get("run_id").and_then(serde_json::Value::as_str);
        let current_query = self
            .deep_research_loop
            .as_ref()
            .map(|state| state.query.as_str());
        if self.state != State::Streaming
            || self.interrupting
            || current_query != Some(query.as_str())
            || current_run_id.is_none()
            || current_run_id != completed_run_id
        {
            return None;
        }
        self.host_tool_abort = None;
        self.host_progress_inflight = false;
        self.rx = None;
        let tool_id = self.host_tool_call_id.take().unwrap_or_else(|| {
            format!(
                "host-dynamic_workflow-{}",
                completed_run_id.unwrap_or("unknown")
            )
        });

        let (output, exit_code, metadata) = match result {
            Ok(result) => (result.output, result.exit_code, result.metadata),
            Err(error) => (error, 1, None),
        };
        self.deep_research_workflow.output = Some(output.clone());
        self.deep_research_workflow.metadata = metadata.clone();
        self.deep_research_workflow.args = Some(args.clone());
        let display_output = deep_research_tool_card_output(&output);
        let completed = self.runtime.end_tool(
            &tool_id,
            "dynamic_workflow".to_string(),
            Some(args.clone()),
            display_output.clone(),
            exit_code,
        );
        self.messages.finish_tool_with_state(
            &tool_id,
            "dynamic_workflow".to_string(),
            completed.args.clone(),
            completed.output.clone(),
            completed.exit_code,
            metadata.clone(),
            completed.state,
            true,
        );
        self.rebuild_viewport();
        self.record_runtime_tool_evidence("dynamic_workflow");
        if metadata
            .as_ref()
            .is_some_and(|value| json_contains_tool_evidence(value, "runtime"))
        {
            self.record_runtime_tool_evidence("runtime");
        }
        if metadata
            .as_ref()
            .is_some_and(|value| json_contains_tool_evidence(value, "parallel_task"))
        {
            self.record_runtime_parallel_evidence();
        }
        self.backfill_parallel_subagents_from_workflow_metadata(metadata.as_ref());
        if completed.first_terminal {
            self.capture_workflow("dynamic_workflow", completed.args.as_ref());
        }
        if let Some(spec) = self.find_remote_view_spec(&output) {
            self.remember_remote_view(spec);
        }
        let evidence_scope = deep_research_evidence_scope_from_args(&args, &query);
        if let Some(status) = deep_research_plan_status(&output) {
            self.push_line(&Style::new().fg(TN_GRAY).render(&status));
        }

        if accepted_evidence.is_empty()
            || !deep_research_evidence_package_is_complete_for_query(
                &query,
                evidence_scope,
                &output,
                metadata.as_ref(),
            )
        {
            self.loop_remaining = 0;
            self.deep_research_outcome = DeepResearchRunOutcome::Degraded;
            let status = match materialize_deep_research_recovery_report(
                Path::new(&self.cwd),
                &query,
                &format!(
                    "Evidence collection ended without a validated evidence package. Convergence decision: {}.",
                    convergence.reason
                ),
                &output,
                metadata.as_ref(),
            ) {
                Ok(artifacts) => {
                    self.stage_deep_research_report(
                        &artifacts,
                        DeepResearchRunOutcome::Degraded,
                    );
                    format!(
                        "DeepResearch stopped after bounded evidence collection because {}. A low-confidence recovery report was written to `{}`.",
                        convergence.reason,
                        artifacts.html.display()
                    )
                }
                Err(error) => format!(
                    "DeepResearch stopped after bounded evidence collection and could not write its recovery report: {error}"
                ),
            };
            self.push_line(&Style::new().fg(TN_YELLOW).render(&format!("  ⚠ {status}")));
            self.mark_assistant_text(&status);
            self.turn_text.clear();
            self.turn_text.push_str(&status);
            self.messages
                .push(TranscriptEntry::assistant_markdown(status));
            self.rebuild_viewport();
            return self.complete_turn();
        }

        let synthesis_evidence = accepted_evidence_synthesis_payload(&accepted_evidence, &output);
        let prompt = if exit_code == 0 {
            self.push_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render("  ⇉ evidence gathered · synthesizing source-backed report…"),
            );
            deep_research_synthesis_prompt_with_scope(
                &query,
                os_runtime,
                &synthesis_evidence,
                None,
                evidence_scope,
            )
        } else {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  ⚠ dynamic workflow failed; starting recovery synthesis…"),
            );
            deep_research_recovery_prompt_with_scope(
                &query,
                os_runtime,
                &synthesis_evidence,
                None,
                evidence_scope,
            )
        };
        self.deep_research_report_tool_gate.set_synthesis_only();
        if !self.queue.is_empty() {
            // Treat messages submitted during evidence collection as user
            // follow-ups. Run them before report synthesis, then resume this
            // exact synthesis once the user queue is empty.
            let display = format!("✦\u{200A}synthesize {query}");
            self.pending_deep_research_synthesis = Some((prompt, display));
            self.finish();
            return self.drain_queue();
        }
        self.start_deep_research_report_generation(
            prompt,
            format!("✦\u{200A}synthesize {query}"),
            DeepResearchReportGenerationPhase::Synthesis,
        )
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
            self.deep_research_outcome = DeepResearchRunOutcome::Completed;
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
                        self.stage_deep_research_report(
                            &artifacts,
                            DeepResearchRunOutcome::Completed,
                        );
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
        let partial_text = self.turn_text.clone();
        if let Some(artifacts) = materialize_deep_research_timeout_completed_report(
            &workspace,
            &query,
            &partial_text,
            self.deep_research_workflow.last_synthesis_text.as_deref(),
            &workflow_output,
            workflow_metadata.as_ref(),
        ) {
            self.stage_deep_research_report(&artifacts, DeepResearchRunOutcome::Completed);
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
