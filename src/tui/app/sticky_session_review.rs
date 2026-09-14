//! Sticky reply findings ↔ Core `session_review` (Desktop authority model).
//!
//! Desktop Science Auto-review stores Findings on [`AgentSession`]'s durable
//! session-review store under `reply.transcript`. Code TUI sticky `/reviewer`
//! uses the same store as authority; `App.open_reply_findings` is a UI
//! projection synced from pending Findings.

use a3s_code_core::{
    AgentSession, SessionReviewAnchorV1, SessionReviewFindingV1, SessionReviewSeverityV1,
    SCENARIO_REPLY_TRANSCRIPT,
};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::tui::panels::review::ReviewIssue;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn severity_from_issue(issue: &ReviewIssue) -> SessionReviewSeverityV1 {
    match issue.severity.trim().to_ascii_lowercase().as_str() {
        "blocker" => SessionReviewSeverityV1::Blocker,
        "high" | "error" => SessionReviewSeverityV1::Error,
        "medium" | "warning" | "warn" => SessionReviewSeverityV1::Warning,
        _ => SessionReviewSeverityV1::Info,
    }
}

fn category_from_issue(issue: &ReviewIssue) -> String {
    let title = issue.title.trim();
    if let Some((category, _)) = title.split_once(':') {
        let category = category.trim();
        if !category.is_empty() {
            return category.to_string();
        }
    }
    if issue.verdict.eq_ignore_ascii_case("fail") {
        "claim_record".to_string()
    } else {
        "other".to_string()
    }
}

fn stable_finding_id(issue: &ReviewIssue, index: usize) -> String {
    let explicit = issue.finding_id.trim();
    if !explicit.is_empty() {
        return explicit.to_string();
    }
    let digest = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(issue.title.as_bytes());
        hasher.update(b"\0");
        hasher.update(issue.detail.as_bytes());
        hasher.update(b"\0");
        hasher.update(index.to_string().as_bytes());
        format!("{:x}", hasher.finalize())
    };
    format!("reply-{}", &digest[..12.min(digest.len())])
}

fn issue_from_finding(finding: &SessionReviewFindingV1) -> ReviewIssue {
    let severity = match finding.severity {
        SessionReviewSeverityV1::Blocker => "blocker",
        SessionReviewSeverityV1::Error => "high",
        SessionReviewSeverityV1::Warning => "medium",
        SessionReviewSeverityV1::Info => "low",
    }
    .to_string();
    ReviewIssue {
        finding_id: finding.finding_id.clone(),
        severity,
        file: "assistant-reply".to_string(),
        line: None,
        title: if finding.category.is_empty() {
            finding.claim.clone()
        } else {
            format!("{}: {}", finding.category, finding.claim)
        },
        detail: finding.evidence.clone(),
        verdict: "fail".to_string(),
        evidence_refs: Vec::new(),
        status: "open".to_string(),
    }
}

/// Desktop Accept reconcile: empty re-review accepts all addressed; otherwise
/// reopen only when the same stable id is still reported open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AcceptanceIdDecision {
    Accept,
    Reopen,
}

pub(crate) fn acceptance_decision_for_addressed_id(
    addressed_id: &str,
    still_reported_ids: &HashSet<&str>,
    accept_batch_empty: bool,
) -> AcceptanceIdDecision {
    if accept_batch_empty || !still_reported_ids.contains(addressed_id) {
        AcceptanceIdDecision::Accept
    } else {
        AcceptanceIdDecision::Reopen
    }
}

/// Pending Core finding ids that a replacing reply report must waive.
pub(crate) fn finding_ids_to_waive_on_replace(
    pending_ids: &[String],
    reported_open_ids: &HashSet<String>,
) -> Vec<String> {
    pending_ids
        .iter()
        .filter(|id| !reported_open_ids.contains(*id))
        .cloned()
        .collect()
}

