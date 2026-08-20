//! Terminal event/update loop and footer presentation for the Code TUI.

use super::*;

const STARTUP_TRANSCRIPT_RENDER_LIMIT: usize = 128;

impl Model for App {
    type Msg = Msg;

    fn init(&mut self) -> Option<Cmd<Msg>> {
        if self.messages.is_empty() {
            self.viewport.set_content(&self.banner());
        } else if self.messages.len() > STARTUP_TRANSCRIPT_RENDER_LIMIT {
            // A very large resumed history must not make terminal takeover
            // wait for thousands of Markdown layouts. Show a useful recent
            // window first, then restore the complete scrollback when the
            // user first requests older history.
            self.rebuild_viewport_recent(STARTUP_TRANSCRIPT_RENDER_LIMIT);
            self.viewport.update(ViewportMsg::Bottom);
            self.startup_transcript_bounded = true;
        } else {
            // Resumed session — show the prior conversation, scrolled to the end.
            self.rebuild_viewport();
            self.viewport.update(ViewportMsg::Bottom);
        }

        // Program dispatches Model::init before it renders. The only initial
        // future therefore waits on the one-way gate that `cursor` opens after
        // Renderer::render has flushed the first frame. Optional I/O cannot
        // race the renderer merely because the Tokio scheduler is eager.
        let first_frame = self.first_frame.clone();
        Some(cmd::cmd(move || async move {
            first_frame.wait().await;
            Msg::FirstFrameReady
        }))
    }

    fn update(&mut self, msg: Msg) -> Option<Cmd<Msg>> {
        self.update_message(msg)
    }

