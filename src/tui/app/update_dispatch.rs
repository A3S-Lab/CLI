//! Exhaustive terminal and asynchronous message dispatch for the Code TUI.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerAttachmentKeyAction {
    StageClipboardImage,
    RemoveLastImage,
    SubmitImageOnly,
}

fn composer_attachment_key_action(
    key: &KeyEvent,
    draft: &str,
    image_count: usize,
) -> Option<ComposerAttachmentKeyAction> {
    if key.code == KeyCode::Char('v') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(ComposerAttachmentKeyAction::StageClipboardImage);
    }
    if key.code == KeyCode::Backspace
        && key.modifiers == KeyModifiers::NONE
        && draft.is_empty()
        && image_count > 0
    {
        return Some(ComposerAttachmentKeyAction::RemoveLastImage);
    }
    if key.code == KeyCode::Enter
        && key.modifiers == KeyModifiers::NONE
        && draft.trim().is_empty()
        && image_count > 0
    {
        return Some(ComposerAttachmentKeyAction::SubmitImageOnly);
    }
    None
}

impl App {
    pub(super) fn update_message(&mut self, msg: Msg) -> Option<Cmd<Msg>> {
        if self.quitting {
            return match msg {
                Msg::QuitReady => self.finish_graceful_quit(),
                Msg::StreamStarted {
                    token,
                    session,
                    join,
                    ..
                } => Some(discard_started_stream(session, join, token)),
                _ => None,
            };
        }

        match msg {
            Msg::Term(Event::Resize { width, height }) => {
                let viewport_anchor = self.capture_viewport_anchor();
                self.selection = None; // screen-coord selection is stale after resize
                self.width = width;
                self.height = height;
                if let Some(transcript) = self.transcript_view.as_mut() {
                    transcript.resize(width, height);
                }
                self.relayout();
                self.textarea.set_width(textarea_width_for(width));
                // An open /ide image buffer is rasterized at open-time width —
                // re-render it for the new panel size or its rows overflow the
                // frame (styled rows can't be re-truncated).
                if let Some(f) = self
                    .ide
                    .as_mut()
                    .and_then(|i| i.file.as_mut())
                    .filter(|f| f.image)
                {
                    let inner = panels::spf::ide_split(width as usize).1.saturating_sub(2);
                    let body = (height as usize).saturating_sub(5);
                    f.lines = render_image_file(&f.path, inner, body)
                        .unwrap_or_else(|| vec!["<cannot decode image>".into()]);
                    f.scroll = 0;
                }
                // Reflow the active message from its lossless raw source while
                // preserving the committed/table-holdback state.
                self.streaming.set_width(self.transcript_markdown_width());
                if !self.streaming.raw_content().is_empty() {
                    self.last_paint = Some(Instant::now());
                    self.update_viewport_with_stream_from(viewport_anchor);
                } else {
                    self.rebuild_viewport_from(viewport_anchor);
                }
            }

            // Bracketed paste: drop the whole pasted block into the input as
            // one edit (newlines become real line breaks) instead of N submitted
            // lines / a3s-lane queue spam — Claude-Code-style paste DX.
            Msg::Term(Event::Paste(text)) => {
                self.last_activity = Instant::now();
                if self.composer_input_is_hidden() {
                    return None;
                }
                if self.ide.is_some() {
                    self.ide_paste_text(&text);
                    return None;
                }
                self.textarea.insert_str(&text);
                self.relayout();
            }

            Msg::Term(Event::Key(key)) => {
                self.last_activity = Instant::now();
                // Any keypress dismisses the copy highlight.
                self.selection = None;
                // Ctrl+C is a global quit key. Keep it before panels, approval
                // prompts, and streaming handlers so terminal variants cannot
                // route it into hidden input instead of exiting.
                if is_quit_key(&key) {
                    let now = Instant::now();
                    if quit_is_confirmed(self.quit_armed, now) {
                        return self.begin_graceful_quit();
                    }
                    self.quit_armed = Some(now);
                    self.push_line(
                        &Style::new()
                            .fg(TN_YELLOW)
                            .render("  press Ctrl+C again to exit"),
                    );
                    return None;
                }
                // The startup paused-goal picker is a true modal. It owns all
                // non-quit keys so execution-mode shortcuts, panels, and the
                // composer cannot change behind it.
                if self.goal_resume_prompt.is_some() {
                    return self.handle_goal_resume_key(&key);
                }
                // Esc is the goal kill switch even while a tool confirmation
                // overlay owns the keyboard. Rejecting one tool is not enough:
                // it would let the next unbounded goal iteration restart.
                if self.goal_run.is_some()
                    && self.state == State::Awaiting
                    && key.code == KeyCode::Esc
                {
                    self.cancel_goal_state("interrupted by Esc");
                    self.interrupting = true;
                    let status_entry = self
                        .push_tracked_line(&Style::new().fg(TN_YELLOW).render("  ⎋ interrupting…"));
                    let session = self.session.clone();
                    let join = self.stream_join.take();
                    let host_abort = self.host_tool_abort.take();
                    return Some(cmd::cmd(move || async move {
                        if let Some(host_abort) = host_abort {
                            host_abort.abort();
                        }
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
                        Msg::Interrupted {
                            goal_cancelled: true,
                            status_entry,
                        }
                    }));
                }
                // Tool approval is the top-most modal in both rendering and
                // input dispatch. No page, picker, or global mode shortcut may
                // consume keys while a tool is waiting.
                if self.state == State::Awaiting {
                    return self.handle_approval_key(&key);
                }
                // The semantic transcript is a true modal surface. It keeps
                // all styles and owns navigation until explicitly closed.
                if let Some(transcript) = self.transcript_view.as_mut() {
                    if transcript.handle_key(&key) == TranscriptViewportAction::CloseRequested {
                        self.transcript_view = None;
                    }
                    return None;
                }
                // The /help overlay owns its own close + scroll keys.
                if self.help_open {
                    return self.handle_help_key(&key);
                }
                // /memory panel takes all keys while open.
                if self.memory.is_some() {
                    return self.memory_key(&key);
                }
                // Asset resource panels take all keys while open.
                if self.asset_list.is_some() {
                    return self.handle_asset_list_key(&key);
                }
                if self.runtime_activity.is_some() {
                    return self.handle_runtime_activity_key(&key);
                }
                // /kb panel takes all keys while open.
                if self.kb.is_some() {
                    return self.handle_kb_key(&key);
                }
                // /ide panel takes all keys while open.
                if self.ide.is_some() {
                    if self
                        .ide
                        .as_ref()
                        .is_some_and(|ide| ide.intelligence.is_some())
                    {
                        return self.handle_ide_intelligence_key(&key);
                    }
                    if let Some(command) = self.try_submit_ide_intelligence_prompt(&key) {
                        return command;
                    }
                    self.ide_key(&key);
                    return None;
                }
                // `/tasks` / Ctrl+B is a modal delegated-work inspector. It
                // remains available while a turn streams, but its keys never
                // leak into the live composer or interrupt the parent turn.
                if self.task_panel.is_some() {
                    return self.handle_task_panel_key(&key);
                }
                // `/relay` is a modal session picker; execution-mode shortcuts
                // and composer input must not change behind it.
                if self.relay_panel.is_some() {
                    return self.handle_relay_key(&key);
                }
                // Shift+Tab cycles run mode in any state.
                if key.code == KeyCode::BackTab {
                    self.mode = self.mode.next();
                    return None;
                }
                // /model picker takes keys while open — consume EVERY key so
                // nothing leaks to the hidden input box behind the overlay.
                if self.model_menu.is_some() {
                    return self.handle_model_key(&key).unwrap_or(None);
                }
                // /effort slider takes keys while open.
                if let Some(sel) = self.effort_panel {
                    // Once the Ultracode activation has started, keep the
                    // confirmed selection stable until the flourish hands off
                    // to the session rebuild. Esc remains an explicit cancel.
                    if self.effort_anim.is_some() {
                        if key.code == KeyCode::Esc {
                            self.effort_panel = None;
                            self.effort_anim = None;
                            self.gradient_frame = 0;
                            advance_ultracode_animation_epoch(&mut self.ultracode_animation_epoch);
                        }
                        return None;
                    }
                    match key.code {
                        KeyCode::Left => self.effort_panel = Some(sel.saturating_sub(1)),
                        KeyCode::Right => {
                            self.effort_panel = Some((sel + 1).min(EFFORT_LEVELS.len() - 1))
                        }
                        KeyCode::Enter => {
                            return self.confirm_effort_selection(sel);
                        }
                        KeyCode::Esc => {
                            self.effort_panel = None;
                            self.effort_anim = None;
                        }
                        _ => {}
                    }
                    return None;
                }
                // /theme picker: ↑/↓ preview, Enter apply, Esc cancel.
                if let Some(sel) = self.theme_panel {
                    match key.code {
                        KeyCode::Up => self.theme_panel = Some(sel.saturating_sub(1)),
                        KeyCode::Down => self.theme_panel = Some((sel + 1).min(THEMES.len() - 1)),
                        KeyCode::Enter => self.apply_theme_selection(sel),
                        KeyCode::Esc => self.theme_panel = None,
                        _ => {}
                    }
                    return None;
                }
                // /plugin panel: ↑/↓ select, Space enable/disable, Esc close.
                if let Some(sel) = self.plugins_panel {
                    let last = self.skills.len().saturating_sub(1);
                    match key.code {
                        KeyCode::Up => self.plugins_panel = Some(sel.saturating_sub(1)),
                        KeyCode::Down => self.plugins_panel = Some((sel + 1).min(last)),
                        KeyCode::Char(' ') => {
                            self.toggle_plugin_skill(sel.min(last));
                        }
                        KeyCode::Esc => self.plugins_panel = None,
                        _ => {}
                    }
                    return None;
                }
                // Asset review issue checklist: consume EVERY key while open.
                if self.review_open {
                    return self.handle_review_key(&key);
                }
                // `/flow` DAG picker: same.
                if self.flow.is_some() {
                    return self.handle_flow_key(&key);
                }
                // `/agent` definition picker: same.
                if self.agent_picker.is_some() {
                    return self.handle_agent_key(&key);
                }
                // `/mcp` asset selector: same.
                if self.mcp_picker.is_some() {
                    return self.handle_mcp_key(&key);
                }
                if self.skill_picker.is_some() {
                    return self.handle_skill_key(&key);
                }
                if self.okf_picker.is_some() {
                    return self.handle_okf_package_key(&key);
                }
                // `/loop` engineered-loop dashboard: same.
                if self.loop_panel.is_some() {
                    return self.handle_loop_key(&key);
                }
                // Cross-platform terminal control for delegated work. Higher
                // decision modals and focused panels retain priority above it.
                if panels::tasks::is_task_panel_key(&key) {
                    return self.toggle_task_panel();
                }
                // Codex-style transcript shortcut: Ctrl+T owns the complete
                // semantic conversation, including live tool output and the
                // current Markdown tail. Keep the prompt draft intact.
                if is_tool_output_key(&key) {
                    self.open_transcript_view();
                    return None;
                }
                // Shift+End jumps to the latest output and resumes auto-follow.
                if key.code == KeyCode::End && key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.viewport.update(ViewportMsg::Bottom);
                    self.viewport.set_auto_scroll(true);
                    return None;
                }
                if let Some(action) = self.keymap.resolve(&key) {
                    let m = match action {
                        Action::ScrollUp => ViewportMsg::PageUp,
                        Action::ScrollDown => ViewportMsg::PageDown,
                        Action::ScrollTop => ViewportMsg::Top,
                        Action::ScrollBottom => ViewportMsg::Bottom,
                    };
                    self.viewport.update(m);
                    // Pause auto-follow while scrolled up; resume once back at the
                    // bottom — so streaming output doesn't yank the view down.
                    self.viewport.set_auto_scroll(self.viewport.at_bottom());
                    return None;
                }
                if self.state == State::Rebuilding
                    && self.goal_run.is_some()
                    && key.code == KeyCode::Esc
                {
                    self.cancel_goal_state("interrupted by Esc");
                    self.push_line(
                        &Style::new()
                            .fg(TN_YELLOW)
                            .render("  ⎋ goal loop interrupted"),
                    );
                    return None;
                }
                // Esc interrupts the in-progress run (input stays usable otherwise).
                if self.state == State::Streaming && key.code == KeyCode::Esc {
                    if self.stream_join_settling {
                        // The model turn is already terminal. Esc means “consume
                        // my queued follow-up now”: abort only the stale cleanup
                        // worker, then let StreamJoinSettled start the queue head.
                        if !self.queue.is_empty() {
                            if let Some(abort) = self.stream_settle_abort.take() {
                                abort.abort();
                            }
                        }
                        return None;
                    }
                    if self.deep_research_subagent_settlement_inflight {
                        if !self.queue.is_empty() {
                            // The parent stream is already terminal and no
                            // single-flight lease remains. Keep the bounded
                            // child cleanup command running in the background,
                            // invalidate its UI generation, and let the queued
                            // user steer run now just like Codex's Esc path.
                            self.deep_research_subagent_settlement_inflight = false;
                            self.invalidate_subagent_snapshots();
                            self.state = State::Idle;
                            self.running_task = None;
                            self.spinner.stop();
                            self.restore_autonomy();
                            self.relayout();
                            self.rebuild_viewport();
                            return self.drain_queue();
                        }
                        return None;
                    }
                    if self.interrupting {
                        return None;
                    }
                    let goal_cancelled = self.cancel_goal_state("interrupted by Esc");
                    self.interrupting = true;
                    if self.stream_join.is_none()
                        && self.rx.is_none()
                        && !self.host_progress_inflight
                    {
                        self.interrupted_stream_start_token = Some(self.stream_start_token);
                    }
                    self.stream_start_token = self.stream_start_token.wrapping_add(1);
                    self.deep_research_stream_timeout_token =
                        self.deep_research_stream_timeout_token.wrapping_add(1);
                    let status_entry = self
                        .push_tracked_line(&Style::new().fg(TN_YELLOW).render("  ⎋ interrupting…"));
                    let session = self.session.clone();
                    let join = self.stream_join.take();
                    let host_abort = self.host_tool_abort.take();
                    return Some(cmd::cmd(move || async move {
                        if let Some(host_abort) = host_abort {
                            host_abort.abort();
                        }
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
                        Msg::Interrupted {
                            goal_cancelled,
                            status_entry,
                        }
                    }));
                }
                // During goal retry backoff the app is idle, but the goal is
                // still active. Esc invalidates the pending generation and
                // restores normal Ultracode planning immediately.
                if self.state == State::Idle && self.goal_run.is_some() && key.code == KeyCode::Esc
                {
                    self.cancel_goal_state("interrupted by Esc");
                    self.push_line(
                        &Style::new()
                            .fg(TN_YELLOW)
                            .render("  ⎋ goal loop interrupted"),
                    );
                    return self.restore_goal_planning_mode();
                }
                // Outside a live run, Esc leaves shell/research mode while
                // preserving the partial command or query for normal editing.
                if should_exit_prompt_mode(&self.state, self.shell_mode, self.research_mode, &key) {
                    self.shell_mode = false;
                    self.research_mode = false;
                    return None;
                }
                if self.state == State::Idle && self.agent_dev.is_some() && key.code == KeyCode::Esc
                {
                    self.exit_agent_dev();
                    return None;
                }
                if self.state == State::Idle && self.mcp_dev.is_some() && key.code == KeyCode::Esc {
                    self.exit_mcp_dev();
                    return None;
                }
                if self.state == State::Idle && self.skill_dev.is_some() && key.code == KeyCode::Esc
                {
                    self.exit_skill_dev();
                    return None;
                }
                if self.state == State::Idle && self.okf_dev.is_some() && key.code == KeyCode::Esc {
                    self.exit_okf_dev();
                    return None;
                }
                // Slash-command menu: ↑/↓ select, Enter run, Tab complete, Esc
                // dismiss — takes priority over history recall while open.
                if self.slash_menu_open() {
                    if let Some(result) = self.handle_slash_key(&key) {
                        return result;
                    }
                }
                // `@` file picker takes nav keys while open.
                if self.file_menu_open() {
                    if let Some(result) = self.handle_file_key(&key) {
                        return result;
                    }
                }
                // ↑/↓ recall prompt history (single-line input only, so multi-line
                // editing keeps normal cursor movement).
                if matches!(key.code, KeyCode::Up | KeyCode::Down)
                    && !self.textarea.value().contains('\n')
                    && !self.history.is_empty()
                {
                    self.history_recall(key.code == KeyCode::Up);
                    return None;
                }
                match composer_attachment_key_action(
                    &key,
                    &self.textarea.value(),
                    self.pending_images.len(),
                ) {
                    // Staging only: paste must never emit Submit or start a turn.
                    Some(ComposerAttachmentKeyAction::StageClipboardImage) => {
                        self.paste_clipboard_image();
                        return None;
                    }
                    // With an empty draft, Backspace removes the most recently
                    // pasted chip without stealing Backspace from ordinary text.
                    Some(ComposerAttachmentKeyAction::RemoveLastImage) => {
                        self.pending_images.pop();
                        self.relayout();
                        return None;
                    }
                    // Textarea intentionally ignores an empty submit, so the
                    // composer owns the image-only message case.
                    Some(ComposerAttachmentKeyAction::SubmitImageOnly) => {
                        return Some(cmd::msg(Msg::Submit(String::new())));
                    }
                    None => {}
                }
                // Input is always live (you can keep typing while the agent works);
                // a submit while busy is queued and run when the current turn ends.
                if let Some(TextareaMsg::Submit(text)) = self.textarea.handle_key(&key) {
                    return Some(cmd::msg(Msg::Submit(text)));
                }
                // A leading `!` enters shell mode and a leading `?` enters
                // deep-research mode. Each stays on until Esc or submit.
                let val = self.textarea.value();
                if !self.shell_mode && !self.research_mode {
                    if let Some(rest) = val.strip_prefix('!') {
                        self.shell_mode = true;
                        self.textarea.set_value(rest);
                    } else if let Some(rest) = val.strip_prefix('?') {
                        self.research_mode = true;
                        self.textarea.set_value(rest);
                    }
                }
            }

            Msg::Term(Event::Mouse(m)) => {
                use a3s_tui::event::{MouseButton, MouseEventKind};
                if self.goal_resume_prompt.is_some() {
                    return self.handle_goal_resume_mouse(&m);
                }
                if self.state == State::Awaiting {
                    return self.handle_approval_mouse(&m);
                }
                if let Some(transcript) = self.transcript_view.as_mut() {
                    transcript.handle_mouse(&m);
                    return None;
                }
                if self.task_panel.is_some() {
                    return self.handle_task_panel_mouse(&m);
                }
                if self.relay_panel.is_some() {
                    return self.handle_relay_mouse(&m);
                }
                if self.model_menu.is_some() {
                    return self.handle_model_mouse(&m);
                }
                if self.effort_panel.is_some() {
                    self.handle_effort_mouse(&m);
                    return None;
                }
                if self.theme_panel.is_some() {
                    self.handle_theme_mouse(&m);
                    return None;
                }
                if self.file_menu_open() {
                    self.handle_file_mouse(&m);
                    return None;
                }
                if self.plugins_panel.is_some() {
                    self.handle_plugins_mouse(&m);
                    return None;
                }
                if self.slash_menu_open() {
                    return self.handle_slash_mouse(&m);
                }
                if self.flow.is_some() {
                    return self.handle_flow_mouse(&m);
                }
                if self.agent_picker.is_some() {
                    return self.handle_agent_mouse(&m);
                }
                if self.mcp_picker.is_some() {
                    return self.handle_mcp_mouse(&m);
                }
                if self.skill_picker.is_some() {
                    return self.handle_skill_mouse(&m);
                }
                if self.okf_picker.is_some() {
                    return self.handle_okf_package_mouse(&m);
                }
                if self.help_open {
                    match m.kind {
                        MouseEventKind::ScrollUp => self.scroll_help_by(-3),
                        MouseEventKind::ScrollDown => self.scroll_help_by(3),
                        _ => {}
                    }
                    return None;
                }
                // Full-screen /ide //config //kb page: the transcript isn't
                // visible, so transcript scroll/select must not act on it
                // (a drag would silently copy hidden text).
                if self.ide.is_some()
                    || self.kb.is_some()
                    || self.asset_list.is_some()
                    || self.runtime_activity.is_some()
                {
                    return None;
                }
                if let Some(action) = self.attachment_action_at(m.row, m.column) {
                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => match action {
                            AttachmentAction::Preview(index) => {
                                let preview =
                                    self.pending_images.get(index).map(PendingImage::preview);
                                match preview {
                                    Some(Ok(spec)) => self.open_remote_view(&spec),
                                    Some(Err(error)) => self.push_notice(
                                        NoticeKind::Warning,
                                        format!("Image preview unavailable: {error}"),
                                    ),
                                    None => {}
                                }
                            }
                            AttachmentAction::Remove(index) => {
                                if index < self.pending_images.len() {
                                    self.pending_images.remove(index);
                                    self.relayout();
                                }
                            }
                        },
                        MouseEventKind::Drag(MouseButton::Left)
                        | MouseEventKind::Up(MouseButton::Left) => {}
                        _ => {}
                    }
                    return None;
                }
                let vp_rows = self.viewport_rows();
                // Content columns exclude the rightmost scrollbar column.
                let max_col = (self.width as usize).saturating_sub(2) as u16;
                match m.kind {
                    MouseEventKind::ScrollUp => {
                        self.selection = None;
                        self.viewport.update(ViewportMsg::ScrollUp(3));
                    }
                    MouseEventKind::ScrollDown => {
                        self.selection = None;
                        self.viewport.update(ViewportMsg::ScrollDown(3));
                    }
                    // Drag to select transcript text. Capture stays on so the wheel
                    // still scrolls; the app owns selection, so scroll + copy work
                    // together (no mode toggle). Release copies to the clipboard.
                    MouseEventKind::Down(MouseButton::Left) => {
                        self.selection = viewport_mouse_cell(m.row, m.column, vp_rows, max_col)
                            .map(|p| Selection { anchor: p, head: p });
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        if let Some(s) = self.selection.as_mut() {
                            if let Some(p) =
                                viewport_mouse_cell_clamped(m.row, m.column, vp_rows, max_col)
                            {
                                s.head = p;
                            }
                        }
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        if let Some(mut s) = self.selection {
                            if let Some(p) =
                                viewport_mouse_cell_clamped(m.row, m.column, vp_rows, max_col)
                            {
                                s.head = p;
                            }
                            let view = self.viewport.view();
                            if is_remote_view_click(&view, s) {
                                self.selection = None;
                                if let Some(spec) = self.last_view.clone() {
                                    self.open_remote_view(&spec);
                                } else {
                                    self.push_line(&Style::new().fg(TN_GRAY).render(
                                        "  no trusted view is available for this Open view marker yet",
                                    ));
                                }
                            } else if s.is_empty() {
                                self.selection = None;
                            } else {
                                let (r1, c1, r2, c2) = s.ordered();
                                let text = selection_to_text(&view, r1, c1, r2, c2);
                                if text.trim().is_empty() {
                                    self.selection = None;
                                } else {
                                    // Keep the highlight visible as "copied" feedback.
                                    copy_to_clipboard(&text);
                                }
                            }
                        }
                    }
                    _ => {}
                }
                // Pause auto-follow while scrolled up (so streaming output won't
                // yank the view down); resume once back at the bottom.
                self.viewport.set_auto_scroll(self.viewport.at_bottom());
            }

