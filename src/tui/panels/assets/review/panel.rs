//! App handlers for review capture, checklist keys, and overlay.

use super::super::super::*;
use super::checklist::{review_menu_lines, review_state};
use super::chrome::{
    deferred_checklist_ready_line, reply_review_finished_notice, review_checklist_close_message,
    review_found_issues_action, review_waive_message, should_admit_deferred_review_checklist,
    should_open_review_checklist_immediately,
};
use super::report::{
    next_open_reply_findings_after_capture, parse_review_report, review_address_reply_prompt,
    review_fix_prompt, ReviewIssue, ReviewReportKind, REVIEW_FENCE,
};

impl App {
    /// Scan a finished review turn for the report; on a hit, end the
    /// review loop and open (or refresh) the issue checklist. Gated on
    /// `review_pending` so a turn that merely quotes an a3s-review block
    /// (docs, examples, reviewing this source tree) can't open a phantom panel.
    pub(crate) fn capture_review(&mut self, text: &str) {
        if !self.review_pending || !text.contains(REVIEW_FENCE) {
            return;
        }
        let Some((asset_dir, reported_kind, issues)) = parse_review_report(text) else {
            // The agent tried but the block is malformed: say so (silence here
            // is indistinguishable from "no report") and stay pending, so a
            // loop continuation or a manual "re-emit the report" can land it.
            self.push_line(&Style::new().fg(TN_YELLOW).render(
                "  ⚑ review report was malformed — ask the agent to re-emit the a3s-review block",
            ));
            return;
        };
        let kind = self.review_pending_kind.take().unwrap_or(reported_kind);
        // The deliverable arrived — stop the loop that was driving it.
        self.review_pending = false;
        self.loop_remaining = 0;
        self.review_open = false;
        let n = issues.len();
        self.open_reply_findings = next_open_reply_findings_after_capture(
            kind,
            &issues,
            std::mem::take(&mut self.open_reply_findings),
        );
        if kind == ReviewReportKind::Reply {
            self.messages.push(TranscriptEntry::notice(
                NoticeKind::Info,
                reply_review_finished_notice(n, self.open_reply_findings.len()),
            ));
        }
        self.review = review_state(asset_dir, kind, issues);
        if n == 0 {
            self.push_line(
                &Style::new()
                    .fg(TN_GREEN)
                    .render(&format!("  ✔ {}: no issues found", kind.label())),
            );
            return;
        }
        // Only pop the checklist open when nothing else is going on: the panel
        // consumes every key, so opening over in-flight typing or a queued
        // message would steal keystrokes ('a' = check all, Enter = fix!).
        if should_open_review_checklist_immediately(
            self.textarea.value().is_empty(),
            self.queue.is_empty(),
        ) {
            self.review_open = true;
            self.review_checklist_deferred = false;
        } else {
            self.review_checklist_deferred = true;
        }
        let hint = if self.review_open {
            ""
        } else {
            " · clears when the composer/queue is idle"
        };
        let action = review_found_issues_action(kind);
        self.push_line(&gutter(
            TN_PURPLE,
            &Style::new().bold().render(&format!(
                "⚑ {} found {n} issues — {action}{hint}",
                kind.label()
            )),
        ));
    }

    /// Open a deferred issue checklist once the composer and queue are idle.
    pub(crate) fn maybe_open_deferred_review_checklist(&mut self) {
        if !should_admit_deferred_review_checklist(
            self.review_checklist_deferred,
            self.review_open,
            self.review.is_some(),
            self.textarea.value().is_empty(),
            self.queue.is_empty(),
        ) {
            if self.review_checklist_deferred && self.review.is_none() {
                self.review_checklist_deferred = false;
            }
            return;
        }
        self.review_open = true;
        self.review_checklist_deferred = false;
        self.push_line(
            &Style::new()
                .fg(TN_PURPLE)
                .render(deferred_checklist_ready_line()),
        );
    }

