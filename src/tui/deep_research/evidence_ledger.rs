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
    #[serde(default)]
    pub(crate) quote_or_fact: Option<String>,
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
            let source_facts = item
                .sources
                .iter()
                .filter_map(|source| source.quote_or_fact.as_deref())
                .collect::<Vec<_>>();
            let summary = format!(
                "Retained {} source-backed fact excerpt(s).",
                source_facts.len()
            );
            serde_json::json!({
                "evidence_id": item.id,
                "summary": summary,
                "sources": item.sources.iter().map(|source| serde_json::json!({
                    "source_id": source.id,
                    "title": source.title,
                    "url_or_path": source.anchor,
                    "date": source.date,
                    "reliability": source.reliability,
                    "quote_or_fact": source.quote_or_fact,
                    "tier": source.tier,
                })).collect::<Vec<_>>(),
                "confidence": item.confidence,
            })
        })
        .collect::<Vec<_>>();
    let mut payload = serde_json::json!({
        "collection_status": if items.is_empty() { "degraded" } else { "completed" },
        "evidence_items": items,
    });
    if let Some(context) = synthesis_report_context(workflow_output, evidence) {
        payload["report_context"] = context;
    }
    serde_json::to_string(&payload).unwrap_or_else(|_| {
        "{\"collection_status\":\"degraded\",\"evidence_items\":[]}".to_string()
    })
}

