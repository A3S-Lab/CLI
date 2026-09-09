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

/// Paint unpainted cells with [`CANVAS`] so the session void is pure black.
///
/// `Layout::vertical` Fill pads with empty strings, and `Terminal::draw` clears
/// to the host terminal default background. Empty / short / plain-space rows
/// therefore show the theme charcoal instead of `#000000` unless we fill them.
///
/// Intentionally styled blank rows (user-bubble vertical padding, composer
/// surfaces) keep their background — only unstyled void is rewritten.
pub(crate) fn paint_canvas_rows(view: &str, width: usize) -> String {
    if width == 0 {
        return view.to_string();
    }
    let canvas_space = |n: usize| Style::new().bg(CANVAS).render(&" ".repeat(n));
    view.lines()
        .map(|line| {
            let vis = a3s_tui::style::visible_len(line);
            if vis == 0 {
                return canvas_space(width);
            }
            let plain = a3s_tui::style::strip_ansi(line);
            // Full-width unstyled spaces (common after banner dismiss / pad)
            // still carry the host theme bg — treat them as empty canvas.
            // Styled blank rows (e.g. user bubble pad with SURFACE_USER) must
            // keep their painted height.
            let blank_plain = plain.chars().all(|c| c == ' ');
            let unstyled = line == plain;
            if blank_plain && unstyled {
                return canvas_space(width);
            }
            if vis >= width {
                return line.to_string();
            }
            format!("{line}{}", canvas_space(width - vis))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render a user-authored turn with the same cell geometry as Codex CLI.
///
/// The user surface owns one blank row above and below its content, uses `› `
/// only on the first content row, aligns continuations under the text, and
/// reserves one right-hand wrapping column. The low-contrast surface already
/// matches Codex's 12% white blend over the dark terminal canvas.
pub(crate) fn user_bubble(content: &str, width: usize) -> String {
    if content.is_empty() || width == 0 {
        return String::new();
    }

    let margin = PAD;
    // Keep at least one display column for message content after the marker and
    // its gap. On smaller viewports the surface remains, but the rail yields.
    let marker = if width.saturating_sub(margin) >= 3 {
        "›"
    } else {
        ""
    };
    let gap = usize::from(!marker.is_empty());
    let prefix_width = a3s_tui::style::visible_len(marker) + gap;
    let right_margin = usize::from(width.saturating_sub(prefix_width) > 1);
    let body_width = width
        .saturating_sub(margin)
        .saturating_sub(prefix_width)
        .saturating_sub(right_margin)
        .max(1);
    let lines = content
        .split('\n')
        .flat_map(|line| wrap_user_line(line, body_width))
        .collect::<Vec<_>>();
    let padding = Style::new()
        .bg(SURFACE_USER)
        .render(&" ".repeat(width.saturating_sub(margin)));
    let mut rows = Vec::with_capacity(lines.len().saturating_add(2));
    rows.push(format!("{}{padding}", " ".repeat(margin)));
    for (index, line) in lines.into_iter().enumerate() {
        let prefix = if index == 0 && !marker.is_empty() {
            format!(
                "{}{}",
                Style::new()
                    .fg(TN_FG)
                    .bg(SURFACE_USER)
                    .bold()
                    .dim()
                    .render(marker),
                Style::new().bg(SURFACE_USER).render(&" ".repeat(gap))
            )
        } else {
            Style::new()
                .bg(SURFACE_USER)
                .render(&" ".repeat(prefix_width))
        };
        let content = a3s_tui::style::fit_visible(&line, body_width);
        let content = format!("{content}{}", " ".repeat(right_margin));
        rows.push(format!(
            "{}{prefix}{}",
            " ".repeat(margin),
            Style::new().fg(TN_FG).bg(SURFACE_USER).render(&content)
        ));
    }
    rows.push(format!("{}{padding}", " ".repeat(margin)));
    rows.join("\n")
}

fn wrap_user_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    let mut rows = Vec::new();
    let mut remaining = line;
    while !remaining.is_empty() {
        let end = visible_prefix_byte_end(remaining, width);
        let end = if end == 0 {
            remaining
                .char_indices()
                .nth(1)
                .map(|(offset, _)| offset)
                .unwrap_or(remaining.len())
        } else {
            end
        };
        rows.push(remaining[..end].to_string());
        remaining = &remaining[end..];
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

/// Return a source byte boundary that fits whole display cells in `width`.
///
/// Advancing by source bytes prevents a wide glyph that crosses a display
/// column boundary from being omitted from the next row. Zero-width combining
/// marks remain attached to the preceding display cell.
fn visible_prefix_byte_end(value: &str, width: usize) -> usize {
    if width == 0 {
        return 0;
    }
    let mut used = 0usize;
    let mut end = 0usize;
    for (offset, ch) in value.char_indices() {
        let char_width = a3s_tui::style::visible_len(&ch.to_string());
        if char_width > 0 && used.saturating_add(char_width) > width {
            break;
        }
        used = used.saturating_add(char_width);
        end = offset + ch.len_utf8();
    }
    end
}

/// Prefix a message block with a Codex-style colored • gutter on its first line
/// and align the rest under the text.
pub(crate) fn gutter(color: Color, content: &str) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }

    a3s_tui::components::GutterBlock::lines(lines)
        .margin(PAD)
        .marker("•")
        .marker_color(color)
        .view()
}

/// Render one assistant-authored Markdown cell.
///
/// Vertical separation belongs to the transcript compositor, not the cell.
/// Keeping cells spacing-free gives messages, notices, and tool calls the same
/// boundary rule and avoids doubled rows when two padded cells meet.
pub(crate) fn assistant_block(content: &str, width: usize) -> String {
    assistant_body(content, width)
}

/// Split a streaming assistant cell at its stable/tail boundary. The stable
/// fragment is newline-terminated so `Viewport::set_content_parts` can retain
/// its wrapped rows while replacing only the mutable tail. The caller inserts
/// the same transcript gap used by committed entries before this cell.
pub(crate) fn assistant_stream_block_parts(
    stable: &str,
    tail: &str,
    width: usize,
) -> Option<(String, String)> {
    let stable = assistant_body(stable, width);
    let tail = assistant_body(tail, width);
    if stable.is_empty() && tail.is_empty() {
        return None;
    }

    let mut prefix = String::new();
    if !stable.is_empty() {
        prefix.push_str(&stable);
        prefix.push('\n');
    }
    Some((prefix, tail))
}

fn assistant_body(content: &str, width: usize) -> String {
    if content.is_empty() || width == 0 {
        String::new()
    } else {
        gutter(TN_GRAY, content)
    }
}

#[cfg(test)]
fn input_chrome_width(width: usize) -> u16 {
    width.min(u16::MAX as usize) as u16
}

#[cfg(test)]
pub(crate) fn input_rule(width: usize, color: Color) -> String {
    if width == 0 {
        return String::new();
    }

    let theme = agent_chrome_theme();
    agent_chrome(&theme)
        .input_border()
        .margin(PAD)
        .rule_color(color)
        .view(input_chrome_width(width))
}

#[cfg(test)]
pub(crate) fn input_gradient_rule(width: usize, palette: &[Color], offset: usize) -> String {
    if width == 0 || palette.is_empty() {
        return String::new();
    }

    let theme = agent_chrome_theme();
    agent_chrome(&theme)
        .input_border()
        .margin(PAD)
        .rule('━')
        .rainbow(palette.to_vec(), offset)
        .view(input_chrome_width(width))
}

#[cfg(test)]
pub(crate) fn input_status_rule(width: usize, border_color: Color, label: &str) -> String {
    if width == 0 {
        return String::new();
    }

    let theme = agent_chrome_theme();
    agent_chrome(&theme)
        .input_border()
        .margin(PAD)
        .rule_color(border_color)
        .label(label)
        .view(input_chrome_width(width))
}

/// Horizontal inset that floats the PromptBar off the terminal edges.
pub(crate) const COMPOSER_INSET: usize = 1;

/// Top + bottom half-block rows that frame the PromptBar body (compatible).
pub(crate) const COMPOSER_BAR_CAP_ROWS: u16 = 2;

/// Full chrome height for the composer PromptBar (body + ▄/▀ caps).
pub(crate) fn composer_chrome_height(body_rows: u16) -> u16 {
    body_rows.saturating_add(COMPOSER_BAR_CAP_ROWS)
}

/// Floating PromptBar: inset + half-block caps over the black canvas.
///
/// Body fill is [`SURFACE_COMPOSER`] (raised gray). Caps use the same gray as
/// foreground so ▄/▀ continue the box edge on [`CANVAS`].
pub(crate) fn composer_prompt_bar(
    prompt: &str,
    color: Color,
    text: &str,
    tint_text: bool,
    prompt_bold: bool,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }
    let inset = COMPOSER_INSET.min(width.saturating_sub(1) / 2);
    let inner = width.saturating_sub(inset.saturating_mul(2)).max(1);
    let side = if inset == 0 {
        String::new()
    } else {
        Style::new().bg(CANVAS).render(&" ".repeat(inset))
    };
    let body_inner = input_prompt_line(prompt, color, text, tint_text, prompt_bold, inner);
    let body = body_inner
        .lines()
        .map(|line| format!("{side}{line}{side}"))
        .collect::<Vec<_>>()
        .join("\n");
    let cap = "▄".repeat(inner);
    let floor = "▀".repeat(inner);
    // Caps share the body gray so the PromptBar reads as one raised box, not
    // lighter ▄/▀ rails framing a near-black middle.
    let top = format!(
        "{side}{}{side}",
        Style::new().fg(SURFACE_COMPOSER).bg(CANVAS).render(&cap)
    );
    let bottom = format!(
        "{side}{}{side}",
        Style::new().fg(SURFACE_COMPOSER).bg(CANVAS).render(&floor)
    );
    if body.is_empty() {
        format!("{top}\n{bottom}")
    } else {
        format!("{top}\n{body}\n{bottom}")
    }
}

pub(crate) fn input_prompt_line(
    prompt: &str,
    color: Color,
    text: &str,
    tint_text: bool,
    prompt_bold: bool,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    // PromptBar: elevated surface behind the full-width composer.
    // Agent `→` stays muted/non-bold; shell/research mode marks stay bold.
    let mut prompt_style = Style::new().fg(color).bg(SURFACE_COMPOSER);
    if prompt_bold {
        prompt_style = prompt_style.bold();
    }
    let mut line = chrome
        .prompt(format!("{prompt} "))
        .text(text)
        .margin(PAD)
        .width(width)
        .background_color(SURFACE_COMPOSER)
        .prompt_style(prompt_style);
    if tint_text {
        line = line.text_style(Style::new().fg(color).bg(SURFACE_COMPOSER));
    } else {
        line = line.text_style(Style::new().fg(TN_FG).bg(SURFACE_COMPOSER));
    }
    let rendered = line.view();
    rendered
}

/// Collapsed completed-thought body budget (6 lines).
pub(crate) const COMPACT_THINKING_BODY_LINES: usize = 6;
/// Live streaming thinking body budget (5 lines).
pub(crate) const LIVE_THINKING_BODY_LINES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThoughtPhase {
    Live,
    Done,
}

/// Live thinking stream — dim content only (no Thought header).
pub(crate) fn thinking_block(text: &str, width: usize) -> String {
    render_thought_block(
        text,
        width,
        ThoughtPhase::Live,
        None,
        Some(LIVE_THINKING_BODY_LINES),
    )
}

/// Completed thought block for the main stream / Ctrl+T transcript.
pub(crate) fn thought_block(
    text: &str,
    width: usize,
    duration: Option<Duration>,
    max_body_lines: Option<usize>,
) -> String {
    render_thought_block(text, width, ThoughtPhase::Done, duration, max_body_lines)
}

/// thought / thinking block.
///
/// Live: dim body only (status line owns `Thinking…`); keep the newest lines.
/// Done: `… Thought` / `… Thought for Xs` + dim body.
/// Truncation mirrors overflow-markdown: hide older lines above and keep
/// the visible tail (`... (N lines hidden above)[ · ctrl+t to expand]`).
fn render_thought_block(
    text: &str,
    width: usize,
    phase: ThoughtPhase,
    duration: Option<Duration>,
    max_body_lines: Option<usize>,
) -> String {
    let text = text.trim();
    if text.is_empty() || width == 0 {
        return String::new();
    }

    let header = match phase {
        ThoughtPhase::Live => None,
        ThoughtPhase::Done => {
            let title = match duration {
                Some(duration) if duration.as_secs() > 0 => {
                    format!("Thought for {}", fmt_elapsed(duration))
                }
                _ => "Thought".to_string(),
            };
            Some(format!(
                "{} {}",
                Style::new().fg(TN_SUBTLE).render("…"),
                Style::new().fg(TN_GRAY).render(&title)
            ))
        }
    };
    let body_width = width.saturating_sub(2).max(1);
    let mut body_rows = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            body_rows.push(String::new());
            continue;
        }
        for wrapped in wrap_words(line, body_width) {
            // Overflow-markdown uses dimmed text (not italic) for thinking.
            body_rows.push(format!("  {}", Style::new().fg(TN_GRAY).render(&wrapped)));
        }
    }

    let hidden_above = max_body_lines
        .map(|limit| body_rows.len().saturating_sub(limit))
        .unwrap_or(0);
    if let Some(limit) = max_body_lines {
        if body_rows.len() > limit {
            let keep_from = body_rows.len() - limit;
            body_rows = body_rows.split_off(keep_from);
        }
    }

    let mut rows = Vec::new();
    if let Some(header) = header {
        rows.push(header);
    }
    if hidden_above > 0 {
        // "... (N lines hidden above)" (+ " · ctrl+t to expand" when done/compact).
        let hint = match phase {
            ThoughtPhase::Live => format!("... ({hidden_above} lines hidden above)"),
            ThoughtPhase::Done => {
                format!("... ({hidden_above} lines hidden above) · ctrl+t to expand")
            }
        };
        rows.push(Style::new().fg(TN_SUBTLE).render(&format!("  {hint}")));
    }
    rows.extend(body_rows);

    rows.into_iter()
        .map(|line| a3s_tui::style::fit_visible(&line, width))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
