//! Headless TUI and DeepResearch smoke-mode execution.

use super::*;

/// Headless probe of the same `session.stream()` / `AgentEvent` path the TUI
/// uses. A headless process has no human authority, so any unexpected
/// confirmation request is rejected rather than manufacturing consent.
pub(super) async fn run_smoke(
    session: Arc<AgentSession>,
    workspace: &Path,
    deep_research_report_tool_gate: DeepResearchReportToolGate,
) -> anyhow::Result<()> {
    let prompt = std::env::var("A3S_CODE_TUI_PROMPT")
        .unwrap_or_else(|_| "Reply with exactly one short sentence: what is 2 + 2?".to_string());
    if let Some(query) = prompt.trim().strip_prefix('?') {
        let query = query.trim().to_string();
        if query.is_empty() {
            anyhow::bail!("A3S_CODE_TUI_PROMPT starts with `?` but has no DeepResearch query");
        }
        return run_smoke_deep_research(session, workspace, query, deep_research_report_tool_gate)
            .await;
    }
    eprintln!("[smoke] prompt: {prompt}");
    let _ = stream_smoke_prompt(session.as_ref(), prompt.as_str()).await?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SmokePhaseDeadline {
    pub(super) phase: &'static str,
    pub(super) run_deadline: Instant,
    pub(super) phase_deadline: Instant,
    pub(super) selected_timeout: Duration,
}

pub(super) fn deep_research_smoke_run_deadline(started_at: Instant) -> Instant {
    started_at + Duration::from_millis(DEEP_RESEARCH_RUN_HARD_TIMEOUT_MS)
}

pub(super) fn deep_research_smoke_execution_deadline(run_deadline: Instant) -> Instant {
    run_deadline
        .checked_sub(Duration::from_millis(
            DEEP_RESEARCH_SMOKE_FINALIZATION_RESERVE_MS,
        ))
        .unwrap_or(run_deadline)
}

pub(super) fn deep_research_smoke_remaining_budget(
    run_deadline: Instant,
    now: Instant,
) -> Duration {
    run_deadline.saturating_duration_since(now)
}

pub(super) fn deep_research_smoke_phase_deadline(
    run_deadline: Instant,
    now: Instant,
    phase_limit: Duration,
    phase: &'static str,
) -> Option<SmokePhaseDeadline> {
    deep_research_smoke_bounded_phase_deadline(
        run_deadline,
        deep_research_smoke_execution_deadline(run_deadline),
        now,
        phase_limit,
        phase,
    )
}

pub(super) fn deep_research_smoke_finalization_phase_deadline(
    run_deadline: Instant,
    now: Instant,
    phase_limit: Duration,
    phase: &'static str,
) -> Option<SmokePhaseDeadline> {
    deep_research_smoke_bounded_phase_deadline(run_deadline, run_deadline, now, phase_limit, phase)
}

fn deep_research_smoke_bounded_phase_deadline(
    run_deadline: Instant,
    budget_deadline: Instant,
    now: Instant,
    phase_limit: Duration,
    phase: &'static str,
) -> Option<SmokePhaseDeadline> {
    let selected_timeout = budget_deadline
        .saturating_duration_since(now)
        .min(phase_limit);
    if selected_timeout.is_zero() {
        return None;
    }
    Some(SmokePhaseDeadline {
        phase,
        run_deadline,
        phase_deadline: now + selected_timeout,
        selected_timeout,
    })
}

pub(super) fn deep_research_smoke_exhausted_phase_message(phase: &str) -> String {
    format!(
        "DeepResearch {phase} model call timed out after 0 ms because the bounded execution budget was exhausted before the phase could start."
    )
}

impl SmokePhaseDeadline {
    fn phase_remaining(self, now: Instant) -> Duration {
        self.phase_deadline.saturating_duration_since(now)
    }

    fn run_remaining(self, now: Instant) -> Duration {
        deep_research_smoke_remaining_budget(self.run_deadline, now)
    }

    fn selected_timeout_ms(self) -> u64 {
        self.selected_timeout.as_millis().min(u128::from(u64::MAX)) as u64
    }

    fn timeout_message(self) -> String {
        format!(
            "DeepResearch {} model call timed out after {} ms.",
            self.phase,
            self.selected_timeout_ms()
        )
    }
}

fn deep_research_smoke_deadline_error(phase: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "DeepResearch smoke exhausted its absolute {} ms run budget before {phase}",
        DEEP_RESEARCH_RUN_HARD_TIMEOUT_MS
    )
}

