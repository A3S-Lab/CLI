//! Headless TUI and DeepResearch smoke-mode execution.

use super::app_submit::direct_shell_tool_args;
use super::*;

/// Headless probe of the same `session.stream()` / `AgentEvent` path the TUI
/// uses. A headless process has no human authority, so any unexpected
/// confirmation request is rejected rather than manufacturing consent.
pub(super) async fn run_smoke(
    session: Arc<AgentSession>,
    workspace: &Path,
    code_config: CodeConfig,
    memory_dir: PathBuf,
    deep_research_report_tool_gate: DeepResearchReportToolGate,
) -> anyhow::Result<()> {
    let prompt = std::env::var("A3S_CODE_TUI_PROMPT")
        .unwrap_or_else(|_| "Reply with exactly one short sentence: what is 2 + 2?".to_string());
    if let Some(query) = prompt.trim().strip_prefix('?') {
        let query = query.trim().to_string();
        if query.is_empty() {
            anyhow::bail!("A3S_CODE_TUI_PROMPT starts with `?` but has no DeepResearch query");
        }
        return run_smoke_deep_research(
            workspace,
            query,
            code_config,
            memory_dir,
            deep_research_report_tool_gate,
        )
        .await;
    }
    if let Some(command) = prompt.trim().strip_prefix('!') {
        let command = command.trim();
        if command.is_empty() {
            anyhow::bail!("A3S_CODE_TUI_PROMPT starts with `!` but has no shell command");
        }
        return run_smoke_shell(session.as_ref(), command).await;
    }
    if prompt.trim().eq_ignore_ascii_case("@mechanisms") {
        return run_smoke_mechanisms(session).await;
    }
    eprintln!("[smoke] prompt: {prompt}");
    let _ = stream_smoke_prompt(session.as_ref(), prompt.as_str()).await?;
    Ok(())
}

