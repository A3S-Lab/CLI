//! Cursor-style tool-approval surface with an optional countdown bar.
//!
//! The transcript owns the operation preview. This overlay keeps the pending
//! decision above the composer: cyan remaining-time bar, cyan title, `->`
//! selection, and auto Skip & tell when the bar runs out.

use std::time::{Duration, Instant};

use a3s_tui::event::{MouseButton, MouseEvent, MouseEventKind};
use a3s_tui::style::{fit_visible, strip_ansi, truncate_visible, visible_len, wrap_words, Style};

use super::{TN_CYAN, TN_FG, TN_GRAY, TN_RED, TN_SUBTLE, TN_YELLOW};

const OPTION_COUNT: usize = 4;
const MAX_DETAIL_ROWS: usize = 2;
const DEFAULT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(15);
const MIN_APPROVAL_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
pub(super) const APPROVAL_TIMEOUT_ENV: &str = "A3S_CODE_APPROVAL_TIMEOUT_MS";
pub(super) const APPROVAL_TIMEOUT_REASON: &str = "approval timed out";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ApprovalPromptMsg {
    Selected(usize),
}

/// Approval-only picker: countdown bar, cyan title, Cursor-like `->` rows.
#[derive(Clone, Debug)]
pub(super) struct ApprovalPrompt {
    label: String,
    selected: usize,
    y_offset: u16,
    denial_feedback: bool,
    saving_project_rule: bool,
    /// Remaining time fraction in `[0.0, 1.0]`. `None` = no countdown bar.
    remaining: Option<f64>,
}

impl ApprovalPrompt {
    pub(super) fn new(label: impl Into<String>, selected: usize) -> Self {
        Self {
            label: sanitize_label(&label.into()),
            selected: selected.min(OPTION_COUNT - 1),
            y_offset: 0,
            denial_feedback: false,
            saving_project_rule: false,
            remaining: None,
        }
    }

    pub(super) fn with_denial_feedback(mut self, enabled: bool) -> Self {
        self.denial_feedback = enabled;
        self
    }

    pub(super) fn with_project_rule_saving(mut self, enabled: bool) -> Self {
        self.saving_project_rule = enabled;
        self
    }

    pub(super) fn with_countdown(mut self, remaining: Option<f64>) -> Self {
        self.remaining = remaining.map(|value| value.clamp(0.0, 1.0));
        self
    }

