//! Small formatting + layout helpers shared across the TUI.

use super::*;

/// Pad a (possibly styled) string with spaces to `width` display columns.
pub(crate) fn pad_to(s: &str, width: usize) -> String {
    let vis = a3s_tui::style::visible_len(s);
    if vis >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - vis))
    }
}

/// A user-input message rendered with a subtle background "bubble" so it stands
/// out from agent output in the transcript.
pub(crate) fn user_bubble(content: &str, width: usize) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }

    // Keep the historical shape: 2 columns outside the bubble and at least an
    // 8-column colored body for very narrow terminals.
    let bubble_width = width.saturating_sub(PAD).max(PAD + 8);
    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    chrome
        .gutter(lines.join("\n"))
        .margin(PAD)
        .marker(" ●")
        .gap(" ")
        .width(bubble_width)
        .background_color(SURFACE_SOFT)
        .view()
}

/// Prefix a message block with a colored ● gutter on its first line and align
/// the rest under the text.
pub(crate) fn gutter(color: Color, content: &str) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }

    a3s_tui::components::GutterBlock::lines(lines)
        .margin(PAD)
        .marker_color(color)
        .view()
}

fn input_chrome_width(width: usize) -> u16 {
    width.saturating_sub(PAD).min(u16::MAX as usize) as u16
}

pub(crate) fn input_rule(width: usize, color: Color) -> String {
    if width == 0 {
        return String::new();
    }

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    chrome
        .input_border()
        .margin(PAD)
        .rule_color(color)
        .view(input_chrome_width(width))
}

pub(crate) fn input_gradient_rule(width: usize, palette: &[Color], offset: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    chrome
        .input_border()
        .margin(PAD)
        .rule('━')
        .rainbow(palette.to_vec(), offset)
        .view(input_chrome_width(width))
}

pub(crate) fn input_status_rule(
    width: usize,
    border_color: Color,
    context: &str,
    label: &str,
) -> String {
    if width == 0 {
        return String::new();
    }

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    chrome
        .input_border()
        .margin(PAD)
        .rule_color(border_color)
        .context(context)
        .label(label)
        .view(input_chrome_width(width))
}

pub(crate) fn input_prompt_line(
    prompt: &str,
    color: Color,
    text: &str,
    tint_text: bool,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    let mut line = chrome
        .prompt(format!("{prompt} "))
        .text(text)
        .margin(PAD)
        .width(width)
        .prompt_style(Style::new().fg(color).bold());
    if tint_text {
        line = line.text_style(Style::new().fg(color));
    }
    line.view()
}

pub(crate) fn thinking_block(text: &str, width: usize) -> String {
    let text = text.trim();
    if text.is_empty() || width == 0 {
        return String::new();
    }

    a3s_tui::components::WrappedPrefixBlock::new(text)
        .margin(PAD)
        .width(width)
        .prefixes("💭 ", "   ")
        .style(Style::new().fg(TN_GRAY).italic())
        .view()
}

pub(crate) fn compact_progress_line(elapsed: Duration, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let pct = ((elapsed.as_secs_f64() / 30.0) * 100.0).min(95.0);
    let prefix = Style::new().fg(ACCENT).render(&format!(
        "  ✦ Compacting context… ({}) ",
        fmt_elapsed(elapsed)
    ));
    let progress_width = width
        .saturating_sub(a3s_tui::style::visible_len(&prefix))
        .min(29)
        .max(1);
    let progress = a3s_tui::components::Progress::new()
        .value(pct / 100.0)
        .width(progress_width.min(u16::MAX as usize) as u16)
        .filled_char('▰')
        .empty_char('▱')
        .filled_color(ACCENT)
        .empty_color(TN_GRAY)
        .view();

    a3s_tui::style::fit_visible(&format!("{prefix}{progress}"), width)
}

/// Greedy word-wrap of plain text to `width` display columns, with blank lines
/// dropped so compact previews stay single-spaced.
pub(crate) fn wrap_words(text: &str, width: usize) -> Vec<String> {
    a3s_tui::style::wrap_words_compact(text, width)
}

/// Byte offset of the char at index `char_idx` (for in-place string edits).
pub(crate) fn char_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// A fresh session id for each launch (timestamp + pid; UUID-ish, no dep).
pub(crate) fn new_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:016x}-{:x}", nanos, std::process::id())
}

/// "1h 32m" / "1m 05s" / "42s".
pub(crate) fn fmt_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// "79.9k" / "512".
pub(crate) fn humanize(n: usize) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// Render `text` with a soft highlight gliding left-to-right (loading shimmer).
pub(crate) fn shimmer(text: &str, phase: usize) -> String {
    a3s_tui::components::ShimmerText::new(text)
        .phase(phase)
        .colors(ACCENT, Color::Rgb(211, 229, 255))
        .spread(5.0)
        .speed_divisor(3)
        .cycle_gap(12)
        .view()
}

