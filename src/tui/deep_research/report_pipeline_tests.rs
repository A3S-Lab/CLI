use super::*;

use a3s::research::{
    replay, EvidenceRef, InquiryEvent, InquiryLimits, InquiryPhase, Question, ResearchMethod,
};

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
fn bounded_supporting_obligation_produces_a_qualified_reportable_inquiry() {
    let mut supporting = Question::queued(
        "question:supporting",
        None,
        "Which supporting context can be established?",
    );
    supporting.material = false;
    let events = vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![
                Question::queued(
                    "question:core",
                    None,
                    "What evidence determines the core conclusion?",
                ),
                supporting,
            ],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:core",
                vec!["claim:core".to_string()],
                vec!["source:core".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:core".to_string(),
            answer: "The retained source supports the core conclusion.".to_string(),
            evidence_ids: vec!["evidence:core".to_string()],
        },
        InquiryEvent::QuestionBounded {
            question_id: "question:supporting".to_string(),
            reason: "The supporting context remains unavailable.".to_string(),
        },
    ];
    let state = replay(&events, &InquiryLimits::default()).expect("qualified inquiry projection");
    assert_eq!(state.phase, InquiryPhase::Outlining);

    let output = serde_json::json!({
        "mode": "direct_web",
        "checker": {
            "decision": "finalize",
            "coverage_summary": "The core conclusion is supported. Supporting context remains bounded."
        },
        "research": {
            "status": "success",
            "results": [{
                "success": true,
                "structured": {
                    "summary": "The accepted evidence establishes the core conclusion.",
                    "sources": [{
                        "title": "Core source",
                        "url_or_path": "https://example.com/core",
                        "quote_or_fact": "The source supports the core conclusion.",
                        "reliability": "Authoritative source"
                    }],
                    "key_evidence": ["The core conclusion is source-backed."],
                    "contradictions": [],
                    "confidence": "high",
                    "gaps": ["Supporting context remains unavailable."]
                }
            }]
        },
        "inquiry": {
            "events": events,
            "state": state
        }
    });

    assert_eq!(
        deep_research_report_outcome_for_workflow(
            "Determine the core conclusion with useful supporting context",
            DeepResearchEvidenceScope::WebAndWorkspace,
            &output.to_string(),
            None,
        ),
        DeepResearchRunOutcome::Qualified,
    );
}