    fn view(&self) -> String {
        if let Some(prompt) = self.render_goal_resume_prompt() {
            return self.present_full_screen_page(prompt);
        }
        if let Some(transcript) = &self.transcript_view {
            return self.present_full_screen_page(transcript.render());
        }
        if self.help_open {
            return self.present_full_screen_page(self.render_help());
        }
        if let Some(m) = &self.memory {
            return self.present_full_screen_page(self.render_memory(m));
        }
        if let Some(panel) = &self.evolution {
            return self.present_full_screen_page(self.render_evolution(panel));
        }
        if let Some(panel) = &self.asset_list {
            return self.present_full_screen_page(self.render_asset_list(panel));
        }
        if let Some(panel) = &self.runtime_activity {
            return self.present_full_screen_page(self.render_runtime_activity(panel));
        }
        if let Some(kb) = &self.kb {
            let page = self.render_kb(kb);
            return self.present_full_screen_page(page);
        }
        if let Some(panel) = &self.loop_panel {
            return self.present_full_screen_page(self.render_loop_panel(panel));
        }
        if let Some(ide) = &self.ide {
            // A pending tool approval overlays the full-screen page so it is
            // never invisible (its keys take priority in the key dispatch).
            let page = self.render_ide(ide);
            return self.present_full_screen_page(page);
        }
        let width = self.width as usize;
        let composer_width = self.viewport_content_width();
        let raw_view = self.viewport.view();
        // Paint an active text-selection over the visible rows, then add the bar.
        let shown = match &self.selection {
            Some(s) if !s.is_empty() => {
                let (r1, c1, r2, c2) = s.ordered();
                highlight_selection(&raw_view, r1, c1, r2, c2)
            }
            _ => raw_view,
        };
        let viewport_view = append_scrollbar(
            &shown,
            width,
            self.viewport.total_lines(),
            self.viewport.scroll_percent(),
        );
        // Input mode hint: `!` = shell command (red), `?` = deep research
        // (cyan), `/agent` dev = local agent development (green), `/mcp` dev =
        // local MCP development (cyan), otherwise the normal prompt (accent
        // blue).
        let (sym, icolor, border): (&str, Color, Color) = if self.shell_mode {
            ("!", TN_RED, TN_RED)
        } else if self.research_mode {
            ("?", TN_CYAN, TN_CYAN)
        } else if self.agent_dev.is_some() {
            ("◇", TN_GREEN, TN_GREEN)
        } else if self.mcp_dev.is_some() {
            ("◆", TN_CYAN, TN_CYAN)
        } else if self.skill_dev.is_some() {
            ("✦", TN_CYAN, TN_CYAN)
        } else if self.okf_dev.is_some() {
            ("⌁", TN_CYAN, TN_CYAN)
        } else {
            ("❯", ACCENT, TN_GRAY)
        };
        // Ultracode owns a short A3S-brand transition. The normal composer
        // keeps its original outlined shape and the rest of the UI continues
        // to use the neutral semantic palette.
        let gradient = self
            .gradient_until
            .is_some_and(|t| t.elapsed() < ULTRACODE_BORDER_ANIMATION);
        let elabel = if self.research_mode {
            deep_research_input_scope_hint().to_string()
        } else {
            let profile = &EFFORT_LEVELS[self.effort];
            match self.codex_effort_status_for_index(self.effort) {
                Some(status) if status.capped || self.effort == ULTRACODE => {
                    let cap = if status.capped { " (cap)" } else { "" };
                    format!("◇ {} · Codex:{}{cap}", profile.label, status.effective)
                }
                _ => format!("◇ {}", profile.label),
            }
        };
        let (top_separator, separator) = if gradient {
            let lower_phase = self.gradient_frame + BRAND_GRADIENT.len() / 2;
            (
                input_gradient_rule(composer_width, &BRAND_GRADIENT, self.gradient_frame),
                input_gradient_rule(composer_width, &BRAND_GRADIENT, lower_phase),
            )
        } else {
            (
                input_status_rule(composer_width, border, &elabel),
                input_rule(composer_width, border),
            )
        };

        // Activity line directly above the input: spinner while the agent works,
        // an inline approval prompt while awaiting, empty when idle.
        let activity = if self.updating.is_some() {
            // The upgrade itself runs in the shell after exit (real brew
            // progress); in-TUI this is just the quick version check.
            Style::new().fg(TN_GREEN).render("⬇ checking for updates…")
        } else if let Some(t0) = self.compacting {
            compact_progress_line(t0.elapsed(), width)
        } else if let Some(startup) = self.startup_loading_line() {
            startup
        } else {
            match self.state {
                State::Streaming => {
                    // Pulsing sparkle + "Thinking…" with live elapsed + token count.
                    let g = ['✶', '✸', '✹', '✺', '✹', '✷'][(self.blink_tick as usize / 2) % 6];
                    let spark = Style::new().fg(ACCENT).render(&g.to_string());
                    let activity_label = a3s_tui::style::truncate_visible(
                        &self.core_run_status.activity_label(),
                        width.saturating_sub(4).max(1),
                    );
                    let working = shimmer(&activity_label, self.blink_tick as usize);
                    let mut tail = String::new();
                    if let Some(t0) = self.stream_started {
                        // Live output estimate: finalized output tokens + a
                        // CJK-aware estimate of the in-flight reasoning + answer
                        // (snaps to exact completion usage on End).
                        let est = self.output_tokens
                            + estimate_tokens(self.streaming.raw_content())
                            + estimate_tokens(&self.thinking);
                        tail.push_str(&format!(" ({}", fmt_elapsed(t0.elapsed())));
                        if est > 0 {
                            tail.push_str(&format!(" · ↓ {} tokens", humanize(est)));
                        }
                        tail.push(')');
                    }
                    let tail = Style::new().fg(TN_GRAY).render(&tail);
                    format!("{spark} {working}{tail}")
                }
                // The approval options panel (overlay_approval) is the UI now.
                State::Awaiting => String::new(),
                State::Rebuilding => {
                    let g = ['✶', '✸', '✹', '✺', '✹', '✷'][(self.blink_tick as usize / 2) % 6];
                    let spark = Style::new().fg(ACCENT).render(&g.to_string());
                    format!(
                        "{spark} {}",
                        shimmer("Updating session…", self.blink_tick as usize)
                    )
                }
                State::Idle => String::new(),
            }
        };

        let typed = self.textarea.view();
        let tint_input = sym != "❯";
        let input_view = input_prompt_line(sym, icolor, &typed, tint_input, composer_width);
        let attachments = attachment_strip(&self.pending_images, composer_width);
        let attachment_view = attachments.rows.join("\n");

        // Codex-style single footer. Claude-style task/subagent blocks remain
        // separate below it, but persistent session state has only one owner.
        let status = self.session_status_line(width);

        // Gap line between transcript and loading — or a floating "jump to
        // latest" hint when the user has scrolled up away from the bottom.
        let spacer = if self.viewport.at_bottom() {
            String::new()
        } else {
            jump_to_latest_hint(width)
        };
        let bottom = self.bottom_pane_projection();
        let task_block = bottom.tasks.join("\n");
        // Plan/TODO panel stays pinned above the input.
        let plan_block = bottom.plan.join("\n");
        // Parallel-subagent tracker is pinned below the single footer.
        let sub_block = bottom.subagents.join("\n");
        let composed = Layout::vertical()
            .item(&viewport_view, Constraint::Fill)
            .item(&spacer, Constraint::Fixed(1))
            .item(&activity, Constraint::Fixed(1))
            .item(&plan_block, Constraint::Fixed(bottom.plan.len() as u16))
            .item(&top_separator, Constraint::Fixed(1))
            .item(
                &attachment_view,
                Constraint::Fixed(attachments.rows.len().min(u16::MAX as usize) as u16),
            )
            .item(&input_view, Constraint::Fixed(self.input_height()))
            .item(&separator, Constraint::Fixed(1))
            .item(&status, Constraint::Fixed(1))
            .item(&sub_block, Constraint::Fixed(bottom.subagents.len() as u16))
            .item(&task_block, Constraint::Fixed(bottom.tasks.len() as u16))
            .render(self.height);

        let composed = self.overlay_slash_menu(composed);
        let composed = self.overlay_file_menu(composed);
        let composed = self.overlay_model_menu(composed);
        let composed = self.overlay_relay_menu(composed);
        let composed = self.overlay_task_menu(composed);
        let composed = self.overlay_permission_menu(composed);
        let composed = self.overlay_history_menu(composed);
        let composed = self.overlay_review_menu(composed);
        let composed = self.overlay_flow_menu(composed);
        let composed = self.overlay_agent_menu(composed);
        let composed = self.overlay_mcp_menu(composed);
        let composed = self.overlay_skill_menu(composed);
        let composed = self.overlay_okf_package_menu(composed);
        let composed = self.overlay_effort(composed);
        let composed = self.overlay_theme(composed);
        let composed = self.overlay_plugins(composed);
        let composed = self.overlay_packages(composed);
        self.overlay_decision_modals(composed)
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        // a3s-tui invokes cursor only after Renderer::render returns, and that
        // renderer flushes the terminal before returning. This is the exact
        // first-frame acknowledgement used by every deferred startup task.
        self.first_frame
            .acknowledge_flushed_then("workspace_manifest_activation", || {
                self.workspace_manifest.activate();
            });

        // Modal ownership wins before any underlying page computes a cursor.
        // In particular, an approval or semantic transcript may be rendered
        // over an existing IDE buffer and must not leak its editor cursor.
        if self.composer_input_is_hidden() {
            return None;
        }

        // In the /ide editor, place the cursor at the edit position — inside
        // the right panel: tree width + its left border + the `%4d ` gutter.
        if let Some(ide) = &self.ide {
            if ide.focus_editor && ide.intelligence.is_none() {
                if let Some(f) = &ide.file {
                    let width = self.width as usize;
                    let (tw, _) = panels::spf::ide_split(width);
                    let gutter = if panels::spf::ide_gutter_on(width) {
                        5
                    } else {
                        0
                    };
                    let x = tw + 1 + gutter + f.display_col().saturating_sub(f.hscroll);
                    let col = x.min(width.saturating_sub(2)) as u16;
                    let row = (1 + f.row.saturating_sub(f.scroll)) as u16;
                    return Some((col, row));
                }
            }
            return None;
        }
        // Real cursor at the input insertion point whenever the input is live —
        // idle or streaming (you can keep typing while the agent works).
        // Below the input: footer separator + the single session footer, then
        // the subagent and queue panels. Use the same immutable projection as
        // rendering so a terminal event cannot leave a one-frame cursor jump.
        let bottom = self.bottom_pane_projection();
        let row = bottom.input_cursor_row(
            self.height,
            self.input_height(),
            self.textarea.cursor_row() as u16,
        );
        let col = (PAD + 2) as u16 + self.textarea.cursor_display_col() as u16; // PAD + "› "
        Some((col, row))
    }
}

