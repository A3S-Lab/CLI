//! Progressive-disclosure rendering for file changes.
//!
//! Compact history gets a width-aware preview that cannot consume an entire
//! terminal. Ctrl+T uses the same visual grammar without the compact row cap.

use a3s_tui::style::{fit_visible, strip_ansi, Color, Style};

use super::{
    agent_chrome, agent_chrome_theme, highlight_diff_spans, lang_of, DiffLineKind, DiffSpan,
    ACCENT, TN_FG, TN_GRAY, TN_GREEN, TN_SUBTLE,
};

/// DiffView tokens.
///
/// Line backgrounds are the opaque blend of GitHub Dark
/// `diffEditor.*LineBackground` alphas over [`CANVAS`] (`#0d1117`):
/// `#23863626` → insert, `#da363326` → delete.
pub(super) const DIFF_HEADER_BULLET: Color = TN_SUBTLE;
pub(super) const DIFF_HEADER_ACTION: Color = TN_FG;
pub(super) const DIFF_HEADER_DETAIL: Color = TN_GRAY;
pub(super) const DIFF_CONTEXT_GUTTER: Color = TN_SUBTLE;
pub(super) const DIFF_INSERT_MARKER: Color = TN_GREEN;
pub(super) const DIFF_DELETE_MARKER: Color = Color::Rgb(255, 123, 114); // #ff7b72
pub(super) const DIFF_INSERT_BG: Color = Color::Rgb(16, 34, 28);
pub(super) const DIFF_DELETE_BG: Color = Color::Rgb(44, 23, 27);
const DIFF_CODE_FG: Color = TN_FG;

pub(super) fn compact_diff_row_budget(width: usize) -> usize {
    match width {
        // brief history: a few hunk lines, then Ctrl+T.
        0..=39 => 4,
        40..=79 => 6,
        _ => 8,
    }
}

/// Keep compact DiffView short. Large edits stay a peek; Ctrl+T owns the full
/// hunk (progressive disclosure).
pub(super) fn compact_diff_row_budget_for_change(width: usize, before: &str, after: &str) -> usize {
    let base = compact_diff_row_budget(width);
    let changed = estimate_changed_lines(before, after);
    if changed <= base {
        base
    } else if changed <= base.saturating_add(8) {
        base.saturating_add(2).min(10)
    } else {
        base
    }
}

fn estimate_changed_lines(before: &str, after: &str) -> usize {
    let before_lines = before.lines().count();
    let after_lines = after.lines().count();
    before_lines.abs_diff(after_lines).max(
        similar::TextDiff::from_lines(before, after)
            .iter_all_changes()
            .filter(|change| !matches!(change.tag(), similar::ChangeTag::Equal))
            .count(),
    )
}

pub(super) fn render_compact_file_change(
    action: &str,
    path: &str,
    before: &str,
    after: &str,
    width: usize,
) -> String {
    let max_rows = compact_diff_row_budget_for_change(width, before, after);
    let rendered = render_file_change(action, path, before, after, width, max_rows);
    rewrite_compact_truncation_notice(&rendered, width)
}

pub(crate) fn render_full_file_change(
    action: &str,
    path: &str,
    before: &str,
    after: &str,
    width: usize,
) -> String {
    render_file_change(action, path, before, after, width, u16::MAX as usize)
}