/// Headless E2E for host mechanisms that do not need a model turn:
/// Plan style hot-switch, lexical BM25 via `search`, optional `web_search`,
/// and a direct shell sanity check.
async fn run_smoke_mechanisms(session: Arc<AgentSession>) -> anyhow::Result<()> {
    eprintln!("[smoke] mechanisms: plan style + bm25 + optional web_search + shell");

    session
        .set_agent_style(Some(a3s_code_core::AgentStyle::Plan))
        .map_err(|error| anyhow::anyhow!("set_agent_style(Plan) failed: {error:#}"))?;
    eprintln!("[smoke] plan style: set_agent_style(Plan) ok");
    session
        .set_agent_style(None)
        .map_err(|error| anyhow::anyhow!("set_agent_style(None) failed: {error:#}"))?;
    eprintln!("[smoke] plan style: clear ok");

    let marker = "mechanism_e2e_marker_token";
    let bm25_deadline = Instant::now() + Duration::from_secs(60);
    let mut last_bm25 = String::from("bm25 search never started");
    let bm25_hit = loop {
        match session
            .tool(
                "search",
                serde_json::json!({
                    "mode": "bm25",
                    "query": marker,
                    "limit": 5,
                }),
            )
            .await
        {
            Ok(result) if result.exit_code == 0 && result.output.contains(marker) => {
                eprintln!(
                    "[smoke] bm25: hit\n{}",
                    result.output.lines().take(8).collect::<Vec<_>>().join("\n")
                );
                break true;
            }
            Ok(result) => {
                last_bm25 = format!(
                    "exit {} · {}",
                    result.exit_code,
                    result
                        .output
                        .lines()
                        .take(4)
                        .collect::<Vec<_>>()
                        .join(" | ")
                );
                eprintln!("[smoke] bm25: warming ({last_bm25})");
            }
            Err(error) => {
                last_bm25 = format!("{error:#}");
                eprintln!("[smoke] bm25: error ({last_bm25})");
            }
        }
        if Instant::now() >= bm25_deadline {
            break false;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    if !bm25_hit {
        anyhow::bail!("bm25 search did not find `{marker}` within 60s: {last_bm25}");
    }

    if std::env::var_os("A3S_CODE_TUI_SMOKE_SKIP_WEB").is_none() {
        let web_timeout = Duration::from_secs(90);
        eprintln!(
            "[smoke] web_search: probing (timeout {}s)",
            web_timeout.as_secs()
        );
        match tokio::time::timeout(
            web_timeout,
            session.tool(
                "web_search",
                serde_json::json!({
                    "query": "A3S Lab",
                    "limit": 3,
                }),
            ),
        )
        .await
        {
            Ok(Ok(result)) if result.exit_code == 0 => {
                eprintln!(
                    "[smoke] web_search: ok\n{}",
                    result.output.lines().take(6).collect::<Vec<_>>().join("\n")
                );
            }
            Ok(Ok(result)) => {
                eprintln!(
                    "[smoke] web_search: soft-fail exit {}\n{}",
                    result.exit_code,
                    result.output.lines().take(8).collect::<Vec<_>>().join("\n")
                );
            }
            Ok(Err(error)) => {
                eprintln!("[smoke] web_search: soft-fail {error:#}");
            }
            Err(_) => {
                eprintln!("[smoke] web_search: soft-fail timeout after {web_timeout:?}");
            }
        }
    } else {
        eprintln!("[smoke] web_search: skipped (A3S_CODE_TUI_SMOKE_SKIP_WEB)");
    }

    run_smoke_shell(session.as_ref(), "echo mechanisms-ok").await?;
    eprintln!("[smoke] mechanisms: all hard checks passed");
    Ok(())
}

async fn run_smoke_shell(session: &AgentSession, command: &str) -> anyhow::Result<()> {
    eprintln!("[smoke] direct shell: {command}");
    let result = session
        .tool("bash", direct_shell_tool_args(command))
        .await?;
    let output =
        enrich_tool_failure_output(&result.output, result.exit_code, result.error_kind.as_ref());
    if !output.trim().is_empty() {
        print!("{output}");
        if !output.ends_with('\n') {
            println!();
        }
    }
    eprintln!("[shell end] exit {}", result.exit_code);
    if result.exit_code != 0 {
        anyhow::bail!("direct shell exited with status {}", result.exit_code);
    }
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

#[cfg(test)]
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

async fn stream_smoke_prompt(session: &AgentSession, prompt: &str) -> anyhow::Result<String> {
    stream_smoke_prompt_inner(session, prompt, None).await
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
    workspace: &Path,
    query: String,
    code_config: CodeConfig,
    memory_dir: PathBuf,
    deep_research_report_tool_gate: DeepResearchReportToolGate,
) -> anyhow::Result<()> {
    let evidence_scope = deep_research_default_evidence_scope();
    deep_research_report_tool_gate.set_workspace(workspace);
    deep_research_report_tool_gate.set_evidence_scope(evidence_scope);
    eprintln!("[smoke] deepresearch workflow: typed CodeDeepResearchRunner");
    let synthesis = crate::commands::code::research_runtime::execute_deepresearch_query_in(
        &query,
        Some(evidence_scope),
        deep_research_default_budget(),
        workspace,
        code_config,
        memory_dir,
    )
    .await?;
    deep_research_report_tool_gate.reset();
    let outcome = match synthesis.status {
        PublicationOutcome::Synthesized => DeepResearchRunOutcome::Completed,
        PublicationOutcome::Qualified => DeepResearchRunOutcome::Qualified,
        PublicationOutcome::SourceBacked => DeepResearchRunOutcome::SourceBacked,
        PublicationOutcome::NoEvidence => DeepResearchRunOutcome::NoEvidence,
    };
    if !synthesis.text.trim().is_empty() {
        println!("{}", synthesis.text);
    }
    eprintln!(
        "[smoke] deepresearch run {} · {:?} · {}",
        synthesis.run_id,
        synthesis.status,
        synthesis.artifacts.html.display()
    );
    outcome.ensure_smoke_success(&synthesis.artifacts)
}
