//! DeepResearch evidence-workflow launch and completion controller actions.

use super::*;

impl App {
    pub(super) fn start_deep_research_workflow(
        &mut self,
        query: String,
        evidence_scope: DeepResearchEvidenceScope,
        runtime_expectation: Option<RuntimeExpectation>,
    ) -> Option<Cmd<Msg>> {
        self.auto_review.on_user_turn();
        self.last_activity = Instant::now();
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
            evidence_scope,
            started_at: run_started_at,
        });
        self.deep_research_workflow.reset_for_run();
        self.deep_research_outcome = DeepResearchRunOutcome::Active;
        self.deep_research_subagent_settlement_inflight = false;
        self.deep_research_journal_finalization_inflight = false;
        self.deep_research_terminal_artifacts = None;
        self.deep_research_agent_event_sequence = 0;
        self.deep_research_projection = None;
        self.pending_deep_research_report_view = None;
        self.deep_research_report_tool_gate
            .set_workspace(Path::new(&self.cwd));
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
                .render("  ⇉ retrieving and preserving source evidence…"),
        );
        self.rebuild_viewport();

        let budget = deep_research_budget_for_effort_index(self.effort, self.context_limit);
        let mut args = deep_research_workflow_args_for_budget(&query, evidence_scope, budget);
        ensure_deep_research_workflow_run_id(&mut args);
        self.deep_research_workflow.args = Some(args.clone());
        let (progress_rx, workflow_join) =
            spawn_deep_research_evidence_first(Arc::clone(&self.session), args.clone());
        let progress_rx = Arc::new(Mutex::new(progress_rx));
        self.rx = Some(progress_rx.clone());
        self.stream_join = None;
        self.host_tool_abort = Some(workflow_join.abort_handle());
        self.host_progress_inflight = true;
        self.host_tool_call_id = None;
        self.interrupting = false;
        let workflow_abort = workflow_join.abort_handle();
        let configured_timeout_ms = DEEP_RESEARCH_EVIDENCE_FIRST_HOST_TIMEOUT_MS;
        let timeout = Duration::from_millis(configured_timeout_ms).min(
            Duration::from_millis(DEEP_RESEARCH_RUN_HARD_TIMEOUT_MS)
                .saturating_sub(run_started_at.elapsed()),
        );
        let timeout_ms = timeout.as_millis().min(u128::from(u64::MAX)) as u64;
        let workflow_session = Arc::clone(&self.session);
        let workflow_workspace = PathBuf::from(&self.cwd);
        let args_for_timeout = args.clone();
        let journal_run_id = args
            .get("run_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let journal_spec = deep_research_evidence_first_research_spec(&args);
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
                        let _ = workflow_session
                            .cancel_and_settle(
                                Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                                Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
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
                let result = result.map(|mut result| {
                    result.output = deep_research_canonical_workflow_output(
                        &result.output,
                        result.metadata.as_ref(),
                    );
                    result
                });
                let (workflow_output, workflow_metadata) = match &result {
                    Ok(result) => (result.output.as_str(), result.metadata.as_ref()),
                    Err(error) => (error.as_str(), None),
                };
                let published = journal_run_id.as_deref().map(|run_id| {
                    resolve_deep_research_run_publication(
                        &workflow_workspace,
                        &query,
                        run_id,
                        workflow_output,
                    )
                });
                let inquiry_projection =
                    inquiry_projection_from_workflow(workflow_output, workflow_metadata);
                let convergence = match published {
                    Some(Ok(Some(_))) => ConvergenceDecision {
                        action: ConvergenceAction::Finalize,
                        reason: "the Host published the evidence-first report artifacts"
                            .to_string(),
                    },
                    Some(Err(error)) => ConvergenceDecision {
                        action: ConvergenceAction::Degrade,
                        reason: format!(
                            "the evidence-first publication failed validation: {error}"
                        ),
                    },
                    Some(Ok(None)) | None => match inquiry_projection.as_ref() {
                        Ok(Some((_, state))) => evaluate_terminal_inquiry_convergence(state),
                        Ok(None) => ConvergenceDecision {
                            action: ConvergenceAction::Degrade,
                            reason:
                                "the DeepResearch run returned without a publishable Host report"
                                    .to_string(),
                        },
                        Err(error) => ConvergenceDecision {
                            action: ConvergenceAction::Degrade,
                            reason: format!(
                                "the DeepResearch inquiry projection failed strict replay: {error}"
                            ),
                        },
                    },
                };
                let accepted_evidence =
                    accepted_evidence_ledger(workflow_output, workflow_metadata);
                if let Some(run_id) = journal_run_id.as_deref() {
                    let _ = record_deep_research_workflow_completed(
                        &workflow_workspace,
                        run_id,
                        result.is_ok(),
                    )
                    .await;
                    if let Ok(Some((events, state))) = inquiry_projection.as_ref() {
                        let _ = record_deep_research_inquiry_state(
                            &workflow_workspace,
                            run_id,
                            events,
                            state,
                        )
                        .await;
                    }
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
        if completed.first_terminal {
            self.capture_workflow("dynamic_workflow", completed.args.as_ref());
        }
        if let Some(spec) = self.find_remote_view_spec(&output) {
            self.remember_remote_view(spec);
        }
        if let Some(status) = deep_research_plan_status(&output) {
            self.push_line(&Style::new().fg(TN_GRAY).render(&status));
        }

        let publication_failure = match resolve_deep_research_run_publication(
            Path::new(&self.cwd),
            &query,
            completed_run_id.expect("validated DeepResearch completion requires a run ID"),
            &output,
        ) {
            Ok(Some(published)) => {
                let outcome = match published.publication {
                    DeepResearchEvidenceFirstPublication::Synthesized => {
                        DeepResearchRunOutcome::Completed
                    }
                    DeepResearchEvidenceFirstPublication::Qualified => {
                        DeepResearchRunOutcome::Qualified
                    }
                    DeepResearchEvidenceFirstPublication::SourceBacked => {
                        DeepResearchRunOutcome::Degraded
                    }
                    DeepResearchEvidenceFirstPublication::NoEvidence => {
                        DeepResearchRunOutcome::Degraded
                    }
                };
                let final_text = clean_deep_research_final_text_from_artifacts(
                    &published.artifacts,
                    Path::new(&self.cwd),
                )
                .unwrap_or_else(|| {
                    "DeepResearch published a report, but its Markdown preview was unavailable."
                        .to_string()
                });
                self.loop_remaining = 0;
                self.stage_deep_research_report(
                    &published.artifacts,
                    outcome,
                    DeepResearchTerminalArtifactAuthority::ValidatedPublication,
                );
                self.mark_assistant_text(&final_text);
                self.turn_text.clear();
                self.turn_text.push_str(&final_text);
                self.messages
                    .push(TranscriptEntry::assistant_markdown(final_text));
                match published.publication {
                    DeepResearchEvidenceFirstPublication::Synthesized => {
                        self.push_line(&Style::new().fg(TN_GREEN).render(&format!(
                            "  ✓ DeepResearch published a quality-gated report at {}",
                            published.artifacts.html.display()
                        )));
                    }
                    DeepResearchEvidenceFirstPublication::Qualified => {
                        self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                            "  ◐ DeepResearch published a qualified report with explicit evidence boundaries at {}",
                            published.artifacts.html.display()
                        )));
                    }
                    DeepResearchEvidenceFirstPublication::SourceBacked => {
                        self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                            "  ⚠ Report synthesis did not pass the quality gate; a degraded source snapshot was written at {}",
                            published.artifacts.html.display()
                        )));
                    }
                    DeepResearchEvidenceFirstPublication::NoEvidence => {
                        self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                            "  ⚠ Retrieval produced no safely publishable evidence; a degraded boundary report was written at {}",
                            published.artifacts.html.display()
                        )));
                    }
                }
                self.rebuild_viewport();
                return self.complete_turn();
            }
            Ok(None) => {
                "the standalone DeepResearch engine returned without its required Host publication"
                    .to_string()
            }
            Err(error) => format!("the Host publication failed validation: {error}"),
        };

        self.loop_remaining = 0;
        self.deep_research_outcome = DeepResearchRunOutcome::Degraded;
        let evidence_note = if accepted_evidence.is_empty() {
            "No accepted evidence was journaled.".to_string()
        } else {
            format!(
                "{} accepted evidence record(s) remain available in the run journal.",
                accepted_evidence.len()
            )
        };
        let recovery_reason = format!(
            "{publication_failure}. Terminal contract assessment: {}. {evidence_note}",
            convergence.reason
        );
        let status = match materialize_deep_research_recovery_report(
            Path::new(&self.cwd),
            &query,
            &recovery_reason,
            &output,
            metadata.as_ref(),
        ) {
            Ok(artifacts) => {
                self.stage_deep_research_report(
                    &artifacts,
                    DeepResearchRunOutcome::Degraded,
                    DeepResearchTerminalArtifactAuthority::VerifiedRecovery,
                );
                format!(
                    "DeepResearch did not produce a validated standalone-engine publication. A low-confidence recovery report was written to `{}`.",
                    artifacts.html.display()
                )
            }
            Err(error) => format!(
                "DeepResearch did not produce a validated standalone-engine publication and could not write its recovery report: {error}"
            ),
        };
        self.push_line(&Style::new().fg(TN_YELLOW).render(&format!("  ⚠ {status}")));
        self.mark_assistant_text(&status);
        self.turn_text.clear();
        self.turn_text.push_str(&status);
        self.messages
            .push(TranscriptEntry::assistant_markdown(status));
        self.rebuild_viewport();
        self.complete_turn()
    }
}
