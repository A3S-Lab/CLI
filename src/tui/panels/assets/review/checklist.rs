//! Review issue checklist menu rendering.

use super::report::{ReviewIssue, ReviewReportKind, ReviewState};
use super::super::super::*;
use a3s_tui::components::{MenuItem, MenuPanel};

/// Row colour by severity (critical/high red, medium yellow, rest gray).
pub(crate) fn severity_color(sev: &str) -> Color {
    match sev.to_ascii_lowercase().as_str() {
        "critical" | "high" => TN_RED,
        "medium" => TN_YELLOW,
        _ => TN_GRAY,
    }
}

pub(crate) fn review_menu_hint(width: usize, kind: ReviewReportKind) -> String {
    let action = match kind {
        ReviewReportKind::Code => "Enter fix checked",
        ReviewReportKind::Reply => "Enter address · w waive open",
    };
    truncate(
        &format!("  ↑/↓ move · Space check · a all · {action} · Esc close"),
        width,
    )
}

pub(crate) fn review_menu_lines(review: &ReviewState, width: usize, height: usize) -> Vec<String> {
    let total = review.issues.len();
    if total == 0 || width == 0 {
        return Vec::new();
    }

    let checked = review.checked.iter().filter(|checked| **checked).count();
    let selected = review.sel.min(total - 1);
    let max_items = height.saturating_sub(8).clamp(3, 12);
    let scroll = selected.saturating_add(1).saturating_sub(max_items);
    let label_width = width.saturating_sub(22).clamp(14, 42);
    let items = review
        .issues
        .iter()
        .enumerate()
        .map(|(index, issue)| {
            let line = issue
                .line
                .map(|line| format!(":{line}"))
                .unwrap_or_default();
            MenuItem::new(format!("{} {}{}", issue.severity, issue.file, line))
                .description(issue.title.clone())
                .checked(review.checked.get(index).copied().unwrap_or(false))
                .color(severity_color(&issue.severity))
        })
        .collect::<Vec<_>>();

    MenuPanel::new(format!(
        "⚑ {} — {checked}/{total} checked · {}",
        review.kind.label(),
        review.asset_dir
    ))
    .subtitle(review_menu_hint(width, review.kind).trim_start())
    .items(items)
    .selected(selected)
    .scroll(scroll)
    .max_items(max_items)
    .label_width(label_width)
    .show_scroll(total > max_items)
    .indent(2)
    .marker("▸")
    .title_color(TN_PURPLE)
    .subtitle_color(TN_GRAY)
    .text_color(TN_FG)
    .muted_color(TN_GRAY)
    .checked_color(TN_GREEN)
    .selected_colors(TN_FG, SURFACE_SELECTED)
    .view(width.min(u16::MAX as usize) as u16, max_items + 3)
    .lines()
    .map(str::to_string)
    .collect()
}

pub(crate) fn review_state(
    asset_dir: String,
    kind: ReviewReportKind,
    issues: Vec<ReviewIssue>,
) -> Option<ReviewState> {
    let n = issues.len();
    if n == 0 {
        return None;
    }
    Some(ReviewState {
        asset_dir,
        kind,
        checked: vec![false; n],
        issues,
        sel: 0,
    })
}
