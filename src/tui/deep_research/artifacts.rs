use super::deep_research_convergence::{validated_inquiry_projection, ValidatedInquiryProjection};
use super::deep_research_report_generation::{
    validate_report_obligation_coverage, GeneratedDeepResearchReport,
};
#[cfg(test)]
use super::deep_research_report_generation::{
    ReportEditorialPlan, ReportPresentation, ReportTrackCoverage, ReportTrackStatus,
};
#[cfg(test)]
use super::{accepted_evidence_ledger, deep_research_prompt_workflow_output};
use super::{
    deep_research_canonical_workflow_output, deep_research_collection_status,
    deep_research_inquiry_publication_outcome, deep_research_sanitize_evidence_text,
    deep_research_workflow_metadata_digest, deep_research_workflow_output_digest,
    RESEARCH_VIEW_MARKER,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[path = "artifacts/evidence_parser.rs"]
mod evidence_parser;
#[path = "artifacts/html.rs"]
mod html;
#[cfg(test)]
#[path = "artifacts/proposal_tests.rs"]
mod proposal_tests;
#[path = "artifacts/recovery.rs"]
mod recovery;
#[cfg(test)]
#[path = "artifacts/source_backed_tests.rs"]
mod source_backed_tests;
#[path = "artifacts/sources.rs"]
mod sources;
#[cfg(test)]
#[path = "artifacts/validation_tests.rs"]
mod validation_tests;

pub(crate) use evidence_parser::parse_embedded_structured_evidence_json;
use html::{
    deep_research_completed_report_html, deep_research_completed_report_html_with_presentation,
};
#[cfg(test)]
pub(crate) fn deep_research_completed_report_html_for_test(query: &str, markdown: &str) -> String {
    deep_research_completed_report_html(query, markdown)
}
#[cfg(test)]
pub(crate) fn deep_research_write_report_pair_for_test(
    markdown_path: &Path,
    markdown: impl AsRef<[u8]>,
    html_path: &Path,
    html: impl AsRef<[u8]>,
) -> Result<(), String> {
    write_research_report_pair(markdown_path, markdown, html_path, html)
}
use recovery::{looks_like_deep_research_recovery_report, recovery_research_report_artifacts};
use sources::{
    deep_research_workflow_evidence_omitted_count, deep_research_workflow_source_anchors,
    deep_research_workflow_source_omitted_count,
};
use std::io::Write;

// Keep the artifact pipeline in one module while splitting each concern into a
// reviewable source file without widening internal visibility.
include!("artifacts/publication.rs");
include!("artifacts/generated.rs");
include!("artifacts/resolution.rs");
include!("artifacts/quality.rs");
include!("artifacts/fallback.rs");
include!("artifacts/source_quality.rs");
include!("artifacts/source_backed.rs");
include!("artifacts/proposal.rs");
include!("artifacts/outcome_extract.rs");
include!("artifacts/artifact_tests.rs");