/// Upsert open sticky findings into the durable session-review store.
///
/// Reply reports **replace** the open set: pending Core findings whose ids are
/// not in `issues` are waived so a clean / superseded sticky verdict cannot keep
/// injecting stale Findings (false-complete).
pub(crate) fn upsert_open_reply_findings(
    session: &AgentSession,
    issues: &[ReviewIssue],
    source_review_id: &str,
) -> Vec<ReviewIssue> {
    let session_id = session.session_id().to_string();
    let observed_at_ms = now_ms();
    let mut projected = Vec::new();
    let mut claimed_ids = HashSet::new();
    for (index, issue) in issues.iter().enumerate() {
        let status = issue.status.trim().to_ascii_lowercase();
        if !(status.is_empty() || status == "open") {
            continue;
        }
        let finding_id = stable_finding_id(issue, index);
        if !claimed_ids.insert(finding_id.clone()) {
            continue;
        }
        let Ok(anchor) = SessionReviewAnchorV1::new(format!("sticky-{source_review_id}")) else {
            continue;
        };
        let Ok(mut finding) = SessionReviewFindingV1::new_transcript(
            finding_id.clone(),
            session_id.clone(),
            severity_from_issue(issue),
            category_from_issue(issue),
            issue.title.clone(),
            if issue.detail.trim().is_empty() {
                issue.title.clone()
            } else {
                issue.detail.clone()
            },
            anchor,
            source_review_id,
            observed_at_ms,
        ) else {
            continue;
        };
        if let Ok(tagged) = finding.clone().with_scenario_id(SCENARIO_REPLY_TRANSCRIPT) {
            finding = tagged;
        }
        if session.upsert_session_review_finding(finding).is_err() {
            continue;
        }
        let mut projected_issue = issue.clone();
        projected_issue.finding_id = finding_id;
        projected_issue.status = "open".to_string();
        projected.push(projected_issue);
    }

    let pending_ids: Vec<String> = session
        .pending_session_review_findings_for_scenario(SCENARIO_REPLY_TRANSCRIPT)
        .iter()
        .map(|finding| finding.finding_id.clone())
        .collect();
    for orphan_id in finding_ids_to_waive_on_replace(&pending_ids, &claimed_ids) {
        let _ = session.waive_session_review_finding(&orphan_id, observed_at_ms);
    }

    projected
}

/// After a sticky re-review, accept addressed findings that cleared or reopen
/// those still reported (Desktop `reconcile_addressed_findings_by_id`).
pub(crate) fn reconcile_addressed_reply_findings(
    session: &AgentSession,
    still_open_ids: &[String],
    source_review_id: &str,
) {
    let still: HashSet<&str> = still_open_ids.iter().map(String::as_str).collect();
    let accept_batch_empty = still.is_empty();
    let at = now_ms();
    for addressed in session.addressed_session_review_findings() {
        if addressed.scenario_id != SCENARIO_REPLY_TRANSCRIPT {
            continue;
        }
        match acceptance_decision_for_addressed_id(
            addressed.finding_id.as_str(),
            &still,
            accept_batch_empty,
        ) {
            AcceptanceIdDecision::Accept => {
                let _ = session.accept_session_review_finding(
                    &addressed.finding_id,
                    source_review_id,
                    at,
                );
            }
            AcceptanceIdDecision::Reopen => {
                let _ = session.reopen_session_review_finding(
                    &addressed.finding_id,
                    "still present after re-review",
                    at,
                );
            }
        }
    }
}

/// Project pending `reply.transcript` findings for UI + inject.
pub(crate) fn pending_open_reply_findings(session: &AgentSession) -> Vec<ReviewIssue> {
    session
        .pending_session_review_findings_for_scenario(SCENARIO_REPLY_TRANSCRIPT)
        .iter()
        .map(issue_from_finding)
        .collect()
}

/// Refresh the UI projection from Core (Desktop session-authoritative inject).
pub(crate) fn sync_open_reply_findings(session: &AgentSession) -> Vec<ReviewIssue> {
    pending_open_reply_findings(session)
}

