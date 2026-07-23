use super::{research_spec_digest, stable_id, valid_text, ResearchContract};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const MAX_SOURCE_TITLE_CHARS: usize = 500;
const MAX_SOURCE_ANCHOR_CHARS: usize = 4_000;
const MAX_SOURCE_CHUNK_CHARS: usize = 16_000;
const MAX_ATTEMPT_REASON_CHARS: usize = 1_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceChunk {
    pub(super) id: String,
    pub(super) text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceProvenance {
    pub(super) query_id: String,
    pub(super) source_target_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceRecord {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) requested_anchor: String,
    pub(super) canonical_anchor: String,
    pub(super) captured_at: String,
    pub(super) provenance: Vec<SourceProvenance>,
    pub(super) chunks: Vec<SourceChunk>,
    pub(super) content_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub(super) enum AcquisitionOutcome {
    Fetched,
    NoCandidates,
    Failed { reason: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AcquisitionAttempt {
    pub(super) query_id: String,
    pub(super) source_target_ids: Vec<String>,
    pub(super) outcome: AcquisitionOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceCatalog {
    pub(super) spec_digest: String,
    pub(super) attempts: Vec<AcquisitionAttempt>,
    pub(super) sources: Vec<SourceRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum CatalogError {
    #[error("source catalog belongs to a different research spec")]
    SpecDigestMismatch,
    #[error("source catalog contains invalid field `{field}`")]
    InvalidField { field: &'static str },
    #[error("duplicate source ID `{source_id}`")]
    DuplicateSourceId { source_id: String },
    #[error("duplicate source chunk ID `{chunk_id}`")]
    DuplicateChunkId { chunk_id: String },
    #[error("catalog references unknown query `{query_id}`")]
    UnknownQuery { query_id: String },
    #[error("catalog references unknown source target `{target_id}`")]
    UnknownTarget { target_id: String },
    #[error("query `{query_id}` did not declare target `{target_id}`")]
    QueryTargetMismatch { query_id: String, target_id: String },
    #[error("source `{source_id}` has no valid query/target provenance edge")]
    MissingSourceProvenance { source_id: String },
    #[error(
        "source `{source_id}` provenance edge `{query_id}` -> `{target_id}` has no fetched attempt"
    )]
    MissingFetchedAttempt {
        source_id: String,
        query_id: String,
        target_id: String,
    },
    #[error("source `{source_id}` content digest does not match its immutable chunks")]
    ContentDigestMismatch { source_id: String },
}

pub(super) fn source_content_digest(chunks: &[SourceChunk]) -> String {
    let encoded = serde_json::to_vec(chunks).expect("SourceChunk serialization is infallible");
    format!("{:x}", Sha256::digest(encoded))
}

pub(super) fn validate_source_catalog(
    contract: &ResearchContract,
    catalog: &SourceCatalog,
) -> Result<(), CatalogError> {
    if catalog.spec_digest != research_spec_digest(&contract.spec) {
        return Err(CatalogError::SpecDigestMismatch);
    }

    for attempt in &catalog.attempts {
        let Some(query) = contract.query(&attempt.query_id) else {
            return Err(CatalogError::UnknownQuery {
                query_id: attempt.query_id.clone(),
            });
        };
        if attempt.source_target_ids.is_empty() || has_duplicates(&attempt.source_target_ids) {
            return Err(CatalogError::InvalidField {
                field: "attempt_source_target_ids",
            });
        }
        for target_id in &attempt.source_target_ids {
            if contract.target(target_id).is_none() {
                return Err(CatalogError::UnknownTarget {
                    target_id: target_id.clone(),
                });
            }
            if !query.source_target_ids.contains(target_id) {
                return Err(CatalogError::QueryTargetMismatch {
                    query_id: query.id.clone(),
                    target_id: target_id.clone(),
                });
            }
        }
        if let AcquisitionOutcome::Failed { reason } = &attempt.outcome {
            if !valid_text(reason, MAX_ATTEMPT_REASON_CHARS) {
                return Err(CatalogError::InvalidField {
                    field: "attempt_failure_reason",
                });
            }
        }
    }

    let mut source_ids = BTreeSet::new();
    let mut chunk_ids = BTreeSet::new();
    for source in &catalog.sources {
        if !stable_id(&source.id)
            || !valid_text(&source.title, MAX_SOURCE_TITLE_CHARS)
            || !valid_text(&source.requested_anchor, MAX_SOURCE_ANCHOR_CHARS)
            || !valid_text(&source.canonical_anchor, MAX_SOURCE_ANCHOR_CHARS)
            || source.chunks.is_empty()
            || has_duplicate_provenance(&source.provenance)
        {
            return Err(CatalogError::InvalidField {
                field: "source_record",
            });
        }
        if chrono::DateTime::parse_from_rfc3339(&source.captured_at).is_err() {
            return Err(CatalogError::InvalidField {
                field: "captured_at",
            });
        }
        if !source_ids.insert(source.id.as_str()) {
            return Err(CatalogError::DuplicateSourceId {
                source_id: source.id.clone(),
            });
        }

        if source.provenance.is_empty() {
            return Err(CatalogError::MissingSourceProvenance {
                source_id: source.id.clone(),
            });
        }
        for provenance in &source.provenance {
            let Some(query) = contract.query(&provenance.query_id) else {
                return Err(CatalogError::UnknownQuery {
                    query_id: provenance.query_id.clone(),
                });
            };
            if contract.target(&provenance.source_target_id).is_none() {
                return Err(CatalogError::UnknownTarget {
                    target_id: provenance.source_target_id.clone(),
                });
            }
            if !query
                .source_target_ids
                .contains(&provenance.source_target_id)
            {
                return Err(CatalogError::QueryTargetMismatch {
                    query_id: provenance.query_id.clone(),
                    target_id: provenance.source_target_id.clone(),
                });
            }
            let has_fetched_attempt = catalog.attempts.iter().any(|attempt| {
                attempt.query_id == provenance.query_id
                    && attempt
                        .source_target_ids
                        .contains(&provenance.source_target_id)
                    && attempt.outcome == AcquisitionOutcome::Fetched
            });
            if !has_fetched_attempt {
                return Err(CatalogError::MissingFetchedAttempt {
                    source_id: source.id.clone(),
                    query_id: provenance.query_id.clone(),
                    target_id: provenance.source_target_id.clone(),
                });
            }
        }

        for chunk in &source.chunks {
            if !stable_id(&chunk.id) || !valid_text(&chunk.text, MAX_SOURCE_CHUNK_CHARS) {
                return Err(CatalogError::InvalidField {
                    field: "source_chunk",
                });
            }
            if !chunk_ids.insert(chunk.id.as_str()) {
                return Err(CatalogError::DuplicateChunkId {
                    chunk_id: chunk.id.clone(),
                });
            }
        }
        if source.content_digest != source_content_digest(&source.chunks) {
            return Err(CatalogError::ContentDigestMismatch {
                source_id: source.id.clone(),
            });
        }
    }
    Ok(())
}

fn has_duplicates(values: &[String]) -> bool {
    let mut seen = BTreeSet::new();
    values.iter().any(|value| !seen.insert(value))
}

fn has_duplicate_provenance(values: &[SourceProvenance]) -> bool {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .any(|value| !seen.insert((value.query_id.as_str(), value.source_target_id.as_str())))
}
