//! Host-accepted evidence relationships used by inquiry replay.

use serde::{Deserialize, Serialize};

/// One accepted evidence item and the claim/source IDs it makes addressable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub evidence_id: String,
    pub claim_ids: Vec<String>,
    pub source_ids: Vec<String>,
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
        }
    }
}