    pub(super) fn lines(&self, width: usize) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }

        if self.denial_feedback {
            return self.denial_feedback_lines(width);
        }
        if self.saving_project_rule {
            return self.project_rule_saving_lines(width);
        }

        let mut lines = Vec::new();
        if let Some(remaining) = self.remaining {
            lines.push(approval_countdown_bar(width, remaining));
        }
        lines.push(self.title_line(width));
        lines.extend(self.detail_lines(width));
        lines.extend((0..OPTION_COUNT).map(|index| self.option_line(index, width)));
        lines.push(fit_visible(
            &Style::new()
                .fg(TN_SUBTLE)
                .render(&format!("{}{}", indent(width), self.footer_hint())),
            width,
        ));
        lines
    }

    pub(super) fn selected_index(&self) -> usize {
        self.selected.min(OPTION_COUNT - 1)
    }

    pub(super) fn set_y_offset(&mut self, y_offset: u16) {
        self.y_offset = y_offset;
    }

    pub(super) fn choice_start_row(&self, width: usize) -> usize {
        let bar = usize::from(self.remaining.is_some());
        1 + bar + self.detail_lines(width).len()
    }

    pub(super) fn handle_mouse(
        &mut self,
        mouse: &MouseEvent,
        width: usize,
    ) -> Option<ApprovalPromptMsg> {
        if width == 0 || self.denial_feedback || self.saving_project_rule {
            return None;
        }
        let local_row = mouse.row.checked_sub(self.y_offset)? as usize;
        if local_row >= self.lines(width).len() {
            return None;
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.selected = self.selected_index().saturating_sub(1);
                None
            }
            MouseEventKind::ScrollDown => {
                self.selected = (self.selected_index() + 1).min(OPTION_COUNT - 1);
                None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let index = local_row.checked_sub(self.choice_start_row(width))?;
                if index < OPTION_COUNT {
                    self.selected = index;
                    Some(ApprovalPromptMsg::Selected(index))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn footer_hint(&self) -> &'static str {
        if self.remaining.is_some() {
            "↑/↓ navigate · Enter select · auto-rejects when the bar runs out"
        } else {
            "↑/↓ navigate · Enter select · Esc deny"
        }
    }

    fn title_line(&self, width: usize) -> String {
        let prefix = indent(width);
        let title = Style::new()
            .fg(TN_CYAN)
            .bold()
            .render("Permission required");
        fit_visible(&format!("{prefix}{title}"), width)
    }

    fn detail_lines(&self, width: usize) -> Vec<String> {
        if self.label.is_empty() {
            return Vec::new();
        }

        let first_prefix = format!(
            "{}{}  ",
            indent(width),
            Style::new().fg(TN_SUBTLE).render("Run")
        );
        let continuation = " ".repeat(visible_len(&first_prefix));
        let available = width.saturating_sub(visible_len(&first_prefix)).max(1);
        let mut rows = wrap_words(&self.label, available);
        if rows.len() > MAX_DETAIL_ROWS {
            rows.truncate(MAX_DETAIL_ROWS);
            if let Some(last) = rows.last_mut() {
                *last = ellipsize(last, available);
            }
        }

        rows.into_iter()
            .enumerate()
            .map(|(index, row)| {
                let prefix = if index == 0 {
                    first_prefix.as_str()
                } else {
                    continuation.as_str()
                };
                let detail = Style::new()
                    .fg(if index == 0 { TN_FG } else { TN_GRAY })
                    .render(&truncate_visible(&row, available));
                fit_visible(&format!("{prefix}{detail}"), width)
            })
            .collect()
    }

    fn option_line(&self, index: usize, width: usize) -> String {
        let (label, hotkey) = match index {
            0 => ("Allow", "(y)"),
            1 => ("Allow session", "(s)"),
            2 => ("Add project rule", "(p)"),
            _ => ("Skip & tell", "(n or esc)"),
        };
        let selected = index == self.selected_index();
        let marker = if selected { "->" } else { "  " };
        let body = format!("{marker} {label} {hotkey}");
        let raw = format!("{}{body}", indent(width));
        if selected {
            Style::new()
                .fg(TN_CYAN)
                .bold()
                .render(&fit_visible(&raw, width))
        } else {
            let color = if index == 3 { TN_RED } else { TN_GRAY };
            Style::new().fg(color).render(&fit_visible(&raw, width))
        }
    }

    fn denial_feedback_lines(&self, width: usize) -> Vec<String> {
        let prefix = indent(width);
        let glyph = Style::new().fg(TN_RED).bold().render("⊘");
        let title = Style::new()
            .fg(TN_FG)
            .bold()
            .render("Tell the agent what to do instead");
        let mut lines = vec![fit_visible(&format!("{prefix}{glyph} {title}"), width)];
        lines.extend(self.detail_lines(width));
        lines.push(fit_visible(
            &Style::new()
                .fg(TN_GRAY)
                .render(&format!("{prefix}Type guidance in the composer below.")),
            width,
        ));
        lines.push(fit_visible(
            &Style::new()
                .fg(TN_SUBTLE)
                .render(&format!("{prefix}Enter deny with feedback · Esc back")),
            width,
        ));
        lines
    }

    fn project_rule_saving_lines(&self, width: usize) -> Vec<String> {
        let prefix = indent(width);
        let glyph = Style::new().fg(TN_YELLOW).bold().render("◆");
        let title = Style::new()
            .fg(TN_FG)
            .bold()
            .render("Saving project permission rule…");
        let mut lines = vec![fit_visible(&format!("{prefix}{glyph} {title}"), width)];
        lines.extend(self.detail_lines(width));
        lines.push(fit_visible(
            &Style::new().fg(TN_SUBTLE).render(&format!(
                "{prefix}The tool remains paused until the ACL write succeeds."
            )),
            width,
        ));
        lines
    }
}

/// Parse `A3S_CODE_APPROVAL_TIMEOUT_MS`. `None` = disabled. Missing/invalid → 15s.
pub(super) fn approval_timeout_from_env() -> Option<Duration> {
    approval_timeout_from_env_value(std::env::var(APPROVAL_TIMEOUT_ENV).ok().as_deref())
}

pub(super) fn approval_timeout_from_env_value(raw: Option<&str>) -> Option<Duration> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Some(DEFAULT_APPROVAL_TIMEOUT);
    };
    let Ok(ms) = raw.parse::<u64>() else {
        return Some(DEFAULT_APPROVAL_TIMEOUT);
    };
    if ms == 0 {
        return None;
    }
    let duration = Duration::from_millis(ms);
    Some(duration.clamp(MIN_APPROVAL_TIMEOUT, MAX_APPROVAL_TIMEOUT))
}