/// Waive every currently open sticky finding on the session store.
pub(crate) fn waive_open_reply_findings(session: &AgentSession) -> usize {
    let pending = session.pending_session_review_findings_for_scenario(SCENARIO_REPLY_TRANSCRIPT);
    let at = now_ms();
    let mut count = 0usize;
    for finding in pending {
        if session
            .waive_session_review_finding(&finding.finding_id, at)
            .is_ok()
        {
            count += 1;
        }
    }
    count
}

/// Mark selected sticky findings addressed after a main Address turn.
pub(crate) fn mark_reply_findings_addressed(
    session: &AgentSession,
    finding_ids: &[String],
    run_id: &str,
) {
    let at = now_ms();
    for finding_id in finding_ids {
        let _ = session.mark_session_review_addressed(finding_id, run_id, at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acceptance_reconcile_accepts_when_clean_or_id_cleared() {
        let still = HashSet::from(["f2"]);
        assert_eq!(
            acceptance_decision_for_addressed_id("f1", &still, false),
            AcceptanceIdDecision::Accept
        );
        assert_eq!(
            acceptance_decision_for_addressed_id("f2", &still, false),
            AcceptanceIdDecision::Reopen
        );
        assert_eq!(
            acceptance_decision_for_addressed_id("f2", &HashSet::new(), true),
            AcceptanceIdDecision::Accept
        );
    }

    #[test]
    fn finding_ids_to_waive_on_replace_drops_orphans_only() {
        let pending = vec![
            "stale-a".to_string(),
            "keep-b".to_string(),
            "also-stale".to_string(),
        ];
        let reported = HashSet::from(["keep-b".to_string(), "new-c".to_string()]);
        let orphans = finding_ids_to_waive_on_replace(&pending, &reported);
        assert_eq!(
            orphans.into_iter().collect::<HashSet<_>>(),
            HashSet::from(["stale-a".to_string(), "also-stale".to_string()])
        );
        assert!(finding_ids_to_waive_on_replace(&pending, &HashSet::new()).len() == 3);
        assert!(finding_ids_to_waive_on_replace(&[], &reported).is_empty());
    }

    #[test]
    fn reply_replace_waives_orphan_pending_in_session_review_store() {
        // Effect: same algorithm production upsert uses (helper → Core waive)
        // against a real session-review store — not set algebra alone.
        use a3s_code_core::{SessionReviewAnchorV1, SessionReviewSeverityV1, SessionReviewStoreV1};

        let session_id = "session-orphan-replace";
        let mut store = SessionReviewStoreV1::empty(session_id).expect("empty store");
        for (id, claim) in [("stale-a", "old claim"), ("keep-b", "kept claim")] {
            let mut finding = SessionReviewFindingV1::new_transcript(
                id,
                session_id,
                SessionReviewSeverityV1::Error,
                "claim_record",
                claim,
                "evidence",
                SessionReviewAnchorV1::new(format!("turn-{id}")).expect("anchor"),
                "review-1",
                1_000,
            )
            .expect("finding");
            finding = finding
                .with_scenario_id(SCENARIO_REPLY_TRANSCRIPT)
                .expect("scenario");
            store.upsert(finding).expect("upsert");
        }
        assert_eq!(
            store.pending_for_scenario(SCENARIO_REPLY_TRANSCRIPT).len(),
            2
        );

        let pending_ids: Vec<String> = store
            .pending_for_scenario(SCENARIO_REPLY_TRANSCRIPT)
            .iter()
            .map(|finding| finding.finding_id.clone())
            .collect();
        let reported = HashSet::from(["keep-b".to_string()]);
        for orphan_id in finding_ids_to_waive_on_replace(&pending_ids, &reported) {
            store.waive(&orphan_id, 2_000).expect("waive orphan");
        }

        let remaining: Vec<&str> = store
            .pending_for_scenario(SCENARIO_REPLY_TRANSCRIPT)
            .iter()
            .map(|finding| finding.finding_id.as_str())
            .collect();
        assert_eq!(remaining, vec!["keep-b"]);
    }
}
