//! Reviewer / checklist chrome copy (pure strings for UX hermetics).

use super::report::ReviewReportKind;

/// Lane enqueue acknowledgement (manual or sticky).
pub(crate) fn reviewer_lane_enqueued_line(
    origin: &str,
    display: &str,
    depth: usize,
) -> String {
    format!("  ⚖ reviewer lane · {origin} · {display} · depth {depth}")
}

/// Side-session admitted — main stream stays free.
pub(crate) fn reviewer_started_line(display: &str) -> String {
    format!("  ⚖ reviewer started · {display} · async bus (main stream free)")
}

pub(crate) fn reviewer_empty_finish_line() -> &'static str {
    "  ⚖ reviewer finished with empty output"
}

pub(crate) fn reviewer_failed_finish_prefix() -> &'static str {
    "  ⚖ "
}

/// How a finished ReviewerLane answer should be handled (pure; no App I/O).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BackgroundReviewFinishKind {
    Empty,
    Failed,
    Capture,
}

pub(crate) fn classify_background_review_finish(text: &str) -> BackgroundReviewFinishKind {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        BackgroundReviewFinishKind::Empty
    } else if trimmed.starts_with("reviewer failed:") {
        BackgroundReviewFinishKind::Failed
    } else {
        BackgroundReviewFinishKind::Capture
    }
}

pub(crate) fn deferred_checklist_ready_line() -> &'static str {
    "  ⚑ review checklist ready"
}

pub(crate) fn review_found_issues_action(kind: ReviewReportKind) -> &'static str {
    match kind {
        ReviewReportKind::Code => "pick which to fix",
        ReviewReportKind::Reply => "pick which to address (w waives open)",
    }
}

pub(crate) fn review_checklist_close_message(kind: ReviewReportKind) -> &'static str {
    match kind {
        ReviewReportKind::Code => "  issue checklist closed — run /review again to reopen",
        ReviewReportKind::Reply => {
            "  reply-review checklist closed — open findings still inject on next turn (w to waive)"
        }
    }
}

pub(crate) fn review_waive_message(n: usize) -> String {
    format!(
        "  ⚖ waived {n} open reply-review finding(s) — next turns will not inject them"
    )
}

pub(crate) fn prefer_hub_tip_line(preferred: &str) -> String {
    format!("  prefer {preferred}")
}

pub(crate) fn memory_panel_loading_note() -> &'static str {
    "loading…"
}

/// `/reviewer` toggle — sticky claim-vs-record (not git `/review`).
pub(crate) fn reviewer_mode_on_notice() -> &'static str {
    "  reviewer · sticky async claim-vs-record reply verifier on priority lane · uses turn tool evidence · never blocks the main agent stream · Shift+Tab cycles"
}

pub(crate) fn reviewer_mode_off_notice() -> &'static str {
    "  reviewer off · background reviews disarmed"
}

pub(crate) fn reply_review_finished_notice(findings: usize, open: usize) -> String {
    format!(
        "⚖ reply review finished · {findings} finding(s) · {open} open for next-turn injection"
    )
}

/// Open the checklist immediately only when the composer and queue are idle.
pub(crate) fn should_open_review_checklist_immediately(
    composer_empty: bool,
    queue_empty: bool,
) -> bool {
    composer_empty && queue_empty
}

/// Admit a deferred checklist once the composer/queue clear and state is valid.
pub(crate) fn should_admit_deferred_review_checklist(
    deferred: bool,
    already_open: bool,
    has_review: bool,
    composer_empty: bool,
    queue_empty: bool,
) -> bool {
    deferred && !already_open && has_review && composer_empty && queue_empty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewer_bus_chrome_keeps_main_stream_free_and_names_async_bus() {
        let started = reviewer_started_line("reply review · turn");
        assert!(started.contains("async bus"));
        assert!(started.contains("main stream free"));
        assert!(started.contains("⚖ reviewer started"));
        assert!(!started.contains("blocked"));
    }

    #[test]
    fn reviewer_lane_and_finish_chrome_are_compact() {
        let enqueued = reviewer_lane_enqueued_line("sticky", "reply review", 2);
        assert!(enqueued.contains("reviewer lane · sticky"));
        assert!(enqueued.contains("depth 2"));
        assert!(reviewer_empty_finish_line().contains("empty output"));
        assert!(reviewer_failed_finish_prefix().starts_with("  ⚖"));
        assert_eq!(
            classify_background_review_finish(""),
            BackgroundReviewFinishKind::Empty
        );
        assert_eq!(
            classify_background_review_finish("  \n"),
            BackgroundReviewFinishKind::Empty
        );
        assert_eq!(
            classify_background_review_finish("reviewer failed: timeout"),
            BackgroundReviewFinishKind::Failed
        );
        assert_eq!(
            classify_background_review_finish("```a3s-review\n{}\n```"),
            BackgroundReviewFinishKind::Capture
        );
    }

    #[test]
    fn deferred_checklist_and_close_copy_split_reply_vs_code() {
        assert_eq!(
            deferred_checklist_ready_line(),
            "  ⚑ review checklist ready"
        );
        assert!(review_checklist_close_message(ReviewReportKind::Code).contains("/review"));
        let reply_close = review_checklist_close_message(ReviewReportKind::Reply);
        assert!(reply_close.contains("w to waive"));
        assert!(reply_close.contains("still inject"));
        assert!(!reply_close.contains("/review again"));
        assert!(review_found_issues_action(ReviewReportKind::Reply).contains("waive"));
        assert!(review_waive_message(3).contains("waived 3"));
    }

    #[test]
    fn deferred_checklist_policy_waits_for_idle_composer_and_queue() {
        assert!(should_open_review_checklist_immediately(true, true));
        assert!(!should_open_review_checklist_immediately(false, true));
        assert!(!should_open_review_checklist_immediately(true, false));
        assert!(should_admit_deferred_review_checklist(
            true, false, true, true, true
        ));
        assert!(!should_admit_deferred_review_checklist(
            true, false, true, false, true
        ));
        assert!(!should_admit_deferred_review_checklist(
            true, true, true, true, true
        ));
        assert!(!should_admit_deferred_review_checklist(
            true, false, false, true, true
        ));
        assert!(!should_admit_deferred_review_checklist(
            false, false, true, true, true
        ));
    }

    #[test]
    fn memory_hub_tip_and_loading_note() {
        assert_eq!(
            prefer_hub_tip_line("/ctx memory"),
            "  prefer /ctx memory"
        );
        assert_eq!(memory_panel_loading_note(), "loading…");
    }

    #[test]
    fn reviewer_mode_notice_is_claim_vs_record_not_git_review() {
        let on = reviewer_mode_on_notice();
        assert!(on.contains("claim-vs-record"));
        assert!(on.contains("reply verifier"));
        assert!(on.contains("never blocks the main agent stream"));
        assert!(on.contains("tool evidence"));
        assert!(!on.contains("working-tree"));
        assert!(!on.contains("async code review"));
        assert!(!on.to_ascii_lowercase().contains("git diff"));
        assert_eq!(
            reviewer_mode_off_notice(),
            "  reviewer off · background reviews disarmed"
        );
    }

    #[test]
    fn reply_capture_finish_names_open_injection() {
        let notice = reply_review_finished_notice(2, 1);
        assert!(notice.contains("reply review finished"));
        assert!(notice.contains("2 finding(s)"));
        assert!(notice.contains("1 open for next-turn injection"));
        assert!(!notice.contains("working-tree"));
        assert!(!notice.contains("/review again"));
    }
}