            Msg::Submit(text) => return self.on_submit(text),

            Msg::GoalContinue { generation, prompt } => {
                return self.handle_goal_continue(generation, prompt);
            }

            Msg::GoalCleared => {
                self.finalize_streaming();
                self.review_pending = false;
                self.sleep_pending = false;
                self.finish();
                return self.restore_goal_planning_mode();
            }

            Msg::StreamStarted {
                token,
                session,
                rx,
                join,
                submitted_images: _submitted_images,
            } => {
                if self.interrupted_stream_start_token == Some(token) {
                    self.discard_active_queued_turn(token);
                    return Some(discard_started_stream(session, join, token));
                }
                if token != self.stream_start_token
                    || self.state != State::Streaming
                    || self.interrupting
                {
                    self.discard_active_queued_turn(token);
                    return Some(discard_started_stream(session, join, token));
                }
                self.commit_active_queued_turn(token);
                self.stream_join_settling = false;
                self.rx = Some(rx.clone());
                self.stream_join = Some(join);
                self.host_tool_abort = None;
                self.host_progress_inflight = false;
                self.interrupting = false;
                return Some(pump(rx));
            }

            Msg::StreamJoinSettled { token, synthesis } => {
                if token != self.stream_start_token || !self.stream_join_settling {
                    return None;
                }
                self.stream_join_settling = false;
                self.stream_settle_abort = None;
                self.state = State::Idle;
                self.relayout();
                return self.continue_after_stream_settled(synthesis);
            }

