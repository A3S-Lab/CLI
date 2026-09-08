//! Diff review overlay (Cursor Ctrl+R review spirit; bound to Ctrl+G).
//!
//! Reconstructs successful file-change tools from the latest user turn in the
//! transcript and presents a full DiffView with file switching and follow-up
//! seeding into the composer. Does not steal Ctrl+R (prompt history).

use super::super::file_change_view::render_full_file_change;
use super::super::render::{is_file_change_tool, resolve_file_change_sides};
use super::super::runtime_projection::ToolCallState;
use super::super::*;
use a3s_tui::style::{fit_visible, strip_ansi, Style};

/// One successful file mutation from the latest user turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffReviewChange {
    pub(crate) action: String,
    pub(crate) path: String,
    pub(crate) before: String,
    pub(crate) after: String,
}

#[derive(Clone, Debug)]
pub(crate) struct DiffReviewState {
    pub(crate) changes: Vec<DiffReviewChange>,
    pub(crate) index: usize,
    pub(crate) scroll: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiffReviewAction {
    None,
    Close,
    /// Close review and seed the PromptBar with a follow-up draft.
    SeedComposer(String),
}

pub(crate) fn is_diff_review_key(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('g') && key.modifiers.contains(KeyModifiers::CONTROL)
}

impl DiffReviewState {
    pub(crate) fn new(changes: Vec<DiffReviewChange>) -> Self {
        Self {
            changes,
            index: 0,
            scroll: 0,
        }
    }

    pub(crate) fn current(&self) -> Option<&DiffReviewChange> {
        self.changes.get(self.index)
    }

    pub(crate) fn move_file(&mut self, delta: isize) {
        if self.changes.is_empty() {
            return;
        }
        let len = self.changes.len() as isize;
        let next = (self.index as isize + delta).rem_euclid(len) as usize;
        if next != self.index {
            self.index = next;
            self.scroll = 0;
        }
    }

    pub(crate) fn scroll_by(&mut self, delta: isize, max_scroll: usize) {
        if delta.is_negative() {
            self.scroll = self.scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.scroll = self.scroll.saturating_add(delta as usize).min(max_scroll);
        }
    }

    pub(crate) fn handle_key(&mut self, key: &KeyEvent, body_rows: usize) -> DiffReviewAction {
        match key.code {
            KeyCode::Esc => DiffReviewAction::Close,
            KeyCode::Left | KeyCode::Char('h') => {
                self.move_file(-1);
                DiffReviewAction::None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.move_file(1);
                DiffReviewAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll_by(-1, usize::MAX);
                DiffReviewAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll_by(1, usize::MAX);
                DiffReviewAction::None
            }
            KeyCode::PageUp => {
                self.scroll_by(-(body_rows as isize).max(1), usize::MAX);
                DiffReviewAction::None
            }
            KeyCode::PageDown => {
                self.scroll_by((body_rows as isize).max(1), usize::MAX);
                DiffReviewAction::None
            }
            KeyCode::Home => {
                self.scroll = 0;
                DiffReviewAction::None
            }
            KeyCode::End => {
                // Clamped against the rendered body on the next host handle /
                // overlay paint (same pattern as PageDown overshoot).
                self.scroll = usize::MAX;
                DiffReviewAction::None
            }
            KeyCode::Char('i') | KeyCode::Char('I') => {
                let Some(change) = self.current() else {
                    return DiffReviewAction::Close;
                };
                DiffReviewAction::SeedComposer(seed_follow_up_draft(change))
            }
            _ => DiffReviewAction::None,
        }
    }
}

pub(crate) fn seed_follow_up_draft(change: &DiffReviewChange) -> String {
    format!(
        "About the change to `{}` ({}):\n\nPlease ",
        change.path, change.action
    )
}

/// Host-side effect of a review key action (panel stays vs closes vs seeds draft).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiffReviewHostEffect {
    KeepOpen,
    Close,
    SeedComposer { draft: String },
}