pub(crate) fn compact_progress_line(elapsed: Duration, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let prefix = Style::new().fg(ACCENT).render(&format!(
        "✦ Compacting context… {} / {} ",
        fmt_elapsed(elapsed),
        fmt_elapsed(crate::compact::MANUAL_COMPACT_TIMEOUT),
    ));
    let progress_width = width
        .saturating_sub(a3s_tui::style::visible_len(&prefix))
        .clamp(1, 29);
    let phase = (elapsed.as_millis() / 120) % 20;
    let pulse = if phase <= 10 { phase } else { 20 - phase };
    let value = 0.15 + (pulse as f64 / 10.0) * 0.7;
    let progress = a3s_tui::components::Progress::new()
        .value(value)
        .width(progress_width.min(u16::MAX as usize) as u16)
        .show_percentage(false)
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

/// Render `text` with a soft highlight gliding left-to-right (loading shimmer).
pub(crate) fn shimmer(text: &str, phase: usize) -> String {
    a3s_tui::components::ShimmerText::new(text)
        .phase(phase)
        .colors(TN_GRAY, TN_FG)
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
        assistant_block, assistant_stream_block_parts, compact_progress_line,
        composer_chrome_height, composer_prompt_bar, gutter, input_gradient_rule,
        input_prompt_line, input_rule, input_status_rule, paint_canvas_rows, shimmer,
        thinking_block, thought_block, truncate, user_bubble, wrap_words, ACCENT,
        CANVAS, COMPOSER_CHROME, COMPOSER_INSET, SURFACE_COMPOSER, SURFACE_USER, TN_FG, TN_GRAY,
    };
    use a3s_tui::layout::{Constraint, Layout};
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

        assert_eq!(plain, "• hello\n  world");
        assert!(rendered.contains("\x1b[1;32m•\x1b[0m"));
    }

    #[test]
    fn gutter_preserves_styled_content() {
        let styled = Style::new().fg(Color::Yellow).render("styled");
        let rendered = gutter(Color::Green, &styled);

        assert!(rendered.contains("\x1b[33mstyled\x1b[0m"));
        assert_eq!(strip_ansi(&rendered), "• styled");
    }

    #[test]
    fn gutter_keeps_empty_input_empty() {
        assert_eq!(gutter(Color::Green, ""), "");
    }

    #[test]
    fn assistant_block_leaves_vertical_spacing_to_the_transcript() {
        let rendered = assistant_block("hello\nworld", 20);
        let plain = strip_ansi(&rendered);

        assert_eq!(plain.lines().collect::<Vec<_>>(), ["• hello", "  world"]);
        assert!(plain.lines().all(|row| !row.trim().is_empty()));
        assert!(rendered.lines().all(|row| visible_len(row) <= 20));
        assert_eq!(assistant_block("", 20), "");
        assert_eq!(assistant_block("hello", 0), "");
    }

    #[test]
    fn streaming_assistant_parts_do_not_create_private_gap_rows() {
        for (stable, tail) in [("stable", ""), ("", "tail"), ("stable", "tail")] {
            let (prefix, suffix) =
                assistant_stream_block_parts(stable, tail, 20).expect("stream parts");
            if stable.is_empty() {
                assert!(prefix.is_empty());
            } else {
                assert!(prefix.ends_with('\n'));
            }
            let rendered = strip_ansi(&format!("{prefix}{suffix}"));
            let rows = rendered.lines().collect::<Vec<_>>();

            assert_eq!(
                rows.iter().filter(|row| row.trim().is_empty()).count(),
                0,
                "{rendered:?}"
            );
        }

        let (stable_prefix, stable_suffix) =
            assistant_stream_block_parts("stable", "", 20).expect("stable parts");
        assert_eq!(
            format!("{stable_prefix}{stable_suffix}").trim_end_matches('\n'),
            assistant_block("stable", 20)
        );
        let (tail_prefix, tail_suffix) =
            assistant_stream_block_parts("", "tail", 20).expect("tail parts");
        assert_eq!(
            format!("{tail_prefix}{tail_suffix}"),
            assistant_block("tail", 20)
        );
        assert!(assistant_stream_block_parts("", "", 20).is_none());
        assert!(assistant_stream_block_parts("stable", "tail", 0).is_none());
    }

    #[test]
    fn user_bubble_uses_codex_surface_and_message_rail() {
        let rendered = user_bubble("hello\nworld", 20);
        let plain = strip_ansi(&rendered);
        let rows = plain.lines().collect::<Vec<_>>();

        assert_eq!(
            rows,
            vec![
                "                    ",
                "› hello             ",
                "  world             ",
                "                    ",
            ]
        );
        assert!(rendered.lines().all(|row| visible_len(row) == 20));
        assert!(rendered.contains(
            &Style::new()
                .fg(TN_FG)
                .bg(SURFACE_USER)
                .bold()
                .dim()
                .render("›")
        ));
        assert!(rendered.contains(
            &Style::new()
                .fg(TN_FG)
                .bg(SURFACE_USER)
                .render("hello             ")
        ));
        assert!(rows[0].trim().is_empty() && rows[3].trim().is_empty());
        assert!(rendered
            .lines()
            .all(|row| row.contains(&format!("\x1b[{}m", SURFACE_USER.bg_ansi()))));
    }

    #[test]
    fn user_bubble_keeps_empty_input_empty() {
        assert_eq!(user_bubble("", 20), "");
        assert_eq!(user_bubble("hello", 0), "");
    }

    #[test]
    fn user_bubble_stays_within_narrow_viewport() {
        let rendered = user_bubble("hi", 6);
        let rows = strip_ansi(&rendered)
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();

        assert_eq!(rows, vec!["      ", "› hi  ", "      "]);
        assert!(rendered.lines().all(|line| visible_len(line) == 6));

        for width in 1..=5 {
            let rendered = user_bubble("hi", width);
            let plain = strip_ansi(&rendered);
            assert!(
                rendered.lines().all(|line| visible_len(line) == width),
                "width {width}: {plain:?}"
            );
            assert!(plain.contains('h'), "width {width}: {plain:?}");
            assert!(plain.contains('i'), "width {width}: {plain:?}");
            assert!(!plain.contains('…'), "width {width}: {plain:?}");
        }
    }

    #[test]
    fn user_bubble_wraps_long_and_wide_text_without_losing_content() {
        let rendered = user_bubble("abcdefghij\n中文测试", 8);
        let rows = strip_ansi(&rendered)
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();

        assert_eq!(
            rows,
            vec![
                "        ",
                "› abcde ",
                "  fghij ",
                "  中文  ",
                "  测试  ",
                "        ",
            ]
        );
        assert!(rendered.lines().all(|line| visible_len(line) == 8));
    }

    #[test]
    fn paint_canvas_rows_fills_layout_void_with_pure_black() {
        // Runtime evidence: Layout Fill leaves empty strings (no CANVAS bg).
        let raw = Layout::vertical()
            .item("", Constraint::Fill)
            .item("footer", Constraint::Fixed(1))
            .render(5);
        let empty_before = raw
            .lines()
            .filter(|l| a3s_tui::style::visible_len(l) == 0)
            .count();
        assert!(empty_before >= 3, "expected unpainted fill rows: {raw:?}");
        assert!(
            !raw.contains(&CANVAS.bg_ansi()),
            "raw layout must not already paint CANVAS: {raw:?}"
        );

        let painted = paint_canvas_rows(&raw, 12);
        let empty_after = painted
            .lines()
            .filter(|l| a3s_tui::style::visible_len(l) == 0)
            .count();
        assert_eq!(empty_after, 0, "canvas paint must fill void: {painted:?}");
        assert!(
            painted.contains(&CANVAS.bg_ansi()),
            "painted rows must emit CANVAS bg: {painted:?}"
        );
        assert!(
            painted.contains("48;2;0;0;0"),
            "CANVAS must be pure black RGB: {painted:?}"
        );

        // Full-width unstyled spaces must also become canvas (charcoal leak).
        let plain_spaces = " ".repeat(12);
        let repainted = paint_canvas_rows(&plain_spaces, 12);
        assert!(
            repainted.contains(&CANVAS.bg_ansi()),
            "blank plain spaces must paint CANVAS: {repainted:?}"
        );
    }

    #[test]
    fn paint_canvas_rows_preserves_user_bubble_vertical_padding() {
        let bubble = user_bubble("工作区有哪些文件？", 24);
        let painted = paint_canvas_rows(&bubble, 24);
        let rows: Vec<&str> = painted.lines().collect();
        assert!(
            rows.len() >= 3,
            "bubble must keep pad + content + pad: {rows:?}"
        );
        let user_bg = SURFACE_USER.bg_ansi();
        assert!(
            rows[0].contains(&user_bg),
            "top pad must keep SURFACE_USER: {}",
            rows[0]
        );
        assert!(
            rows[rows.len() - 1].contains(&user_bg),
            "bottom pad must keep SURFACE_USER: {}",
            rows[rows.len() - 1]
        );
        assert!(
            rows.iter().filter(|row| row.contains(&user_bg)).count() >= 3,
            "content and both pads must stay on SURFACE_USER: {painted:?}"
        );
        // Padding rows are blank after strip — paint must not rewrite them to CANVAS.
        let top_plain = strip_ansi(rows[0]);
        assert!(top_plain.chars().all(|c| c == ' '), "{top_plain:?}");
        assert!(
            !rows[0].contains(&CANVAS.bg_ansi()),
            "top pad must not become CANVAS: {}",
            rows[0]
        );
    }

    #[test]
    fn composer_prompt_bar_frames_body_with_half_block_caps() {
        let rendered = composer_prompt_bar("→", COMPOSER_CHROME.faint, "", false, false, 16);
        let plain = strip_ansi(&rendered);
        let rows: Vec<&str> = plain.lines().collect();
        assert_eq!(rows.len(), 3, "{plain:?}");
        // Inset floats the bar: canvas gutters + ▄/▀ caps over CANVAS.
        assert!(rows[0].contains('▄'), "{rows:?}");
        assert!(
            rows[0].starts_with(' ') && rows[0].ends_with(' '),
            "{rows:?}"
        );
        assert!(rows[1].contains('→'), "{rows:?}");
        assert!(rows[2].contains('▀'), "{rows:?}");
        assert_eq!(visible_len(rows[0]), 16);
        assert_eq!(visible_len(rows[1]), 16);
        assert_eq!(visible_len(rows[2]), 16);
        assert!(
            rendered.contains(&SURFACE_COMPOSER.bg_ansi()),
            "PromptBar body must use raised gray: {rendered:?}"
        );
        assert!(
            rendered.contains(&SURFACE_COMPOSER.fg_ansi()),
            "floating caps must match composer gray: {rendered:?}"
        );
        assert!(
            rendered.contains(&CANVAS.bg_ansi()),
            "floating caps/gutters must sit on black canvas: {rendered:?}"
        );
        assert_ne!(
            SURFACE_COMPOSER, CANVAS,
            "composer gray must contrast with black canvas"
        );
        assert_ne!(
            SURFACE_COMPOSER, SURFACE_USER,
            "composer must be lighter than user bubble on pure black"
        );
        assert_eq!(composer_chrome_height(1), 3);
        assert_eq!(COMPOSER_INSET, 1);
        // Agent arrow is muted, not bold accent — matches Cursor PromptBar.
        assert!(rendered.contains(
            &Style::new()
                .fg(COMPOSER_CHROME.faint)
                .bg(SURFACE_COMPOSER)
                .render("→ ")
        ));
        assert!(
            !rendered.contains(&Style::new().fg(COMPOSER_CHROME.faint).bold().render("→")),
            "agent arrow must not be bold: {rendered:?}"
        );
    }

    #[test]
    fn input_prompt_line_uses_shared_prompt_component() {
        let rendered = input_prompt_line("→", Color::Cyan, "cargo test\n--all", false, true, 24);
        let plain = strip_ansi(&rendered);
        let rows = plain.lines().collect::<Vec<_>>();

        assert!(rows[0].starts_with("→ cargo test"));
        assert!(rows[1].starts_with("  --all"));
        assert!(rendered.lines().all(|line| visible_len(line) == 24));
        assert!(rendered.contains(
            &Style::new()
                .fg(Color::Cyan)
                .bold()
                .bg(SURFACE_COMPOSER)
                .render("→ ")
        ));
        assert!(
            rendered.contains(&format!("\x1b[{}m", SURFACE_COMPOSER.bg_ansi())),
            "composer PromptBar must paint an elevated surface: {rendered:?}"
        );
        assert!(
            rendered
                .lines()
                .all(|line| line.contains(&SURFACE_COMPOSER.bg_ansi())),
            "every composer row must keep the surface fill: {rendered:?}"
        );
    }

    #[test]
    fn input_prompt_line_can_tint_modal_input_text() {
        let rendered = input_prompt_line("?", Color::Cyan, "research mode", true, true, 28);

        assert_eq!(strip_ansi(&rendered).trim_end(), "? research mode");
        assert!(rendered.contains(
            &Style::new()
                .fg(Color::Cyan)
                .bold()
                .bg(SURFACE_COMPOSER)
                .render("? ")
        ));
        assert!(rendered.contains(
            &Style::new()
                .fg(Color::Cyan)
                .bg(SURFACE_COMPOSER)
                .render("research mode")
        ));
        assert!(rendered.contains(&SURFACE_COMPOSER.bg_ansi()));
    }

    #[test]
    fn input_rules_restore_outlined_composer_and_status_chip() {
        let plain = strip_ansi(&input_rule(20, Color::BrightBlack));
        assert_eq!(visible_len(&plain), 20);
        assert!(plain.starts_with('─'));

        let status = input_status_rule(48, Color::BrightBlack, "◇ high");
        let status_plain = strip_ansi(&status);
        assert_eq!(visible_len(&status), 48);
        assert!(status_plain.starts_with('─'), "{status_plain}");
        assert!(!status_plain.contains("context"), "{status_plain}");
        assert!(status_plain.contains("◇ high"), "{status_plain}");
        assert!(status.contains(&ACCENT.fg_ansi()));
    }

    #[test]
    fn input_gradient_rule_animates_brand_ribbon_without_a_surface_fill() {
        let palette = [
            Color::Rgb(86, 156, 255),
            Color::Rgb(70, 214, 255),
            Color::Rgb(255, 101, 155),
            Color::Rgb(190, 124, 255),
        ];
        let first = input_gradient_rule(24, &palette, 0);
        let next = input_gradient_rule(24, &palette, 1);

        assert_eq!(visible_len(&first), 24);
        assert_eq!(strip_ansi(&first), "━━━━━━━━━━━━━━━━━━━━━━━━");
        assert_ne!(first, next);
        assert!(palette.iter().all(|color| first.contains(&color.fg_ansi())));
        assert!(!first.contains(&SURFACE_USER.bg_ansi()));
    }

    #[test]
    fn thinking_block_renders_dim_content() {
        let rendered = thinking_block("alpha beta gamma delta\nsecond line kept\nthird", 40);
        let plain = strip_ansi(&rendered);
        let rows = plain.lines().collect::<Vec<_>>();

        // Live thinking has no Thought/Thinking header — status owns "Thinking…".
        assert!(!plain.contains("… Thinking"), "{plain}");
        assert!(!plain.contains("Thought"), "{plain}");
        assert!(plain.contains("alpha beta"), "{plain}");
        assert!(plain.contains("second line kept"), "{plain}");
        assert!(rows.len() >= 2, "{plain}");
        assert!(rendered.contains(&TN_GRAY.fg_ansi()));
        assert!(!rendered.contains("\x1b[3;"));
    }

    #[test]
    fn thinking_block_keeps_empty_input_empty() {
        assert_eq!(thinking_block("   ", 40), "");
        assert_eq!(thinking_block("thinking", 0), "");
    }

    #[test]
    fn thinking_block_truncates_with_hidden_above_keeps_tail() {
        let text = (0..20)
            .map(|i| format!("line-{i} of long thinking"))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = thinking_block(&text, 48);
        let plain = strip_ansi(&rendered);
        assert!(!plain.contains("… Thinking"), "{plain}");
        assert!(plain.contains("lines hidden above"), "{plain}");
        assert!(!plain.contains("ctrl+t to expand"), "{plain}");
        assert!(!plain.contains("line-0 "), "{plain}");
        assert!(plain.contains("line-19"), "{plain}");
        assert!(rendered.lines().all(|line| visible_len(line) <= 48));
    }

    #[test]
    fn thought_block_compact_keeps_tail_with_expand_hint() {
        let text = (0..20)
            .map(|i| format!("thought-line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = thought_block(&text, 48, Some(Duration::from_secs(4)), Some(6));
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("… Thought for 4s"), "{plain}");
        assert!(plain.contains("lines hidden above"), "{plain}");
        assert!(plain.contains("ctrl+t to expand"), "{plain}");
        assert!(!plain.contains("thought-line-0"), "{plain}");
        assert!(plain.contains("thought-line-19"), "{plain}");
    }

    #[test]
    fn thought_block_uses_completed_grammar() {
        let rendered = thought_block(
            "Inspect the event ordering.",
            48,
            Some(Duration::from_secs(3)),
            None,
        );
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("… Thought for 3s"), "{plain}");
        assert!(plain.contains("Inspect the event ordering"), "{plain}");
        assert!(!plain.contains("Reasoning"), "{plain}");
    }

    #[test]
    fn compact_progress_line_uses_shared_progress_bar() {
        let rendered = compact_progress_line(Duration::from_secs(15), 80);
        let plain = strip_ansi(&rendered);

        assert!(plain.starts_with("✦ Compacting context"), "{plain}");
        assert!(plain.contains("Compacting context"), "{plain}");
        assert!(plain.contains("15s / 1m 00s"), "{plain}");
        assert!(!plain.contains('%'), "{plain}");
        assert!(plain.contains('▰'), "{plain}");
        assert!(plain.contains('▱'), "{plain}");
        assert!(rendered.contains(&format!("\x1b[{}m", ACCENT.fg_ansi())));
        assert!(visible_len(&rendered) <= 80);
    }

    #[test]
    fn compact_progress_line_animates_without_claiming_completion_percentage() {
        let first = strip_ansi(&compact_progress_line(Duration::from_millis(600), 80));
        let second = strip_ansi(&compact_progress_line(Duration::from_millis(1_800), 80));

        assert_ne!(first, second);
        assert!(!first.contains('%'));
        assert!(!second.contains('%'));
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
            .colors(TN_GRAY, TN_FG)
            .spread(5.0)
            .speed_divisor(3)
            .cycle_gap(12)
            .view();

        assert_eq!(rendered, expected);
        assert_eq!(strip_ansi(&rendered), "Working…");
        assert!(rendered.contains(&Style::new().fg(TN_FG).bold().render("W")));
        assert!(!rendered.contains("38;2;125;182;255"));
    }

    #[test]
    fn shimmer_preserves_complex_glyph_display_width() {
        let rendered = shimmer("工作e\u{301}", 0);

        assert_eq!(strip_ansi(&rendered), "工作e\u{301}");
        assert_eq!(visible_len(&rendered), 5);
    }
}
