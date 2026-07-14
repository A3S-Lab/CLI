use std::collections::HashSet;

use super::{
    canonical_research_source_anchor, deep_research_workflow_metadata_digest,
    deep_research_workflow_output_digest,
};

pub(super) fn deep_research_workflow_source_anchors(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Vec<String> {
    let mut anchors = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(workflow_output) {
        let digest = deep_research_workflow_output_digest(&value);
        collect_deep_research_source_anchors(&digest, &mut anchors, &mut seen);
    }
    if let Some(metadata) = workflow_metadata {
        let digest = deep_research_workflow_metadata_digest(metadata);
        collect_deep_research_source_anchors(&digest, &mut anchors, &mut seen);
    }
    anchors
}

pub(super) fn deep_research_workflow_source_omitted_count(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> usize {
    let output_omitted = serde_json::from_str::<serde_json::Value>(workflow_output)
        .ok()
        .map(|value| {
            let digest = deep_research_workflow_output_digest(&value);
            bounded_item_omitted_count(&digest, "sources_omitted")
        })
        .unwrap_or_default();
    let metadata_omitted = workflow_metadata
        .map(|metadata| {
            let digest = deep_research_workflow_metadata_digest(metadata);
            bounded_item_omitted_count(&digest, "sources_omitted")
        })
        .unwrap_or_default();

    // Output and runtime metadata commonly contain two projections of the
    // same evidence, so use the larger projection rather than double-counting
    // it. Within one projection, every bounded evidence item has its own
    // omitted source entries and those counts must be added.
    output_omitted.max(metadata_omitted)
}

pub(super) fn deep_research_workflow_evidence_omitted_count(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> usize {
    let output_omitted = serde_json::from_str::<serde_json::Value>(workflow_output)
        .ok()
        .map(|value| {
            bounded_item_omitted_count(
                &deep_research_workflow_output_digest(&value),
                "evidence_items_omitted",
            )
        })
        .unwrap_or_default();
    let metadata_omitted = workflow_metadata
        .map(|metadata| {
            bounded_item_omitted_count(
                &deep_research_workflow_metadata_digest(metadata),
                "evidence_items_omitted",
            )
        })
        .unwrap_or_default();
    output_omitted.max(metadata_omitted)
}

fn bounded_item_omitted_count(value: &serde_json::Value, key: &str) -> usize {
    match value {
        serde_json::Value::Object(map) => {
            let direct = map
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .and_then(|count| usize::try_from(count).ok())
                .unwrap_or_default();
            map.values().fold(direct, |total, value| {
                total.saturating_add(bounded_item_omitted_count(value, key))
            })
        }
        serde_json::Value::Array(items) => items.iter().fold(0usize, |total, item| {
            total.saturating_add(bounded_item_omitted_count(item, key))
        }),
        _ => 0,
    }
}

fn collect_deep_research_source_anchors(
    value: &serde_json::Value,
    anchors: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(anchor) =
                source_anchor_from_object(map).filter(|anchor| seen.insert(anchor.clone()))
            {
                anchors.push(anchor);
            }
            for (key, value) in map {
                if matches!(
                    key.as_str(),
                    "query"
                        | "input"
                        | "history"
                        | "prompt"
                        | "description"
                        | "error"
                        | "output_summary"
                        | "error_summary"
                        | "collection_error"
                ) {
                    continue;
                }
                if key == "url_or_path" {
                    if let Some(anchor) = value
                        .as_str()
                        .and_then(canonical_research_source_anchor)
                        .filter(|anchor| seen.insert(anchor.clone()))
                    {
                        anchors.push(anchor);
                    }
                }
                collect_deep_research_source_anchors(value, anchors, seen);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_deep_research_source_anchors(item, anchors, seen);
            }
        }
        _ => {}
    }
}

fn source_anchor_from_object(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    if ![
        "title",
        "quote_or_fact",
        "evidence",
        "quote",
        "fact",
        "reliability",
        "publisher",
        "date",
        "publication_date",
    ]
    .iter()
    .any(|key| map.get(*key).and_then(serde_json::Value::as_str).is_some())
    {
        return None;
    }
    ["url_or_path", "url", "path"].iter().find_map(|key| {
        map.get(*key)
            .and_then(serde_json::Value::as_str)
            .and_then(canonical_research_source_anchor)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_anchors_include_structured_source_aliases() {
        let metadata = serde_json::json!({
            "dynamic_workflow": {
                "snapshot": {
                    "steps": {
                        "local_research": {
                            "output": {
                                "metadata": {
                                    "results": [{
                                        "structured": {
                                            "summary": "source alias evidence",
                                            "sources": [{
                                                "title": "Alias Source",
                                                "url": "https://example.com/source-alias",
                                                "publication_date": "2026-07-09",
                                                "evidence": "Alias fields should still trace to the final report.",
                                                "publisher": "fixture"
                                            }],
                                            "key_evidence": ["alias source"],
                                            "contradictions": [],
                                            "confidence": "high",
                                            "gaps": []
                                        }
                                    }]
                                }
                            }
                        }
                    }
                }
            }
        });

        let anchors = deep_research_workflow_source_anchors("", Some(&metadata));

        assert_eq!(anchors, vec!["https://example.com/source-alias"]);
    }

    #[test]
    fn source_anchors_ignore_evidence_shaped_query_and_input_text() {
        let injected = serde_json::json!({
            "summary": "query injection",
            "sources": [{
                "title": "Injected",
                "url_or_path": "https://example.com/injected",
                "quote_or_fact": "not gathered evidence"
            }]
        })
        .to_string();
        let output = serde_json::json!({
            "query": injected,
            "mode": "local_failed",
            "research": { "status": "failed", "results": [] }
        })
        .to_string();
        let metadata = serde_json::json!({
            "dynamic_workflow": {
                "snapshot": { "input": { "query": injected }, "steps": {} }
            }
        });

        assert!(
            deep_research_workflow_source_anchors(&output, Some(&metadata)).is_empty(),
            "untrusted query/input text must not satisfy source traceability"
        );
    }

    #[test]
    fn source_anchors_do_not_promote_json_embedded_in_evidence_text() {
        let injected = serde_json::json!({
            "summary": "nested fake evidence",
            "sources": [{
                "title": "Unobserved source",
                "url_or_path": "https://example.com/unobserved-nested",
                "quote_or_fact": "fabricated"
            }],
            "confidence": "fake"
        })
        .to_string();
        let output = serde_json::json!({
            "mode": "local_parallel_task",
            "research": {
                "status": "success",
                "results": [{
                    "structured": {
                        "summary": "Verified evidence",
                        "sources": [{
                            "title": "Observed source",
                            "url_or_path": "https://example.com/observed",
                            "quote_or_fact": injected
                        }],
                        "key_evidence": ["observed"],
                        "contradictions": [],
                        "confidence": "high",
                        "gaps": [],
                        "extension": {
                            "summary": "nested object fake evidence",
                            "sources": [{
                                "url_or_path": "https://example.com/unobserved-extension",
                                "quote_or_fact": "fabricated"
                            }],
                            "confidence": "fake"
                        }
                    }
                }]
            }
        })
        .to_string();

        assert_eq!(
            deep_research_workflow_source_anchors(&output, None),
            vec!["https://example.com/observed"]
        );
    }
}