pub(crate) fn diff_review_host_effect(action: DiffReviewAction) -> DiffReviewHostEffect {
    match action {
        DiffReviewAction::None => DiffReviewHostEffect::KeepOpen,
        DiffReviewAction::Close => DiffReviewHostEffect::Close,
        DiffReviewAction::SeedComposer(draft) => DiffReviewHostEffect::SeedComposer { draft },
    }
}

/// Collect successful file-change tools after the latest user message.
/// Last write wins per path. Failed / non-file tools are skipped.
pub(crate) fn latest_turn_file_changes(transcript: &Transcript) -> Vec<DiffReviewChange> {
    let entries: Vec<&TranscriptEntry> = transcript.iter().collect();
    let Some(user_idx) = entries
        .iter()
        .rposition(|entry| matches!(entry, TranscriptEntry::User { .. }))
    else {
        return Vec::new();
    };

    let mut by_path: Vec<(String, DiffReviewChange)> = Vec::new();
    for entry in entries.iter().skip(user_idx + 1) {
        let TranscriptEntry::Tool(tool) = entry else {
            if matches!(entry, TranscriptEntry::User { .. }) {
                break;
            }
            continue;
        };
        if tool.state() != ToolCallState::Succeeded {
            continue;
        }
        if !is_file_change_tool(tool.name()) {
            continue;
        }
        let args = tool.args_value();
        let meta = tool.metadata_value();
        let Some((action, path, before, after, _)) =
            resolve_file_change_sides(tool.name(), meta.as_ref(), args.as_ref())
        else {
            continue;
        };
        let change = DiffReviewChange {
            action: action.to_string(),
            path: path.to_string(),
            before: before.to_string(),
            after: after.to_string(),
        };
        if let Some(existing) = by_path.iter_mut().find(|(p, _)| p == &change.path) {
            *existing = (change.path.clone(), change);
        } else {
            by_path.push((change.path.clone(), change));
        }
    }
    by_path.into_iter().map(|(_, change)| change).collect()
}

/// Count in-flight file-change tools after the latest user message (not yet
/// succeeded). Used so Ctrl+G mid-turn does not claim "no edits".
pub(crate) fn latest_turn_pending_file_change_count(transcript: &Transcript) -> usize {
    let entries: Vec<&TranscriptEntry> = transcript.iter().collect();
    let Some(user_idx) = entries
        .iter()
        .rposition(|entry| matches!(entry, TranscriptEntry::User { .. }))
    else {
        return 0;
    };

    let mut pending = 0usize;
    for entry in entries.iter().skip(user_idx + 1) {
        let TranscriptEntry::Tool(tool) = entry else {
            if matches!(entry, TranscriptEntry::User { .. }) {
                break;
            }
            continue;
        };
        if !is_file_change_tool(tool.name()) {
            continue;
        }
        if matches!(
            tool.state(),
            ToolCallState::Preparing | ToolCallState::AwaitingApproval | ToolCallState::Running
        ) {
            pending = pending.saturating_add(1);
        }
    }
    pending
}

pub(crate) fn diff_review_empty_notice(pending_file_changes: usize) -> &'static str {
    if pending_file_changes > 0 {
        "File edits still running — try Ctrl+G again when the turn finishes"
    } else {
        "No file changes in the latest turn to review"
    }
}

fn review_header(state: &DiffReviewState, width: usize) -> String {
    let total = state.changes.len().max(1);
    let index = state.index.saturating_add(1).min(total);
    let path = state
        .current()
        .map(|c| c.path.as_str())
        .unwrap_or("(empty)");
    let action = state.current().map(|c| c.action.as_str()).unwrap_or("");
    let title = format!(" Review · {index}/{total} · {action} · {path} ");
    let hint = " ←→ files · j/k scroll · i instruct · Esc ";
    let title = fit_visible(
        &title,
        width.saturating_sub(a3s_tui::style::visible_len(hint)),
    );
    let line = format!("{title}{hint}");
    Style::new()
        .fg(TN_CYAN)
        .bold()
        .render(&fit_visible(&line, width))
}