#[test]
fn collect_only_inquiry_projection_owns_completion_and_fails_closed() {
    let mut question = Question::queued(
        "question:core",
        None,
        "What evidence determines the conclusion?",
    );
    question.obligation_ids = vec!["obligation:core".to_string()];
    let events = vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        InquiryEvent::ResearchObligationsCommitted {
            obligations: vec![a3s::research::ResearchObligation::new(
                "obligation:core",
                "Core conclusion",
                "Establish the core conclusion",
                true,
                vec!["The conclusion is supported by traceable evidence".to_string()],
            )],
            stop_conditions: vec!["The core conclusion is traceable".to_string()],
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![question],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:core",
                vec!["claim:core".to_string()],
                vec!["source:core".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:core".to_string(),
            answer: "The retained source supports the conclusion.".to_string(),
            evidence_ids: vec!["evidence:core".to_string()],
        },
        InquiryEvent::ResearchContractAssessed {
            assessment: a3s::research::ResearchContractAssessment {
                obligations: vec![a3s::research::ResearchObligationAssessment {
                    obligation_id: "obligation:core".to_string(),
                    criteria: vec![a3s::research::CompletionCriterionAssessment {
                        criterion_index: 0,
                        status: a3s::research::ContractAssessmentStatus::Satisfied,
                        rationale: "The accepted evidence satisfies the criterion.".to_string(),
                        evidence_ids: vec!["evidence:core".to_string()],
                    }],
                    primary_source: None,
                    independent_corroboration: None,
                }],
                stop_conditions: vec![a3s::research::StopConditionAssessment {
                    condition_index: 0,
                    status: a3s::research::ContractAssessmentStatus::Satisfied,
                    rationale: "The conclusion is traceable.".to_string(),
                    evidence_ids: vec!["evidence:core".to_string()],
                }],
                diagnostics: Vec::new(),
            },
        },
    ];
    let state = replay(&events, &InquiryLimits::default()).expect("completed inquiry projection");
    assert_eq!(state.phase, InquiryPhase::Outlining);
    let mut output = serde_json::json!({
        "mode": "inquiry_collection_wave",
        "execution": {
            "mode": "collect_only",
            "terminal_authority": "host_inquiry_reducer"
        },
        "research": {
            "status": "success",
            "results": [{
                "success": true,
                "structured": {
                    "summary": "The accepted evidence establishes the conclusion.",
                    "sources": [{
                        "title": "Core source",
                        "url_or_path": "https://example.com/core",
                        "quote_or_fact": "The source supports the conclusion.",
                        "reliability": "Authoritative source"
                    }],
                    "key_evidence": ["The conclusion is source-backed."],
                    "contradictions": [],
                    "confidence": "high",
                    "gaps": []
                }
            }]
        },
        "inquiry": {
            "events": events,
            "state": state
        }
    });

    assert_eq!(deep_research_collection_status(&output), "completed");
    assert!(!deep_research_workflow_needs_recovery_report(
        &output.to_string()
    ));
    assert!(deep_research_evidence_package_is_complete_for_query(
        "Determine the conclusion",
        DeepResearchEvidenceScope::WebAndWorkspace,
        &output.to_string(),
        None,
    ));
    assert_eq!(
        deep_research_report_outcome_for_workflow(
            "Determine the conclusion",
            DeepResearchEvidenceScope::WebAndWorkspace,
            &output.to_string(),
            None,
        ),
        DeepResearchRunOutcome::Completed,
    );

    let nonterminal_events = output["inquiry"]["events"]
        .as_array()
        .expect("inquiry events")
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>();
    let decoded_nonterminal_events: Vec<InquiryEvent> =
        serde_json::from_value(serde_json::Value::Array(nonterminal_events.clone()))
            .expect("decode nonterminal inquiry events");
    let nonterminal_state = replay(&decoded_nonterminal_events, &InquiryLimits::default())
        .expect("replay nonterminal inquiry");
    let mut nonterminal = output.clone();
    nonterminal["inquiry"]["events"] = serde_json::Value::Array(nonterminal_events);
    nonterminal["inquiry"]["state"] = serde_json::to_value(nonterminal_state).unwrap();
    assert_eq!(deep_research_collection_status(&nonterminal), "degraded");
    assert_eq!(
        deep_research_report_outcome_for_workflow(
            "Determine the conclusion",
            DeepResearchEvidenceScope::WebAndWorkspace,
            &nonterminal.to_string(),
            None,
        ),
        DeepResearchRunOutcome::Degraded,
    );

    let mut missing_projection = output.clone();
    missing_projection
        .as_object_mut()
        .unwrap()
        .remove("inquiry");
    assert_eq!(
        deep_research_collection_status(&missing_projection),
        "degraded"
    );
    assert_eq!(
        deep_research_report_outcome_for_workflow(
            "Determine the conclusion",
            DeepResearchEvidenceScope::WebAndWorkspace,
            &missing_projection.to_string(),
            None,
        ),
        DeepResearchRunOutcome::Degraded,
    );

    output["inquiry"]["state"]["events_applied"] = serde_json::json!(999);
    assert_eq!(deep_research_collection_status(&output), "degraded");
    assert!(deep_research_workflow_needs_recovery_report(
        &output.to_string()
    ));
    assert_eq!(
        deep_research_report_outcome_for_workflow(
            "Determine the conclusion",
            DeepResearchEvidenceScope::WebAndWorkspace,
            &output.to_string(),
            None,
        ),
        DeepResearchRunOutcome::Degraded,
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
