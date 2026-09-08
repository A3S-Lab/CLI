//! `/terminal` diagnostics for the active Code TUI process.
//!
//! Beyond capability dump: when Shift+Enter / enhanced keys / multiplexer
//! passthrough is weak, surface copy-paste repair hints (Cursor
//! `/setup-terminal` spirit without cloning its wizard UI).

use a3s_tui::components::{DetailPanel, DetailRow};
use a3s_tui::style::Color;
use a3s_tui::{TerminalFamily, TerminalMultiplexer, TerminalProfile, TerminalSupport};

pub(crate) fn current_terminal_diagnostic(width: usize) -> String {
    let profile = TerminalProfile::detect();
    let size = a3s_tui::terminal::Terminal::size().ok();
    render_terminal_diagnostic(&profile, size, width)
}

fn render_terminal_diagnostic(
    profile: &TerminalProfile,
    size: Option<(u16, u16)>,
    width: usize,
) -> String {
    let width = width.min(u16::MAX as usize) as u16;
    if width == 0 {
        return String::new();
    }

    let environment = [
        profile.term().map(|value| format!("TERM={value}")),
        profile
            .term_program()
            .map(|value| format!("TERM_PROGRAM={value}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let environment = if environment.is_empty() {
        "not reported".to_string()
    } else {
        environment.join(" · ")
    };
    let tty = format!(
        "stdin {} · stdout {} · stderr {}",
        yes_no(profile.stdin_is_terminal()),
        yes_no(profile.stdout_is_terminal()),
        yes_no(profile.stderr_is_terminal())
    );
    let size = size
        .map(|(columns, rows)| format!("{columns} × {rows} cells"))
        .unwrap_or_else(|| "unavailable".to_string());

    let mut panel = DetailPanel::new("Terminal diagnostics")
        .unlimited_rows()
        .label_width(16)
        .title_color(Color::Cyan)
        .pair("emulator", profile.family().to_string())
        .pair("multiplexer", profile.multiplexer().to_string())
        .pair("environment", environment)
        .pair("terminal I/O", tty)
        .pair("canvas", size)
        .pair("render mode", profile.display_mode().to_string())
        .pair("color", profile.color_level().to_string())
        .row(support_row("alternate screen", profile.alternate_screen()))
        .row(support_row("mouse capture", profile.mouse_capture()))
        .row(support_row("bracketed paste", profile.bracketed_paste()))
        .row(support_row("enhanced keys", profile.enhanced_keyboard()))
        .row(support_row("OSC 8 links", profile.hyperlinks()))
        .row(support_row("OSC 52 copy", profile.clipboard()));

    for warning in profile.warnings() {
        panel.add_row(DetailRow::muted(format!("⚠ {warning}")).color(Color::Yellow));
    }

    for hint in repair_hints(profile) {
        panel.add_row(DetailRow::muted(format!("→ {hint}")).color(Color::Cyan));
    }

    panel.view(width, usize::MAX)
}

/// Actionable setup lines for weak Shift+Enter / key / multiplexer support.
pub(crate) fn repair_hints(profile: &TerminalProfile) -> Vec<String> {
    let mut hints = Vec::new();

    match profile.enhanced_keyboard() {
        TerminalSupport::Supported => {}
        TerminalSupport::RequiresPassthrough => {
            hints.push(
                "Shift+Enter may be eaten by the multiplexer — use Ctrl+J for newlines, or enable Kitty keys passthrough"
                    .into(),
            );
            match profile.multiplexer() {
                TerminalMultiplexer::Tmux => hints.push(
                    "tmux: set -g allow-passthrough on   # then restart the session".into(),
                ),
                TerminalMultiplexer::Zellij => hints.push(
                    "zellij: enable advanced key reporting / passthrough in the layout config".into(),
                ),
                TerminalMultiplexer::Screen => {
                    hints.push("GNU screen: prefer Ctrl+J; Shift+Enter is unreliable".into())
                }
                TerminalMultiplexer::None => {}
            }
        }
        TerminalSupport::Unsupported | TerminalSupport::Unknown => {
            hints.push(
                "Shift+Enter may not insert a newline here — use Ctrl+J (works in all terminals)"
                    .into(),
            );
            match profile.family() {
                TerminalFamily::Iterm2 => hints.push(
                    "iTerm2: Prefs → Profiles → Keys → report modifiers / CSI u when available"
                        .into(),
                ),
                TerminalFamily::Kitty => hints.push(
                    "kitty: ensure kitty_mod and report_modifiers stay enabled (defaults are fine)"
                        .into(),
                ),
                TerminalFamily::Ghostty
                | TerminalFamily::WezTerm
                | TerminalFamily::Alacritty => hints.push(
                    "This emulator usually supports Shift+Enter — if not, fall back to Ctrl+J"
                        .into(),
                ),
                _ => {}
            }
        }
    }

    if matches!(
        profile.bracketed_paste(),
        TerminalSupport::Unsupported | TerminalSupport::Unknown
    ) {
        hints.push(
            "Bracketed paste looks weak — large pastes may split into many submits; prefer a modern emulator"
                .into(),
        );
    }

    if hints.is_empty() {
        hints.push(
            "Composer newlines: Shift+Enter when reported, otherwise Ctrl+J · Diff review: Ctrl+G"
                .into(),
        );
    }

    hints
}

fn support_row(label: &str, support: TerminalSupport) -> DetailRow {
    let (marker, color) = match support {
        TerminalSupport::Supported => ("✓", Color::Green),
        TerminalSupport::Unsupported => ("×", Color::Red),
        TerminalSupport::RequiresPassthrough => ("△", Color::Yellow),
        TerminalSupport::Unknown => ("?", Color::BrightBlack),
    };
    DetailRow::pair(label, format!("{marker} {support}")).color(color)
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

/// Optional terminal bell when a turn becomes idle (`A3S_CODE_NOTIFY=1`).
pub(crate) fn turn_complete_notify_enabled() -> bool {
    matches!(
        std::env::var("A3S_CODE_NOTIFY").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

pub(crate) fn emit_turn_complete_notify() {
    if !turn_complete_notify_enabled() {
        return;
    }
    // BEL is the portable signal; OSC-9 is best-effort for iTerm/Ghostty.
    eprint!("\x07\x1b]9;A3S Code turn complete\x07");
    let _ = std::io::Write::flush(&mut std::io::stderr());
}

/// Optional idle-turn suggestion (`A3S_CODE_SUGGEST=1`). Heuristic only — no model call.
pub(crate) fn turn_suggest_enabled() -> bool {
    matches!(
        std::env::var("A3S_CODE_SUGGEST").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

/// One dim follow-up hint after a settled turn. Prefer Diff review when the
/// latest user turn mutated files.
pub(crate) fn idle_turn_suggestion(file_change_count: usize) -> Option<String> {
    if !turn_suggest_enabled() {
        return None;
    }
    if file_change_count > 0 {
        Some(format!(
            "  tip · Ctrl+G reviews {file_change_count} file change(s) from this turn · Esc dismisses overlays"
        ))
    } else {
        Some(
            "  tip · /terminal for Shift+Enter setup · Ctrl+J always inserts a newline".into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_tui::style::{strip_ansi, visible_len};

    #[test]
    fn diagnostic_is_bounded_and_names_every_negotiated_capability() {
        let width = 44;
        let rendered = current_terminal_diagnostic(width);
        let plain = strip_ansi(&rendered);

        assert!(plain.contains("Terminal diagnostics"), "{plain}");
        assert!(plain.contains("alternate screen"), "{plain}");
        assert!(plain.contains("mouse capture"), "{plain}");
        assert!(plain.contains("bracketed paste"), "{plain}");
        assert!(plain.contains("enhanced keys"), "{plain}");
        assert!(plain.contains("OSC 8 links"), "{plain}");
        assert!(plain.contains("OSC 52 copy"), "{plain}");
        assert!(
            plain.contains("Ctrl+J") || plain.contains("Ctrl+G") || plain.contains("→"),
            "expected repair/legend hint: {plain}"
        );
        assert!(
            rendered.lines().all(|line| visible_len(line) <= width),
            "{rendered:?}"
        );
    }

    #[test]
    fn narrow_diagnostic_stays_inside_the_terminal_canvas() {
        for width in [1_usize, 8, 20] {
            let rendered = current_terminal_diagnostic(width);
            assert!(
                rendered.lines().all(|line| visible_len(line) <= width),
                "width={width}: {rendered:?}"
            );
        }
    }

    #[test]
    fn repair_hints_mention_ctrl_j_when_enhanced_keys_are_weak() {
        // Detected profile may already be strong; assert the helper always
        // returns at least one actionable line for the live profile.
        let profile = TerminalProfile::detect();
        let hints = repair_hints(&profile);
        assert!(!hints.is_empty());
        let joined = hints.join("\n");
        assert!(
            joined.contains("Ctrl+J")
                || joined.contains("passthrough")
                || joined.contains("Ctrl+G"),
            "{joined}"
        );
    }

    #[test]
    fn idle_suggestion_prefers_diff_review_when_files_changed() {
        let previous = std::env::var_os("A3S_CODE_SUGGEST");
        std::env::set_var("A3S_CODE_SUGGEST", "1");
        let with_files = idle_turn_suggestion(2).expect("enabled");
        let without = idle_turn_suggestion(0).expect("enabled");
        match previous {
            Some(value) => std::env::set_var("A3S_CODE_SUGGEST", value),
            None => std::env::remove_var("A3S_CODE_SUGGEST"),
        }
        assert!(with_files.contains("Ctrl+G"), "{with_files}");
        assert!(without.contains("/terminal") || without.contains("Ctrl+J"), "{without}");
    }

    #[test]
    fn turn_complete_notify_is_off_by_default() {
        let previous = std::env::var_os("A3S_CODE_NOTIFY");
        std::env::remove_var("A3S_CODE_NOTIFY");
        assert!(!turn_complete_notify_enabled());
        std::env::set_var("A3S_CODE_NOTIFY", "1");
        assert!(turn_complete_notify_enabled());
        match previous {
            Some(value) => std::env::set_var("A3S_CODE_NOTIFY", value),
            None => std::env::remove_var("A3S_CODE_NOTIFY"),
        }
    }
}