fn review_footer(width: usize) -> String {
    Style::new().fg(TN_GRAY).render(&fit_visible(
        " Ctrl+G reopen · full DiffView for the latest turn's file edits ",
        width,
    ))
}

pub(crate) fn review_overlay_lines(
    state: &DiffReviewState,
    width: usize,
    max_rows: usize,
) -> Vec<String> {
    if max_rows == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    lines.push(review_header(state, width));
    if max_rows == 1 {
        return lines;
    }
    let footer_budget = usize::from(max_rows > 2);
    let body_budget = max_rows
        .saturating_sub(1)
        .saturating_sub(footer_budget)
        .max(1);

    let body = if let Some(change) = state.current() {
        let rendered = render_full_file_change(
            &change.action,
            &change.path,
            &change.before,
            &change.after,
            width,
        );
        let mut body_lines: Vec<String> = rendered.lines().map(str::to_string).collect();
        let max_scroll = body_lines.len().saturating_sub(body_budget);
        // Caller may pass a stale scroll; clamp for display.
        let scroll = state.scroll.min(max_scroll);
        if scroll > 0 {
            body_lines = body_lines.into_iter().skip(scroll).collect();
        }
        body_lines.truncate(body_budget);
        while body_lines.len() < body_budget {
            body_lines.push(String::new());
        }
        body_lines
    } else {
        vec![Style::new().fg(TN_GRAY).render(" (no file change) ")]
    };
    lines.extend(body);
    if footer_budget > 0 {
        lines.push(review_footer(width));
    }
    lines
}

/// Visible body rows available for scroll clamping (excludes header/footer).
pub(crate) fn review_body_row_budget(max_rows: usize) -> usize {
    max_rows.saturating_sub(2).max(1)
}

pub(crate) fn review_max_rows(screen_height: usize) -> usize {
    screen_height.saturating_sub(6).clamp(6, 24)
}

/// Keep the previously focused path after a Ctrl+G refresh when it still exists.
pub(crate) fn retain_diff_review_index(
    changes: &[DiffReviewChange],
    previous_path: Option<&str>,
) -> usize {
    previous_path
        .and_then(|path| changes.iter().position(|c| c.path == path))
        .unwrap_or(0)
}

impl App {
    pub(crate) fn open_diff_review(&mut self) {
        let changes = latest_turn_file_changes(&self.messages);
        if changes.is_empty() {
            let was_open = self.diff_review.take().is_some();
            if was_open {
                self.relayout();
            }
            let pending = latest_turn_pending_file_change_count(&self.messages);
            self.push_notice(NoticeKind::Info, diff_review_empty_notice(pending));
            return;
        }
        // History / other panels must not stack under review.
        self.history_panel = None;
        // Prefer keeping the previously selected path across Ctrl+G refresh.
        let previous_path = self
            .diff_review
            .as_ref()
            .and_then(|state| state.current().map(|c| c.path.clone()));
        let mut state = DiffReviewState::new(changes);
        state.index = retain_diff_review_index(&state.changes, previous_path.as_deref());
        self.diff_review = Some(state);
        self.relayout();
    }