impl App {
    fn startup_loading_line(&self) -> Option<String> {
        if self.state != State::Idle || !self.startup_loading.is_loading() {
            return None;
        }
        let remaining = self.startup_loading.remaining();
        let glyph = ['◌', '◔', '◑', '◕'][self.anim as usize % 4];
        let label = if remaining == 0 {
            format!("{glyph} Loading workspace…")
        } else {
            format!("{glyph} Loading background services · {remaining} remaining · input ready")
        };
        Some(
            Style::new()
                .fg(TN_CYAN)
                .render(&a3s_tui::style::truncate_visible(
                    &label,
                    self.width as usize,
                )),
        )
    }

    /// Full-screen startup views do not render the ordinary composer activity
    /// row. Overlay the same non-blocking status at the top so first-launch IDE
    /// and paused-goal screens still provide immediate, visible feedback.
    fn present_full_screen_page(&self, page: String) -> String {
        let page = match self.startup_loading_line() {
            Some(line) => self.overlay_list_with_rows_below(
                page,
                &[line],
                usize::from(self.height.saturating_sub(1)),
            ),
            None => page,
        };
        self.overlay_decision_modals(page)
    }

    fn deferred_startup_command(&self, operation: &'static str, command: Cmd<Msg>) -> Cmd<Msg> {
        let first_frame = self.first_frame.clone();
        Box::pin(async move {
            // The handler is reached through FirstFrameReady, but retain the
            // gate here as a local invariant if dispatch is refactored later.
            first_frame.wait().await;
            first_frame.record_deferred_operation(operation);
            command.await
        })
    }