fn render_file_change(
    action: &str,
    path: &str,
    before: &str,
    after: &str,
    width: usize,
    max_rows: usize,
) -> String {
    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    let lang = lang_of(std::path::Path::new(path));
    let rendered = chrome
        .diff_texts(path, before, after)
        .action(action)
        .header_colors(DIFF_HEADER_BULLET, DIFF_HEADER_ACTION, DIFF_HEADER_DETAIL)
        .context_color(DIFF_CODE_FG)
        .separator_color(DIFF_CONTEXT_GUTTER)
        // Line-number gutters stay muted. Header +/- counts keep marker colors;
        // per-line +/- glyphs use the gutter color in DiffView::style_segment.
        .gutter_colors(
            DIFF_CONTEXT_GUTTER,
            DIFF_CONTEXT_GUTTER,
            DIFF_CONTEXT_GUTTER,
        )
        .marker_colors(DIFF_INSERT_MARKER, DIFF_DELETE_MARKER)
        .changed_content_colors(DIFF_CODE_FG, mix_diff_color(DIFF_CODE_FG, DIFF_DELETE_BG))
        .changed_backgrounds(Some(DIFF_INSERT_BG), Some(DIFF_DELETE_BG))
        .highlight_content(|kind, content| {
            highlight_diff_spans(content, lang)
                .into_iter()
                .map(|span| {
                    let color = span.color.unwrap_or(DIFF_CODE_FG);
                    let color = if kind == DiffLineKind::Delete {
                        mix_diff_color(color, DIFF_DELETE_BG)
                    } else {
                        color
                    };
                    DiffSpan::new(span.content).color(color)
                })
                .collect()
        })
        .emphasize_inline_changes()
        .max_lines(max_rows)
        .view(
            width.min(u16::MAX as usize) as u16,
            max_rows.saturating_add(2),
        );
    rendered
}