fn ensure_deep_research_smoke_budget(run_deadline: Instant, phase: &str) -> anyhow::Result<()> {
    if deep_research_smoke_remaining_budget(run_deadline, Instant::now()).is_zero() {
        Err(deep_research_smoke_deadline_error(phase))
    } else {
        Ok(())
    }
}

pub(super) fn run_deep_research_smoke_artifact_step<T>(
    run_deadline: Instant,
    phase: &str,
    operation: impl FnOnce() -> T,
) -> anyhow::Result<T> {
    ensure_deep_research_smoke_budget(run_deadline, phase)?;
    let result = operation();
    ensure_deep_research_smoke_budget(run_deadline, phase)?;
    Ok(result)
}

pub(super) async fn finalize_deep_research_smoke_journal(
    workspace: &Path,
    run_id: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    requested_outcome: DeepResearchRunOutcome,
    artifacts: &ResearchReportArtifacts,
) -> anyhow::Result<DeepResearchRunOutcome> {
    let successful_report = requested_outcome.report_ready();
    match inquiry_projection_from_workflow(workflow_output, workflow_metadata) {
        Ok(Some((events, state))) => {
            if successful_report && state.phase != a3s::research::InquiryPhase::Completed {
                anyhow::bail!(
                    "DeepResearch smoke cannot publish from non-completed Inquiry phase {:?}",
                    state.phase
                );
            }
            record_deep_research_inquiry_state(workspace, run_id, &events, &state).await?;
        }
        Ok(None) if successful_report => {
            anyhow::bail!(
                "host-managed DeepResearch smoke cannot publish without an Inquiry projection"
            )
        }
        Err(error) if successful_report => return Err(anyhow::Error::msg(error)),
        Ok(None) | Err(_) => {
            // A planner or collection failure can precede the first complete
            // Inquiry projection. The run journal still has to become terminal
            // so smoke diagnostics never leave an active workflow behind.
        }
    }

    let requested_outcome = match requested_outcome {
        DeepResearchRunOutcome::Completed => ResearchOutcome::Completed,
        DeepResearchRunOutcome::Qualified => ResearchOutcome::Qualified,
        DeepResearchRunOutcome::Degraded => ResearchOutcome::Degraded,
        DeepResearchRunOutcome::Active => {
            anyhow::bail!("DeepResearch smoke cannot journal an active terminal outcome")
        }
    };
    let projection =
        record_deep_research_run_terminal(workspace, run_id, requested_outcome, Some(artifacts))
            .await?;
    if !projection.outcome.is_terminal()
        || !projection.active_steps.is_empty()
        || !projection.active_children.is_empty()
    {
        anyhow::bail!(
            "DeepResearch smoke journal did not settle: outcome={:?}, active_steps={}, active_children={}",
            projection.outcome,
            projection.active_steps.len(),
            projection.active_children.len()
        );
    }
    Ok(match projection.outcome {
        ResearchOutcome::Completed => DeepResearchRunOutcome::Completed,
        ResearchOutcome::Qualified => DeepResearchRunOutcome::Qualified,
        ResearchOutcome::Degraded | ResearchOutcome::Failed => DeepResearchRunOutcome::Degraded,
        ResearchOutcome::Active => unreachable!("terminal projection cannot remain active"),
    })
}

async fn stream_smoke_prompt(session: &AgentSession, prompt: &str) -> anyhow::Result<String> {
    stream_smoke_prompt_inner(session, prompt, None).await
}

async fn generate_smoke_sectioned_report(
    session: &AgentSession,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    run_id: &str,
    deadline: SmokePhaseDeadline,
) -> Result<(GeneratedDeepResearchReport, Option<serde_json::Value>), String> {
    let remaining = deadline.phase_remaining(Instant::now());
    if remaining.is_zero() {
        return Err(deadline.timeout_message());
    }
    let result = match tokio::time::timeout(
        remaining,
        generate_sectioned_report(
            session,
            query,
            workflow_output,
            workflow_metadata,
            run_id,
            deadline.phase_deadline,
        ),
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            let _ = session
                .cancel_and_settle(
                    Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                    Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                )
                .await;
            return Err(deadline.timeout_message());
        }
    };
    let report = deep_research_report_from_generation(&result.output, result.exit_code)?;
    Ok((report, result.metadata))
}

