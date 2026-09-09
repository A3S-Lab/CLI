//! Terminal event/update loop and footer presentation for the Code TUI.

use super::*;

const STARTUP_TRANSCRIPT_RENDER_LIMIT: usize = 128;

impl Model for App {
    type Msg = Msg;

    fn init(&mut self) -> Option<Cmd<Msg>> {
        let showed_banner = self.messages.is_empty();
        if showed_banner {
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
        let cmd = self.update_message(msg);
        self.maybe_open_deferred_review_checklist();
        cmd
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
        // Session chrome owns transcript → spacer → status → attachments
        // → composer, then transient overlays. See `session_chrome.rs`.
        self.render_session_chrome_main()
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
            composer_chrome_height(self.input_height()),
            // +1 skips the top half-block cap (prompt bar geometry).
            1u16.saturating_add(self.textarea.cursor_row() as u16),
        );
        let col = (PAD + COMPOSER_INSET + 2) as u16 + self.textarea.cursor_display_col() as u16; // inset + "→ "
        Some((col, row))
    }
}

impl App {
    fn startup_loading_line(&self) -> Option<String> {
        // Deferred first-frame services stay tracked for completion, but the
        // editor is already ready — do not paint a background-loading chrome
        // that makes startup feel unfinished.
        None
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
        let evolution_memory = Arc::clone(&self.memory_store);
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
        if let Some(command) = self.deferred_plugin_manager.take() {
            startup_pending |= STARTUP_PLUGIN_MANAGER;
            commands.push(self.deferred_startup_command("plugin_manager", command));
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

    // Prompt footer: quiet dim rows under the composer, with identity on
    // the left and secondary metadata on the right.
    //   mode · model · ctx% · [live…]          ⬆ version
    //   ~/path                                 branch
    const FOOTER_MARGIN: usize = 2;
    let chips = chips.into_iter().collect::<Vec<_>>();
    let mode = chips.iter().find(|chip| is_footer_mode_chip(chip)).cloned();
    let (right_chips, live): (Vec<_>, Vec<_>) = chips
        .into_iter()
        .filter(|chip| !is_footer_mode_chip(chip))
        .partition(|chip| chip.glyph() == "⬆");

    let mode_text = mode.as_ref().map(footer_mode_segment).unwrap_or_default();
    let model_text = model
        .filter(|model| !model.is_empty())
        .map(|model| {
            let short = model
                .rsplit('/')
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or(model);
            Style::new().fg(COMPOSER_CHROME.faint).render(short)
        })
        .unwrap_or_default();
    let context = footer_context_segments(context_limit, last_prompt_tokens, output_tokens);

    let live_segments = live.iter().map(footer_chip_segment).collect::<Vec<_>>();
    let right_text = right_chips
        .iter()
        .map(footer_chip_segment)
        .collect::<Vec<_>>()
        .join(" · ");

    let left_candidates = [
        footer_join(
            0,
            " · ",
            [
                mode_text.as_str(),
                model_text.as_str(),
                context.full.as_str(),
            ]
            .into_iter()
            .chain(live_segments.iter().map(String::as_str)),
        ),
        footer_join(
            0,
            " · ",
            [
                mode_text.as_str(),
                model_text.as_str(),
                context.compact.as_str(),
            ]
            .into_iter()
            .chain(live_segments.iter().map(String::as_str)),
        ),
        footer_join(
            0,
            " · ",
            [
                mode_text.as_str(),
                model_text.as_str(),
                context.compact.as_str(),
            ],
        ),
        footer_join(0, " · ", [mode_text.as_str(), context.compact.as_str()]),
        footer_join(0, " · ", [mode_text.as_str(), context.tiny.as_str()]),
    ];

    let status = left_candidates
        .into_iter()
        .find_map(|left| {
            let row = footer_spread(FOOTER_MARGIN, &left, &right_text, width);
            (a3s_tui::style::visible_len(&row) <= width).then_some(row)
        })
        .unwrap_or_else(|| {
            footer_spread(
                FOOTER_MARGIN,
                &footer_join(0, " · ", [mode_text.as_str(), context.tiny.as_str()]),
                "",
                width,
            )
        });

    let location = footer_location_line(cwd, branch, width);
    let status = a3s_tui::style::fit_visible(&status, width);
    if location.is_empty() {
        status
    } else {
        format!("{status}\n{location}")
    }
}

fn footer_join<'a>(
    margin: usize,
    separator: &str,
    segments: impl IntoIterator<Item = &'a str>,
) -> String {
    let body = segments
        .into_iter()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(separator);
    if body.is_empty() {
        String::new()
    } else {
        format!("{}{body}", " ".repeat(margin))
    }
}

/// Left/right footer row with a flexible gap (`space-between` feel).
fn footer_spread(margin: usize, left: &str, right: &str, width: usize) -> String {
    let margin = margin.min(width);
    let margin_pad = " ".repeat(margin);
    let inner = width.saturating_sub(margin);
    if right.is_empty() {
        return format!("{margin_pad}{left}");
    }
    if left.is_empty() {
        return format!(
            "{margin_pad}{}",
            a3s_tui::style::right_visible(right, inner)
        );
    }
    let right_budget = inner / 2;
    let right_t = a3s_tui::style::truncate_visible(right, right_budget.max(1));
    let right_len = a3s_tui::style::visible_len(&right_t);
    let left_budget = inner.saturating_sub(right_len.saturating_add(1));
    let left_t = a3s_tui::style::truncate_visible(left, left_budget.max(1));
    let left_len = a3s_tui::style::visible_len(&left_t);
    if left_len + 1 + right_len > inner {
        return format!("{margin_pad}{left_t}");
    }
    let gap = inner.saturating_sub(left_len).saturating_sub(right_len);
    format!("{margin_pad}{left_t}{}{right_t}", " ".repeat(gap))
}

fn footer_location_line(cwd: &str, branch: Option<&str>, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    const FOOTER_MARGIN: usize = 2;
    let path = footer_home_path(cwd);
    let branch = branch
        .filter(|branch| !branch.is_empty())
        .map(|branch| Style::new().fg(COMPOSER_CHROME.faint).render(branch))
        .unwrap_or_default();
    let path = Style::new().fg(COMPOSER_CHROME.faint).render(&path);
    let row = footer_spread(FOOTER_MARGIN, &path, &branch, width);
    if row.trim().is_empty() {
        return String::new();
    }
    a3s_tui::style::fit_visible(&row, width)
}

fn footer_home_path(cwd: &str) -> String {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from);
    if let Some(home) = home.as_ref() {
        let cwd_path = std::path::Path::new(cwd);
        if let Ok(relative) = cwd_path.strip_prefix(home) {
            let rel = relative.to_string_lossy();
            return if rel.is_empty() {
                "~".to_string()
            } else {
                format!("~/{rel}")
            };
        }
    }
    cwd.to_string()
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SandboxStatusReport {
    pub(super) handle_attached: bool,
    pub(super) verified: bool,
    pub(super) mode: Mode,
}

pub(super) fn render_sandbox_status_report(report: &SandboxStatusReport, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let handle = if report.handle_attached {
        "attached"
    } else {
        "missing"
    };
    let verified = if report.verified {
        "ready · host Bash admitted under mode policy"
    } else {
        "unavailable · host Bash fail-closed until verified"
    };
    let mode = format!(
        "{} · {}",
        report.mode.name(),
        permission_mode_summary(report.mode)
    );
    let next = if report.verified {
        "/permissions cycles mode · /sandbox rechecks this boundary"
    } else {
        "fix sandbox install/probe · host shell stays denied until verified"
    };

    [
        Style::new().fg(ACCENT).bold().render("  Sandbox status"),
        status_report_row("handle", handle, width),
        status_report_row("verified", verified, width),
        status_report_row("mode", &mode, width),
        status_report_row("next", next, width),
    ]
    .into_iter()
    .map(|row| a3s_tui::style::fit_visible(&row, width))
    .collect::<Vec<_>>()
    .join("\n")
}

/// Append an optional external statusline decorator without replacing the
/// authoritative SessionMeter fields (status-line protocol intent).
pub(super) fn merge_status_line_extension(
    base: &str,
    extension: Option<&str>,
    width: usize,
) -> String {
    let Some(extension) = extension.map(str::trim).filter(|value| !value.is_empty()) else {
        return fit_status_block(base, width);
    };
    if width == 0 {
        return String::new();
    }
    let mut lines = base.lines();
    let first = lines.next().unwrap_or("");
    let rest = lines.collect::<Vec<_>>();
    let sep = " · ";
    let budget = width
        .saturating_sub(a3s_tui::style::visible_len(first))
        .saturating_sub(a3s_tui::style::visible_len(sep));
    let first = if budget == 0 {
        a3s_tui::style::fit_visible(first, width)
    } else {
        let extension = a3s_tui::style::truncate_visible(extension, budget);
        a3s_tui::style::fit_visible(&format!("{first}{sep}{extension}"), width)
    };
    if rest.is_empty() {
        first
    } else {
        let mut out = first;
        for line in rest {
            out.push('\n');
            out.push_str(&a3s_tui::style::fit_visible(line, width));
        }
        out
    }
}

fn fit_status_block(base: &str, width: usize) -> String {
    base.lines()
        .map(|line| a3s_tui::style::fit_visible(line, width))
        .collect::<Vec<_>>()
        .join("\n")
}

fn permission_mode_summary(mode: Mode) -> &'static str {
    match mode {
        Mode::Default => "risk-aware; side effects ask",
        Mode::Plan => "read-only planning",
        Mode::Reviewer => "sticky claim↔record reply verifier; main stream unchanged",
        Mode::Auto => "non-interactive; hard guardrails remain",
        Mode::Yolo => "force/yolo; high-risk auto-allowed, critical denials remain",
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
            format!("out:{output_tokens}")
        } else {
            "ctx:?".to_string()
        };
        let styled = Style::new().fg(COMPOSER_CHROME.faint).render(&label);
        return FooterContextSegments {
            full: styled.clone(),
            compact: styled,
            tiny: Style::new().fg(COMPOSER_CHROME.faint).render("?"),
        };
    }

    let limit = context_limit as usize;
    let percent = footer_context_percent(last_prompt_tokens, limit);
    let color = footer_context_color(percent);
    // Prompt footer keeps a quiet percentage — no meter wall.
    let compact = Style::new().fg(color).render(&format!("{percent}%"));
    FooterContextSegments {
        full: compact.clone(),
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
        COMPOSER_CHROME.faint
    }
}

fn footer_chip_segment(chip: &SessionStatusChip) -> String {
    let glyph_color = chip.color_value().unwrap_or(COMPOSER_CHROME.faint);
    format!(
        "{} {}",
        Style::new().fg(glyph_color).render(chip.glyph()),
        Style::new().fg(COMPOSER_CHROME.faint).render(chip.label())
    )
}

fn is_footer_mode_chip(chip: &SessionStatusChip) -> bool {
    matches!(
        chip.label(),
        "agent" | "plan" | "reviewer" | "auto" | "yolo"
    )
}

/// Mode chip: glyph + label share the mode color so Shift+Tab states stay distinct.
pub(super) fn footer_mode_segment(chip: &SessionStatusChip) -> String {
    let color = chip.color_value().unwrap_or(COMPOSER_CHROME.faint);
    format!(
        "{} {}",
        Style::new().fg(color).render(chip.glyph()),
        Style::new().fg(color).render(chip.label())
    )
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
    SessionStatusChip::new(mode.glyph(), mode.name()).color(mode.color())
}
