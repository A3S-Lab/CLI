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
                Style::new()
                    .fg(Color::White)
                    .bg(bg)
                    .render(&pad_to(&inner, bar))
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

/// Indent every line of `content` by `cols` spaces (keeps blocks off the edge).
pub(crate) fn indent(content: &str, cols: usize) -> String {
    let pad = " ".repeat(cols);
    content
        .lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
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

/// "1m 05s" / "42s".
pub(crate) fn fmt_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 60 {
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

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}
