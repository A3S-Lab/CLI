//! Large text pastes collapse into composer pills (Cursor CLI grammar).
//!
//! The agent still receives the full pasted body on submit; the PromptBar only
//! shows a compact chip so six-row auto-grow is not blown out by a dump.

use std::ops::Range;

use super::*;

/// Paste collapses when either threshold is met (Cursor-like large-paste pill).
pub(super) const LARGE_PASTE_LINE_THRESHOLD: usize = 15;
pub(super) const LARGE_PASTE_CHAR_THRESHOLD: usize = 400;

/// One large paste staged above the PromptBar until submit / remove / expand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingPaste {
    text: String,
    lines: usize,
    chars: usize,
}

impl PendingPaste {
    pub(super) fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let lines = text.lines().count().max(1);
        let chars = text.chars().count();
        Self { text, lines, chars }
    }

    pub(super) fn text(&self) -> &str {
        &self.text
    }

    pub(super) fn lines(&self) -> usize {
        self.lines
    }

    pub(super) fn chars(&self) -> usize {
        self.chars
    }
}

pub(super) fn is_large_paste(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    text.lines().count() >= LARGE_PASTE_LINE_THRESHOLD
        || text.chars().count() >= LARGE_PASTE_CHAR_THRESHOLD
}

pub(super) fn paste_reference_line(pastes: &[PendingPaste]) -> String {
    if pastes.is_empty() {
        return String::new();
    }
    pastes
        .iter()
        .enumerate()
        .map(|(index, paste)| {
            format!(
                "[Paste #{} · {} line{} · {} chars]",
                index + 1,
                paste.lines(),
                if paste.lines() == 1 { "" } else { "s" },
                paste.chars()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn merge_paste_bodies(pastes: &[PendingPaste], typed: &str) -> String {
    let body = pastes
        .iter()
        .map(PendingPaste::text)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    match (body.is_empty(), typed.trim().is_empty()) {
        (true, _) => typed.to_string(),
        (false, true) => body,
        (false, false) => format!("{body}\n\n{typed}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PasteAction {
    /// Expand the pill back into the textarea (full text).
    Expand(usize),
    Remove(usize),
}

#[derive(Debug)]
struct PasteHitRegion {
    row: usize,
    expand: Range<usize>,
    remove: Range<usize>,
    index: usize,
}

#[derive(Debug, Default)]
pub(super) struct PasteStrip {
    pub(super) rows: Vec<String>,
    hits: Vec<PasteHitRegion>,
}

impl PasteStrip {
    pub(super) fn hit_test(&self, row: usize, column: usize) -> Option<PasteAction> {
        self.hits.iter().find_map(|hit| {
            if hit.row != row {
                return None;
            }
            if hit.remove.contains(&column) {
                Some(PasteAction::Remove(hit.index))
            } else if hit.expand.contains(&column) {
                Some(PasteAction::Expand(hit.index))
            } else {
                None
            }
        })
    }
}

struct RenderedPasteChip {
    view: String,
    width: usize,
    remove_start: usize,
}

fn render_paste_chip(index: usize, paste: &PendingPaste, width: usize) -> RenderedPasteChip {
    let label = format!(
        " Paste #{} · {}L · {}c ",
        index + 1,
        paste.lines(),
        paste.chars()
    );
    let close = "× ";
    let close_width = a3s_tui::style::visible_len(close).min(width);
    let content_budget = width.saturating_sub(close_width);
    let label = a3s_tui::style::fit_visible(&label, content_budget);
    let content_width = a3s_tui::style::visible_len(&label);
    let close = a3s_tui::style::fit_visible(close, close_width);
    let close_width = a3s_tui::style::visible_len(&close);
    let label_view = Style::new()
        .fg(TN_CYAN)
        .bg(SURFACE_COMPOSER)
        .bold()
        .render(&label);
    let close_view = Style::new()
        .fg(TN_GRAY)
        .bg(SURFACE_COMPOSER)
        .bold()
        .render(&close);
    RenderedPasteChip {
        view: format!("{label_view}{close_view}"),
        width: content_width.saturating_add(close_width),
        remove_start: content_width,
    }
}

pub(super) fn paste_strip(pastes: &[PendingPaste], width: usize) -> PasteStrip {
    if pastes.is_empty() || width == 0 {
        return PasteStrip::default();
    }
    let mut strip = PasteStrip::default();
    let mut row = String::new();
    let mut row_width = 0usize;
    let mut row_index = 0usize;
    for (index, paste) in pastes.iter().enumerate() {
        let chip = render_paste_chip(index, paste, width);
        let gap = usize::from(row_width > 0);
        if row_width > 0 && row_width.saturating_add(gap).saturating_add(chip.width) > width {
            strip.rows.push(row);
            row = String::new();
            row_width = 0;
            row_index += 1;
        }
        if row_width > 0 {
            row.push(' ');
            row_width += 1;
        }
        let start = row_width;
        row.push_str(&chip.view);
        row_width = row_width.saturating_add(chip.width);
        strip.hits.push(PasteHitRegion {
            row: row_index,
            expand: start..start.saturating_add(chip.remove_start),
            remove: start.saturating_add(chip.remove_start)..start.saturating_add(chip.width),
            index,
        });
    }
    if !row.is_empty() {
        strip.rows.push(row);
    }
    strip
}

impl App {
    pub(super) fn stage_large_paste(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        self.pending_pastes.push(PendingPaste::new(text));
        self.relayout();
    }

    pub(super) fn expand_pending_paste(&mut self, index: usize) {
        if index >= self.pending_pastes.len() {
            return;
        }
        let paste = self.pending_pastes.remove(index);
        if !self.textarea.value().is_empty() && !self.textarea.value().ends_with('\n') {
            self.textarea.insert_str("\n");
        }
        self.textarea.insert_str(paste.text());
        self.relayout();
    }

    pub(crate) fn composer_paste_rows(&self) -> usize {
        paste_strip(&self.pending_pastes, self.viewport_content_width())
            .rows
            .len()
    }

    pub(crate) fn composer_staged_rows(&self) -> usize {
        self.composer_paste_rows()
            .saturating_add(self.composer_attachment_rows())
    }

    /// Top terminal row of the PromptBar chrome (half-block cap).
    fn composer_chrome_top_row(&self) -> usize {
        self.bottom_pane_projection().input_cursor_row(
            self.height,
            composer_chrome_height(self.input_height()),
            0,
        ) as usize
    }

    /// Rows stacked above the PromptBar: pastes, images, working, follow-ups.
    pub(super) fn composer_stack_geometry(&self) -> ComposerStackGeometry {
        let chrome_top = self.composer_chrome_top_row();
        let follow_rows = usize::from(follow_up_strip_row_count(self.queue.len()));
        let working_rows = usize::from(self.ephemeral_working_label().is_some());
        let image_rows = self.composer_attachment_rows();
        let paste_rows = self.composer_paste_rows();
        let follow_end = chrome_top;
        let follow_start = follow_end.saturating_sub(follow_rows);
        let working_end = follow_start;
        let working_start = working_end.saturating_sub(working_rows);
        let image_end = working_start;
        let image_start = image_end.saturating_sub(image_rows);
        let paste_end = image_start;
        let paste_start = paste_end.saturating_sub(paste_rows);
        ComposerStackGeometry {
            paste_start,
            paste_end,
            image_start,
            image_end,
        }
    }

    pub(super) fn paste_action_at(
        &self,
        terminal_row: u16,
        terminal_column: u16,
    ) -> Option<PasteAction> {
        let strip = paste_strip(&self.pending_pastes, self.viewport_content_width());
        if strip.rows.is_empty() {
            return None;
        }
        let geo = self.composer_stack_geometry();
        let row = terminal_row as usize;
        if row < geo.paste_start || row >= geo.paste_end {
            return None;
        }
        strip.hit_test(row - geo.paste_start, terminal_column as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ComposerStackGeometry {
    pub(super) paste_start: usize,
    pub(super) paste_end: usize,
    pub(super) image_start: usize,
    pub(super) image_end: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_paste_thresholds_cover_lines_or_chars() {
        assert!(!is_large_paste("short"));
        assert!(!is_large_paste(&"x\n".repeat(14)));
        assert!(is_large_paste(&"x\n".repeat(15)));
        assert!(is_large_paste(&"a".repeat(LARGE_PASTE_CHAR_THRESHOLD)));
        assert!(!is_large_paste(""));
    }

    #[test]
    fn merge_paste_bodies_prefers_pastes_then_typed() {
        let pastes = vec![PendingPaste::new("one\ntwo"), PendingPaste::new("three")];
        assert_eq!(merge_paste_bodies(&pastes, ""), "one\ntwo\n\nthree");
        assert_eq!(
            merge_paste_bodies(&pastes, "note"),
            "one\ntwo\n\nthree\n\nnote"
        );
        assert_eq!(merge_paste_bodies(&[], "only"), "only");
    }

    #[test]
    fn paste_reference_line_numbers_chips() {
        let pastes = vec![PendingPaste::new("a\nb\nc")];
        let line = paste_reference_line(&pastes);
        assert!(line.contains("[Paste #1"), "{line}");
        assert!(line.contains("3 lines"), "{line}");
    }

    #[test]
    fn paste_strip_renders_compact_chips() {
        let pastes = vec![PendingPaste::new(&"line\n".repeat(20))];
        let strip = paste_strip(&pastes, 80);
        assert_eq!(strip.rows.len(), 1);
        let plain = a3s_tui::style::strip_ansi(&strip.rows[0]);
        assert!(plain.contains("Paste #1"), "{plain}");
        assert!(plain.contains('×'), "{plain}");
    }

    #[test]
    fn paste_strip_hit_test_distinguishes_expand_and_remove() {
        let pastes = vec![PendingPaste::new(&"line\n".repeat(20))];
        let strip = paste_strip(&pastes, 80);
        let hit = strip.hits.first().expect("chip hit region");
        assert_eq!(
            strip.hit_test(0, hit.expand.start),
            Some(PasteAction::Expand(0))
        );
        assert_eq!(
            strip.hit_test(0, hit.remove.start),
            Some(PasteAction::Remove(0))
        );
    }

    #[test]
    fn composer_grammar_thresholds_keep_small_pastes_inline() {
        // Integrated grammar: small pastes stay below the pill thresholds used
        // by the PromptBar paste path; large dumps cross either gate.
        let small = "hello\nworld";
        assert!(!is_large_paste(small));
        let many_lines = "x\n".repeat(LARGE_PASTE_LINE_THRESHOLD);
        assert!(is_large_paste(&many_lines));
        let merged = merge_paste_bodies(&[PendingPaste::new(many_lines.clone())], "note");
        assert!(merged.ends_with("\n\nnote"));
        assert!(merged.contains("x\n"));
    }
}