fn synthesis_report_context(
    workflow_output: &str,
    evidence: &[AcceptedEvidence],
) -> Option<serde_json::Value> {
    let workflow = serde_json::from_str::<serde_json::Value>(workflow_output.trim()).ok()?;
    let plan = workflow.get("plan").and_then(serde_json::Value::as_object);
    let checker = workflow
        .get("checker")
        .and_then(serde_json::Value::as_object);
    let verification = workflow
        .get("verification")
        .and_then(serde_json::Value::as_object);
    if plan.is_none() && checker.is_none() && verification.is_none() {
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
        let tracks = bounded_plan_tracks(plan.get("tracks"), 6, 500);
        if !tracks.is_empty() {
            plan_context.insert("tracks".to_string(), serde_json::json!(tracks));
        }
        if !plan_context.is_empty() {
            context.insert("plan".to_string(), serde_json::Value::Object(plan_context));
        }
    }
    if let Some(checker) = checker {
        let mut checker_context = serde_json::Map::new();
        if let Some(decision) = checker.get("decision").and_then(serde_json::Value::as_str) {
            checker_context.insert(
                "decision".to_string(),
                serde_json::Value::String(bounded_string(decision, 40)),
            );
        }
        let source_anchors = accepted_source_anchors(evidence);
        let grounding_texts = accepted_grounding_texts(evidence);
        let mut supported_findings = Vec::new();
        let mut unresolved_obligations = Vec::new();
        for (key, label_key) in [
            ("track_assessments", "track"),
            ("stop_condition_assessments", "stop_condition"),
        ] {
            let assessments = bounded_checker_assessments(
                checker.get(key),
                label_key,
                &source_anchors,
                &grounding_texts,
            );
            for assessment in &assessments {
                let status = assessment.get("status").and_then(serde_json::Value::as_str);
                let label = assessment
                    .get(label_key)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("planned obligation");
                if status == Some("supported") {
                    if let Some(finding) = assessment
                        .get("finding")
                        .and_then(serde_json::Value::as_str)
                    {
                        supported_findings.push(finding.to_string());
                    }
                } else {
                    unresolved_obligations
                        .push(format!("{label}: {}", status.unwrap_or("bounded")));
                }
            }
            if !assessments.is_empty() {
                checker_context.insert(key.to_string(), serde_json::Value::Array(assessments));
            }
        }
        supported_findings.sort();
        supported_findings.dedup();
        unresolved_obligations.sort();
        unresolved_obligations.dedup();
        if !supported_findings.is_empty() {
            checker_context.insert(
                "verified_findings".to_string(),
                serde_json::json!(supported_findings),
            );
        }
        if !unresolved_obligations.is_empty() {
            checker_context.insert(
                "unresolved_obligations".to_string(),
                serde_json::json!(unresolved_obligations),
            );
        }
        if !checker_context.is_empty() {
            context.insert(
                "checker".to_string(),
                serde_json::Value::Object(checker_context),
            );
        }
    }
    if let Some(verification) = verification {
        let mut verification_context = serde_json::Map::new();
        for key in ["status", "checker_completed", "prior_checker_retained"] {
            if let Some(value) = verification.get(key) {
                verification_context.insert(key.to_string(), value.clone());
            }
        }
        if !verification_context.is_empty() {
            context.insert(
                "verification".to_string(),
                serde_json::Value::Object(verification_context),
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

fn bounded_checker_assessments(
    value: Option<&serde_json::Value>,
    label_key: &str,
    accepted_source_anchors: &HashSet<String>,
    grounding_texts: &[String],
) -> Vec<serde_json::Value> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_object)
        .take(6)
        .filter_map(|assessment| {
            let plan_index = assessment.get("plan_index")?.as_u64()?;
            let mut status = assessment.get("status")?.as_str()?;
            if !matches!(status, "supported" | "bounded" | "uncovered") {
                return None;
            }
            let finding = assessment.get("finding")?.as_str()?;
            let source_urls = bounded_string_array(assessment.get("source_urls"), 4, 1_000)
                .into_iter()
                .filter(|url| accepted_source_anchors.contains(url))
                .collect::<Vec<_>>();
            if status == "supported"
                && (source_urls.is_empty()
                    || !super::deep_research_report_audit::quantitative_claim_is_grounded(
                        finding,
                        grounding_texts,
                    ))
            {
                status = "bounded";
            }
            let mut compact = serde_json::Map::new();
            compact.insert("plan_index".to_string(), serde_json::json!(plan_index));
            compact.insert("status".to_string(), serde_json::json!(status));
            if status == "supported" {
                compact.insert(
                    "finding".to_string(),
                    serde_json::json!(bounded_string(finding, 1_000)),
                );
            }
            if let Some(label) = assessment
                .get(label_key)
                .and_then(serde_json::Value::as_str)
            {
                compact.insert(
                    label_key.to_string(),
                    serde_json::json!(bounded_string(label, 300)),
                );
            }
            if !source_urls.is_empty() {
                compact.insert("source_urls".to_string(), serde_json::json!(source_urls));
            }
            Some(serde_json::Value::Object(compact))
        })
        .collect()
}

fn accepted_source_anchors(evidence: &[AcceptedEvidence]) -> HashSet<String> {
    evidence
        .iter()
        .flat_map(|item| &item.sources)
        .map(|source| source.anchor.clone())
        .collect()
}

fn accepted_grounding_texts(evidence: &[AcceptedEvidence]) -> Vec<String> {
    evidence
        .iter()
        .flat_map(|item| &item.sources)
        .flat_map(source_grounding_texts)
        .collect()
}

fn source_grounding_texts(source: &AcceptedSource) -> impl Iterator<Item = String> + '_ {
    [
        source.title.as_deref(),
        source.date.as_deref(),
        source.reliability.as_deref(),
        source.quote_or_fact.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::to_string)
}

fn grounding_texts_for_sources(sources: &[AcceptedSource]) -> Vec<String> {
    sources
        .iter()
        .flat_map(|source| source_grounding_texts(source))
        .collect()
}

pub(crate) fn report_grounding_texts(query: &str, evidence: &[AcceptedEvidence]) -> Vec<String> {
    let mut grounding = accepted_grounding_texts(evidence);
    grounding.push(query.to_string());
    let today = chrono::Local::now().date_naive();
    grounding.push(today.format("%Y-%m-%d").to_string());
    grounding.push(today.format("%Y年%m月%d日").to_string());
    let source_count = evidence
        .iter()
        .flat_map(|item| &item.sources)
        .map(|source| source.id.as_str())
        .collect::<HashSet<_>>()
        .len();
    grounding.push(format!("{source_count} sources"));
    grounding
}

fn bounded_plan_tracks(
    value: Option<&serde_json::Value>,
    item_limit: usize,
    char_limit: usize,
) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|track| {
            track
                .as_str()
                .or_else(|| track.get("title").and_then(serde_json::Value::as_str))
        })
        .map(str::trim)
        .filter(|track| !track.is_empty())
        .take(item_limit)
        .map(|track| bounded_string(track, char_limit))
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
                quote_or_fact: string_field(source, "quote_or_fact")
                    .or_else(|| string_field(source, "excerpt"))
                    .or_else(|| string_field(source, "fact")),
                tier,
            })
        })
        .collect::<Vec<_>>();
    if sources.is_empty() {
        return None;
    }
    let grounding_texts = grounding_texts_for_sources(&sources);
    let claims = string_array(value.get("key_evidence"), 32)
        .into_iter()
        .filter(|text| {
            super::deep_research_report_audit::quantitative_claim_is_grounded(
                text,
                &grounding_texts,
            )
        })
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
        assert!(synthesis.contains("Released on July 12."));
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
    fn rejects_key_evidence_with_a_threshold_absent_from_source_facts() {
        let output = serde_json::json!({
            "structured": {
                "summary": "A collector proposed a decision threshold.",
                "sources": [{
                    "title": "Benchmark",
                    "url_or_path": "https://example.com/benchmark",
                    "quote_or_fact": "The benchmark covers 1M-10M vectors.",
                    "reliability": "Published benchmark"
                }],
                "key_evidence": [
                    "The benchmark covers 1M-10M vectors.",
                    "Use the product below 1M vectors."
                ],
                "contradictions": [],
                "gaps": [],
                "confidence": "medium"
            }
        });
        let evidence = accepted_evidence_ledger(&output.to_string(), None);
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].claims.len(), 1, "{evidence:#?}");
        assert!(evidence[0].claims[0].text.contains("1M-10M"));
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
                quote_or_fact: Some("The primary finding is supported.".to_string()),
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
                "tracks": ["Mechanism", "Counterevidence"],
                "stop_conditions": ["Evidence is corroborated"],
                "search_queries": ["internal retrieval instruction"]
            },
            "checker": {
                "decision": "finalize",
                "coverage_summary": "Coverage is sufficient.",
                "report_summary": "The evidence supports a bounded recommendation.",
                "verified_findings": ["The primary finding is supported."],
                "track_assessments": [{
                    "plan_index": 0,
                    "track": "Mechanism",
                    "status": "supported",
                    "finding": "The mechanism finding is supported.",
                    "source_urls": ["https://example.com/source"]
                }, {
                    "plan_index": 1,
                    "track": "Counterevidence",
                    "status": "bounded",
                    "finding": "Independent counterevidence remains incomplete.",
                    "source_urls": []
                }],
                "stop_condition_assessments": [{
                    "plan_index": 0,
                    "stop_condition": "Evidence is corroborated",
                    "status": "bounded",
                    "finding": "Corroboration remains incomplete.",
                    "source_urls": ["https://example.com/source"]
                }],
                "unresolved_gaps": ["One benchmark remains unavailable."],
                "limitations": ["The retained time window is bounded."],
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
            !payload.contains("One benchmark remains unavailable"),
            "{payload}"
        );
        assert!(payload.contains("Mechanism"), "{payload}");
        assert!(payload.contains("Counterevidence"), "{payload}");
        assert!(
            payload.contains("The mechanism finding is supported"),
            "{payload}"
        );
        assert!(
            !payload.contains("Corroboration remains incomplete"),
            "{payload}"
        );
        assert!(
            !payload.contains("The retained time window is bounded"),
            "{payload}"
        );
        assert!(payload.contains("unresolved_obligations"), "{payload}");
        assert!(
            !payload.contains("internal retrieval instruction"),
            "{payload}"
        );
        assert!(!payload.contains("must not be copied"), "{payload}");
    }

    #[test]
    fn synthesis_payload_exposes_verification_facts_without_internal_publication_status() {
        let evidence = AcceptedEvidence {
            id: "evidence:1".to_string(),
            summary: "A traceable result survived the checker failure.".to_string(),
            confidence: Some("medium".to_string()),
            sources: vec![AcceptedSource {
                id: "source:1".to_string(),
                anchor: "https://example.com/source".to_string(),
                title: Some("Source".to_string()),
                date: None,
                reliability: Some("Official".to_string()),
                quote_or_fact: Some("A traceable result survived the checker failure.".to_string()),
                tier: SourceTier::Authoritative,
            }],
            claims: vec![],
            contradictions: vec![],
            gaps: vec![],
        };
        let workflow = serde_json::json!({
            "plan": { "report_title": "Qualified result" },
            "verification": {
                "status": "degraded",
                "checker_completed": false,
                "prior_checker_retained": true,
                "error": "internal provider failure must not leak"
            }
        })
        .to_string();

        let payload = synthesis_payload_with_context(&[evidence], &workflow);
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(payload["report_context"]
            .get("publication_status")
            .is_none());
        assert_eq!(
            payload["report_context"]["verification"]["checker_completed"],
            false
        );
        assert!(!payload
            .to_string()
            .contains("internal provider failure must not leak"));
    }
}
