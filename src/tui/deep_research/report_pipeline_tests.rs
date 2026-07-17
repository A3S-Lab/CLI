use super::*;

#[test]
fn completed_event_snapshot_is_authoritative_when_tool_output_is_diagnostic_text() {
    let final_output = serde_json::json!({
        "mode": "hybrid_direct_web_parallel",
        "plan": {
            "report_title": "Evidence-backed comparison",
            "answer_shape": "investigation",
            "execution_route": "direct_then_maker",
            "tracks": ["Primary evidence"],
            "stop_conditions": ["The finding has a traceable source"],
            "budget": {}
        },
        "checker": {
            "decision": "finalize",
            "coverage_summary": "The planned evidence obligation is supported."
        },
        "research": {
            "results": [{
                "success": true,
                "structured": {
                    "summary": "The event journal retained the completed source-backed result.",
                    "sources": [{
                        "title": "Official evidence",
                        "url_or_path": "https://example.com/official",
                        "quote_or_fact": "The official source supports the retained finding.",
                        "reliability": "Official source"
                    }],
                    "key_evidence": ["The retained finding is source-backed."],
                    "contradictions": [],
                    "confidence": "high",
                    "gaps": []
                }
            }]
        }
    });
    let metadata = serde_json::json!({
        "dynamic_workflow": {
            "status": "Completed",
            "snapshot": {
                "status": "completed",
                "output": final_output
            }
        }
    });
    let display_output = "Program script completed. Internal workflow events were captured.";

    let canonical = deep_research_canonical_workflow_output(display_output, Some(&metadata));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&canonical).unwrap(),
        metadata["dynamic_workflow"]["snapshot"]["output"]
    );
    assert!(
        !deep_research_workflow_needs_recovery_report_with_metadata(
            display_output,
            Some(&metadata),
        ),
        "a completed event snapshot must not be demoted because display output contains diagnostics"
    );
    assert_eq!(
        deep_research_report_outcome_for_workflow(
            "Compare the documented mechanisms",
            DeepResearchEvidenceScope::WebAndWorkspace,
            display_output,
            Some(&metadata),
        ),
        DeepResearchRunOutcome::Completed,
    );
    assert!(
        deep_research_cli_report_plan(
            "Compare the documented mechanisms",
            display_output,
            Some(&metadata),
        )
        .is_ok(),
        "CLI synthesis must consume the event-sourced result instead of generic recovery"
    );
}

#[test]
fn deep_research_synthesis_prompt_preserves_typed_evidence_and_checker_context() {
    let accepted_payload = serde_json::json!({
        "collection_status": "completed",
        "evidence_items": [{
            "summary": "The retained source establishes the isolation boundary.",
            "sources": [{
                "title": "Project source",
                "url_or_path": "https://example.com/project/source",
                "quote_or_fact": "The workload runs behind a separate guest kernel.",
                "reliability": "Authoritative project documentation."
            }],
            "key_evidence": ["The workload runs behind a separate guest kernel."],
            "contradictions": [],
            "gaps": [],
            "confidence": "high"
        }],
        "report_context": {
            "plan": {
                "report_title": "Production isolation decision",
                "answer_shape": "investigation",
                "tracks": ["Isolation boundary", "Operational tradeoffs"]
            },
            "checker": {
                "decision": "degrade",
                "report_summary": "One conclusion is supported and one remains provisional.",
                "verified_findings": ["The guest-kernel boundary is source-backed."],
                "unresolved_gaps": ["Independent overhead measurements remain unavailable."]
            }
        }
    })
    .to_string();

    let prompt = deep_research_synthesis_prompt_with_scope(
        "Choose a production isolation boundary",
        false,
        &accepted_payload,
        None,
        DeepResearchEvidenceScope::WebAndWorkspace,
    );

    for required in [
        "https://example.com/project/source",
        "The workload runs behind a separate guest kernel",
        "Production isolation decision",
        "The guest-kernel boundary is source-backed",
        "Independent overhead measurements remain unavailable",
    ] {
        assert!(prompt.contains(required), "missing {required}: {prompt}");
    }
    assert!(!prompt.contains("publication_status"), "{prompt}");
}