async fn stream_smoke_prompt_inner(
    session: &AgentSession,
    prompt: &str,
    deadline: Option<SmokePhaseDeadline>,
) -> anyhow::Result<String> {
    let (mut rx, join) = if let Some(deadline) = deadline {
        let remaining = deadline.phase_remaining(Instant::now());
        if remaining.is_zero() {
            let message = deadline.timeout_message();
            eprintln!("\n[smoke] {message}");
            return Ok(message);
        }
        match tokio::time::timeout(remaining, session.stream(prompt, None)).await {
            Ok(result) => result?,
            Err(_) => {
                if let Some(abort_deadline) = deep_research_smoke_finalization_phase_deadline(
                    deadline.run_deadline,
                    Instant::now(),
                    Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                    "abort",
                ) {
                    let cancel_budget = abort_deadline.phase_remaining(Instant::now());
                    if !cancel_budget.is_zero() {
                        let _ = tokio::time::timeout(
                            cancel_budget,
                            session.cancel_and_settle(Duration::ZERO, cancel_budget),
                        )
                        .await;
                    }
                }
                let message = deadline.timeout_message();
                eprintln!("\n[smoke] {message}");
                return Ok(message);
            }
        }
    } else {
        session.stream(prompt, None).await?
    };
    let abort = join.abort_handle();
    let mut streamed = String::new();
    let mut end_text = String::new();
    let mut phase_timer = deadline
        .map(|deadline| Box::pin(tokio::time::sleep(deadline.phase_remaining(Instant::now()))));
    loop {
        let event = if let Some(phase_timer) = phase_timer.as_mut() {
            tokio::select! {
                event = rx.recv() => event,
                _ = phase_timer.as_mut() => {
                    let deadline = deadline.expect("phase timer implies deadline");
                    let abort_deadline = deep_research_smoke_finalization_phase_deadline(
                        deadline.run_deadline,
                        Instant::now(),
                        Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                        "abort",
                    );
                    if let Some(abort_deadline) = abort_deadline {
                        let cancel_budget = abort_deadline.phase_remaining(Instant::now());
                        if !cancel_budget.is_zero() {
                            let _ = tokio::time::timeout(
                                cancel_budget,
                                session.cancel_and_settle(Duration::ZERO, cancel_budget),
                            )
                            .await;
                        }
                        let join_budget = abort_deadline.phase_remaining(Instant::now());
                        if join_budget.is_zero()
                            || tokio::time::timeout(join_budget, join).await.is_err()
                        {
                            abort.abort();
                        }
                    } else {
                        abort.abort();
                    }
                    let message = deadline.timeout_message();
                    eprintln!("\n[smoke] {message}");
                    return Ok(message);
                }
            }
        } else {
            rx.recv().await
        };
        let Some(event) = event else {
            break;
        };
        match event {
            AgentEvent::TextDelta { text } => {
                streamed.push_str(&text);
                print!("{text}");
            }
            AgentEvent::ToolStart { name, .. } => eprintln!("\n[tool start] {name}"),
            AgentEvent::ToolEnd {
                name,
                exit_code,
                output,
                ..
            } => {
                eprintln!(
                    "[tool end] {name} (exit {exit_code}): {}",
                    output.lines().take(2).collect::<Vec<_>>().join(" | ")
                );
            }
            AgentEvent::ConfirmationRequired {
                tool_id, tool_name, ..
            } => {
                eprintln!("[confirm] rejecting {tool_name}: headless smoke has no approver");
                let reason = Some(
                    "Denied because headless smoke execution cannot obtain human approval."
                        .to_string(),
                );
                if let Some(deadline) = deadline {
                    let confirmation_budget = deadline
                        .phase_remaining(Instant::now())
                        .min(deadline.run_remaining(Instant::now()));
                    if !confirmation_budget.is_zero() {
                        let _ = tokio::time::timeout(
                            confirmation_budget,
                            session.confirm_tool_use(&tool_id, false, reason),
                        )
                        .await;
                    }
                } else {
                    let _ = session.confirm_tool_use(&tool_id, false, reason).await;
                }
            }
            AgentEvent::End { text, .. } => {
                if streamed.trim().is_empty() && !text.trim().is_empty() {
                    print!("{text}");
                }
                end_text = text;
                eprintln!("\n[end]");
                break;
            }
            AgentEvent::Error { message } => eprintln!("\n[error] {message}"),
            _ => {}
        }
    }
    // Let the stream task finish (incl. auto-save/persist) before we exit.
    if let Some(deadline) = deadline {
        // An End event already gives us the model result. Persisting the stream
        // worker may use the execution phase's remaining time, but it must not
        // consume the window reserved for recovery artifact publication.
        let join_budget = deadline
            .phase_remaining(Instant::now())
            .min(Duration::from_secs(30));
        if join_budget.is_zero() {
            abort.abort();
        } else {
            match tokio::time::timeout(join_budget, join).await {
                Ok(result) => result?,
                Err(_) => {
                    abort.abort();
                    eprintln!(
                        "[smoke] stream worker did not finish before the execution deadline; continuing with artifact finalization"
                    );
                }
            }
        }
    } else {
        tokio::time::timeout(Duration::from_secs(30), join)
            .await
            .map_err(|_| {
                anyhow::anyhow!("smoke stream worker did not finish after AgentEvent::End")
            })??;
    }
    if end_text.trim().is_empty() {
        Ok(streamed)
    } else {
        Ok(end_text)
    }
}