/// Remaining fraction in `[0.0, 1.0]` for a deadline armed at `deadline - total`.
pub(super) fn approval_remaining_fraction(deadline: Instant, now: Instant, total: Duration) -> f64 {
    if total.is_zero() {
        return 0.0;
    }
    if now >= deadline {
        return 0.0;
    }
    let remaining = deadline.saturating_duration_since(now);
    (remaining.as_secs_f64() / total.as_secs_f64()).clamp(0.0, 1.0)
}

pub(super) fn approval_deadline_expired(deadline: Instant, now: Instant) -> bool {
    now >= deadline
}

/// Cyan fill for remaining time; dim remainder. Full at start, shrinks to empty.
pub(super) fn approval_countdown_bar(width: usize, fraction: f64) -> String {
    if width == 0 {
        return String::new();
    }
    let fraction = fraction.clamp(0.0, 1.0);
    let filled = ((width as f64) * fraction).round() as usize;
    let filled = filled.min(width);
    let empty = width.saturating_sub(filled);
    let bar = format!(
        "{}{}",
        Style::new().fg(TN_CYAN).render(&"█".repeat(filled)),
        Style::new().fg(TN_SUBTLE).render(&"░".repeat(empty)),
    );
    fit_visible(&bar, width)
}

fn indent(width: usize) -> String {
    " ".repeat(width.min(2))
}

fn ellipsize(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if visible_len(value) < width {
        return format!("{value}…");
    }
    format!("{}…", truncate_visible(value, width.saturating_sub(1)))
}

