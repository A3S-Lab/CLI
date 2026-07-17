//! Host-accepted evidence relationships used by inquiry replay.

use serde::{Deserialize, Serialize};

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
            diagnostics: Vec::new(),
        }
    }

    pub fn with_diagnostics(mut self, diagnostics: Vec<EvidenceDiagnostic>) -> Self {
        self.diagnostics = diagnostics;
        self
    }
}
