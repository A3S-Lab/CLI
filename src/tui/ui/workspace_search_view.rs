//! Typed terminal presentation for session-scoped workspace retrieval.
//!
//! Semantic and hybrid search metadata contains useful readiness, ranking, and
//! verification evidence alongside source anchors. Project only the bounded,
//! documented fields below so unknown provider metadata can never become TUI
//! copy or a terminal-control injection surface.

use serde_json::Value;

const MAX_RESULTS: usize = 1_000;
const MAX_CHANNELS: usize = 4;
const MAX_COMPACT_CHARS: usize = 320;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceSearchKind {
    Semantic,
    Hybrid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IndexPhase {
    Disabled,
    Building,
    Partial,
    Ready,
    Degraded,
    Closed,
}

impl IndexPhase {
    fn label(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Building => "building",
            Self::Partial => "partial",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Closed => "closed",
        }
    }

    fn is_degraded(self) -> bool {
        self != Self::Ready
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IndexStatus {
    phase: IndexPhase,
    coverage_bps: u16,
    indexed_files: usize,
    eligible_files: usize,
    indexed_chunks: usize,
    vector_records: usize,
    catalog_revision: u64,
    source_revision: u64,
    vector_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ChannelSummary {
    label: &'static str,
    candidate_count: usize,
    truncated: bool,
    fallback: Option<&'static str>,
    fallback_present: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RerankSummary {
    requested_mode: Option<&'static str>,
    applied_mode: Option<&'static str>,
    input_candidates: usize,
    evaluated_candidates: usize,
    selected_candidates: usize,
    near_duplicate_candidates: usize,
    candidate_truncated: bool,
    fallback: Option<&'static str>,
    fallback_present: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceSearchSummary {
    kind: WorkspaceSearchKind,
    returned_results: Option<usize>,
    verified_results: usize,
    results_present: bool,
    searched_records: Option<usize>,
    algorithm: Option<&'static str>,
    algorithm_present: bool,
    status: Option<IndexStatus>,
    status_present: bool,
    channels: Vec<ChannelSummary>,
    channels_present: bool,
    rerank: Option<RerankSummary>,
    fallback: Option<&'static str>,
    fallback_present: bool,
    truncated: bool,
    catalog_revision: Option<u64>,
    source_revision: Option<u64>,
}

impl WorkspaceSearchSummary {
    pub(crate) fn from_tool(
        name: &str,
        args: Option<&Value>,
        metadata: Option<&Value>,
    ) -> Option<Self> {
        let kind = workspace_search_kind(name, args)?;
        let metadata = metadata?.as_object()?;
        let recognized = match kind {
            WorkspaceSearchKind::Semantic => {
                metadata.contains_key("status")
                    || metadata.contains_key("searched_records")
                    || metadata.contains_key("results")
            }
            WorkspaceSearchKind::Hybrid => {
                metadata.contains_key("semantic_status")
                    || metadata.contains_key("channels")
                    || metadata.contains_key("rerank")
            }
        };
        if !recognized {
            return None;
        }

        let results = metadata.get("results").and_then(Value::as_array);
        let results_present = results.is_some();
        let verified_results = results
            .into_iter()
            .flatten()
            .take(MAX_RESULTS)
            .filter(|result| result.get("digest_verified").and_then(Value::as_bool) == Some(true))
            .count();
        let returned_results = bounded_usize(metadata.get("returned_results"))
            .or_else(|| results.map(Vec::len))
            .map(|count| count.min(MAX_RESULTS));
        let status_value = match kind {
            WorkspaceSearchKind::Semantic => metadata.get("status"),
            WorkspaceSearchKind::Hybrid => metadata.get("semantic_status"),
        };
        let status_present = status_value.is_some_and(|value| !value.is_null());
        let status = status_value.and_then(parse_index_status);
        let algorithm_value = metadata.get("algorithm");
        let algorithm_present = algorithm_value.is_some_and(|value| !value.is_null());
        let algorithm = algorithm_value
            .and_then(Value::as_str)
            .and_then(display_algorithm);
        let fallback_value = metadata.get("fallback");
        let fallback_present = fallback_value.is_some_and(|value| !value.is_null());
        let fallback = fallback_value
            .and_then(Value::as_str)
            .and_then(display_fallback);
        let channels_value = metadata.get("channels");
        let channels_present = channels_value.is_some_and(Value::is_array);
        let channels = channels_value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_channel)
            .take(MAX_CHANNELS)
            .collect();
        let rerank = metadata.get("rerank").and_then(parse_rerank);

        Some(Self {
            kind,
            returned_results,
            verified_results,
            results_present,
            searched_records: bounded_usize(metadata.get("searched_records")),
            algorithm,
            algorithm_present,
            status,
            status_present,
            channels,
            channels_present,
            rerank,
            fallback,
            fallback_present,
            truncated: metadata
                .get("truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            catalog_revision: bounded_u64(metadata.get("catalog_revision")),
            source_revision: bounded_u64(metadata.get("source_revision")),
        })
    }

    pub(crate) fn compact_label(&self) -> String {
        let mut parts = Vec::new();
        if let Some(results) = self.result_label() {
            parts.push(results);
        }
        match self.kind {
            WorkspaceSearchKind::Semantic => {
                if let Some(status) = &self.status {
                    let semantic = match status.phase {
                        IndexPhase::Partial => {
                            format!("semantic {}", compact_percent(status.coverage_bps))
                        }
                        phase => format!("semantic {}", phase.label()),
                    };
                    parts.push(semantic);
                } else {
                    parts.push("semantic".to_string());
                }
            }
            WorkspaceSearchKind::Hybrid if !self.channels.is_empty() => parts.push(format!(
                "{} {}",
                self.channels.len(),
                if self.channels.len() == 1 {
                    "channel"
                } else {
                    "channels"
                }
            )),
            WorkspaceSearchKind::Hybrid => parts.push("hybrid".to_string()),
        }
        if let Some(algorithm) = self.algorithm {
            parts.push(algorithm.to_string());
        } else if self.algorithm_present {
            parts.push("unrecognized ranking".to_string());
        }
        if self.kind == WorkspaceSearchKind::Hybrid {
            if let Some(status) = &self.status {
                if status.phase != IndexPhase::Ready {
                    parts.push(status.phase.label().to_string());
                }
            }
        } else if self
            .status
            .as_ref()
            .is_some_and(|status| status.phase == IndexPhase::Partial)
        {
            parts.push("partial".to_string());
        }
        if self.fallback_present {
            parts.push(format!(
                "fallback: {}",
                self.fallback.unwrap_or("unrecognized reason")
            ));
        }
        if self.truncated {
            parts.push("limited".to_string());
        }
        bounded_text(&parts.join(" · "), MAX_COMPACT_CHARS)
    }

    pub(crate) fn transcript_detail(&self) -> String {
        let mut lines = Vec::new();
        if let Some(results) = self.result_label() {
            let searched = self
                .searched_records
                .map(|records| format!(" · {records} vector records searched"))
                .unwrap_or_default();
            let limited = if self.truncated {
                " · result limit reached"
            } else {
                ""
            };
            lines.push(format!("Results: {results}{searched}{limited}"));
        }
        if let Some(status) = &self.status {
            lines.push(format!(
                "Index: {} · {} · {}/{} files · {} chunks · {} vector records",
                status.phase.label(),
                precise_percent(status.coverage_bps),
                status.indexed_files,
                status.eligible_files,
                status.indexed_chunks,
                status.vector_records,
            ));
        } else if self.status_present {
            lines.push("Index: unrecognized status metadata".to_string());
        }
        if !self.channels.is_empty() {
            let channels = self
                .channels
                .iter()
                .map(|channel| {
                    let mut labels = vec![format!("{} {}", channel.label, channel.candidate_count)];
                    if channel.truncated {
                        labels.push("limited".to_string());
                    }
                    if channel.fallback_present {
                        labels.push(format!(
                            "fallback: {}",
                            channel.fallback.unwrap_or("unrecognized reason")
                        ));
                    }
                    labels.join(", ")
                })
                .collect::<Vec<_>>()
                .join(" · ");
            lines.push(format!("Channels: {channels}"));
        } else if self.channels_present {
            lines.push("Channels: no recognized channel metadata".to_string());
        }
        if let Some(rerank) = &self.rerank {
            let mut ranking = vec![self.algorithm.unwrap_or("unrecognized ranking").to_string()];
            if let (Some(requested), Some(applied)) = (rerank.requested_mode, rerank.applied_mode) {
                ranking.push(if requested == applied {
                    format!("{applied} applied")
                } else {
                    format!("{requested} requested / {applied} applied")
                });
            }
            ranking.push(format!(
                "{} input / {} evaluated / {} selected",
                rerank.input_candidates, rerank.evaluated_candidates, rerank.selected_candidates,
            ));
            if rerank.near_duplicate_candidates > 0 {
                ranking.push(format!(
                    "{} near-duplicate candidates",
                    rerank.near_duplicate_candidates
                ));
            }
            if rerank.candidate_truncated {
                ranking.push("candidate limit reached".to_string());
            }
            if rerank.fallback_present {
                ranking.push(format!(
                    "fallback: {}",
                    rerank.fallback.unwrap_or("unrecognized reason")
                ));
            }
            lines.push(format!("Ranking: {}", ranking.join(" · ")));
        } else if self.algorithm_present {
            lines.push(format!(
                "Ranking: {}",
                self.algorithm.unwrap_or("unrecognized ranking")
            ));
        }
        let catalog_revision = self
            .catalog_revision
            .or_else(|| self.status.as_ref().map(|status| status.catalog_revision));
        let source_revision = self
            .source_revision
            .or_else(|| self.status.as_ref().map(|status| status.source_revision));
        let vector_revision = self.status.as_ref().map(|status| status.vector_revision);
        if catalog_revision.is_some() || source_revision.is_some() || vector_revision.is_some() {
            lines.push(format!(
                "Revisions: catalog {} · source {} · vector {}",
                display_revision(catalog_revision),
                display_revision(source_revision),
                display_revision(vector_revision),
            ));
        }
        if self.fallback_present {
            lines.push(format!(
                "Fallback: {}",
                self.fallback.unwrap_or("unrecognized reason reported")
            ));
        }
        if let Some(returned) = self.returned_results {
            let verification = if returned == self.verified_results && self.results_present {
                "all returned source digests verified".to_string()
            } else if self.results_present {
                format!(
                    "{}/{} source digests verified",
                    self.verified_results, returned
                )
            } else {
                "digest evidence unavailable".to_string()
            };
            lines.push(format!("Verification: {verification}"));
        }
        lines.join("\n")
    }

    pub(crate) fn is_degraded(&self) -> bool {
        self.fallback_present
            || self.truncated
            || (self.algorithm_present && self.algorithm.is_none())
            || self
                .status
                .as_ref()
                .is_some_and(|status| status.phase.is_degraded())
            || (self.status_present && self.status.is_none())
            || self
                .returned_results
                .is_some_and(|returned| self.results_present && returned != self.verified_results)
            || (self.channels_present
                && self.kind == WorkspaceSearchKind::Hybrid
                && self.channels.is_empty())
            || self
                .channels
                .iter()
                .any(|channel| channel.truncated || channel.fallback_present)
            || self
                .rerank
                .as_ref()
                .is_some_and(|rerank| rerank.candidate_truncated || rerank.fallback_present)
    }

    fn result_label(&self) -> Option<String> {
        let returned = self.returned_results?;
        if returned == 0 {
            return Some("0 verified results".to_string());
        }
        if self.results_present && returned == self.verified_results {
            return Some(format!(
                "{returned} verified {}",
                if returned == 1 { "result" } else { "results" }
            ));
        }
        if self.results_present {
            return Some(format!(
                "{}/{} verified results",
                self.verified_results, returned
            ));
        }
        Some(format!(
            "{returned} {}",
            if returned == 1 { "result" } else { "results" }
        ))
    }
}

fn workspace_search_kind(name: &str, args: Option<&Value>) -> Option<WorkspaceSearchKind> {
    match name {
        "semantic" => Some(WorkspaceSearchKind::Semantic),
        "hybrid" => Some(WorkspaceSearchKind::Hybrid),
        "search" => match args?.get("mode").and_then(Value::as_str) {
            Some("semantic") => Some(WorkspaceSearchKind::Semantic),
            Some("hybrid") => Some(WorkspaceSearchKind::Hybrid),
            _ => None,
        },
        _ => None,
    }
}

fn parse_index_status(value: &Value) -> Option<IndexStatus> {
    let status = value.as_object()?;
    let indexed_chunks = bounded_usize(status.get("indexedChunks"))?;
    let phase = match status.get("phase")?.as_str()? {
        "disabled" => IndexPhase::Disabled,
        "building" if indexed_chunks > 0 => IndexPhase::Partial,
        "building" => IndexPhase::Building,
        "ready" => IndexPhase::Ready,
        "degraded" => IndexPhase::Degraded,
        "closed" => IndexPhase::Closed,
        _ => return None,
    };
    Some(IndexStatus {
        phase,
        coverage_bps: status
            .get("coverageBps")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            .min(10_000) as u16,
        indexed_files: bounded_usize(status.get("indexedFiles")).unwrap_or_default(),
        eligible_files: bounded_usize(status.get("eligibleFiles")).unwrap_or_default(),
        indexed_chunks,
        vector_records: bounded_usize(status.get("vectorRecords")).unwrap_or_default(),
        catalog_revision: bounded_u64(status.get("catalogRevision")).unwrap_or_default(),
        source_revision: bounded_u64(status.get("sourceRevision")).unwrap_or_default(),
        vector_revision: bounded_u64(status.get("vectorRevision")).unwrap_or_default(),
    })
}

fn parse_channel(value: &Value) -> Option<ChannelSummary> {
    let channel = value.as_object()?;
    let label = match channel.get("channel")?.as_str()? {
        "exact" => "exact",
        "lexical" => "lexical",
        "structural" => "structural",
        "semantic" => "semantic",
        _ => return None,
    };
    let fallback_value = channel.get("fallback");
    Some(ChannelSummary {
        label,
        candidate_count: bounded_usize(channel.get("candidateCount")).unwrap_or_default(),
        truncated: channel
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        fallback: fallback_value
            .and_then(Value::as_str)
            .and_then(display_fallback),
        fallback_present: fallback_value.is_some_and(|value| !value.is_null()),
    })
}

fn parse_rerank(value: &Value) -> Option<RerankSummary> {
    let rerank = value.as_object()?;
    let fallback_value = rerank.get("fallback");
    Some(RerankSummary {
        requested_mode: rerank
            .get("requestedMode")
            .and_then(Value::as_str)
            .and_then(display_rerank_mode),
        applied_mode: rerank
            .get("appliedMode")
            .and_then(Value::as_str)
            .and_then(display_rerank_mode),
        input_candidates: bounded_usize(rerank.get("inputCandidates")).unwrap_or_default(),
        evaluated_candidates: bounded_usize(rerank.get("evaluatedCandidates")).unwrap_or_default(),
        selected_candidates: bounded_usize(rerank.get("selectedCandidates")).unwrap_or_default(),
        near_duplicate_candidates: bounded_usize(rerank.get("nearDuplicateCandidates"))
            .unwrap_or_default(),
        candidate_truncated: rerank
            .get("candidateTruncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        fallback: fallback_value
            .and_then(Value::as_str)
            .and_then(display_rerank_fallback),
        fallback_present: fallback_value.is_some_and(|value| !value.is_null()),
    })
}

fn display_algorithm(value: &str) -> Option<&'static str> {
    match value {
        "exact_cosine" => Some("exact cosine"),
        "rrf_k60" => Some("RRF k=60"),
        "rrf_k60+deterministic_mmr_v1" => Some("RRF k=60 + deterministic MMR"),
        _ => None,
    }
}

fn display_fallback(value: &str) -> Option<&'static str> {
    match value {
        "unavailable" => Some("channel unavailable"),
        "building" => Some("index building"),
        "degraded" => Some("index degraded"),
        "closed" => Some("index closed"),
        "query_embedding_failed" => Some("query embedding failed"),
        "vector_search_failed" => Some("vector search failed"),
        "structural_query_failed" => Some("symbol search failed"),
        "revision_changed" => Some("source revision changed"),
        "filtered_stale_hits" => Some("stale hits removed"),
        _ => None,
    }
}

fn display_rerank_mode(value: &str) -> Option<&'static str> {
    match value {
        "rrf_only" => Some("RRF only"),
        "deterministic" => Some("deterministic rerank"),
        _ => None,
    }
}

fn display_rerank_fallback(value: &str) -> Option<&'static str> {
    match value {
        "scratch_budget_exceeded" => Some("scratch budget exceeded"),
        "invalid_configuration" => Some("invalid rerank configuration"),
        _ => None,
    }
}

fn bounded_usize(value: Option<&Value>) -> Option<usize> {
    usize::try_from(value?.as_u64()?).ok()
}

fn bounded_u64(value: Option<&Value>) -> Option<u64> {
    value?.as_u64()
}

fn compact_percent(coverage_bps: u16) -> String {
    if coverage_bps.is_multiple_of(100) {
        format!("{}%", coverage_bps / 100)
    } else {
        precise_percent(coverage_bps)
    }
}

fn precise_percent(coverage_bps: u16) -> String {
    format!("{:.2}%", f64::from(coverage_bps) / 100.0)
}

fn display_revision(revision: Option<u64>) -> String {
    revision
        .map(|revision| revision.to_string())
        .unwrap_or_else(|| "n/a".to_string())
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    crate::system_agents::sanitize_display_text(value, max_chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_summary_projects_readiness_and_digest_verification() {
        let args = serde_json::json!({"mode": "semantic", "query": "session cache"});
        let metadata = serde_json::json!({
            "mode": "semantic",
            "algorithm": "exact_cosine",
            "status": {
                "phase": "ready",
                "catalogRevision": 7,
                "sourceRevision": 8,
                "vectorRevision": 6,
                "eligibleFiles": 10,
                "indexedFiles": 10,
                "indexedChunks": 24,
                "coverageBps": 10000,
                "vectorRecords": 24
            },
            "searched_records": 24,
            "returned_results": 2,
            "results": [
                {"digest_verified": true},
                {"digest_verified": true}
            ],
            "credential": "metadata-secret-sentinel"
        });

        let summary = WorkspaceSearchSummary::from_tool("search", Some(&args), Some(&metadata))
            .expect("semantic summary");

        assert_eq!(
            summary.compact_label(),
            "2 verified results · semantic ready · exact cosine"
        );
        assert!(!summary.is_degraded());
        let detail = summary.transcript_detail();
        assert!(detail.contains("24 vector records searched"), "{detail}");
        assert!(detail.contains("Revisions: catalog 7 · source 8 · vector 6"));
        assert!(detail.contains("all returned source digests verified"));
        assert!(!detail.contains("metadata-secret-sentinel"));
        assert!(!detail.contains("session cache"));
    }

    #[test]
    fn hybrid_summary_makes_partial_channels_rerank_and_fallback_explicit() {
        let args = serde_json::json!({"mode": "hybrid", "query": "shutdown index"});
        let metadata = serde_json::json!({
            "algorithm": "rrf_k60+deterministic_mmr_v1",
            "catalog_revision": 12,
            "source_revision": 13,
            "semantic_status": {
                "phase": "building",
                "catalogRevision": 12,
                "sourceRevision": 13,
                "vectorRevision": 11,
                "eligibleFiles": 10,
                "indexedFiles": 4,
                "indexedChunks": 12,
                "coverageBps": 4000,
                "vectorRecords": 12
            },
            "channels": [
                {"channel": "exact", "candidateCount": 1, "truncated": false},
                {"channel": "lexical", "candidateCount": 4, "truncated": false},
                {"channel": "structural", "candidateCount": 2, "truncated": false},
                {"channel": "semantic", "candidateCount": 3, "truncated": false, "fallback": "building"}
            ],
            "rerank": {
                "requestedMode": "deterministic",
                "appliedMode": "deterministic",
                "inputCandidates": 8,
                "evaluatedCandidates": 8,
                "selectedCandidates": 3,
                "nearDuplicateCandidates": 1,
                "candidateTruncated": false,
                "fallback": null
            },
            "fallback": "building",
            "returned_results": 3,
            "results": [
                {"digest_verified": true},
                {"digest_verified": true},
                {"digest_verified": true}
            ]
        });

        let summary = WorkspaceSearchSummary::from_tool("search", Some(&args), Some(&metadata))
            .expect("hybrid summary");

        assert_eq!(
            summary.compact_label(),
            "3 verified results · 4 channels · RRF k=60 + deterministic MMR · partial · fallback: index building"
        );
        assert!(summary.is_degraded());
        let detail = summary.transcript_detail();
        assert!(
            detail.contains("semantic 3, fallback: index building"),
            "{detail}"
        );
        assert!(detail.contains("deterministic rerank applied"), "{detail}");
        assert!(
            detail.contains("8 input / 8 evaluated / 3 selected"),
            "{detail}"
        );
        assert!(detail.contains("Fallback: index building"), "{detail}");
    }

    #[test]
    fn unknown_metadata_is_never_reflected_into_terminal_copy() {
        let args = serde_json::json!({"mode": "hybrid", "query": "safe"});
        let metadata = serde_json::json!({
            "algorithm": "metadata-secret-sentinel\u{001b}]8;;https://bad.invalid\u{0007}",
            "semantic_status": {"phase": "unknown-secret", "indexedChunks": 0},
            "channels": [{"channel": "secret-channel", "candidateCount": 999}],
            "fallback": "secret-fallback",
            "returned_results": 0,
            "results": [],
            "endpoint": "https://credential.invalid"
        });

        let summary = WorkspaceSearchSummary::from_tool("search", Some(&args), Some(&metadata))
            .expect("bounded hybrid summary");
        let rendered = format!(
            "{}\n{}",
            summary.compact_label(),
            summary.transcript_detail()
        );

        assert!(summary.is_degraded());
        assert!(rendered.contains("unrecognized ranking"));
        assert!(rendered.contains("unrecognized reason"));
        assert!(!rendered.contains("metadata-secret-sentinel"));
        assert!(!rendered.contains("secret-channel"));
        assert!(!rendered.contains("secret-fallback"));
        assert!(!rendered.contains("credential.invalid"));
        assert!(!rendered.contains('\u{001b}'));
    }
}
