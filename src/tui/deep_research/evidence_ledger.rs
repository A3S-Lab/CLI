//! Typed, bounded evidence accepted from a DeepResearch workflow.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SourceTier {
    Authoritative,
    Secondary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AcceptedSource {
    pub(crate) id: String,
    pub(crate) anchor: String,
    pub(crate) title: Option<String>,
    pub(crate) date: Option<String>,
    pub(crate) reliability: Option<String>,
    pub(crate) tier: SourceTier,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AcceptedClaim {
    pub(crate) id: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AcceptedEvidence {
    pub(crate) id: String,
    pub(crate) summary: String,
    pub(crate) confidence: Option<String>,
    pub(crate) sources: Vec<AcceptedSource>,
    pub(crate) claims: Vec<AcceptedClaim>,
    pub(crate) contradictions: Vec<String>,
    pub(crate) gaps: Vec<String>,
}

pub(crate) fn accepted_evidence_ledger(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Vec<AcceptedEvidence> {
    let mut raw = serde_json::from_str::<serde_json::Value>(workflow_output)
        .ok()
        .map(|value| super::deep_research_collect_structured_evidence(&value))
        .unwrap_or_default();
    if let Some(metadata) = workflow_metadata {
        raw.extend(super::deep_research_collect_structured_evidence(metadata));
    }
    let mut seen = HashSet::new();
    raw.into_iter()
        .filter_map(normalize_evidence)
        .filter(|evidence| seen.insert(evidence.id.clone()))
        .take(64)
        .collect()
}

#[cfg(test)]
pub(crate) fn synthesis_payload(evidence: &[AcceptedEvidence]) -> String {
    synthesis_payload_with_context(evidence, "")
}

pub(crate) fn synthesis_payload_with_context(
    evidence: &[AcceptedEvidence],
    workflow_output: &str,
) -> String {
    let items = evidence
        .iter()
        .map(|item| {
            serde_json::json!({
                "summary": item.summary,
                "sources": item.sources.iter().map(|source| serde_json::json!({
                    "title": source.title,
                    "url_or_path": source.anchor,
                    "date": source.date,
                    "reliability": source.reliability,
                    "tier": source.tier,
                })).collect::<Vec<_>>(),
                "key_evidence": item.claims.iter().map(|claim| claim.text.as_str()).collect::<Vec<_>>(),
                "contradictions": item.contradictions,
                "gaps": item.gaps,
                "confidence": item.confidence,
            })
        })
        .collect::<Vec<_>>();
    let mut payload = serde_json::json!({
        "collection_status": if items.is_empty() { "degraded" } else { "completed" },
        "evidence_items": items,
    });
    if let Some(context) = synthesis_report_context(workflow_output) {
        payload["report_context"] = context;
    }
    serde_json::to_string(&payload).unwrap_or_else(|_| {
        "{\"collection_status\":\"degraded\",\"evidence_items\":[]}".to_string()
    })
}

fn synthesis_report_context(workflow_output: &str) -> Option<serde_json::Value> {
    let workflow = serde_json::from_str::<serde_json::Value>(workflow_output.trim()).ok()?;
    let plan = workflow.get("plan").and_then(serde_json::Value::as_object);
    let checker = workflow
        .get("checker")
        .and_then(serde_json::Value::as_object);
    if plan.is_none() && checker.is_none() {
        return None;
    }

    let mut context = serde_json::Map::new();
    if let Some(plan) = plan {
        let mut plan_context = serde_json::Map::new();
        for key in ["report_title", "answer_shape", "execution_route"] {
            if let Some(value) = plan
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(|value| bounded_string(value, 300))
            {
                plan_context.insert(key.to_string(), serde_json::Value::String(value));
            }
        }
        for key in ["phases", "stop_conditions"] {
            let values = bounded_string_array(plan.get(key), 6, 500);
            if !values.is_empty() {
                plan_context.insert(key.to_string(), serde_json::json!(values));
            }
        }
        if !plan_context.is_empty() {
            context.insert("plan".to_string(), serde_json::Value::Object(plan_context));
        }
    }
    if let Some(checker) = checker {
        let mut checker_context = serde_json::Map::new();
        for key in ["decision", "report_summary", "coverage_summary"] {
            if let Some(value) = checker
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(|value| bounded_string(value, 1_200))
            {
                checker_context.insert(key.to_string(), serde_json::Value::String(value));
            }
        }
        for key in ["verified_findings", "unresolved_gaps", "contradictions"] {
            let values = bounded_string_array(checker.get(key), 10, 1_000);
            if !values.is_empty() {
                checker_context.insert(key.to_string(), serde_json::json!(values));
            }
        }
        if !checker_context.is_empty() {
            context.insert(
                "checker".to_string(),
                serde_json::Value::Object(checker_context),
            );
        }
    }
    (!context.is_empty()).then_some(serde_json::Value::Object(context))
}

fn bounded_string(value: &str, limit: usize) -> String {
    value.trim().chars().take(limit).collect()
}

fn bounded_string_array(
    value: Option<&serde_json::Value>,
    item_limit: usize,
    char_limit: usize,
) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .take(item_limit)
        .map(|value| bounded_string(value, char_limit))
        .collect()
}

fn normalize_evidence(value: serde_json::Value) -> Option<AcceptedEvidence> {
    let summary = value.get("summary")?.as_str()?.trim();
    if summary.is_empty() {
        return None;
    }
    let mut source_ids = HashSet::new();
    let sources = value
        .get("sources")?
        .as_array()?
        .iter()
        .filter_map(|source| {
            let anchor = super::deep_research_traceable_source_anchor(source)?;
            let id = stable_id("source", &anchor);
            if !source_ids.insert(id.clone()) {
                return None;
            }
            let reliability =
                string_field(source, "reliability").or_else(|| string_field(source, "publisher"));
            let tier = source_tier(&anchor, reliability.as_deref());
            Some(AcceptedSource {
                id,
                anchor,
                title: string_field(source, "title"),
                date: string_field(source, "date"),
                reliability,
                tier,
            })
        })
        .collect::<Vec<_>>();
    if sources.is_empty() {
        return None;
    }
    let claims = string_array(value.get("key_evidence"), 32)
        .into_iter()
        .map(|text| AcceptedClaim {
            id: stable_id("claim", &text.to_ascii_lowercase()),
            text,
        })
        .collect::<Vec<_>>();
    let evidence_key = format!(
        "{}|{}",
        summary.to_ascii_lowercase(),
        sources
            .iter()
            .map(|source| source.id.as_str())
            .collect::<Vec<_>>()
            .join("|")
    );
    Some(AcceptedEvidence {
        id: stable_id("evidence", &evidence_key),
        summary: summary.to_string(),
        confidence: string_field(&value, "confidence"),
        sources,
        claims,
        contradictions: string_array(value.get("contradictions"), 16),
        gaps: string_array(value.get("gaps"), 16),
    })
}

fn source_tier(anchor: &str, reliability: Option<&str>) -> SourceTier {
    let reliability = reliability.unwrap_or_default().to_ascii_lowercase();
    if reliability.contains("official")
        || reliability.contains("authoritative")
        || reliability.contains("primary")
        || anchor.contains(".gov.")
        || anchor.contains("://gov.")
        || anchor.contains(".gov/")
    {
        SourceTier::Authoritative
    } else {
        SourceTier::Secondary
    }
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.chars().take(2_000).collect())
}

fn string_array(value: Option<&serde_json::Value>, limit: usize) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .take(limit)
        .map(|text| text.chars().take(2_000).collect())
        .collect()
}