    pub(super) fn start_deferred_startup(&mut self) -> Cmd<Msg> {
        let mut commands = Vec::new();
        let mut startup_pending = STARTUP_EVOLUTION;

        // Every command in this batch is optional for the first paint. Some
        // futures are cheap timers, while others touch the network, spawn a
        // managed component, or scan local state; all share the same boundary.
        commands.push(self.deferred_startup_command(
            "update_check",
            cmd::cmd(|| async { Msg::UpdateCheck(check_latest_version().await) }),
        ));
        let subagent_snapshot = self.request_subagent_snapshots();
        commands.push(self.deferred_startup_command("subagent_snapshot", subagent_snapshot));
        commands.push(self.deferred_startup_command(
            "workspace_manifest_events",
            pump_manifest(self.workspace_manifest_rx.clone()),
        ));
        let agent_presence = self.refresh_agent_presence();
        commands.push(self.deferred_startup_command("agent_presence", agent_presence));
        commands.push(self.deferred_startup_command("agent_presence_tick", agent_presence_tick()));
        commands.push(
            self.deferred_startup_command(
                "schedule_notification_tick",
                schedule_notification_tick(),
            ),
        );
        commands.push(self.deferred_startup_command(
            "schedule_notifications",
            poll_schedule_notifications(PathBuf::from(&self.cwd)),
        ));
        commands.push(self.deferred_startup_command(
            "schedule_worker",
            ensure_schedule_worker_cmd(PathBuf::from(&self.cwd), self.config_path.clone()),
        ));

        let evolution_workspace = self.cwd.clone();
        let evolution_memory = self.memory_dir.clone();
        let evolution_skill_workspace = self.cwd.clone();
        let evolution_skill_directory = self.asset_directories.skill.clone();
        commands.push(self.deferred_startup_command(
            "evolution_synchronization",
            cmd::cmd(move || async move {
                let evolution = crate::evolution::WorkspaceEvolution::new(evolution_workspace);
                if let Err(error) = evolution.mark_session_assets_activated().await {
                    tracing::warn!(
                        %error,
                        "could not mark learned session assets active after TUI first frame"
                    );
                }
                let synchronization = async {
                    evolution.synchronize_memory_store(evolution_memory).await?;
                    evolution.pending_session_reload_count().await
                }
                .await;
                let (pending_assets, synchronization_error) = match synchronization {
                    Ok(pending_assets) => (pending_assets, None),
                    Err(error) => (0, Some(error.to_string())),
                };
                let result = async {
                    let (skills, skill_count) = tokio::task::spawn_blocking(move || {
                        let dirs = agent_skill_dirs_with_configured(
                            &evolution_skill_workspace,
                            &evolution_skill_directory,
                        );
                        let skills = load_skills(&dirs);
                        let skill_count = count_skill_files(&dirs);
                        (skills, skill_count)
                    })
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("startup skill catalog loader failed: {error}")
                    })?;
                    Ok(StartupEvolutionMetadata {
                        pending_assets,
                        skill_count,
                        skills,
                        synchronization_error,
                    })
                }
                .await;
                Msg::EvolutionStartupSynchronized(
                    result.map_err(|error: anyhow::Error| error.to_string()),
                )
            }),
        ));

        if let Some(command) = self.deferred_webview_setup.take() {
            startup_pending |= STARTUP_WEBVIEW;
            commands.push(self.deferred_startup_command("webview_setup", command));
        }
        if let Some(command) = self.configured_mcp.activation_command() {
            startup_pending |= STARTUP_CONFIGURED_MCP;
            commands.push(self.deferred_startup_command("configured_mcp", command));
        }
        if let Some(command) = self.deferred_sandbox_setup.take() {
            startup_pending |= STARTUP_SANDBOX;
            commands.push(self.deferred_startup_command("sandbox_setup", command));
        }
        if let Some(command) = self.deferred_ui_metadata.take() {
            startup_pending |= STARTUP_UI_METADATA;
            commands.push(self.deferred_startup_command("ui_metadata", command));
        }
        if let Some(command) = self.deferred_research_recovery.take() {
            startup_pending |= STARTUP_RESEARCH_RECOVERY;
            commands.push(self.deferred_startup_command("research_recovery", command));
        }
        if let Some(retrieval) = self.workspace_retrieval_options.clone() {
            startup_pending |= STARTUP_RETRIEVAL;
            commands.push(self.deferred_startup_command(
                "workspace_retrieval",
                cmd::cmd(move || async move {
                    retrieval.activate_background_indexing();
                    Msg::WorkspaceRetrievalStartupActivated
                }),
            ));
        }

        // Heartbeat for every session. BannerTick self-gates the mascot
        // animation and drives idle maintenance; Ultracode has its own tick.
        commands.push(self.deferred_startup_command("banner_tick", banner_tick()));
        let codex_refresh = self.maybe_refresh_codex_models();
        if let Some(refresh) = codex_refresh {
            commands.push(self.deferred_startup_command("codex_model_refresh", refresh));
        }

        self.startup_loading.begin(startup_pending);
        cmd::batch(commands)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_session_status_line(
    cwd: &str,
    branch: Option<&str>,
    model: Option<&str>,
    context_limit: u32,
    last_prompt_tokens: usize,
    output_tokens: usize,
    chips: impl IntoIterator<Item = SessionStatusChip>,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }

    let mut chips = chips.into_iter();
    let mode = chips.next();
    let context = footer_context_segments(context_limit, last_prompt_tokens, output_tokens);
    let mode_full = mode.as_ref().map(footer_mode_segment).unwrap_or_default();
    let mode_compact = mode
        .as_ref()
        .map(footer_compact_mode_segment)
        .unwrap_or_default();
    let mode_tiny = mode
        .as_ref()
        .map(footer_tiny_mode_segment)
        .unwrap_or_default();

    // Select the richest mandatory projection that fits before considering any
    // workspace detail. This keeps permission mode and context visible instead
    // of allowing a long branch/model/goal to be blindly truncated over them.
    let core_candidates = [
        (footer_row(PAD, "  ", [&mode_full, &context.full]), "  "),
        (footer_row(PAD, "  ", [&mode_full, &context.compact]), "  "),
        (
            footer_row(PAD, "  ", [&mode_compact, &context.compact]),
            "  ",
        ),
        (footer_row(PAD, " ", [&mode_tiny, &context.tiny]), " "),
    ];
    // Active state is more important than static identity. Choose a core
    // projection that leaves room for the first live chip (normally `/goal`),
    // then add workspace identity only with the remaining space.
    let live = chips
        .map(|chip| footer_chip_segment(&chip))
        .collect::<Vec<_>>();
    let preferred_core = live.first().and_then(|detail| {
        core_candidates.iter().find(|(candidate, separator)| {
            let joined = if candidate.is_empty() {
                detail.clone()
            } else {
                format!("{candidate}{separator}{detail}")
            };
            a3s_tui::style::visible_len(&joined) <= width
        })
    });
    let (mut row, separator) = preferred_core
        .or_else(|| {
            core_candidates
                .iter()
                .find(|(candidate, _)| a3s_tui::style::visible_len(candidate) <= width)
        })
        .cloned()
        .unwrap_or_else(|| (footer_row(0, " ", [&mode_tiny, &context.tiny]), " "));

    for detail in live {
        let candidate = if row.is_empty() {
            detail
        } else {
            format!("{row}{separator}{detail}")
        };
        if a3s_tui::style::visible_len(&candidate) > width {
            break;
        }
        row = candidate;
    }

    let mut identity = Vec::new();
    let workspace = footer_workspace_segment(cwd);
    if !workspace.is_empty() {
        identity.push(workspace);
    }
    if let Some(branch) = branch.filter(|branch| !branch.is_empty()) {
        identity.push(footer_branch_segment(branch));
    }
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        identity.push(footer_model_segment(model, context_limit));
    }

    for detail in identity {
        let candidate = if row.is_empty() {
            detail
        } else {
            format!("{row}{separator}{detail}")
        };
        if a3s_tui::style::visible_len(&candidate) > width {
            break;
        }
        row = candidate;
    }

    a3s_tui::style::fit_visible(&row, width)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SessionStatusReport {
    pub(super) session_id: String,
    pub(super) workspace: String,
    pub(super) branch: Option<String>,
    pub(super) model: String,
    pub(super) effort: String,
    pub(super) active_mode: Mode,
    pub(super) next_mode: Mode,
    pub(super) context_limit: usize,
    pub(super) prompt_tokens: usize,
    pub(super) output_tokens: usize,
    pub(super) activity: String,
    pub(super) queued_turns: usize,
    pub(super) os_account: String,
    pub(super) workspace_retrieval: crate::workspace_retrieval::WorkspaceRetrievalStatusReport,
    pub(super) active_scope: String,
}

