//! Host-accepted evidence relationships used by inquiry replay.

use serde::{Deserialize, Serialize};

/// A semantic role carried by one source on a specific research-obligation
/// path. The Host never infers these roles from a URL, title, language, or
/// other lexical metadata.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceEvidenceRole {
    Supporting,
    Primary,
    Independent,
}

/// A closed, host-validated source-to-obligation edge produced while the
/// source text is still present in the semantic selection packet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCoverageBinding {
    pub source_id: String,
    pub obligation_id: String,
    pub completion_criterion_indexes: Vec<usize>,
    pub roles: Vec<SourceEvidenceRole>,
}

impl SourceCoverageBinding {
    pub fn new(
        source_id: impl Into<String>,
        obligation_id: impl Into<String>,
        completion_criterion_indexes: Vec<usize>,
        roles: Vec<SourceEvidenceRole>,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            obligation_id: obligation_id.into(),
            completion_criterion_indexes,
            roles,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceDiagnosticKind {
    Contradiction,
    Gap,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceDiagnostic {
    pub id: String,
    pub kind: EvidenceDiagnosticKind,
    pub detail: String,
}

impl EvidenceDiagnostic {
    pub fn new(
        id: impl Into<String>,
        kind: EvidenceDiagnosticKind,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            detail: detail.into(),
        }
    }
}

/// One accepted evidence item and the claim/source IDs it makes addressable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub evidence_id: String,
    pub claim_ids: Vec<String>,
    pub source_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_coverage: Vec<SourceCoverageBinding>,
    #[serde(default)]
    pub diagnostics: Vec<EvidenceDiagnostic>,
}

impl EvidenceRef {
    pub fn new(
        evidence_id: impl Into<String>,
        claim_ids: Vec<String>,
        source_ids: Vec<String>,
    ) -> Self {
        Self {
            evidence_id: evidence_id.into(),
            claim_ids,
            source_ids,
            source_coverage: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn with_source_coverage(mut self, source_coverage: Vec<SourceCoverageBinding>) -> Self {
        self.source_coverage = source_coverage;
        self
    }

    pub fn with_diagnostics(mut self, diagnostics: Vec<EvidenceDiagnostic>) -> Self {
        self.diagnostics = diagnostics;
        self
    }
}
