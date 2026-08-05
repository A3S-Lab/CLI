//! Compact, typed presentation of Code Core's web-search cascade metadata.
//!
//! Core 6.7 exposes structural retrieval requirements, executed tiers, engine
//! outcomes, and bounded-result status as structured metadata. Keep that
//! evidence out of the raw provider body while still making fallback and
//! degradation visible in the terminal UI.

use serde_json::Value;

const MAX_TIER_LABEL_CHARS: usize = 24;
const MAX_TIERS: usize = 4;
const MAX_NOTICE_CHARS: usize = 240;
const MAX_COMPACT_CHARS: usize = 320;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WebSearchSummary {
    returned_results: Option<usize>,
    available_results: Option<usize>,
    tiers: Vec<String>,
    successful_engines: usize,
    failed_engines: usize,
    circuit_open_engines: usize,
    requirements_met: Option<bool>,
    fallback_attempted: bool,
    output_limited: bool,
    status: Option<String>,
    notice: Option<String>,
}

impl WebSearchSummary {
    pub(crate) fn from_metadata(metadata: Option<&Value>) -> Option<Self> {
        let metadata = metadata?.as_object()?;
        let recognized = [
            "retrieval_health",
            "retrieval_requirements",
            "search_quality",
            "search_quality_floor",
            "search_tiers",
            "engine_outcomes",
            "search_fallback",
            "returned_result_count",
            "available_result_count",
        ]
        .iter()
        .any(|key| metadata.contains_key(*key));
        if !recognized {
            return None;
        }

        let health = metadata
            .get("retrieval_health")
            .or_else(|| metadata.get("search_quality"));
        let returned_results = bounded_usize(metadata.get("returned_result_count"))
            .or_else(|| bounded_usize(health.and_then(|value| value.get("usable_result_count"))));
        let available_results =
            bounded_usize(metadata.get("available_result_count")).or(returned_results);
        let tiers = metadata
            .get("search_tiers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|report| report.get("tier").and_then(Value::as_str))
            .map(display_tier)
            .filter(|tier| !tier.is_empty())
            .take(MAX_TIERS)
            .collect::<Vec<_>>();

        let mut successful_engines = 0usize;
        let mut failed_engines = 0usize;
        let mut circuit_open_engines = 0usize;
        for kind in metadata
            .get("engine_outcomes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|outcome| outcome.get("kind").and_then(Value::as_str))
        {
            match kind {
                "success" => successful_engines = successful_engines.saturating_add(1),
                "circuit_open" => circuit_open_engines = circuit_open_engines.saturating_add(1),
                "failure" | "timeout" | "rejected" => {
                    failed_engines = failed_engines.saturating_add(1)
                }
                _ => {}
            }
        }

        let fallback = metadata.get("search_fallback");
        let requirements_met = fallback
            .and_then(|value| value.get("successful"))
            .and_then(Value::as_bool)
            .or_else(|| {
                metadata
                    .get("search_tiers")
                    .and_then(Value::as_array)
                    .and_then(|reports| reports.last())
                    .and_then(|report| report.get("decision"))
                    .and_then(Value::as_str)
                    .map(|decision| decision == "stop")
            });
        let fallback_attempted = fallback
            .and_then(|value| value.get("attempted"))
            .and_then(Value::as_bool)
            .unwrap_or(tiers.len() > 1);
        let output_limited = metadata
            .get("output_limited")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let status = metadata
            .get("status")
            .and_then(Value::as_str)
            .map(|status| bounded_text(status, 24))
            .filter(|status| !status.is_empty());
        let notice = metadata
            .get("notices")
            .and_then(Value::as_array)
            .and_then(|notices| notices.first())
            .and_then(Value::as_str)
            .map(|notice| bounded_text(notice, MAX_NOTICE_CHARS))
            .filter(|notice| !notice.is_empty());

        Some(Self {
            returned_results,
            available_results,
            tiers,
            successful_engines,
            failed_engines,
            circuit_open_engines,
            requirements_met,
            fallback_attempted,
            output_limited,
            status,
            notice,
        })
    }

    pub(crate) fn is_degraded(&self) -> bool {
        self.requirements_met == Some(false)
            || self.failed_engines > 0
            || self.circuit_open_engines > 0
            || self.output_limited
            || self
                .status
                .as_deref()
                .is_some_and(|status| matches!(status, "partial" | "failed"))
    }

    pub(crate) fn compact_label(&self) -> String {
        let mut parts = Vec::new();
        if let Some(results) = self.returned_results {
            parts.push(format!(
                "{results} {}",
                if results == 1 { "result" } else { "results" }
            ));
        }
        if !self.tiers.is_empty() {
            parts.push(self.tiers.join(" → "));
        }
        if let Some(requirements_met) = self.requirements_met {
            parts.push(if requirements_met {
                "requirements met".to_string()
            } else {
                "requirements below target".to_string()
            });
        }
        let total_engines = self
            .successful_engines
            .saturating_add(self.failed_engines)
            .saturating_add(self.circuit_open_engines);
        if total_engines > 0 {
            parts.push(format!(
                "{}/{} engines",
                self.successful_engines, total_engines
            ));
        }
        if self.output_limited {
            parts.push("output limited".to_string());
        }
        if parts.is_empty() {
            parts.push(
                self.status
                    .clone()
                    .unwrap_or_else(|| "search metadata available".to_string()),
            );
        }
        bounded_text(&parts.join(" · "), MAX_COMPACT_CHARS)
    }

    pub(crate) fn transcript_detail(&self) -> String {
        let mut lines = Vec::new();
        match (self.returned_results, self.available_results) {
            (Some(returned), Some(available)) if available != returned => {
                lines.push(format!(
                    "Results: {returned} returned / {available} available"
                ));
            }
            (Some(returned), _) => lines.push(format!("Results: {returned} returned")),
            _ => {}
        }
        if !self.tiers.is_empty() {
            let suffix = if self.fallback_attempted {
                " (fallback used)"
            } else {
                ""
            };
            lines.push(format!("Tiers: {}{suffix}", self.tiers.join(" → ")));
        }
        if let Some(requirements_met) = self.requirements_met {
            lines.push(format!(
                "Retrieval: requirements {}",
                if requirements_met {
                    "met"
                } else {
                    "below target"
                }
            ));
        }
        let mut engine_parts = Vec::new();
        if self.successful_engines > 0 {
            engine_parts.push(format!("{} succeeded", self.successful_engines));
        }
        if self.failed_engines > 0 {
            engine_parts.push(format!("{} failed", self.failed_engines));
        }
        if self.circuit_open_engines > 0 {
            engine_parts.push(format!("{} circuit open", self.circuit_open_engines));
        }
        if !engine_parts.is_empty() {
            lines.push(format!("Engines: {}", engine_parts.join(" · ")));
        }
        if self.output_limited {
            lines.push("Output: bounded before all available results were returned".to_string());
        }
        if let Some(notice) = &self.notice {
            lines.push(format!("Notice: {notice}"));
        }
        lines.join("\n")
    }
}

fn bounded_usize(value: Option<&Value>) -> Option<usize> {
    usize::try_from(value?.as_u64()?).ok()
}

fn display_tier(tier: &str) -> String {
    match tier.trim().to_ascii_lowercase().as_str() {
        "api" => "API".to_string(),
        "http" => "HTTP".to_string(),
        "headless" => "Headless".to_string(),
        _ => bounded_text(tier, MAX_TIER_LABEL_CHARS),
    }
}

fn bounded_text(source: &str, max_chars: usize) -> String {
    crate::system_agents::sanitize_terminal_layout(source, max_chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_structurally_gated_fallback_without_provider_output() {
        let metadata = serde_json::json!({
            "status": "partial",
            "returned_result_count": 5,
            "available_result_count": 7,
            "output_limited": true,
            "search_fallback": { "attempted": true, "successful": false },
            "search_tiers": [
                { "tier": "api", "decision": "continue" },
                { "tier": "http", "decision": "continue" }
            ],
            "engine_outcomes": [
                { "kind": "success" },
                { "kind": "failure" },
                { "kind": "circuit_open" }
            ],
            "notices": ["degraded\u{001b}]8;;https://example.com\u{0007} search"]
        });

        let summary = WebSearchSummary::from_metadata(Some(&metadata)).expect("summary");

        assert!(summary.is_degraded());
        assert_eq!(
            summary.compact_label(),
            "5 results · API → HTTP · requirements below target · 1/3 engines · output limited"
        );
        let detail = summary.transcript_detail();
        assert!(
            detail.contains("Results: 5 returned / 7 available"),
            "{detail}"
        );
        assert!(
            detail.contains("Tiers: API → HTTP (fallback used)"),
            "{detail}"
        );
        assert!(detail.contains("1 circuit open"), "{detail}");
        assert!(!detail.contains('\u{001b}'), "{detail:?}");
        assert!(!detail.contains("https://example.com"), "{detail:?}");
    }

    #[test]
    fn ignores_unrelated_tool_metadata() {
        assert!(WebSearchSummary::from_metadata(Some(&serde_json::json!({
            "file_path": "src/lib.rs"
        })))
        .is_none());
    }

    #[test]
    fn reads_result_count_from_core_6_7_retrieval_health() {
        let metadata = serde_json::json!({
            "retrieval_health": { "usable_result_count": 4 },
            "retrieval_requirements": { "min_usable_results": 3 },
            "search_fallback": { "attempted": false, "successful": true }
        });

        let summary = WebSearchSummary::from_metadata(Some(&metadata)).expect("summary");

        assert_eq!(summary.compact_label(), "4 results · requirements met");
        assert!(!summary.is_degraded());
    }
}