fn stable_id(kind: &str, value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(value.as_bytes());
    format!("{kind}:{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_traceable_evidence_and_assigns_stable_ids() {
        let output = serde_json::json!({
            "structured": {
                "summary": "The release is documented.",
                "sources": [{
                    "title": "Official release",
                    "url_or_path": "https://example.gov/releases/1",
                    "quote_or_fact": "Released on July 12.",
                    "reliability": "Official primary source"
                }],
                "key_evidence": ["The release date is July 12."],
                "contradictions": [],
                "gaps": [],
                "confidence": "high"
            }
        });
        let first = accepted_evidence_ledger(&output.to_string(), None);
        let second = accepted_evidence_ledger(&output.to_string(), None);
        assert_eq!(first, second);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].sources[0].tier, SourceTier::Authoritative);
        assert_eq!(first[0].claims.len(), 1);
        let synthesis = synthesis_payload(&first);
        assert!(synthesis.contains("The release date is July 12."));
        assert!(synthesis.contains("https://example.gov/releases/1"));
    }

    #[test]
    fn rejects_source_free_model_claims() {
        let output = serde_json::json!({
            "structured": {
                "summary": "Unsupported claim",
                "sources": [],
                "key_evidence": ["Invented fact"],
                "contradictions": [],
                "gaps": [],
                "confidence": "high"
            }
        });
        assert!(accepted_evidence_ledger(&output.to_string(), None).is_empty());
        assert!(!synthesis_payload(&[]).contains("Invented fact"));
    }

    #[test]
    fn synthesis_payload_carries_only_bounded_report_plan_and_checker_context() {
        let evidence = AcceptedEvidence {
            id: "evidence:1".to_string(),
            summary: "A source-backed summary.".to_string(),
            confidence: Some("high".to_string()),
            sources: vec![AcceptedSource {
                id: "source:1".to_string(),
                anchor: "https://example.com/source".to_string(),
                title: Some("Source".to_string()),
                date: None,
                reliability: Some("Official".to_string()),
                tier: SourceTier::Authoritative,
            }],
            claims: vec![],
            contradictions: vec![],
            gaps: vec![],
        };
        let workflow = serde_json::json!({
            "query": "must not be copied into report_context",
            "plan": {
                "report_title": "Reader-facing title",
                "answer_shape": "investigation",
                "execution_route": "direct_then_maker",
                "phases": ["Collect", "Compare"],
                "stop_conditions": ["Evidence is corroborated"],
                "search_queries": ["internal retrieval instruction"]
            },
            "checker": {
                "decision": "finalize",
                "coverage_summary": "Coverage is sufficient.",
                "report_summary": "The evidence supports a bounded recommendation.",
                "verified_findings": ["The primary finding is supported."],
                "unresolved_gaps": ["One benchmark remains unavailable."],
                "contradictions": []
            }
        })
        .to_string();

        let payload = synthesis_payload_with_context(&[evidence], &workflow);
        assert!(payload.contains("Reader-facing title"), "{payload}");
        assert!(
            payload.contains("The primary finding is supported"),
            "{payload}"
        );
        assert!(
            payload.contains("One benchmark remains unavailable"),
            "{payload}"
        );
        assert!(
            !payload.contains("internal retrieval instruction"),
            "{payload}"
        );
        assert!(!payload.contains("must not be copied"), "{payload}");
    }
}
