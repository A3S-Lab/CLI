#[cfg(test)]
use super::deep_research_prompt_workflow_output;
use super::deep_research_report_generation::{GeneratedDeepResearchReport, ReportTrackStatus};
#[cfg(test)]
use super::deep_research_report_generation::{
    ReportEditorialPlan, ReportPresentation, ReportTrackCoverage,
};
use super::{
    deep_research_canonical_workflow_output, deep_research_collection_status,
    deep_research_inquiry_publication_outcome, deep_research_sanitize_evidence_text,
    deep_research_workflow_metadata_digest, deep_research_workflow_output_digest,
    RESEARCH_VIEW_MARKER,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[cfg(test)]
#[path = "artifacts/evidence.rs"]
mod evidence;
#[path = "artifacts/evidence_parser.rs"]
mod evidence_parser;
#[path = "artifacts/html.rs"]
mod html;
#[path = "artifacts/recovery.rs"]
mod recovery;
#[path = "artifacts/sources.rs"]
mod sources;
#[cfg(test)]
#[path = "artifacts/validation_tests.rs"]
mod validation_tests;

#[cfg(test)]
use evidence::{
    completed_report_markdown_with_verified_context,
    deep_research_structured_evidence_from_workflow,
};
pub(crate) use evidence_parser::parse_embedded_structured_evidence_json;
use html::{
    deep_research_completed_report_html, deep_research_completed_report_html_with_presentation,
};
use recovery::{looks_like_deep_research_recovery_report, recovery_research_report_artifacts};
use sources::{
    deep_research_workflow_evidence_omitted_count, deep_research_workflow_source_anchors,
    deep_research_workflow_source_omitted_count,
};
use std::io::Write;
use std::time::SystemTime;

// Keep the artifact pipeline in one module while splitting each concern into a
// reviewable source file without widening internal visibility.
include!("artifacts/publication.rs");
include!("artifacts/generated.rs");
include!("artifacts/resolution.rs");
include!("artifacts/quality.rs");
include!("artifacts/fallback.rs");
include!("artifacts/artifact_tests.rs");