pub(super) fn render_session_status_report(report: &SessionStatusReport, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let workspace = match report.branch.as_deref().filter(|branch| !branch.is_empty()) {
        Some(branch) => format!("{} · branch {branch}", report.workspace),
        None => report.workspace.clone(),
    };
    let permission_mode = if report.active_mode == report.next_mode {
        format!(
            "{} · {}",
            report.next_mode.name(),
            permission_mode_summary(report.next_mode)
        )
    } else {
        format!(
            "{} active · {} next · {}",
            report.active_mode.name(),
            report.next_mode.name(),
            permission_mode_summary(report.next_mode)
        )
    };
    let context = if report.context_limit == 0 {
        format!(
            "limit unknown · prompt {} · output {} tokens",
            report.prompt_tokens, report.output_tokens
        )
    } else {
        format!(
            "{} / {} ({}%) · output {} tokens",
            report.prompt_tokens,
            report.context_limit,
            footer_context_percent(report.prompt_tokens, report.context_limit),
            report.output_tokens
        )
    };
    let queue_suffix = match report.queued_turns {
        0 => "no queued turns".to_string(),
        1 => "1 queued turn".to_string(),
        count => format!("{count} queued turns"),
    };

    let mut rows = vec![
        Style::new().fg(ACCENT).bold().render("  Session status"),
        status_report_row("session", &report.session_id, width),
        status_report_row("workspace", &workspace, width),
        status_report_row(
            "model",
            &format!("{} · effort {}", report.model, report.effort),
            width,
        ),
        status_report_row("permissions", &permission_mode, width),
        status_report_row("boundary", "workspace guardrails enforced", width),
        status_report_row("context", &context, width),
        status_report_row(
            "activity",
            &format!("{} · {queue_suffix}", report.activity),
            width,
        ),
        status_report_row("OS account", &report.os_account, width),
    ];
    rows.push(status_report_row(
        "retrieval",
        &report.workspace_retrieval.retrieval,
        width,
    ));
    if let Some(vectors) = report.workspace_retrieval.vectors.as_deref() {
        rows.push(status_report_row("vectors", vectors, width));
    }
    if let Some(embedding) = report.workspace_retrieval.embedding.as_deref() {
        rows.push(status_report_row("embedding", embedding, width));
    }
    rows.push(status_report_row("active", &report.active_scope, width));
    rows.push(status_report_row(
        "resume",
        &format!("a3s code resume {}", report.session_id),
        width,
    ));
    rows.into_iter()
        .map(|row| a3s_tui::style::fit_visible(&row, width))
        .collect::<Vec<_>>()
        .join("\n")
}

