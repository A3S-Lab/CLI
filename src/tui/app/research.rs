//! DeepResearch sectioned-report generation and recovery controller actions.

use super::*;

impl App {
    fn arm_deep_research_report_resume_after_pipeline_failure(
        &mut self,
        workflow_output: &str,
        workflow_metadata: Option<&serde_json::Value>,
        prior_text: &str,
        failure: &str,
    ) -> bool {
        if self.deep_research_report_resume_used
            || deep_research_workflow_needs_recovery_report_with_metadata(
                workflow_output,
                workflow_metadata,
            )
        {
            return false;
        }
        match inquiry_projection_from_workflow(workflow_output, workflow_metadata) {
            Ok(Some((_, state))) if state.phase != a3s::research::InquiryPhase::Outlining => {
                // A generic retry would replace report content without
                // producing matching outline/draft/audit events. Inquiry
                // resume is only safe while the sectioned pipeline can be
                // replayed from its collected Outlining state.
                return false;
            }
            Err(_) => return false,
            Ok(Some(_)) | Ok(None) => {}
        }
        let prior_is_safe =
            !prior_text.trim().is_empty() && !deep_research_output_has_internal_leak(prior_text);
        if prior_is_safe {
            self.deep_research_workflow.last_synthesis_text = Some(prior_text.to_string());
        }
        if !arm_deep_research_report_resume(
            &mut self.loop_remaining,
            &mut self.deep_research_report_resume_used,
        ) {
            return false;
        }
        self.pending_deep_research_report_resume = true;
        self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
            "  ⚠ {failure} Resuming the same durable sectioned report transaction once…"
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
        let workflow_output = self
            .deep_research_workflow
            .output
            .clone()
            .unwrap_or_default();
        let workflow_metadata = self.deep_research_workflow.metadata.clone();
        let sectioned_report_ready =
            sectioned_report_available(&workflow_output, workflow_metadata.as_ref());
        let timeout_ms = DEEP_RESEARCH_SECTIONED_SYNTHESIS_TIMEOUT_MS;
        let session = Arc::clone(&self.session);
        let run_id = self
            .deep_research_workflow
            .args
            .as_ref()
            .and_then(|args| args.get("run_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("deepresearch-report")
            .to_string();
        Some(cmd::batch(vec![
            cmd::cmd(move || async move {
                let result = if sectioned_report_ready {
                    let report_deadline = Instant::now() + Duration::from_millis(timeout_ms);
                    generate_sectioned_report(
                        &session,
                        &query,
                        &workflow_output,
                        workflow_metadata.as_ref(),
                        &run_id,
                        report_deadline,
                    )
                    .await
                } else {
                    Err(
                        "DeepResearch report generation requires a replayed Inquiry in Outlining"
                            .to_string(),
                    )
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
        let mut inquiry_publication_outcome = None;
        if let Ok(generated) = result.as_ref() {
            match self.deep_research_workflow.output.as_mut() {
                Some(workflow_output) => match merge_sectioned_inquiry_projection(
                    workflow_output,
                    self.deep_research_workflow.metadata.as_mut(),
                    generated.metadata.as_ref(),
                ) {
                    Ok(_) => match deep_research_inquiry_publication_outcome(
                        workflow_output,
                        self.deep_research_workflow.metadata.as_ref(),
                    ) {
                        Ok(outcome) => inquiry_publication_outcome = outcome,
                        Err(error) => {
                            projection_merge_error = Some(format!(
                                "DeepResearch publication authority rejected the report: {error}"
                            ));
                        }
                    },
                    Err(error) => {
                        projection_merge_error = Some(format!(
                            "DeepResearch sectioned report projection merge failed: {error}"
                        ));
                    }
                },
                None => {
                    projection_merge_error =
                        Some("DeepResearch report generation has no workflow output".to_string());
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
        if !phase.is_resume() && !report_text.trim().is_empty() {
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
        let report_outcome = inquiry_publication_outcome.unwrap_or_else(|| {
            deep_research_report_outcome_for_workflow(
                &query,
                evidence_scope,
                &workflow_output,
                workflow_metadata.as_ref(),
            )
        });
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
            if phase.is_resume() {
                "resumed"
            } else {
                "synthesis"
            }
        )));

        if !phase.is_resume()
            && self.arm_deep_research_report_resume_after_pipeline_failure(
                &workflow_output,
                workflow_metadata.as_ref(),
                &report_text,
                &format!("The initial structured synthesis was rejected: {diagnostic}."),
            )
        {
            if !self.pending_deep_research_report_resume {
                return None;
            }
            self.pending_deep_research_report_resume = false;
            self.loop_remaining = self.loop_remaining.saturating_sub(1);
            return self.start_deep_research_report_generation(
                format!("✦\u{200A}resume report {query}"),
                DeepResearchReportGenerationPhase::Resume,
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
        self.deep_research_report_resume_used = true;
        self.complete_turn()
    }
}