    pub(crate) fn handle_diff_review_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let max_rows = review_max_rows(self.height as usize);
        let body_rows = review_body_row_budget(max_rows);
        let action = {
            let panel = self.diff_review.as_mut()?;
            // Clamp scroll against current body size before handling.
            if let Some(change) = panel.current() {
                let rendered = render_full_file_change(
                    &change.action,
                    &change.path,
                    &change.before,
                    &change.after,
                    self.width as usize,
                );
                let total = rendered.lines().count();
                let max_scroll = total.saturating_sub(body_rows);
                panel.scroll = panel.scroll.min(max_scroll);
            }
            panel.handle_key(key, body_rows)
        };
        match diff_review_host_effect(action) {
            DiffReviewHostEffect::KeepOpen => None,
            DiffReviewHostEffect::Close => {
                self.diff_review = None;
                self.relayout();
                None
            }
            DiffReviewHostEffect::SeedComposer { draft } => {
                self.diff_review = None;
                self.shell_mode = false;
                self.research_mode = false;
                self.textarea.set_value(&draft);
                self.relayout();
                None
            }
        }
    }

    pub(crate) fn overlay_diff_review(&self, composed: String) -> String {
        let Some(state) = self.diff_review.as_ref() else {
            return composed;
        };
        let max_rows = review_max_rows(self.height as usize);
        let lines = review_overlay_lines(state, self.width as usize, max_rows);
        self.overlay_list(composed, &lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool_entry(name: &str, state: ToolCallState, meta: serde_json::Value) -> TranscriptEntry {
        TranscriptEntry::Tool(ToolTranscriptEntry::from_parts_for_test(
            None,
            name.to_string(),
            state,
            "{}".into(),
            None,
            String::new(),
            Some(meta),
            None,
            None,
            None,
            true,
        ))
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers }
    }

    #[test]
    fn is_diff_review_key_is_ctrl_g_only() {
        assert!(is_diff_review_key(&key(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_diff_review_key(&key(
            KeyCode::Char('g'),
            KeyModifiers::NONE
        )));
        assert!(!is_diff_review_key(&key(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL
        )));
    }

    #[test]
    fn latest_turn_skips_failed_and_keeps_last_path_write() {
        let transcript = Transcript::from_entries(vec![
            TranscriptEntry::User {
                source: "edit files".into(),
                images: Vec::new(),
            },
            tool_entry(
                "edit",
                ToolCallState::Failed,
                json!({"file_path":"a.rs","before":"old","after":"bad"}),
            ),
            tool_entry(
                "edit",
                ToolCallState::Succeeded,
                json!({"file_path":"a.rs","before":"old","after":"v1"}),
            ),
            tool_entry(
                "edit",
                ToolCallState::Succeeded,
                json!({"file_path":"a.rs","before":"v1","after":"v2"}),
            ),
            tool_entry(
                "write",
                ToolCallState::Succeeded,
                json!({"file_path":"b.rs","created":true,"after":"new"}),
            ),
        ]);
        let changes = latest_turn_file_changes(&transcript);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].path, "a.rs");
        assert_eq!(changes[0].after, "v2");
        assert_eq!(changes[1].path, "b.rs");
        assert_eq!(changes[1].action, "Added");
    }

    #[test]
    fn latest_turn_empty_without_user_or_edits() {
        assert!(latest_turn_file_changes(&Transcript::default()).is_empty());
        let only_user = Transcript::from_entries(vec![TranscriptEntry::User {
            source: "hi".into(),
            images: Vec::new(),
        }]);
        assert!(latest_turn_file_changes(&only_user).is_empty());
    }

    #[test]
    fn empty_notice_distinguishes_pending_file_edits() {
        assert_eq!(
            diff_review_empty_notice(0),
            "No file changes in the latest turn to review"
        );
        assert!(diff_review_empty_notice(2).contains("still running"));

        let transcript = Transcript::from_entries(vec![
            TranscriptEntry::User {
                source: "edit".into(),
                images: Vec::new(),
            },
            tool_entry(
                "edit",
                ToolCallState::Running,
                json!({"file_path":"a.rs","before":"old","after":"new"}),
            ),
        ]);
        assert_eq!(latest_turn_pending_file_change_count(&transcript), 1);
        assert!(latest_turn_file_changes(&transcript).is_empty());
    }

    #[test]
    fn move_file_and_seed_draft() {
        let mut state = DiffReviewState::new(vec![
            DiffReviewChange {
                action: "Edited".into(),
                path: "a.rs".into(),
                before: "1".into(),
                after: "2".into(),
            },
            DiffReviewChange {
                action: "Added".into(),
                path: "b.rs".into(),
                before: String::new(),
                after: "x".into(),
            },
        ]);
        assert_eq!(
            state.handle_key(&key(KeyCode::Right, KeyModifiers::NONE), 10),
            DiffReviewAction::None
        );
        assert_eq!(state.index, 1);
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('h'), KeyModifiers::NONE), 10),
            DiffReviewAction::None
        );
        assert_eq!(state.index, 0);
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('l'), KeyModifiers::NONE), 10),
            DiffReviewAction::None
        );
        assert_eq!(state.index, 1);
        assert_eq!(
            state.handle_key(&key(KeyCode::Char('i'), KeyModifiers::NONE), 10),
            DiffReviewAction::SeedComposer(seed_follow_up_draft(state.current().unwrap()))
        );
        let draft = seed_follow_up_draft(state.current().unwrap());
        assert!(draft.contains("`b.rs`"));
        assert!(draft.contains("Added"));
        assert!(draft.ends_with("Please "));
        assert_eq!(
            state.handle_key(&key(KeyCode::Esc, KeyModifiers::NONE), 10),
            DiffReviewAction::Close
        );
        assert_eq!(
            diff_review_host_effect(DiffReviewAction::Close),
            DiffReviewHostEffect::Close
        );
        assert!(matches!(
            diff_review_host_effect(DiffReviewAction::SeedComposer(draft.clone())),
            DiffReviewHostEffect::SeedComposer { draft: d } if d == draft
        ));
        let _ = strip_ansi; // keep import warm for overlay tests elsewhere
    }

    #[test]
    fn overlay_lines_include_header_and_path() {
        let state = DiffReviewState::new(vec![DiffReviewChange {
            action: "Edited".into(),
            path: "src/main.rs".into(),
            before: "fn a() {}\n".into(),
            after: "fn a() { 1 }\n".into(),
        }]);
        let lines = review_overlay_lines(&state, 80, 12);
        assert!(!lines.is_empty());
        let header = strip_ansi(&lines[0]);
        assert!(header.contains("Review"), "{header}");
        assert!(header.contains("src/main.rs"), "{header}");
    }

    #[test]
    fn retain_index_prefers_previous_path_on_refresh() {
        let changes = vec![
            DiffReviewChange {
                action: "Edited".into(),
                path: "a.rs".into(),
                before: "1".into(),
                after: "2".into(),
            },
            DiffReviewChange {
                action: "Added".into(),
                path: "b.rs".into(),
                before: String::new(),
                after: "x".into(),
            },
        ];
        assert_eq!(retain_diff_review_index(&changes, Some("b.rs")), 1);
        assert_eq!(retain_diff_review_index(&changes, Some("gone.rs")), 0);
        assert_eq!(retain_diff_review_index(&changes, None), 0);
    }

    #[test]
    fn home_and_end_jump_scroll_extremes() {
        let mut state = DiffReviewState::new(vec![DiffReviewChange {
            action: "Edited".into(),
            path: "a.rs".into(),
            before: "1\n2\n3\n".into(),
            after: "1\n2\n4\n".into(),
        }]);
        state.scroll = 12;
        assert_eq!(
            state.handle_key(&key(KeyCode::Home, KeyModifiers::NONE), 4),
            DiffReviewAction::None
        );
        assert_eq!(state.scroll, 0);
        assert_eq!(
            state.handle_key(&key(KeyCode::End, KeyModifiers::NONE), 4),
            DiffReviewAction::None
        );
        assert_eq!(state.scroll, usize::MAX);
    }

    #[test]
    fn seed_and_close_are_host_effects_for_app_wiring() {
        // App::handle_diff_review_key maps these without needing a live App.
        assert_eq!(
            diff_review_host_effect(DiffReviewAction::Close),
            DiffReviewHostEffect::Close
        );
        let draft = seed_follow_up_draft(&DiffReviewChange {
            action: "Edited".into(),
            path: "wire.rs".into(),
            before: "a".into(),
            after: "b".into(),
        });
        assert!(matches!(
            diff_review_host_effect(DiffReviewAction::SeedComposer(draft.clone())),
            DiffReviewHostEffect::SeedComposer { draft: d } if d == draft
        ));
        assert!(draft.contains("`wire.rs`"));
    }
}