async fn run_smoke_deep_research(
    session: Arc<AgentSession>,
    workspace: &Path,
    query: String,
    deep_research_report_tool_gate: DeepResearchReportToolGate,
) -> anyhow::Result<()> {
    let run_started_at = Instant::now();
    let run_deadline = deep_research_smoke_run_deadline(run_started_at);
    let evidence_scope = deep_research_inferred_evidence_scope(&query);
    deep_research_report_tool_gate.set_workspace(workspace);
    deep_research_report_tool_gate.set_evidence_scope(evidence_scope);
    eprintln!("[smoke] deepresearch workflow: host-managed");
    let mut workflow_args = deep_research_workflow_args_with_scope(&query, evidence_scope);
    ensure_deep_research_workflow_run_id(&mut workflow_args);
    let run_id = workflow_args
        .get("run_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("DeepResearch smoke workflow has no run_id"))?
        .to_string();
    let total_budget_ms = DEEP_RESEARCH_INQUIRY_HOST_TIMEOUT_MS;
    record_deep_research_workflow_started(
        workspace,
        &run_id,
        ResearchSpec {
            query: query.clone(),
            current_date: workflow_args
                .pointer("/input/current_date")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| chrono::Local::now().date_naive().to_string()),
            evidence_scope: evidence_scope.label().to_string(),
            required_claims: Vec::new(),
            total_budget_ms,
            retrieval_stage_budget_ms: DEEP_RESEARCH_RETRIEVAL_STAGE_TIMEOUT_MS,
            question_review_stage_budget_ms: DEEP_RESEARCH_QUESTION_REVIEW_STAGE_TIMEOUT_MS,
            finalization_reserve_ms: DEEP_RESEARCH_INQUIRY_FINALIZATION_RESERVE_MS,
            host_pid: std::process::id(),
        },
    )
    .await?;
    let (mut progress_rx, mut workflow_join) =
        spawn_deep_research_inquiry(Arc::clone(&session), workflow_args.clone());
    let workflow_abort = workflow_join.abort_handle();
    let progress_drain = tokio::spawn(async move {
        while let Some(event) = progress_rx.recv().await {
            match event {
                AgentEvent::SubagentStart {
                    task_id,
                    agent,
                    description,
                    ..
                } => eprintln!("[smoke] child start: {agent} {task_id} · {description}"),
                AgentEvent::SubagentProgress {
                    task_id, status, ..
                } => eprintln!("[smoke] child progress: {task_id} · {status}"),
                AgentEvent::SubagentEnd {
                    task_id,
                    success,
                    output,
                    ..
                } => eprintln!(
                    "[smoke] child end: {task_id} · {} · {}",
                    if success { "ok" } else { "failed" },
                    output.lines().next().unwrap_or_default()
                ),
                AgentEvent::ToolExecutionStart { name, args, .. } => eprintln!(
                    "[smoke] child tool start: {name} · {}",
                    args.to_string().chars().take(240).collect::<String>()
                ),
                AgentEvent::ToolEnd {
                    name,
                    exit_code,
                    output,
                    ..
                } => eprintln!(
                    "[smoke] child tool end: {name} ({exit_code}) · {}",
                    output
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .chars()
                        .take(240)
                        .collect::<String>()
                ),
                AgentEvent::PermissionDenied {
                    tool_name, reason, ..
                } => eprintln!("[smoke] child tool denied: {tool_name} · {reason}"),
                AgentEvent::Error { message } => eprintln!("[smoke] child error: {message}"),
                _ => {}
            }
        }
    });
    let configured_timeout_ms = DEEP_RESEARCH_INQUIRY_HOST_TIMEOUT_MS;
    let workflow_deadline = deep_research_smoke_phase_deadline(
        run_deadline,
        Instant::now(),
        Duration::from_millis(configured_timeout_ms),
        "workflow",
    )
    .ok_or_else(|| deep_research_smoke_deadline_error("workflow"))?;
    let timeout_ms = workflow_deadline.selected_timeout_ms();
    let workflow = match tokio::time::timeout(
        workflow_deadline.phase_remaining(Instant::now()),
        &mut workflow_join,
    )
    .await
    {
        Ok(Ok(result)) => result.map_err(|err| err.to_string()),
        Ok(Err(err)) => Err(err.to_string()),
        Err(_) => {
            workflow_abort.abort();
            let abort_grace = workflow_deadline
                .run_remaining(Instant::now())
                .min(Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS));
            if !abort_grace.is_zero() {
                let _ = tokio::time::timeout(abort_grace, &mut workflow_join).await;
            }
            let _ = session
                .cancel_and_settle(
                    Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                    Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                )
                .await;
            let message = format!(
                "dynamic_workflow timed out after {timeout_ms} ms while gathering DeepResearch evidence"
            );
            run_deep_research_smoke_artifact_step(
                run_deadline,
                "workflow timeout artifact fallback",
                || deep_research_workflow_timeout_tool_result(workspace, &workflow_args, message),
            )?
        }
    };
    progress_drain.abort();

    let workflow_succeeded = workflow.is_ok();
    let (mut workflow_output, exit_code, mut metadata) = match workflow {
        Ok(result) => (result.output, result.exit_code, result.metadata),
        Err(error) => (error, 1, None),
    };
    workflow_output = deep_research_canonical_workflow_output(&workflow_output, metadata.as_ref());
    eprintln!("[smoke] deepresearch workflow exit: {exit_code}");
    let accepted_evidence = accepted_evidence_ledger(&workflow_output, metadata.as_ref());
    record_deep_research_workflow_completed(workspace, &run_id, workflow_succeeded).await?;
    record_deep_research_evidence_ledger(workspace, &run_id, &accepted_evidence).await?;
    let inquiry_projection = inquiry_projection_from_workflow(&workflow_output, metadata.as_ref());
    let convergence = match inquiry_projection.as_ref() {
        Ok(Some((_, state))) => evaluate_terminal_inquiry_convergence(state),
        Ok(None) => ConvergenceDecision {
            action: ConvergenceAction::Degrade,
            reason: "the DeepResearch smoke run returned without its required Inquiry projection"
                .to_string(),
        },
        Err(error) => ConvergenceDecision {
            action: ConvergenceAction::Degrade,
            reason: format!(
                "the DeepResearch smoke Inquiry projection failed strict replay: {error}"
            ),
        },
    };
    let evidence_outcome = deep_research_report_outcome_for_workflow(
        &query,
        evidence_scope,
        &workflow_output,
        metadata.as_ref(),
    );
    let sectioned_report_ready = sectioned_report_available(&workflow_output, metadata.as_ref());
    if accepted_evidence.is_empty()
        || convergence.action == ConvergenceAction::Degrade
        || matches!(evidence_outcome, DeepResearchRunOutcome::Degraded)
        || !sectioned_report_ready
    {
        deep_research_report_tool_gate.reset();
        let artifacts = run_deep_research_smoke_artifact_step(
            run_deadline,
            "failed-collection recovery report",
            || {
                materialize_deep_research_recovery_report(
                    workspace,
                    &query,
                    &format!(
                        "Evidence collection ended without a reportable Inquiry. Terminal contract assessment: {}. No second retrieval or synthesis pass was started.",
                        convergence.reason
                    ),
                    &workflow_output,
                    metadata.as_ref(),
                )
            },
        )?
        .map_err(anyhow::Error::msg)?;
        eprintln!("[smoke] evidence collection was terminally degraded; skipped model synthesis");
        eprintln!(
            "[smoke] recovery report.md: {}",
            artifacts.markdown.display()
        );
        eprintln!("[smoke] recovery index.html: {}", artifacts.html.display());
        let outcome = finalize_deep_research_smoke_journal(
            workspace,
            &run_id,
            &workflow_output,
            metadata.as_ref(),
            DeepResearchRunOutcome::Degraded,
            &artifacts,
        )
        .await?;
        return outcome.ensure_smoke_success(&artifacts);
    }
    eprintln!("[smoke] deepresearch synthesis");
    deep_research_report_tool_gate.set_synthesis_only();
    let synthesis_timeout_ms = DEEP_RESEARCH_SECTIONED_SYNTHESIS_TIMEOUT_MS;
    let mut generated_report = None;
    let mut final_text = if let Some(synthesis_deadline) = deep_research_smoke_phase_deadline(
        run_deadline,
        Instant::now(),
        Duration::from_millis(synthesis_timeout_ms),
        "synthesis",
    ) {
        let generated = generate_smoke_sectioned_report(
            session.as_ref(),
            &query,
            &workflow_output,
            metadata.as_ref(),
            &run_id,
            synthesis_deadline,
        )
        .await;
        match generated {
            Ok((report, sectioned_metadata)) => {
                if !merge_sectioned_inquiry_projection(
                    &mut workflow_output,
                    metadata.as_mut(),
                    sectioned_metadata.as_ref(),
                )
                .map_err(anyhow::Error::msg)?
                {
                    anyhow::bail!(
                        "sectioned DeepResearch synthesis did not merge a terminal Inquiry projection"
                    );
                }
                deep_research_inquiry_publication_outcome(&workflow_output, metadata.as_ref())
                    .map_err(anyhow::Error::msg)?;
                let markdown = report.markdown.clone();
                generated_report = Some(report);
                markdown
            }
            Err(error) => {
                eprintln!("[smoke] DeepResearch structured synthesis failed: {error}");
                error
            }
        }
    } else {
        let status = deep_research_smoke_exhausted_phase_message("synthesis");
        eprintln!("[smoke] {status}");
        status
    };
    let publication_ready =
        deep_research_inquiry_publication_outcome(&workflow_output, metadata.as_ref()).is_ok();
    let mut artifacts = None;
    if let (true, Some(report)) = (publication_ready, generated_report.as_ref()) {
        match run_deep_research_smoke_artifact_step(
            run_deadline,
            "structured synthesis materialization",
            || {
                materialize_deep_research_completed_report_from_generation(
                    workspace,
                    &query,
                    report,
                    &workflow_output,
                    metadata.as_ref(),
                )
            },
        )? {
            Ok(generated_artifacts) => artifacts = Some(generated_artifacts),
            Err(error) => {
                eprintln!("[smoke] DeepResearch structured synthesis rejected: {error}")
            }
        }
    }

    if let Some(clean_text) = artifacts
        .as_ref()
        .and_then(|artifacts| clean_deep_research_final_text_from_artifacts(artifacts, workspace))
    {
        final_text = clean_text;
    }
    if artifacts.is_none() {
        let diagnostic = deep_research_report_rejection_diagnostic_from_answer_text(
            &query,
            &final_text,
            &workflow_output,
            metadata.as_ref(),
        )
        .unwrap_or_else(|| "report artifacts were not accepted for an unknown reason".to_string());
        eprintln!(
            "[smoke] DeepResearch synthesis report rejected ({} chars): {diagnostic}",
            final_text.chars().count()
        );
    }

    let mut outcome = evidence_outcome;
    if artifacts.is_none() {
        eprintln!("[smoke] deepresearch report missing; materializing recovery report");
        deep_research_report_tool_gate.reset();
        let recovery_artifacts = run_deep_research_smoke_artifact_step(
            run_deadline,
            "recovery artifact fallback",
            || {
                materialize_deep_research_recovery_report(
                    workspace,
                    &query,
                    &final_text,
                    &workflow_output,
                    metadata.as_ref(),
                )
            },
        )?
        .map_err(anyhow::Error::msg)?;
        artifacts = Some(recovery_artifacts);
        outcome = DeepResearchRunOutcome::Degraded;
    }

    let artifacts = artifacts.ok_or_else(|| {
        anyhow::anyhow!(
            "DeepResearch smoke did not produce the required host-materialized `.a3s/research/<slug>/report.md` and `index.html` artifacts"
        )
    })?;
    deep_research_report_tool_gate.reset();
    if !final_text.trim().is_empty() && !deep_research_output_has_internal_leak(&final_text) {
        println!("{final_text}");
    }
    eprintln!("[smoke] report.md: {}", artifacts.markdown.display());
    eprintln!("[smoke] index.html: {}", artifacts.html.display());
    let outcome = finalize_deep_research_smoke_journal(
        workspace,
        &run_id,
        &workflow_output,
        metadata.as_ref(),
        outcome,
        &artifacts,
    )
    .await?;
    run_deep_research_smoke_artifact_step(run_deadline, "final report validation", || {
        outcome.ensure_smoke_success(&artifacts)
    })?
}