fn permission_mode_summary(mode: Mode) -> &'static str {
    match mode {
        Mode::Default => "risk-aware; side effects ask",
        Mode::Plan => "read-only planning",
        Mode::Auto => "non-interactive; hard guardrails remain",
    }
}

fn status_report_row(label: &str, value: &str, width: usize) -> String {
    let label = Style::new().fg(TN_GRAY).render(&format!("  {label:<11}"));
    let value = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    a3s_tui::style::fit_visible(
        &format!("{label}{}", Style::new().fg(TN_FG).render(&value)),
        width,
    )
}

struct FooterContextSegments {
    full: String,
    compact: String,
    tiny: String,
}

fn footer_context_segments(
    context_limit: u32,
    last_prompt_tokens: usize,
    output_tokens: usize,
) -> FooterContextSegments {
    if context_limit == 0 {
        let label = if output_tokens > 0 {
            format!("out:{output_tokens} tok")
        } else {
            "ctx:?".to_string()
        };
        let styled = Style::new().fg(COMPOSER_CHROME.secondary).render(&label);
        return FooterContextSegments {
            full: styled.clone(),
            compact: styled,
            tiny: Style::new().fg(COMPOSER_CHROME.secondary).render("ctx?"),
        };
    }

    let limit = context_limit as usize;
    let percent = footer_context_percent(last_prompt_tokens, limit);
    let color = footer_context_color(percent);
    let compact = Style::new().fg(color).render(&format!("ctx:{percent}%"));
    let meter = Meter::new(percent as f64)
        .width(6)
        .glyphs('▰', '▱')
        .show_value(false)
        .fg(color)
        .empty_fg(COMPOSER_CHROME.faint)
        .view();

    FooterContextSegments {
        full: format!("{compact} {meter}"),
        compact,
        tiny: Style::new().fg(color).render(&format!("{percent}%")),
    }
}

