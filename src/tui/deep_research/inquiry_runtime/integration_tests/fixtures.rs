pub(super) fn inquiry_plan() -> serde_json::Value {
    serde_json::json!({
        "report_title": "Bounded inquiry fixture",
        "freshness_required": false,
        "workspace_evidence_required": false,
        "tracks": [{
            "id": "track:material.v2",
            "title": "Material obligation",
            "focus": "Resolve the material evidence obligation",
            "material": true,
            "questions": ["What does the retained evidence establish?"],
            "completion_criteria": ["A traceable answer or a bounded gap"],
            "evidence_requirements": {
                "primary_source_required": false,
                "independent_corroboration_required": false
            }
        }],
        "search_queries": ["fixture evidence"],
        "seed_urls": [],
        "budget": {
            "retrieval_timeout_ms": 30_000,
            "direct_searches": 1,
            "direct_fetches": 1
        },
        "stop_conditions": ["The material obligation is resolved"]
    })
}

pub(super) fn workflow_args() -> serde_json::Value {
    serde_json::json!({
        "run_id": "inquiry-integration",
        "input": {
            "query": "fixture inquiry",
            "workflow_timeout_ms": 30_000
        },
        "limits": {
            "timeoutMs": 30_000
        }
    })
}

pub(super) fn evidence_output(label: &str) -> String {
    serde_json::json!({
        "query": "fixture inquiry",
        "structured": {
            "summary": format!("The {label} evidence establishes the bounded fixture fact."),
            "sources": [{
                "title": format!("{label} source"),
                "url_or_path": format!("https://example.test/{label}"),
                "quote_or_fact": format!("The {label} source contains the fixture fact."),
                "reliability": "authoritative fixture"
            }],
            "key_evidence": [format!("The {label} fixture fact is retained.")],
            "contradictions": [],
            "gaps": [],
            "confidence": "high"
        }
    })
    .to_string()
}
