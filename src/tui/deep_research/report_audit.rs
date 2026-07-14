//! Deterministic claim/source audit for a materialized research report.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReportAudit {
    pub(crate) passed: bool,
    pub(crate) accepted_claims: usize,
    pub(crate) matched_claims: usize,
    pub(crate) claim_coverage_basis_points: u16,
    pub(crate) accepted_sources: usize,
    pub(crate) cited_sources: usize,
    pub(crate) reason: String,
}

pub(crate) fn audit_report(
    markdown: &str,
    html: &str,
    claims: &[String],
    source_anchors: &[String],
) -> ReportAudit {
    if claims.is_empty() && source_anchors.is_empty() {
        return ReportAudit {
            passed: true,
            accepted_claims: 0,
            matched_claims: 0,
            claim_coverage_basis_points: 10_000,
            accepted_sources: 0,
            cited_sources: 0,
            reason: "legacy report has no event-sourced evidence graph to audit".to_string(),
        };
    }
    let report = normalize(&format!("{markdown}\n{html}"));
    let matched_claims = claims
        .iter()
        .filter(|claim| claim_matches(&report, claim))
        .count();
    let claim_coverage_basis_points = if claims.is_empty() {
        10_000
    } else {
        ((matched_claims.saturating_mul(10_000) / claims.len()).min(10_000)) as u16
    };
    let cited_sources = source_anchors
        .iter()
        .filter(|anchor| markdown.contains(anchor.as_str()) || html.contains(anchor.as_str()))
        .count();
    let sources_pass = !source_anchors.is_empty() && cited_sources > 0;
    let claims_pass = claims.is_empty() || claim_coverage_basis_points >= 5_000;
    let passed = sources_pass && claims_pass;
    let reason = if !sources_pass {
        "report cites none of the accepted evidence sources"
    } else if !claims_pass {
        "report covers less than half of the accepted claims"
    } else {
        "report claims and citations trace to accepted evidence"
    };
    ReportAudit {
        passed,
        accepted_claims: claims.len(),
        matched_claims,
        claim_coverage_basis_points,
        accepted_sources: source_anchors.len(),
        cited_sources,
        reason: reason.to_string(),
    }
}

fn claim_matches(report: &str, claim: &str) -> bool {
    let claim = normalize(claim);
    if claim.is_empty() || report.contains(&claim) {
        return !claim.is_empty();
    }
    let terms = claim
        .split_whitespace()
        .filter(|term| term.len() >= 3)
        .collect::<HashSet<_>>();
    if terms.is_empty() {
        return false;
    }
    let matched = terms
        .iter()
        .filter(|term| report.split_whitespace().any(|word| word == **term))
        .count();
    matched.saturating_mul(100) / terms.len() >= 60
}

fn normalize(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_report_with_source_and_claim_coverage() {
        let audit = audit_report(
            "The release date is July 12. Source: https://example.gov/release",
            "<p>The release was published on July 12.</p>",
            &["The release date is July 12.".to_string()],
            &["https://example.gov/release".to_string()],
        );
        assert!(audit.passed);
        assert_eq!(audit.claim_coverage_basis_points, 10_000);
    }

    #[test]
    fn rejects_polished_report_without_accepted_claims() {
        let audit = audit_report(
            "A polished but unrelated conclusion. https://example.gov/release",
            "<p>Unrelated analysis.</p>",
            &["The release date is July 12.".to_string()],
            &["https://example.gov/release".to_string()],
        );
        assert!(!audit.passed);
        assert!(audit.reason.contains("less than half"));
    }
}