fn footer_context_percent(used: usize, limit: usize) -> usize {
    if limit == 0 || used == 0 {
        0
    } else if used >= limit {
        100
    } else {
        ((used as u128 * 100) / limit as u128) as usize
    }
}

fn footer_context_color(percent: usize) -> Color {
    if percent >= 85 {
        COMPOSER_CHROME.error
    } else if percent >= 70 {
        COMPOSER_CHROME.warning
    } else {
        COMPOSER_CHROME.active
    }
}

fn footer_row<'a>(
    margin: usize,
    separator: &str,
    segments: impl IntoIterator<Item = &'a String>,
) -> String {
    let body = segments
        .into_iter()
        .filter(|segment| !segment.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(separator);
    if body.is_empty() {
        String::new()
    } else {
        format!("{}{body}", " ".repeat(margin))
    }
}

fn footer_workspace_segment(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    let workspace = trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed);
    Style::new()
        .fg(COMPOSER_CHROME.active)
        .bold()
        .render(workspace)
}

fn footer_branch_segment(branch: &str) -> String {
    format!(
        "{}{}{}",
        Style::new().fg(COMPOSER_CHROME.faint).render("git:("),
        Style::new().fg(COMPOSER_CHROME.success).render(branch),
        Style::new().fg(COMPOSER_CHROME.faint).render(")")
    )
}

