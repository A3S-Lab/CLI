//! Ephemeral chrome above the composer.
//!
//! - Working line: rotating/variant status copy while a turn is live
//! - Follow-ups strip: compact queued turns with edit affordances

use super::*;

const FOLLOW_UP_MAX_ROWS: usize = 3;

/// One queued follow-up row for the composer-adjacent strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FollowUpRow {
    pub(crate) sequence: u64,
    pub(crate) preview: String,
    pub(crate) mode_glyph: String,
    pub(crate) selected: bool,
}

/// Working meter above the PromptBar (not the quiet footer).
pub(crate) fn render_working_line(label: &str, phase: usize, width: usize) -> String {
    if width == 0 || label.trim().is_empty() {
        return String::new();
    }
    let margin = " ".repeat(PAD.min(width));
    let budget = width.saturating_sub(PAD).max(1);
    let glyph = ['✶', '✸', '✹', '✺'][phase % 4];
    let body = format!("{glyph} {label}");
    let clipped = truncate(&body, budget);
    let painted = shimmer(&clipped, phase);
    let line = format!("{margin}{painted}");
    truncate_visible_line(&line, width)
}

/// Compact follow-up queue above the PromptBar (queued-message banner).
pub(crate) fn render_follow_up_strip(rows: &[FollowUpRow], width: usize) -> String {
    if width == 0 || rows.is_empty() {
        return String::new();
    }
    let margin = " ".repeat(PAD.min(width));
    let budget = width.saturating_sub(PAD).max(1);
    let visible = rows.iter().take(FOLLOW_UP_MAX_ROWS);
    let mut lines = Vec::new();
    for row in visible {
        let marker = if row.selected { "›" } else { "○" };
        let preview = row.preview.trim();
        let body = if row.mode_glyph.is_empty() {
            format!("{marker} {preview}")
        } else {
            format!("{marker} {} {preview}", row.mode_glyph)
        };
        let clipped = truncate(&body, budget);
        let color = if row.selected {
            COMPOSER_CHROME.active
        } else {
            COMPOSER_CHROME.secondary
        };
        let painted = Style::new().fg(color).render(&clipped);
        lines.push(truncate_visible_line(&format!("{margin}{painted}"), width));
    }
    let omitted = rows.len().saturating_sub(FOLLOW_UP_MAX_ROWS);
    if omitted > 0 {
        let more = format!("  +{omitted} more · /queue");
        lines.push(truncate_visible_line(
            &format!(
                "{margin}{}",
                Style::new()
                    .fg(COMPOSER_CHROME.faint)
                    .render(&truncate(&more, budget))
            ),
            width,
        ));
    } else {
        let hint = "  ↑ edit · enter send · d remove · /queue";
        lines.push(truncate_visible_line(
            &format!(
                "{margin}{}",
                Style::new()
                    .fg(COMPOSER_CHROME.faint)
                    .render(&truncate(hint, budget))
            ),
            width,
        ));
    }
    lines.join("\n")
}

pub(crate) fn follow_up_strip_row_count(queue_len: usize) -> u16 {
    if queue_len == 0 {
        return 0;
    }
    let visible = queue_len.min(FOLLOW_UP_MAX_ROWS);
    // visible rows + hint/more row
    (visible.saturating_add(1)).min(u16::MAX as usize) as u16
}

fn truncate_visible_line(line: &str, width: usize) -> String {
    a3s_tui::style::fit_visible(line, width)
}

/// Build a working label from thinking / live tools / core status.
pub(crate) fn compose_working_label(
    thinking: &str,
    tool_verb: Option<&str>,
    tool_detail: Option<&str>,
    core_label: &str,
    elapsed: Option<Duration>,
) -> String {
    let mut label = if !thinking.trim().is_empty() {
        "Thinking…".to_string()
    } else if let Some(verb) = tool_verb.filter(|v| !v.is_empty()) {
        match tool_detail.filter(|d| !d.is_empty()) {
            Some(detail) => format!("{verb} {detail}…"),
            None => format!("{verb}…"),
        }
    } else {
        let core = core_label.trim();
        if core.is_empty() {
            "Working…".to_string()
        } else {
            core.to_string()
        }
    };
    if let Some(elapsed) = elapsed {
        label.push_str(&format!(" · {}", fmt_elapsed(elapsed)));
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_line_uses_variant_copy_and_fits_width() {
        let line = render_working_line("Searching memory…", 3, 40);
        let plain = a3s_tui::style::strip_ansi(&line);
        assert!(plain.contains("Searching memory…"), "{plain}");
        assert!(a3s_tui::style::visible_len(&line) <= 40);
    }

    #[test]
    fn compose_working_label_prefers_thinking_then_tool_then_core() {
        assert_eq!(
            compose_working_label("hmm", Some("Reading"), Some("main.rs"), "Working…", None),
            "Thinking…"
        );
        assert_eq!(
            compose_working_label("", Some("Reading"), Some("main.rs"), "Working…", None),
            "Reading main.rs…"
        );
        assert_eq!(
            compose_working_label("", None, None, "Planning…", None),
            "Planning…"
        );
    }

    #[test]
    fn follow_up_strip_shows_markers_and_hint() {
        let rendered = render_follow_up_strip(
            &[
                FollowUpRow {
                    sequence: 1,
                    preview: "first".into(),
                    mode_glyph: "❯".into(),
                    selected: true,
                },
                FollowUpRow {
                    sequence: 2,
                    preview: "second".into(),
                    mode_glyph: "❯".into(),
                    selected: false,
                },
            ],
            48,
        );
        let plain = a3s_tui::style::strip_ansi(&rendered);
        assert!(plain.contains('›'), "{plain}");
        assert!(plain.contains('○'), "{plain}");
        assert!(plain.contains("↑ edit"), "{plain}");
        assert_eq!(follow_up_strip_row_count(2), 3);
        assert_eq!(follow_up_strip_row_count(0), 0);
    }
}