/// Truncate to `max` DISPLAY COLUMNS (not chars) with an ellipsis. Callers pass
/// a column budget (panel widths), so counting chars overflowed the fixed-height
/// panels on CJK/wide text (every CJK char is 2 columns) and corrupted the
/// layout. Delegates to the width-aware, ANSI-preserving tui helper.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    a3s_tui::style::truncate_visible(s, max)
}

#[cfg(test)]
mod tests {
    use super::{
        compact_progress_line, gutter, input_gradient_rule, input_prompt_line, input_rule,
        input_status_rule, shimmer, thinking_block, truncate, user_bubble, wrap_words,
    };
    use a3s_tui::style::{strip_ansi, visible_len, Color, Style};
    use std::time::Duration;

    #[test]
    fn wraps_on_word_boundaries_without_splitting_words() {
        let lines = wrap_words("the quick brown fox jumps", 9);
        assert!(lines.iter().all(|l| l.chars().count() <= 9), "{lines:?}");
        // No word is broken: rejoining with spaces reproduces the input words.
        assert_eq!(
            lines.join(" ").split_whitespace().collect::<Vec<_>>(),
            vec!["the", "quick", "brown", "fox", "jumps"]
        );
    }

    #[test]
    fn collapses_blank_lines_to_stay_single_spaced() {
        let lines = wrap_words("alpha\n\n\nbeta", 40);
        assert_eq!(lines, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn hard_breaks_a_word_longer_than_width() {
        let lines = wrap_words("supercalifragilistic", 5);
        assert!(lines.iter().all(|l| l.chars().count() <= 5), "{lines:?}");
        assert_eq!(lines.concat(), "supercalifragilistic");
    }

    #[test]
    fn never_returns_empty_for_blank_input() {
        assert_eq!(wrap_words("   ", 10), vec![String::new()]);
    }

    #[test]
    fn wrap_words_counts_display_columns_for_wide_unicode() {
        // 6 wide chars = 12 columns; this text has no spaces so it's one token that
        // must hard-break by COLUMN budget, never exceeding the width.
        let lines = wrap_words("かなテストあ", 8);
        for l in &lines {
            assert!(
                a3s_tui::style::visible_len(l) <= 8,
                "line wider than 8 columns: {l:?}"
            );
        }
        assert_eq!(lines.concat(), "かなテストあ");
    }

    #[test]
    fn wrap_words_uses_shared_compact_width_helper() {
        let text = "alpha\n\n中文测试内容 beta";

        assert_eq!(
            wrap_words(text, 8),
            a3s_tui::style::wrap_words_compact(text, 8)
        );
    }

    #[test]
    fn truncate_budgets_display_columns_not_chars() {
        // 5 wide chars = 10 columns; a 6-column budget must fit (<= 6 cols incl ...),
        // which char-counting would have overflowed to ~10 columns.
        let out = truncate("アイウエオ", 6);
        assert!(
            a3s_tui::style::visible_len(&out) <= 6,
            "truncated string exceeds 6 columns: {out:?}"
        );
        assert!(out.ends_with('…'));
        // Fits-as-is when within budget.
        assert_eq!(truncate("ok", 6), "ok");
    }

    #[test]
    fn gutter_uses_shared_block_shape() {
        let rendered = gutter(Color::Green, "hello\nworld");
        let plain = strip_ansi(&rendered);

        assert_eq!(plain, "  ● hello\n    world");
        assert!(rendered.contains("\x1b[1;32m●\x1b[0m"));
    }

    #[test]
    fn gutter_preserves_styled_content() {
        let styled = Style::new().fg(Color::Yellow).render("styled");
        let rendered = gutter(Color::Green, &styled);

        assert!(rendered.contains("\x1b[33mstyled\x1b[0m"));
        assert_eq!(strip_ansi(&rendered), "  ● styled");
    }

    #[test]
    fn gutter_keeps_empty_input_empty() {
        assert_eq!(gutter(Color::Green, ""), "");
    }

    #[test]
    fn user_bubble_uses_shared_gutter_block_shape() {
        let rendered = user_bubble("hello\nworld", 20);
        let plain = strip_ansi(&rendered);
        let rows = plain.lines().collect::<Vec<_>>();

        assert_eq!(rows, vec!["   ● hello        ", "     world        "]);
        assert!(rendered.contains("\x1b[38;2;237;237;237;48;2;31;31;31m"));
        assert!(rendered.lines().all(|row| visible_len(row) == 18));
    }

    #[test]
    fn user_bubble_keeps_empty_input_empty() {
        assert_eq!(user_bubble("", 20), "");
    }

    #[test]
    fn user_bubble_keeps_narrow_body_min_width() {
        let rendered = user_bubble("hi", 6);

        assert_eq!(strip_ansi(&rendered), "   ● hi   ");
        assert_eq!(visible_len(&rendered), 10);
    }

    #[test]
    fn input_prompt_line_uses_shared_prompt_component() {
        let rendered = input_prompt_line("❯", Color::Cyan, "cargo test\n--all", false, 24);
        let plain = strip_ansi(&rendered);
        let rows = plain.lines().collect::<Vec<_>>();

        assert!(rows[0].starts_with("  ❯ cargo test"));
        assert!(rows[1].starts_with("    --all"));
        assert!(rendered.lines().all(|line| visible_len(line) == 24));
        assert!(rendered.contains("\x1b[1;36m❯ \x1b[0m"));
    }

    #[test]
    fn input_prompt_line_can_tint_modal_input_text() {
        let rendered = input_prompt_line("?", Color::Cyan, "research mode", true, 28);

        assert_eq!(strip_ansi(&rendered).trim_end(), "  ? research mode");
        assert!(rendered.contains("\x1b[1;36m? \x1b[0m"));
        assert!(rendered.contains("\x1b[36mresearch mode\x1b[0m"));
    }

    #[test]
    fn input_rules_use_shared_border_component_widths() {
        let plain = strip_ansi(&input_rule(20, Color::BrightBlack));
        assert_eq!(visible_len(&plain), 18);
        assert!(plain.starts_with("  ─"));

        let status = input_status_rule(48, Color::BrightBlack, "70% context used  ", "◇ high");
        let status_plain = strip_ansi(&status);
        assert_eq!(visible_len(&status), 46);
        assert!(status_plain.contains("70% context used"));
        assert!(status_plain.contains("◇ high"));
        assert!(status.contains("\x1b[1;38;2;0;112;243m◇ high\x1b[0m"));
    }

    #[test]
    fn input_gradient_rule_preserves_brand_ribbon_width() {
        let rendered = input_gradient_rule(
            20,
            &[
                Color::Rgb(0, 124, 240),
                Color::Rgb(0, 223, 216),
                Color::Rgb(255, 0, 128),
            ],
            1,
        );
        let plain = strip_ansi(&rendered);

        assert_eq!(visible_len(&rendered), 18);
        assert!(plain.starts_with("  ━"));
        assert!(rendered.contains("\x1b[1;38;2;"));
    }

    #[test]
    fn thinking_block_uses_shared_wrapped_prefix_block() {
        let rendered = thinking_block("alpha beta gamma delta", 16);
        let plain = strip_ansi(&rendered);
        let rows = plain.lines().collect::<Vec<_>>();

        assert!(rows[0].starts_with("  💭 alpha"));
        assert!(rows.iter().skip(1).all(|row| row.starts_with("     ")));
        assert!(rendered.lines().all(|line| visible_len(line) == 16));
        assert!(rendered.contains("\x1b[3;38;2;143;143;143m💭 alpha"));
    }

    #[test]
    fn thinking_block_keeps_empty_input_empty() {
        assert_eq!(thinking_block("   ", 40), "");
        assert_eq!(thinking_block("thinking", 0), "");
    }

    #[test]
    fn thinking_block_wraps_wide_unicode_by_display_width() {
        let rendered = thinking_block("中文测试内容", 13);
        let plain = strip_ansi(&rendered);
        let rows = plain.lines().collect::<Vec<_>>();

        assert!(rows[0].starts_with("  💭 中文测试"));
        assert!(rows[1].starts_with("     内容"));
        assert!(rendered.lines().all(|line| visible_len(line) == 13));
    }

    #[test]
    fn compact_progress_line_uses_shared_progress_bar() {
        let rendered = compact_progress_line(Duration::from_secs(15), 80);
        let plain = strip_ansi(&rendered);

        assert!(plain.contains("Compacting context"), "{plain}");
        assert!(plain.contains("50%"), "{plain}");
        assert!(plain.contains('▰'), "{plain}");
        assert!(plain.contains('▱'), "{plain}");
        assert!(rendered.contains("\x1b[38;2;0;112;243m"));
        assert!(visible_len(&rendered) <= 80);
    }

    #[test]
    fn compact_progress_line_caps_estimated_progress() {
        let rendered = compact_progress_line(Duration::from_secs(60), 80);

        assert!(strip_ansi(&rendered).contains("95%"));
    }

    #[test]
    fn compact_progress_line_stays_bounded_on_narrow_widths() {
        let rendered = compact_progress_line(Duration::from_secs(8), 18);

        assert_eq!(visible_len(&rendered), 18);
    }

    #[test]
    fn shimmer_uses_shared_component_settings() {
        let rendered = shimmer("Working…", 0);
        let expected = a3s_tui::components::ShimmerText::new("Working…")
            .phase(0)
            .colors(Color::Rgb(0, 112, 243), Color::Rgb(211, 229, 255))
            .spread(5.0)
            .speed_divisor(3)
            .cycle_gap(12)
            .view();

        assert_eq!(rendered, expected);
        assert_eq!(strip_ansi(&rendered), "Working…");
        assert!(rendered.contains("\x1b[1;38;2;211;229;255mW\x1b[0m"));
    }

    #[test]
    fn shimmer_preserves_complex_glyph_display_width() {
        let rendered = shimmer("工作e\u{301}", 0);

        assert_eq!(strip_ansi(&rendered), "工作e\u{301}");
        assert_eq!(visible_len(&rendered), 5);
    }
}