            Msg::DiscardedStreamSettled { token } => {
                return self.on_interrupted_stream_start_settled(token);
            }

            Msg::QuitReady => return self.finish_graceful_quit(),

            Msg::StreamError {
                token,
                error: e,
                retryable_admission,
                submitted_images,
            } => {
                if self.interrupted_stream_start_token == Some(token) {
                    // The user explicitly interrupted this not-yet-admitted
                    // turn. Its attachments belong to the already-rendered
                    // cancelled message and must not leak into the successor.
                    self.discard_active_queued_turn(token);
                    return self.on_interrupted_stream_start_settled(token);
                }
                if token != self.stream_start_token || self.interrupting {
                    self.discard_active_queued_turn(token);
                    return None;
                }
                let was_queued = self.active_queued_turn_token == Some(token);
                let queued_turn_restored = if was_queued && retryable_admission {
                    self.restore_active_queued_turn(token)
                } else {
                    self.discard_active_queued_turn(token);
                    false
                };
                // A direct composer submission still owns its previews. A
                // queued turn retains Arc-backed images in the lane payload,
                // so its attempted copy must not be moved into the composer.
                if !was_queued {
                    restore_submitted_images(&mut self.pending_images, submitted_images);
                }
                self.relayout();
                self.push_notice(NoticeKind::Error, &e);
                if self.recover_deep_research_report_after_model_error(&e) {
                    return self.complete_turn();
                }
                self.loop_remaining = 0; // a failed turn stops the /loop
                self.review_pending = false; // a turn that never started can't
                self.sleep_pending = false; // deliver a review/sleep report
                self.finish();
                if queued_turn_restored {
                    self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render("    ⋯ queued turn retained · retrying after session settles"),
                    );
                    return Some(self.retry_queued_turn_after_admission_failure());
                }
                if self.goal_run.is_some() {
                    return self.continue_goal_run(Some(e));
                }
                self.restore_autonomy();
                // Don't strand messages queued while this turn was starting.
                return self.drain_queue();
            }

            Msg::QueueRetry { generation } => {
                if generation != self.queue_retry_generation {
                    return None;
                }
                return self.drain_queue();
            }

            Msg::WorkspaceManifest(snapshot) => {
                self.files = snapshot.file_paths();
                self.file_sel = self.file_sel.min(self.files.len().saturating_sub(1));
                return Some(pump_manifest(self.workspace_manifest_rx.clone()));
            }

            Msg::WorkspaceManifestStopped => {
                let snapshot = self.workspace_manifest.snapshot();
                self.files = snapshot.file_paths();
                self.file_sel = self.file_sel.min(self.files.len().saturating_sub(1));
            }

            Msg::IdeIntelligenceCompleted { request_id, result } => {
                self.apply_ide_intelligence_result(request_id, result);
            }

            Msg::IdeIntelligenceJumpCompleted {
                request_id,
                jump_request_id,
                result,
            } => {
                self.apply_ide_intelligence_jump(request_id, jump_request_id, result);
            }

            Msg::Interrupted {
                goal_cancelled,
                status_entry,
            } => {
                // Esc force-aborted the turn. The cancel command awaited the
                // stream join first, so core has committed the interrupted
                // history before any queued continuation starts.
                self.finalize_streaming();
                self.preserve_interrupted_tools();
                self.replace_tracked_line(
                    status_entry,
                    &Style::new().fg(TN_YELLOW).render("  ⎋ interrupted"),
                );
                self.loop_remaining = 0; // Esc also stops a /loop
                self.review_pending = false; // and abandons an asset review
                self.sleep_pending = false; // and a `/sleep` consolidation
                let deep_research_interrupted = self.deep_research_loop.is_some();
                if deep_research_interrupted {
                    self.invalidate_subagent_snapshots();
                }
                self.finish();
                let continuation =
                    Self::interrupted_continuation(goal_cancelled, deep_research_interrupted);
                return self.defer_or_continue_after_interrupt(continuation);
            }

            Msg::Agent { source, event } => {
                if !self
                    .rx
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &source))
                {
                    return None;
                }
                // `tool_with_events` shares AgentEvent as a progress envelope.
                // A nested tool/agent must never be allowed to finish the outer
                // DeepResearch turn; only DeepResearchWorkflowCompleted owns
                // that state transition.
                if self.host_progress_inflight && host_progress_event_is_terminal(&event) {
                    return self.rx.clone().map(pump);
                }
                return self.on_agent_event(*event);
            }

            Msg::StreamEnded(source) => {
                if !self
                    .rx
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &source))
                {
                    return None;
                }
                if self.host_progress_inflight {
                    self.rx = None;
                    return None;
                }
                if self.interrupting || self.state != State::Streaming {
                    return None;
                }
                // Channel closed without a normal End event (abnormal close).
                self.finalize_streaming();
                self.preserve_interrupted_tools();
                if self.deep_research_loop.is_some()
                    && self.recover_deep_research_report_after_model_error(
                        "DeepResearch model stream closed before a terminal event.",
                    )
                {
                    return self.complete_turn();
                }
                // An asset-review report fully streamed before the drop still
                // counts — same for a `/sleep` consolidation report.
                let turn_text = self.turn_text.clone();
                self.capture_review(&turn_text);
                let sleep_save = self.capture_sleep(&turn_text);
                self.disarm_sleep_if_over(sleep_save.is_some());
                return match (sleep_save, self.complete_turn()) {
                    (Some(save), Some(next)) => Some(cmd::batch(vec![save, next])),
                    (save, next) => save.or(next),
                };
            }

            Msg::SpinnerTick => {
                self.spinner.tick();
                self.blink_tick = self.blink_tick.wrapping_add(1);
                if matches!(self.state, State::Streaming | State::Rebuilding) {
                    if self.state == State::Streaming {
                        self.update_viewport_with_stream();
                    }
                    return Some(spinner_tick());
                }
            }

            Msg::StreamCommitTick => {
                if self.state == State::Streaming {
                    if self.streaming.commit_tick(Instant::now()) {
                        self.update_viewport_with_stream();
                    }
                    return Some(stream_commit_tick());
                }
            }

            Msg::BannerTick => {
                // Re-render the animated mascot only while the banner is shown
                // (start screen / after /clear); the heartbeat keeps running so
                // the animation resumes whenever the banner reappears.
                if self.messages.is_empty()
                    && self.state == State::Idle
                    && self.ide.is_none()
                    && self.memory.is_none()
                    && !self.help_open
                {
                    self.anim = self.anim.wrapping_add(1);
                    self.viewport.set_content(&self.banner());
                }
                // Inactivity auto-review: after a quiet stretch with a real
                // Core conversation, summarise its current revision once as a
                // passive review notice. UI notices in `messages` are ignored.
                if self.state == State::Idle
                    && self.last_activity.elapsed() > AUTO_REVIEW_IDLE
                    && !self.auto_review.current_is_reviewed(&self.session_id)
                {
                    let history = self.session.history();
                    if let Some(ticket) = self.auto_review.begin(
                        &self.session_id,
                        auto_review_history_has_user_turn(&history),
                    ) {
                        let agent = self.agent.clone();
                        let workspace = self.cwd.clone();
                        let review = cmd::cmd(move || async move {
                            let conf = a3s_code_core::hitl::ConfirmationPolicy::enabled()
                                .with_timeout(BACKGROUND_CONFIRM_TIMEOUT_MS, TimeoutAction::Reject);
                            let prompt = "Briefly review this conversation so far: summarise the \
                                 key decisions and what's done, then list any open threads or next \
                                 steps. Keep it to a few lines.";
                            let mut answer = String::new();
                            if let Ok(sess) = agent
                                .session_async(workspace, Some(tui_session_options(conf)))
                                .await
                            {
                                if let Ok((mut rx, _j)) = sess.stream(prompt, Some(&history)).await
                                {
                                    while let Some(ev) = rx.recv().await {
                                        match ev {
                                            AgentEvent::TextDelta { text } => {
                                                answer.push_str(&text)
                                            }
                                            AgentEvent::End { text, .. } => {
                                                if answer.trim().is_empty() {
                                                    answer = text;
                                                }
                                                break;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            Msg::AutoReview {
                                ticket,
                                text: answer,
                            }
                        });
                        return Some(cmd::batch(vec![banner_tick(), review]));
                    }
                }
                // Keep the OS access token fresh: refresh proactively before it
                // expires so the agent's $A3S_OS_TOKEN never goes stale mid-session.
                if !self.os_refreshing {
                    if let Some(s) = &self.os_session {
                        if crate::a3s_os::needs_refresh(s) {
                            self.os_refreshing = true;
                            let session = s.clone();
                            let refresh = cmd::cmd(move || async move {
                                Msg::OsRefreshed(
                                    crate::a3s_os::refresh_session(&session)
                                        .await
                                        .map_err(|e| e.to_string()),
                                )
                            });
                            return Some(cmd::batch(vec![banner_tick(), refresh]));
                        }
                    }
                }
                return Some(banner_tick());
            }

            Msg::UltracodeTick { epoch } => {
                if !ultracode_tick_is_current(self.ultracode_animation_epoch, epoch) {
                    return None;
                }
                self.gradient_frame = self.gradient_frame.wrapping_add(1);

                let action = ultracode_tick_action(
                    self.effort_anim.map(|started| started.elapsed()),
                    self.gradient_until.map(|started| started.elapsed()),
                );
                match action {
                    UltracodeTickAction::ContinueConfirm | UltracodeTickAction::ContinueBorder => {
                        return Some(ultracode_tick(epoch))
                    }
                    UltracodeTickAction::BeginRebuild => {
                        self.effort_anim = None;
                        advance_ultracode_animation_epoch(&mut self.ultracode_animation_epoch);
                        let selected = self.effort_panel.take().unwrap_or(self.effort);
                        return self.apply_effort(selected);
                    }
                    UltracodeTickAction::ClearBorder => {
                        self.gradient_until = None;
                        advance_ultracode_animation_epoch(&mut self.ultracode_animation_epoch);
                    }
                    UltracodeTickAction::Idle => {}
                }
            }

            Msg::AutoReview { ticket, text } => {
                let current_has_user_turn =
                    auto_review_history_has_user_turn(&self.session.history());
                if self.auto_review.accept(&ticket, &self.session_id)
                    && current_has_user_turn
                    && !text.trim().is_empty()
                {
                    // Dim + unobtrusive — this is a passive review notice.
                    let dim =
                        |s: &str| format!("  {}", Style::new().fg(TN_GRAY).italic().render(s));
                    let mut lines = vec![dim("⟳ inactivity review")];
                    lines.extend(text.trim().lines().map(dim));
                    self.push_line(&lines.join("\n"));
                }
            }

            Msg::Compacted(result) => {
                let summary = match result {
                    Ok(Some(summary)) if !summary.trim().is_empty() => summary,
                    Ok(_) => {
                        self.compacting = None;
                        self.push_line(
                            &Style::new()
                                .fg(TN_RED)
                                .render("  compaction failed (empty summary)"),
                        );
                        return None;
                    }
                    Err(error) => {
                        self.compacting = None;
                        self.push_line(
                            &Style::new()
                                .fg(TN_RED)
                                .render(&format!("  compaction failed: {error}")),
                        );
                        return None;
                    }
                };
                // Reseed a FRESH session (new id, no history) carrying just the
                // summary in its system prompt — that's the actual compaction.
                let summary = summary.trim().to_string();
                let session_id = new_session_id();
                let mut profile = self.session_rebuild_profile();
                profile.session_id = session_id.clone();
                profile.compact_summary = Some(summary.clone());
                return self.start_session_rebuild(
                    profile,
                    SessionRebuildAction::Compact {
                        summary,
                        session_id,
                    },
                );
            }

            Msg::UpdateCheck(latest) => {
                let current = crate::update::current_version();
                let newer = latest
                    .as_deref()
                    .is_some_and(|l| !crate::update::version_ge(&current, l));
                if newer {
                    self.update_available = latest;
                    // Refresh the start screen so the notice shows in the banner
                    // without clobbering it with a transcript line.
                    if self.messages.is_empty() {
                        self.viewport.set_content(&self.banner());
                    }
                }
            }

            Msg::ModalConfirm {
                tool_id,
                approved,
                approve_all_pending,
            } => {
                let pending = take_pending_tools_for_confirmation(
                    &mut self.pending_tools,
                    &tool_id,
                    approved && approve_all_pending,
                );
                if !pending.is_empty() {
                    self.approval_sel = 0;
                    self.state = if self.pending_tools.is_empty() {
                        State::Streaming
                    } else {
                        State::Awaiting
                    };
                    let session = self.session.clone();
                    return Some(cmd::batch(vec![
                        cmd::cmd(move || async move {
                            for (tool_id, _) in pending {
                                let _ = session.confirm_tool_use(&tool_id, approved, None).await;
                            }
                            Msg::Resume
                        }),
                        spinner_tick(),
                        stream_commit_tick(),
                    ]));
                }
                self.state = if self.pending_tools.is_empty() {
                    State::Streaming
                } else {
                    State::Awaiting
                };
            }

            other => return self.handle_async_message(other),
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers }
    }

    #[test]
    fn clipboard_image_paste_is_staged_and_never_classified_as_submit() {
        let paste = key(KeyCode::Char('v'), KeyModifiers::CONTROL);

        assert_eq!(
            composer_attachment_key_action(&paste, "draft", 0),
            Some(ComposerAttachmentKeyAction::StageClipboardImage)
        );
        assert_ne!(
            composer_attachment_key_action(&paste, "", 1),
            Some(ComposerAttachmentKeyAction::SubmitImageOnly)
        );
    }

    #[test]
    fn staged_image_requires_an_explicit_enter_to_submit() {
        assert_eq!(
            composer_attachment_key_action(&key(KeyCode::Enter, KeyModifiers::NONE), "", 1,),
            Some(ComposerAttachmentKeyAction::SubmitImageOnly)
        );
        assert_eq!(
            composer_attachment_key_action(&key(KeyCode::Backspace, KeyModifiers::NONE), "", 1,),
            Some(ComposerAttachmentKeyAction::RemoveLastImage)
        );
    }
}