    /// Keys while the checklist is open — consumes everything so nothing leaks
    /// to the input box behind the overlay.
    pub(crate) fn handle_review_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let Some(r) = self.review.as_mut() else {
            self.review_open = false;
            return None;
        };
        let kind = r.kind;
        let last = r.issues.len().saturating_sub(1);
        match key.code {
            KeyCode::Up => r.sel = r.sel.saturating_sub(1),
            KeyCode::Down => r.sel = (r.sel + 1).min(last),
            KeyCode::Char(' ') => r.checked[r.sel] = !r.checked[r.sel],
            // `a` checks everything (or unchecks, if everything is checked).
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let all = r.checked.iter().all(|c| *c);
                for c in r.checked.iter_mut() {
                    *c = !all;
                }
            }
            KeyCode::Char('w') | KeyCode::Char('W') if kind == ReviewReportKind::Reply => {
                let n = self.open_reply_findings.len();
                self.open_reply_findings.clear();
                if let Some(review) = self.review.as_mut() {
                    for issue in &mut review.issues {
                        issue.status = "waived".to_string();
                    }
                }
                self.review_open = false;
                self.push_line(&Style::new().fg(TN_GRAY).render(&review_waive_message(n)));
            }
            KeyCode::Esc => {
                self.review_open = false;
                self.push_line(
                    &Style::new()
                        .fg(TN_GRAY)
                        .render(review_checklist_close_message(kind)),
                );
            }
            KeyCode::Enter => {
                let picked: Vec<ReviewIssue> = r
                    .issues
                    .iter()
                    .zip(&r.checked)
                    .filter(|(_, c)| **c)
                    .map(|(i, _)| i.clone())
                    .collect();
                if picked.is_empty() {
                    let msg = match kind {
                        ReviewReportKind::Code => {
                            "  select issues with Space, then press Enter to fix"
                        }
                        ReviewReportKind::Reply => {
                            "  select findings with Space, then press Enter to address"
                        }
                    };
                    self.push_line(&Style::new().fg(TN_GRAY).render(msg));
                    return None;
                }
                for c in r.checked.iter_mut() {
                    *c = false;
                }
                let asset_dir = r.asset_dir.clone();
                let total = r.issues.len();
                self.review_open = false;
                let (prompt, label) = match kind {
                    ReviewReportKind::Code => (
                        review_fix_prompt(&asset_dir, &picked),
                        format!("🛠 fixing {}/{total} review issues", picked.len()),
                    ),
                    ReviewReportKind::Reply => {
                        let titles: Vec<String> =
                            picked.iter().map(|issue| issue.title.clone()).collect();
                        self.open_reply_findings
                            .retain(|issue| !titles.iter().any(|title| title == &issue.title));
                        (
                            review_address_reply_prompt(&picked),
                            format!(
                                "⚖ addressing {}/{total} reply-review findings",
                                picked.len()
                            ),
                        )
                    }
                };
                self.messages.push(TranscriptEntry::preformatted(gutter(
                    TN_PURPLE,
                    &Style::new().bold().render(&label),
                )));
                let execution_mode = self.mode;
                self.enqueue_turn(
                    SYNTHETIC_TURN_PRIORITY,
                    Queued {
                        text: prompt,
                        display: label,
                        images: Vec::new(),
                        pastes: Vec::new(),
                        runtime_expectation: None,
                        deep_research: None,
                        transcript_posted: true,
                    },
                    execution_mode,
                );
                if self.state == State::Idle {
                    return self.drain_queue();
                }
                self.push_line(&Style::new().fg(TN_GRAY).render("    ⋯ queued"));
            }
            _ => {}
        }
        None
    }

    /// Checkbox panel above the input: one row per issue, Space toggles.
    pub(crate) fn overlay_review_menu(&self, composed: String) -> String {
        if !self.review_open {
            return composed;
        }
        let Some(r) = self.review.as_ref() else {
            return composed;
        };
        let width = self.width as usize;
        let menu = review_menu_lines(r, width, self.height as usize);
        self.overlay_list(composed, &menu)
    }
}
