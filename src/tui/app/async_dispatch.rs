//! Asynchronous runtime, account, and resource-result dispatch for the Code TUI.

use super::*;

impl App {
    pub(super) fn handle_async_message(&mut self, msg: Msg) -> Option<Cmd<Msg>> {
        match msg {
            Msg::TaskPanelData {
                session_id,
                generation,
                request_id,
                tasks,
            } => {
                self.apply_task_panel_data(session_id, generation, request_id, tasks);
            }
            Msg::TaskPanelTick { generation } => {
                return self.handle_task_panel_tick(generation);
            }
            Msg::TaskPanelCancelFinished {
                session_id,
                generation,
                task_id,
                cancelled,
            } => {
                return self
                    .apply_task_panel_cancel_result(session_id, generation, task_id, cancelled);
            }
            Msg::BackgroundSubagentFinished {
                session_id,
                generation,
                task_id,
                agent,
                output,
                outcome,
                finished_ms,
            } => {
                let was_watched = self
                    .background_subagent_watches
                    .remove(&(generation, task_id.clone()));
                if !was_watched {
                    // A terminal DeepResearch parent deliberately removes its
                    // child watches after authoritative settlement. Ignore the
                    // late watcher result instead of duplicating transcript
                    // cells or recreating footer state.
                    return None;
                }
                if !subagent_watch_is_current(
                    &self.session_id,
                    self.session_rebuild_seq,
                    &session_id,
                    generation,
                ) {
                    return None;
                }
                let completed = self.runtime.end_subagent_with_outcome(
                    task_id,
                    agent,
                    output,
                    outcome,
                    instant_from_epoch_ms(finished_ms),
                );
                self.push_subagent_completion(completed);
            }

            Msg::BackgroundSubagentWatchStopped {
                session_id,
                generation,
                task_id,
            } => {
                let was_watched = self
                    .background_subagent_watches
                    .remove(&(generation, task_id));
                if !was_watched {
                    return None;
                }
                if !subagent_watch_is_current(
                    &self.session_id,
                    self.session_rebuild_seq,
                    &session_id,
                    generation,
                ) {
                    return None;
                }
                // The terminal event may have raced the parent stream close.
                // Reconcile with the authoritative session tracker so a lost
                // event cannot leave a permanent live footer row.
                return Some(self.request_subagent_snapshots());
            }

            Msg::SubagentSnapshots {
                session_id,
                generation,
                request_id,
                snapshots,
            } => {
                if !subagent_snapshot_is_current(
                    &self.session_id,
                    self.session_rebuild_seq,
                    self.subagent_snapshot_request_id,
                    self.deep_research_subagent_settlement_inflight,
                    &session_id,
                    generation,
                    request_id,
                ) {
                    return None;
                }
                self.background_subagent_watches
                    .retain(|(watch_generation, _)| *watch_generation == generation);
                // A rebuild may switch to an entirely different session. The
                // restored tracker snapshot is authoritative for live rows;
                // durable transcript cells remain independently retained.
                self.runtime.clear_subagent_entities();
                let mut commands = Vec::new();
                for restored in snapshots {
                    let snapshot = restored.snapshot;
                    self.runtime.restore_subagent(
                        snapshot.task_id.clone(),
                        snapshot.agent.clone(),
                        snapshot.description.clone(),
                        instant_from_epoch_ms(snapshot.started_ms),
                        restored.parent_result_expected,
                    );
                    for progress in &snapshot.progress {
                        self.runtime
                            .record_subagent_progress(&snapshot.task_id, &progress.metadata);
                    }
                    if snapshot.status == a3s_code_core::SubagentStatus::Running {
                        if self.session.is_closed() {
                            let completed = self.runtime.end_subagent_with_outcome(
                                snapshot.task_id,
                                snapshot.agent,
                                "Subagent tracking ended with the session before a terminal event was observed."
                                    .to_string(),
                                SubagentOutcome::TrackingLost,
                                instant_from_epoch_ms(snapshot.updated_ms),
                            );
                            self.push_subagent_completion(completed);
                        } else if self
                            .background_subagent_watches
                            .insert((generation, snapshot.task_id.clone()))
                        {
                            commands.push(watch_background_subagent(
                                self.session.clone(),
                                session_id.clone(),
                                generation,
                                snapshot.task_id,
                            ));
                        }
                        continue;
                    }
                    self.background_subagent_watches
                        .remove(&(generation, snapshot.task_id.clone()));
                    let outcome = match snapshot.status {
                        a3s_code_core::SubagentStatus::Completed => SubagentOutcome::Succeeded,
                        a3s_code_core::SubagentStatus::Cancelled => SubagentOutcome::Cancelled,
                        a3s_code_core::SubagentStatus::Failed => SubagentOutcome::Failed,
                        a3s_code_core::SubagentStatus::Running => {
                            unreachable!("running snapshots continue before terminal mapping")
                        }
                        _ => SubagentOutcome::TrackingLost,
                    };
                    let output = snapshot.output.unwrap_or_else(|| match snapshot.status {
                        a3s_code_core::SubagentStatus::Cancelled => "Task cancelled.".to_string(),
                        a3s_code_core::SubagentStatus::Failed => "Task failed.".to_string(),
                        _ => String::new(),
                    });
                    let completed = self.runtime.end_subagent_with_outcome(
                        snapshot.task_id,
                        snapshot.agent,
                        output,
                        outcome,
                        instant_from_epoch_ms(snapshot.finished_ms.unwrap_or(snapshot.updated_ms)),
                    );
                    self.push_subagent_completion(completed);
                }
                self.relayout();
                self.rebuild_viewport();
                if !commands.is_empty() {
                    return Some(cmd::batch(commands));
                }
            }

            Msg::DeepResearchSubagentsSettled {
                session_id,
                generation,
                exit,
                settlements,
            } => {
                if !self.deep_research_subagent_settlement_inflight
                    || !subagent_watch_is_current(
                        &self.session_id,
                        self.session_rebuild_seq,
                        &session_id,
                        generation,
                    )
                {
                    return None;
                }
                self.deep_research_subagent_settlement_inflight = false;
                self.invalidate_subagent_snapshots();
                for settlement in settlements {
                    self.background_subagent_watches
                        .remove(&(generation, settlement.task_id.clone()));
                    let completed = self.runtime.end_subagent_with_outcome(
                        settlement.task_id,
                        settlement.agent,
                        settlement.output,
                        settlement.outcome,
                        instant_from_epoch_ms(settlement.finished_ms),
                    );
                    self.push_subagent_completion(completed);
                }
                self.state = State::Idle;
                self.running_task = None;
                self.spinner.stop();
                self.relayout();
                self.rebuild_viewport();
                return self.finalize_deep_research_settlement(exit);
            }

            Msg::DeepResearchJournalFinalized {
                run_id,
                exit,
                result,
            } => {
                let current_run_id = self
                    .deep_research_workflow
                    .args
                    .as_ref()
                    .and_then(|args| args.get("run_id"))
                    .and_then(serde_json::Value::as_str);
                if !self.deep_research_journal_finalization_inflight
                    || current_run_id != Some(run_id.as_str())
                {
                    return None;
                }
                self.deep_research_journal_finalization_inflight = false;
                match result {
                    Ok(projection) => {
                        debug_assert!(projection.outcome.is_terminal());
                        let projected_outcome = match projection.outcome {
                            ResearchOutcome::Completed => DeepResearchRunOutcome::Completed,
                            ResearchOutcome::Qualified => DeepResearchRunOutcome::Qualified,
                            ResearchOutcome::Degraded | ResearchOutcome::Failed => {
                                DeepResearchRunOutcome::Degraded
                            }
                            ResearchOutcome::Active => {
                                unreachable!("terminal projection is active")
                            }
                        };
                        if projected_outcome != self.deep_research_outcome {
                            let reason = projection
                                .report_audit_reason
                                .as_deref()
                                .unwrap_or("report audit did not pass");
                            self.push_line(
                                &Style::new().fg(TN_YELLOW).render(&format!(
                                    "  ⚠ DeepResearch report downgraded: {reason}"
                                )),
                            );
                        }
                        self.deep_research_outcome = projected_outcome;
                        if !projected_outcome.report_ready() {
                            self.pending_deep_research_report_view = None;
                        }
                        self.deep_research_projection = Some(projection);
                        self.plan.clear();
                        self.runtime.clear_subagent_entities();
                        self.running_task = None;
                        self.relayout();
                        self.rebuild_viewport();
                    }
                    Err(error) => self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                        "  ⚠ DeepResearch terminal state journal failed: {error}"
                    ))),
                }
                return self.complete_deep_research_settlement(exit);
            }
            Msg::DeepResearchJournalEventRecorded { run_id, result } => {
                let current_run_id = self
                    .deep_research_workflow
                    .args
                    .as_ref()
                    .and_then(|args| args.get("run_id"))
                    .and_then(serde_json::Value::as_str);
                if current_run_id != Some(run_id.as_str()) {
                    return None;
                }
                match result {
                    Ok(projection) => self.deep_research_projection = Some(projection),
                    Err(error) => self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                        "  ⚠ DeepResearch event projection failed: {error}"
                    ))),
                }
                self.relayout();
                self.rebuild_viewport();
                return self.rx.clone().map(pump);
            }

            Msg::Resume => {
                if let Some(rx) = self.rx.clone() {
                    return Some(pump(rx));
                }
            }

            Msg::ShellOutput(text) => {
                let body = text.lines().take(40).collect::<Vec<_>>().join("\n");
                self.push_line(&gutter(TN_GRAY, body.trim_end()));
            }
            Msg::ResearchDiagnostic(result) => match result {
                Ok(text) => self.push_line(&gutter(TN_CYAN, &text)),
                Err(error) => self.push_line(
                    &Style::new()
                        .fg(TN_YELLOW)
                        .render(&format!("  research diagnostic failed: {error}")),
                ),
            },

            Msg::DeepResearchWorkflowCompleted {
                query,
                os_runtime,
                args,
                result,
                convergence,
                accepted_evidence,
            } => {
                return self.on_deep_research_workflow_completed(
                    query,
                    os_runtime,
                    args,
                    result,
                    convergence,
                    accepted_evidence,
                )
            }

            Msg::DeepResearchReportGenerated {
                token,
                query,
                phase,
                result,
            } => return self.on_deep_research_report_generated(token, query, phase, result),

            Msg::DeepResearchSynthesisTimedOut { token } => {
                return self.on_deep_research_synthesis_timed_out(token);
            }

            Msg::DeepResearchSynthesisTimedOutAfterCancel {
                token,
                status,
                streamed_text,
                report_completed,
            } => {
                return self.on_deep_research_synthesis_timed_out_after_cancel(
                    token,
                    status,
                    streamed_text,
                    report_completed,
                );
            }

            Msg::UpdatePlan(latest) => {
                self.updating = None;
                self.relayout();
                let current = crate::update::current_version();
                match latest {
                    None => self.push_line(
                        &Style::new()
                            .fg(TN_YELLOW)
                            .render("  couldn't reach the release server — try again later"),
                    ),
                    Some(l) if crate::update::version_ge(&current, &l) => {
                        self.push_line(
                            &Style::new()
                                .fg(TN_GREEN)
                                .render(&format!("  ✓ already up to date (a3s {current})")),
                        );
                    }
                    Some(l) => {
                        // macOS/Linux self-update in place (Homebrew or a direct
                        // download); unsupported platforms get the download link.
                        if crate::update::can_self_update() {
                            if let Ok(mut g) = LATEST.lock() {
                                *g = Some(l.clone());
                            }
                            UPGRADE_ON_EXIT.store(true, std::sync::atomic::Ordering::Relaxed);
                            self.push_line(&Style::new().fg(TN_GREEN).render(&format!(
                                "  → a3s {l} available — closing to upgrade, then restarting…"
                            )));
                            return self.begin_graceful_quit();
                        }
                        self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                            "  → a3s {l} available — download: https://github.com/A3S-Lab/Cli/releases/latest"
                        )));
                    }
                }
            }

            Msg::OsLogin {
                status_entry,
                result,
            } => match result {
                Ok(label) => {
                    // The browser flow already saved to disk; load it into memory
                    // and rebuild so the login-gated skill activates this run.
                    self.os_session = self
                        .os_config
                        .as_ref()
                        .and_then(crate::a3s_os::current_session);
                    if let Some(s) = &self.os_session {
                        crate::a3s_os::export_os_env(s);
                    }
                    let rebuild = self.refresh_after_auth();
                    self.replace_tracked_line(
                        status_entry,
                        &Style::new().fg(TN_GREEN).render(&format!(
                            "  ✓ signed in to OS as {label} · capabilities skill active"
                        )),
                    );
                    // Auto-register this machine's SSH public key with OS so
                    // git-over-SSH works without manual key setup (idempotent,
                    // best-effort — never blocks the completed login).
                    if let Some(s) = self.os_session.clone() {
                        let ssh = cmd::cmd(move || async move {
                            Msg::SshKeySynced(crate::a3s_os::sync_ssh_key(s).await)
                        });
                        return Some(match rebuild {
                            Some(rebuild) => cmd::batch(vec![rebuild, ssh]),
                            None => ssh,
                        });
                    }
                    return rebuild;
                }
                Err(error) => self.replace_tracked_line(
                    status_entry,
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  login failed: {error}")),
                ),
            },

            Msg::SshKeySynced(outcome) => {
                use crate::a3s_os::SshKeyOutcome;
                match outcome {
                    SshKeyOutcome::Registered(fp) => self.push_line(&Style::new().fg(TN_GREEN).render(
                        &format!("  ✓ local SSH public key registered with OS ({fp}) · git clone(ssh) ready"),
                    )),
                    SshKeyOutcome::AlreadyRegistered => self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render("  · SSH public key already registered with OS; skipping"),
                    ),
                    SshKeyOutcome::NoLocalKey => self.push_line(&Style::new().fg(TN_YELLOW).render(
                        "  · no local SSH public key found; create one and run /login again to register it automatically: ssh-keygen -t ed25519",
                    )),
                    SshKeyOutcome::Failed(e) => self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render(&format!("  · SSH key sync skipped: {e}")),
                    ),
                }
            }

            Msg::OsRefreshed(result) => {
                self.os_refreshing = false;
                match result {
                    Ok(session) => {
                        // Re-export the fresh token so the agent's $A3S_OS_TOKEN
                        // stays valid; no session rebuild needed. Re-sync the
                        // runtime tool too because it owns a token snapshot.
                        crate::a3s_os::export_os_env(&session);
                        self.os_session = Some(session);
                        self.sync_runtime_tool();
                    }
                    Err(_) => {
                        // Leave the existing session; the next BannerTick retries
                        // while it's still within the refresh window, and /login
                        // remains the fallback once it truly expires.
                    }
                }
            }

            Msg::CodexModels(result) => {
                self.codex_models_loading = false;
                if let Ok(models) = result {
                    for model in &models {
                        if let Some(context_window) = model.context_window {
                            self.model_ctx.insert(model.slug.clone(), context_window);
                            if self.model.as_deref() == Some(model.slug.as_str()) {
                                self.context_limit = context_window;
                            }
                        }
                    }
                    self.codex_account_models = models;
                    self.codex_models_refreshed_at = Some(Instant::now());
                    self.clamp_open_model_menu_selection();
                    // The Codex override is immutable per materialized session,
                    // so refresh it after the CLI updates account capabilities.
                    // A failed rebuild leaves the existing client/session intact.
                    if self.state == State::Idle
                        && matches!(self.llm_override.as_ref(), Some(LlmOverride::Codex(_)))
                    {
                        let profile = self.session_rebuild_profile();
                        return self.start_session_rebuild(
                            profile,
                            SessionRebuildAction::Refresh {
                                failure_context: None,
                            },
                        );
                    }
                }
            }

            Msg::SessionRebuilt {
                request_id,
                action,
                result,
            } => {
                let selected_effort = match &action {
                    SessionRebuildAction::Effort { selected, .. } => Some(*selected),
                    SessionRebuildAction::GoalStart { .. } => Some(ULTRACODE),
                    _ => None,
                };
                let starts_ultracode_border = ultracode_rebuild_starts_border(
                    selected_effort,
                    matches!(
                        result.as_ref(),
                        panels::model::SessionRebuildResult::Success(..)
                    ),
                );
                let previous_gradient_start = self.gradient_until;
                let follow_up = self.finish_session_rebuild(request_id, action, *result);
                let mut commands = vec![self.request_subagent_snapshots()];
                if starts_ultracode_border
                    && self.gradient_until.is_some()
                    && self.gradient_until != previous_gradient_start
                {
                    let epoch =
                        advance_ultracode_animation_epoch(&mut self.ultracode_animation_epoch);
                    commands.push(ultracode_tick(epoch));
                }
                if let Some(follow_up) = follow_up {
                    commands.push(follow_up);
                }
                return Some(cmd::batch(commands));
            }

            Msg::OsGatewayModels {
                login_at_ms,
                result,
            } => {
                if self
                    .os_session
                    .as_ref()
                    .is_none_or(|session| session.login_at_ms != login_at_ms)
                {
                    return None;
                }
                self.os_gateway_models_loading = false;
                match result {
                    // Record each model's real context window (when the gateway
                    // reports one) so switching to it sizes auto-compact + the
                    // status bar correctly, then cache the ids.
                    Ok(models) => {
                        for m in &models {
                            if let Some(ctx) = m.context {
                                self.model_ctx.insert(m.id.clone(), ctx);
                            }
                        }
                        self.os_gateway_models = Some(models.into_iter().map(|m| m.id).collect());
                        self.os_gateway_error = None;
                    }
                    // Keep the precise reason so the picker + switch attempt can
                    // explain WHY the gateway is unavailable.
                    Err(e) => {
                        self.os_gateway_models = Some(Vec::new());
                        self.os_gateway_error = Some(e);
                    }
                }
                self.clamp_open_model_menu_selection();
            }

            Msg::AccountModels { provider, result } => {
                self.account_models_loading.remove(&provider);
                match result {
                    Ok(models) if !models.is_empty() => {
                        self.account_models.insert(provider, models);
                        self.account_model_errors.remove(&provider);
                    }
                    Ok(_) => {
                        self.account_model_errors
                            .insert(provider, "the account returned no models".to_string());
                    }
                    Err(error) => {
                        self.account_model_errors.insert(provider, error);
                    }
                }
                self.clamp_open_model_menu_selection();
            }

            Msg::Forked { request_id, result } => {
                if self.session_rebuild_pending != Some(request_id) {
                    return None;
                }
                // The snapshot copy and session materialization are one
                // logical single-flight operation. Release the copy phase and
                // immediately reserve the rebuild phase in this same update.
                self.session_rebuild_pending = None;
                match result {
                    Ok(new_id) => {
                        let mut profile = self.session_rebuild_profile();
                        profile.session_id = new_id.clone();
                        return self.start_session_rebuild(
                            profile,
                            SessionRebuildAction::Fork { session_id: new_id },
                        );
                    }
                    Err(e) => {
                        self.push_line(&Style::new().fg(TN_YELLOW).render(&format!("  /fork: {e}")))
                    }
                }
            }
            Msg::RelayData { request_id, result } => {
                self.apply_relay_scan(request_id, result);
            }
            Msg::MemoryLoaded(data) => {
                if let Some(m) = &mut self.memory {
                    let source = if data.loaded_from_session {
                        "session fallback · "
                    } else {
                        ""
                    };
                    m.note = format!(
                        "{source}{} memories · {} entities · {} relations",
                        data.entries.len(),
                        data.graph.stats.entities,
                        data.graph.stats.relations
                    );
                    m.sel = 0;
                    m.apply_data(data);
                }
            }
            Msg::MemoryForgotten(result) => {
                if let Some(m) = &mut self.memory {
                    match result {
                        Ok((id, data)) => {
                            m.note = format!(
                                "forgot {id} · {} memories · {} candidates",
                                data.entries.len(),
                                data.graph.stats.forget_candidates
                            );
                            m.apply_data(data);
                        }
                        Err(error) => {
                            m.note = format!("forget failed: {error}");
                        }
                    }
                }
            }
            Msg::AssetListLoaded(result) => self.on_asset_list(result),
            Msg::RuntimeActivityLoaded(result) => self.on_runtime_activity(result),
            Msg::KbAdded(summary) => {
                let color = if summary.starts_with('✗') {
                    TN_RED
                } else {
                    TN_GRAY
                };
                self.push_line(&Style::new().fg(color).render(&format!("  {summary}")));
                if self.kb.is_some() {
                    self.open_kb_home(Some(summary));
                }
            }
            Msg::CtxResults {
                status_entry,
                result,
            } => self.on_ctx_results(status_entry, result),
            Msg::CtxWindow {
                status_entry,
                result,
            } => self.on_ctx_window(status_entry, result),
            Msg::CtxSaved(res) => self.on_ctx_saved(res),

            Msg::SleepSaved(res) => self.on_sleep_saved(res),

            Msg::FlowOsCompleted {
                status_entry,
                result,
            } => self.on_flow_os_completed(status_entry, result),
            Msg::AgentOsCompleted {
                status_entry,
                result,
            } => self.on_agent_os_completed(status_entry, result),
            Msg::McpOsCompleted {
                status_entry,
                result,
            } => self.on_mcp_os_completed(status_entry, result),
            Msg::SkillOsCompleted {
                status_entry,
                result,
            } => self.on_skill_os_completed(status_entry, result),
            Msg::OkfOsCompleted {
                status_entry,
                result,
            } => self.on_okf_os_completed(status_entry, result),
            Msg::AssetCloned {
                status_entry,
                result,
            } => match result {
                Ok(result) => self.on_asset_cloned(status_entry, result),
                Err(error) => self.replace_tracked_line(
                    status_entry,
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  clone failed: {error}")),
                ),
            },
            Msg::CtxMemorySource(res) => match res {
                Ok((event_id, window)) => {
                    self.memory = None; // leave the panel to show the source
                    self.open_readonly_in_ide(&format!("ctx-source-{event_id}.txt"), &window);
                }
                Err(e) => {
                    if let Some(m) = self.memory.as_mut() {
                        m.note = format!("ctx source unavailable: {e}");
                    }
                }
            },
            _ => {}
        }
        None
    }
}
