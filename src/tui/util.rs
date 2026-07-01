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

/// Prefix a message block with a colored ● gutter on its first line and align
/// the rest under the text — marks user (blue) vs assistant (green) messages.
/// A user-input message rendered with a subtle background "bubble" so it stands
/// out from agent output in the transcript.
pub(crate) fn user_bubble(content: &str, width: usize) -> String {
    let margin = " ".repeat(PAD);
    let bg = Color::Rgb(38, 45, 64);
    // Full-width bar (minus the outer margins) with inner left/right padding.
    let bar = width.saturating_sub(PAD * 2).max(8);
    content
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let inner = if i == 0 {
                format!(" ● {line}")
            } else {
                format!("   {line}")
            };
            format!(
                "{margin}{}",
                Style::new().fg(TN_FG).bg(bg).render(&pad_to(&inner, bar))
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn gutter(color: Color, content: &str) -> String {
    let dot = Style::new().fg(color).bold().render("●");
    let margin = " ".repeat(PAD);
    content
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                format!("{margin}{dot} {line}")
            } else {
                format!("{margin}  {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Greedy word-wrap of plain (unstyled) text to `width` display columns, with
/// blank lines dropped so a preview stays single-spaced. Used for the reasoning
/// ("thinking") block so it lays out like other messages instead of being one
/// giant line the viewport re-wraps badly. Input must be unstyled — width is
/// counted in chars, which only holds without ANSI escapes.
pub(crate) fn wrap_words(text: &str, width: usize) -> Vec<String> {
    // Widths are counted in DISPLAY COLUMNS, not chars — CJK runs (which
    // `split_whitespace` keeps as one token) are 2 columns/char and would
    // otherwise overflow when the hard-break took `width` chars.
    use a3s_tui::style::visible_len as col;
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    for para in text.lines() {
        if para.trim().is_empty() {
            continue; // collapse blank lines — keep the preview compact
        }
        let mut line = String::new();
        for word in para.split_whitespace() {
            if line.is_empty() {
                line.push_str(word);
            } else if col(&line) + 1 + col(word) <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                line.push_str(word);
            }
            // Hard-break a token wider than the whole line, by column budget.
            while col(&line) > width {
                let mut head = String::new();
                let mut w = 0usize;
                for ch in line.chars() {
                    let cw = col(&ch.to_string()).max(1);
                    if w + cw > width {
                        break;
                    }
                    w += cw;
                    head.push(ch);
                }
                if head.is_empty() {
                    break; // width too small for even one char — avoid a loop
                }
                let rest: String = line.chars().skip(head.chars().count()).collect();
                out.push(head);
                line = rest;
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
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

/// Render `text` with a soft highlight gliding left→right (loading shimmer).
/// Each glyph's colour is interpolated from the base accent up to a bright tint
/// by its distance from the moving head, giving a gradient glow rather than a
/// hard band. `phase` is divided down so the glide is slow and gentle.
pub(crate) fn shimmer(text: &str, phase: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return String::new();
    }
    let span = (n + 12) as isize;
    let head = (phase as isize / 3) % span; // sweep speed; +12 = pause between sweeps
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        // Smooth falloff over ~5 glyphs either side of the head.
        let d = (head - i as isize).abs() as f32;
        let t = (1.0 - d / 5.0).clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
        // ACCENT (37,99,235) → bright tint (228,236,255).
        let mut s = Style::new().fg(Color::Rgb(
            lerp(37.0, 228.0),
            lerp(99.0, 236.0),
            lerp(235.0, 255.0),
        ));
        if t > 0.65 {
            s = s.bold();
        }
        out.push_str(&s.render(&c.to_string()));
    }
    out
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
    use super::{truncate, wrap_words};

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
    fn wrap_words_counts_display_columns_for_cjk() {
        // 6 CJK chars = 12 columns; CJK has no spaces so it's one token that
        // must hard-break by COLUMN budget, never exceeding the width.
        let lines = wrap_words("中文测试内容", 8);
        for l in &lines {
            assert!(
                a3s_tui::style::visible_len(l) <= 8,
                "line wider than 8 columns: {l:?}"
            );
        }
        assert_eq!(lines.concat(), "中文测试内容");
    }

    #[test]
    fn truncate_budgets_display_columns_not_chars() {
        // 5 CJK chars = 10 columns; a 6-column budget must fit (≤ 6 cols incl …),
        // which char-counting would have overflowed to ~10 columns.
        let out = truncate("一二三四五", 6);
        assert!(
            a3s_tui::style::visible_len(&out) <= 6,
            "truncated string exceeds 6 columns: {out:?}"
        );
        assert!(out.ends_with('…'));
        // Fits-as-is when within budget.
        assert_eq!(truncate("ok", 6), "ok");
    }
}