fn rewrite_compact_truncation_notice(rendered: &str, width: usize) -> String {
    rendered
        .lines()
        .map(|line| {
            if strip_ansi(line).contains("diff truncated") {
                // Expand grammar: "… · ctrl+o to expand". Ctrl+O is Send now
                // here, so expand stays on Ctrl+T with the same phrasing.
                let plain = fit_visible("    … ctrl+t to expand", width);
                let shortcut = "ctrl+t";
                if let Some(at) = plain.find(shortcut) {
                    let before = &plain[..at];
                    let after = &plain[at + shortcut.len()..];
                    format!(
                        "{}{}{}",
                        Style::new().fg(DIFF_CONTEXT_GUTTER).render(before),
                        Style::new().fg(ACCENT).render(shortcut),
                        Style::new().fg(DIFF_CONTEXT_GUTTER).render(after),
                    )
                } else {
                    Style::new().fg(DIFF_CONTEXT_GUTTER).render(&plain)
                }
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn mix_diff_color(foreground: Color, background: Color) -> Color {
    match (foreground, background) {
        (Color::Rgb(fr, fg, fb), Color::Rgb(br, bg, bb)) => Color::Rgb(
            ((u16::from(fr) + u16::from(br)) / 2) as u8,
            ((u16::from(fg) + u16::from(bg)) / 2) as u8,
            ((u16::from(fb) + u16::from(bb)) / 2) as u8,
        ),
        (color, _) => color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_tui::style::{strip_ansi, visible_len};

    #[test]
    fn compact_budget_grows_with_available_horizontal_space() {
        assert_eq!(compact_diff_row_budget(24), 4);
        assert_eq!(compact_diff_row_budget(48), 6);
        assert_eq!(compact_diff_row_budget(80), 8);
    }

    #[test]
    fn compact_budget_stays_brief_for_larger_edits() {
        let small_before = "a\nb\n";
        let small_after = "a\nc\n";
        assert_eq!(
            compact_diff_row_budget_for_change(80, small_before, small_after),
            8
        );

        let large_before = (0..80)
            .map(|i| format!("old-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let large_after = (0..80)
            .map(|i| format!("new-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let budget = compact_diff_row_budget_for_change(80, &large_before, &large_after);
        assert!(budget <= 10, "expected brief cap, got {budget}");
        assert!(budget >= 8, "expected at least base budget, got {budget}");
    }

    #[test]
    fn compact_diff_points_to_the_complete_transcript() {
        let before = (0..80)
            .map(|index| format!("old-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let after = (0..80)
            .map(|index| format!("new-{index}"))
            .collect::<Vec<_>>()
            .join("\n");

        for width in [24, 48, 80] {
            let rendered =
                render_compact_file_change("Edited", "src/large.rs", &before, &after, width);
            let plain = strip_ansi(&rendered);

            assert!(plain.contains("ctrl+t to expand"), "{plain}");
            assert!(!plain.contains("diff truncated"), "{plain}");
            assert!(!plain.contains("old-79"), "{plain}");
            assert!(
                rendered.lines().count()
                    <= compact_diff_row_budget_for_change(width, &before, &after) + 2,
                "{plain}"
            );
            assert!(rendered.lines().all(|line| visible_len(line) <= width));
            assert!(
                rendered.contains(&ACCENT.fg_ansi())
                    || rendered.contains(&DIFF_CONTEXT_GUTTER.fg_ansi()),
                "expand hint should be styled: {rendered:?}"
            );
        }
    }

    #[test]
    fn full_diff_preserves_the_tail_without_a_compact_hint() {
        let before = (0..40)
            .map(|index| format!("old-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let after = (0..40)
            .map(|index| format!("new-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = render_full_file_change("Edited", "src/large.rs", &before, &after, 80);
        let plain = strip_ansi(&rendered);

        assert!(plain.contains("old-39"), "{plain}");
        assert!(plain.contains("new-39"), "{plain}");
        assert!(!plain.contains("ctrl+t to expand"), "{plain}");
        assert!(!plain.contains("diff · Ctrl+T"), "{plain}");
    }

    #[test]
    fn line_numbers_use_uniform_gutter_while_markers_stay_tinted() {
        let rendered = render_full_file_change(
            "Edited",
            "src/demo.rs",
            "fn old() {}\n",
            "fn new() {}\n",
            80,
        );
        let gutter = DIFF_CONTEXT_GUTTER;
        let insert_num = Style::new().fg(gutter).render("  1");
        let delete_num = Style::new().fg(gutter).render("  1");
        let insert_mark = Style::new().fg(DIFF_CONTEXT_GUTTER).render("+");
        let delete_mark = Style::new().fg(DIFF_CONTEXT_GUTTER).render("-");

        assert!(
            rendered.contains(&insert_num),
            "insert line number should stay muted gray: {rendered:?}"
        );
        assert!(
            rendered.contains(&delete_num),
            "delete line number should stay muted gray: {rendered:?}"
        );
        assert!(
            rendered.contains(&insert_mark),
            "insert marker should stay muted gray: {rendered:?}"
        );
        assert!(
            rendered.contains(&delete_mark),
            "delete marker should stay muted gray: {rendered:?}"
        );
        assert!(
            !rendered.contains(
                &Style::new()
                    .fg(DIFF_INSERT_MARKER)
                    .bg(DIFF_INSERT_BG)
                    .render("+")
            ),
            "insert marker must not use green: {rendered:?}"
        );
        assert!(
            !rendered.contains(
                &Style::new()
                    .fg(DIFF_DELETE_MARKER)
                    .bg(DIFF_DELETE_BG)
                    .render("-")
            ),
            "delete marker must not use red: {rendered:?}"
        );
        assert!(
            !rendered.contains(
                &Style::new()
                    .fg(DIFF_INSERT_MARKER)
                    .bg(DIFF_INSERT_BG)
                    .render("  1")
            ),
            "insert line number must not use marker green: {rendered:?}"
        );
        assert!(
            !rendered.contains(
                &Style::new()
                    .fg(DIFF_DELETE_MARKER)
                    .bg(DIFF_DELETE_BG)
                    .render("  1")
            ),
            "delete line number must not use marker red: {rendered:?}"
        );
        assert!(
            !rendered.contains(&Style::new().fg(gutter).bg(DIFF_INSERT_BG).render("  1")),
            "insert line number must not sit on green row background: {rendered:?}"
        );
        assert!(
            !rendered.contains(&Style::new().fg(gutter).bg(DIFF_DELETE_BG).render("  1")),
            "delete line number must not sit on red row background: {rendered:?}"
        );
    }
}