fn sanitize_label(value: &str) -> String {
    strip_ansi(value)
        .chars()
        .filter_map(|ch| match ch {
            '\n' | '\r' | '\t' => Some(' '),
            ch if ch.is_control() => None,
            ch => Some(ch),
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_tui::style::{strip_ansi, visible_len};
    use std::time::{Duration, Instant};

    #[test]
    fn approval_timeout_env_defaults_clamps_and_disables() {
        assert_eq!(
            approval_timeout_from_env_value(None),
            Some(Duration::from_secs(15))
        );
        assert_eq!(approval_timeout_from_env_value(Some("0")), None);
        assert_eq!(
            approval_timeout_from_env_value(Some("1000")),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            approval_timeout_from_env_value(Some("999999")),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            approval_timeout_from_env_value(Some("bogus")),
            Some(Duration::from_secs(15))
        );
    }

    #[test]
    fn remaining_fraction_shrinks_toward_zero() {
        let total = Duration::from_secs(10);
        let now = Instant::now();
        let deadline = now + total;
        assert!((approval_remaining_fraction(deadline, now, total) - 1.0).abs() < 1e-9);
        assert!(
            (approval_remaining_fraction(deadline, now + Duration::from_secs(5), total) - 0.5)
                .abs()
                < 1e-9
        );
        assert_eq!(
            approval_remaining_fraction(deadline, now + Duration::from_secs(11), total),
            0.0
        );
        assert!(approval_deadline_expired(
            deadline,
            now + Duration::from_secs(10)
        ));
    }

    #[test]
    fn countdown_bar_fills_left_to_right_for_remaining() {
        let full = strip_ansi(&approval_countdown_bar(10, 1.0));
        assert_eq!(full.chars().filter(|c| *c == '█').count(), 10);
        let half = strip_ansi(&approval_countdown_bar(10, 0.5));
        assert_eq!(half.chars().filter(|c| *c == '█').count(), 5);
        assert_eq!(half.chars().filter(|c| *c == '░').count(), 5);
        let empty = strip_ansi(&approval_countdown_bar(10, 0.0));
        assert_eq!(empty.chars().filter(|c| *c == '░').count(), 10);
        assert_eq!(visible_len(&approval_countdown_bar(40, 0.75)), 40);
    }

    #[test]
    fn approval_surface_uses_cursor_style_selection_and_hotkeys() {
        let prompt =
            ApprovalPrompt::new("Bash(cargo test --workspace)", 0).with_countdown(Some(0.9));
        let lines = prompt.lines(72);
        let plain = lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();

        assert!(
            plain[0].chars().any(|c| c == '█' || c == '░'),
            "expected countdown bar: {plain:?}"
        );
        assert!(plain[1].contains("Permission required"), "{plain:?}");
        assert!(
            plain[2].contains("Run  Bash(cargo test --workspace)"),
            "{plain:?}"
        );
        assert!(plain.iter().any(|line| line.contains("-> Allow (y)")));
        assert!(plain.iter().any(|line| line.contains("Allow session (s)")));
        assert!(plain
            .iter()
            .any(|line| line.contains("Add project rule (p)")));
        assert!(plain
            .iter()
            .any(|line| line.contains("Skip & tell (n or esc)")));
        assert!(plain
            .iter()
            .any(|line| line.contains("auto-rejects when the bar runs out")));
        assert!(lines[1].contains(&TN_CYAN.fg_ansi()));
        assert!(lines.iter().all(|line| visible_len(line) == 72));
        assert_eq!(prompt.choice_start_row(72), 3);
    }

    #[test]
    fn countdown_disabled_omits_bar_and_auto_reject_footer() {
        let prompt = ApprovalPrompt::new("Write(a.rs)", 0);
        let plain = prompt
            .lines(56)
            .into_iter()
            .map(|line| strip_ansi(&line))
            .collect::<Vec<_>>();
        assert!(plain[0].contains("Permission required"), "{plain:?}");
        assert!(!plain[0].contains('█'));
        assert!(plain.iter().any(|line| line.contains("Esc deny")));
        assert!(!plain
            .iter()
            .any(|line| line.contains("auto-rejects when the bar runs out")));
        assert_eq!(prompt.choice_start_row(56), 2);
    }

    #[test]
    fn long_operation_is_bounded_without_hiding_the_decision_rows() {
        let prompt = ApprovalPrompt::new(format!("Bash({})", "command ".repeat(40)), 1)
            .with_countdown(Some(1.0));
        for width in [24, 42, 80] {
            let lines = prompt.lines(width);
            assert!(lines.len() <= 9, "{lines:?}");
            assert_eq!(prompt.choice_start_row(width), 4);
            assert!(lines.iter().all(|line| visible_len(line) == width));
            assert!(strip_ansi(&lines[3]).contains('…'));
        }
    }

    #[test]
    fn denial_feedback_replaces_choices_with_composer_guidance() {
        let prompt = ApprovalPrompt::new("Write(src/lib.rs)", 3)
            .with_countdown(Some(0.5))
            .with_denial_feedback(true);
        let plain = prompt
            .lines(56)
            .into_iter()
            .map(|line| strip_ansi(&line))
            .collect::<Vec<_>>();

        assert!(plain[0].contains("Tell the agent what to do instead"));
        assert!(!plain[0].contains('█'));
        assert!(plain.iter().any(|line| line.contains("Type guidance")));
        assert!(plain.iter().any(|line| line.contains("Enter deny")));
        assert!(!plain.iter().any(|line| line.contains("Allow (y)")));
    }
}