fn footer_model_segment(model: &str, context_limit: u32) -> String {
    let short = model
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(model);
    let mut segment = Style::new().fg(COMPOSER_CHROME.secondary).render(short);
    if context_limit > 0 {
        segment.push(' ');
        segment.push_str(&Style::new().fg(COMPOSER_CHROME.secondary).render(&format!(
            "({} context)",
            footer_context_window_label(context_limit as usize)
        )));
    }
    segment
}

fn footer_context_window_label(limit: usize) -> String {
    if limit >= 1_000_000 {
        format!("{}M", limit / 1_000_000)
    } else if limit >= 1_000 {
        format!("{}k", limit / 1_000)
    } else {
        limit.to_string()
    }
}

fn footer_chip_segment(chip: &SessionStatusChip) -> String {
    let glyph_color = chip.color_value().unwrap_or(COMPOSER_CHROME.faint);
    format!(
        "{} {}",
        Style::new().fg(glyph_color).render(chip.glyph()),
        Style::new()
            .fg(COMPOSER_CHROME.secondary)
            .render(chip.label())
    )
}

pub(super) fn footer_mode_segment(chip: &SessionStatusChip) -> String {
    let glyph_color = chip.color_value().unwrap_or(COMPOSER_CHROME.faint);
    format!(
        "{} {}",
        Style::new().fg(glyph_color).render(chip.glyph()),
        Style::new()
            .fg(COMPOSER_CHROME.primary)
            .render(chip.label())
    )
}

fn footer_compact_mode_segment(chip: &SessionStatusChip) -> String {
    let glyph_color = chip.color_value().unwrap_or(COMPOSER_CHROME.faint);
    let label = chip.label().strip_suffix(" mode").unwrap_or(chip.label());
    format!(
        "{} {}",
        Style::new().fg(glyph_color).render(chip.glyph()),
        Style::new().fg(COMPOSER_CHROME.primary).render(label)
    )
}

fn footer_tiny_mode_segment(chip: &SessionStatusChip) -> String {
    Style::new()
        .fg(chip.color_value().unwrap_or(COMPOSER_CHROME.faint))
        .render(chip.glyph())
}

pub(super) fn jump_to_latest_hint(width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let label = InlineAction::new("more below · Shift+End to jump to latest")
        .icon("↓")
        .colors(TN_FG, ACCENT)
        .view();
    let label_width = a3s_tui::style::visible_len(&label);
    if label_width >= width {
        return a3s_tui::style::fit_visible(&label, width);
    }

    let pad = width.saturating_sub(label_width) / 2;
    a3s_tui::style::fit_visible(&format!("{}{}", " ".repeat(pad), label), width)
}

pub(super) fn mode_status_chip(mode: Mode) -> SessionStatusChip {
    SessionStatusChip::new(mode.glyph(), format!("{} mode", mode.name())).color(mode.color())
}
