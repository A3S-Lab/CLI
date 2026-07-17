use super::super::deep_research_evidence_ledger::{AcceptedClaim, AcceptedSource, SourceTier};
use super::*;
use a3s::research::{
    replay, EvidenceRef, InquiryEvent, InquiryLimits, OutlineSection, Question, QuestionStatus,
    ResearchMethod, ResearchOutline, SectionDraft,
};

fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn report_budget_covers_the_unrevised_pipeline_and_bounds_revision_time() {
    let unrevised_local_caps =
        OUTLINE_TIMEOUT_MS + SECTION_WORKFLOW_TIMEOUT_MS + FRAME_WORKFLOW_TIMEOUT_MS;

    assert_eq!(
        SECTIONED_REPORT_BUDGET_MS,
        unrevised_local_caps + REPORT_FINALIZATION_RESERVE_MS
    );
    assert!(SECTIONED_REPORT_BUDGET_MS < 11 * 60 * 1000);
}

#[test]
fn report_deadline_caps_each_tool_to_the_shared_remaining_budget() {
    let started_at = Instant::now();
    let deadline =
        ReportDeadline::new(started_at + Duration::from_millis(SECTIONED_REPORT_BUDGET_MS));

    assert_eq!(
        deadline
            .tool_timeout_ms(started_at, OUTLINE_TIMEOUT_MS, "outline")
            .unwrap(),
        OUTLINE_TIMEOUT_MS
    );

    let ten_seconds_of_active_budget = started_at
        + Duration::from_millis(
            SECTIONED_REPORT_BUDGET_MS - REPORT_FINALIZATION_RESERVE_MS - 10_000,
        );
    assert_eq!(
        deadline
            .tool_timeout_ms(
                ten_seconds_of_active_budget,
                FRAME_WORKFLOW_TIMEOUT_MS,
                "frame",
            )
            .unwrap(),
        10_000
    );

    let finalization_window = started_at
        + Duration::from_millis(SECTIONED_REPORT_BUDGET_MS - REPORT_FINALIZATION_RESERVE_MS);
    let error = deadline
        .tool_timeout_ms(finalization_window, SECTION_WORKFLOW_TIMEOUT_MS, "revision")
        .unwrap_err();
    assert!(error.contains("budget exhausted before revision"));
    assert!(error.contains("reserving"));
}

fn sample_evidence() -> AcceptedEvidence {
    AcceptedEvidence {
        id: "evidence:one".to_string(),
        summary: "Supported".to_string(),
        confidence: Some("high".to_string()),
        sources: vec![AcceptedSource {
            id: "source:one".to_string(),
            anchor: "https://example.com/source".to_string(),
            title: Some("Primary source".to_string()),
            date: None,
            reliability: Some("authoritative".to_string()),
            quote_or_fact: Some("The source supports the answer.".to_string()),
            tier: SourceTier::Authoritative,
        }],
        claims: vec![AcceptedClaim {
            id: "claim:one".to_string(),
            text: "The accepted claim".to_string(),
        }],
        contradictions: Vec::new(),
        gaps: Vec::new(),
    }
}

fn second_evidence() -> AcceptedEvidence {
    AcceptedEvidence {
        id: "evidence:two".to_string(),
        summary: "Independently supported".to_string(),
        confidence: Some("high".to_string()),
        sources: vec![AcceptedSource {
            id: "source:two".to_string(),
            anchor: "https://example.com/second".to_string(),
            title: Some("Second source".to_string()),
            date: None,
            reliability: Some("authoritative".to_string()),
            quote_or_fact: Some("The second source supports only the second claim.".to_string()),
            tier: SourceTier::Authoritative,
        }],
        claims: vec![AcceptedClaim {
            id: "claim:two".to_string(),
            text: "The second accepted claim".to_string(),
        }],
        contradictions: Vec::new(),
        gaps: Vec::new(),
    }
}

fn sectioned_inquiry_snapshots() -> (
    Vec<InquiryEvent>,
    InquiryState,
    Vec<InquiryEvent>,
    InquiryState,
) {
    let mut question = Question::queued("question:one", None, "What does the evidence establish?");
    question.obligation_ids = vec!["obligation:one".to_string()];
    let mut collected = vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        InquiryEvent::ResearchObligationsCommitted {
            obligations: vec![a3s::research::ResearchObligation::new(
                "obligation:one",
                "Evidence-backed answer",
                "Establish what the accepted evidence supports",
                true,
                vec!["The answer is supported by traceable evidence".to_string()],
            )],
            stop_conditions: vec!["The answer is traceable to accepted evidence".to_string()],
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![question],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:one",
                vec!["claim:one".to_string()],
                vec!["source:one".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:one".to_string(),
            answer: "The evidence establishes the accepted claim.".to_string(),
            evidence_ids: vec!["evidence:one".to_string()],
        },
        InquiryEvent::ResearchContractAssessed {
            assessment: a3s::research::ResearchContractAssessment {
                obligations: vec![a3s::research::ResearchObligationAssessment {
                    obligation_id: "obligation:one".to_string(),
                    criteria: vec![a3s::research::CompletionCriterionAssessment {
                        criterion_index: 0,
                        status: a3s::research::ContractAssessmentStatus::Satisfied,
                        rationale: "The accepted evidence satisfies the criterion.".to_string(),
                        evidence_ids: vec!["evidence:one".to_string()],
                    }],
                    primary_source: None,
                    independent_corroboration: None,
                }],
                stop_conditions: vec![a3s::research::StopConditionAssessment {
                    condition_index: 0,
                    status: a3s::research::ContractAssessmentStatus::Satisfied,
                    rationale: "The answer is traceable.".to_string(),
                    evidence_ids: vec!["evidence:one".to_string()],
                }],
                diagnostics: Vec::new(),
            },
        },
    ];
    let collected_state = replay(&collected, &InquiryLimits::default()).unwrap();
    let collected_len = collected.len();
    collected.extend([
        InquiryEvent::OutlineCommitted {
            outline: ResearchOutline {
                sections: vec![OutlineSection {
                    id: "section:answer".to_string(),
                    heading: "Answer".to_string(),
                    purpose: "Answer the question.".to_string(),
                    perspective_ids: Vec::new(),
                    question_ids: vec!["question:one".to_string()],
                    claim_ids: vec!["claim:one".to_string()],
                    source_ids: vec!["source:one".to_string()],
                    composition_hint: "Lead with the evidence.".to_string(),
                }],
            },
        },
        InquiryEvent::SectionDrafted {
            section_id: "section:answer".to_string(),
            content: "The accepted claim is supported by the source.".to_string(),
            citation_ids: vec!["claim:one".to_string(), "source:one".to_string()],
        },
        InquiryEvent::AuditCompleted {
            passed: true,
            issues: Vec::new(),
        },
    ]);
    let completed_state = replay(&collected, &InquiryLimits::default()).unwrap();
    (
        collected[..collected_len].to_vec(),
        collected_state,
        collected,
        completed_state,
    )
}

#[test]
fn sectioned_merge_preserves_retrieval_context_and_commits_only_terminal_projection() {
    let (collected_events, collected_state, completed_events, completed_state) =
        sectioned_inquiry_snapshots();
    let mut workflow = serde_json::json!({
        "mode": "inquiry_collection_wave",
        "execution": {
            "mode": "collect_only",
            "terminal_authority": "host_inquiry_reducer"
        },
        "inquiry": {
            "events": collected_events,
            "state": collected_state,
            "scout": {"query": "source landscape"},
            "retrieval_waves": [{"id": "wave:2"}]
        }
    })
    .to_string();
    let generation = serde_json::json!({
        "inquiry": {"events": completed_events, "state": completed_state}
    });

    assert!(merge_sectioned_inquiry_projection(&mut workflow, None, Some(&generation)).unwrap());
    let merged: Value = serde_json::from_str(&workflow).unwrap();
    assert_eq!(
        merged["inquiry"]["scout"],
        serde_json::json!({"query": "source landscape"})
    );
    assert_eq!(
        merged["inquiry"]["retrieval_waves"],
        serde_json::json!([{"id": "wave:2"}])
    );
    assert_eq!(merged["inquiry"]["state"]["phase"], "completed");
}

#[test]
fn inquiry_backed_sectioned_merge_rejects_missing_or_nonterminal_generation_projection() {
    let (collected_events, collected_state, _, _) = sectioned_inquiry_snapshots();
    let mut workflow = serde_json::json!({
        "mode": "inquiry_collection_wave",
        "execution": {"terminal_authority": "host_inquiry_reducer"},
        "inquiry": {"events": collected_events, "state": collected_state}
    })
    .to_string();

    let missing = merge_sectioned_inquiry_projection(&mut workflow, None, None).unwrap_err();
    assert!(
        missing.contains("required terminal Inquiry projection"),
        "{missing}"
    );

    let parsed: Value = serde_json::from_str(&workflow).unwrap();
    let nonterminal = serde_json::json!({"inquiry": parsed["inquiry"].clone()});
    let error =
        merge_sectioned_inquiry_projection(&mut workflow, None, Some(&nonterminal)).unwrap_err();
    assert!(error.contains("must reach Completed"), "{error}");
}

#[test]
fn outline_schema_closes_all_reference_catalogs() {
    let mut context = OutlineValidationContext {
        allowed_perspective_ids: ids(&["perspective:a", "perspective:b"]),
        allowed_question_ids: ids(&["question:a"]),
        allowed_claim_ids: ids(&["claim:a", "claim:b"]),
        allowed_source_ids: ids(&["source:a"]),
        ..OutlineValidationContext::default()
    };
    let schema = closed_outline_schema(&context, MAX_FOCUSED_REPORT_SECTIONS).unwrap();
    assert_eq!(
        schema["properties"]["sections"]["maxItems"],
        MAX_FOCUSED_REPORT_SECTIONS
    );
    let properties = &schema["properties"]["sections"]["items"]["properties"];
    for (field, allowed) in [
        ("perspective_ids", &["perspective:a", "perspective:b"][..]),
        ("question_ids", &["question:a"][..]),
        ("claim_ids", &["claim:a", "claim:b"][..]),
        ("source_ids", &["source:a"][..]),
    ] {
        assert_eq!(properties[field]["maxItems"], allowed.len());
        assert_eq!(
            properties[field]["items"]["enum"],
            serde_json::json!(allowed)
        );
    }
    context.allowed_perspective_ids.clear();
    context.allowed_question_ids.clear();
    let schema = closed_outline_schema(&context, MAX_REPORT_SECTIONS).unwrap();
    let properties = &schema["properties"]["sections"]["items"]["properties"];
    for field in ["perspective_ids", "question_ids"] {
        assert_eq!(properties[field]["maxItems"], 0);
        assert!(properties[field]["items"].get("enum").is_none());
    }
}

#[test]
fn section_schema_separates_closed_claim_and_source_ids_with_duplicate_catalogs() {
    let section = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Answer the query".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
        composition_hint: "Lead with the finding".to_string(),
    };
    let evidence = vec![sample_evidence(), sample_evidence()];
    let args =
        section_generation_args("query", &section, &InquiryState::default(), &evidence).unwrap();
    let properties = &args["schema"]["properties"];
    assert!(properties.get("citation_ids").is_none());
    assert_eq!(
        properties["claim_ids"]["items"]["enum"],
        serde_json::json!(["claim:one"])
    );
    assert_eq!(
        properties["source_ids"]["items"]["enum"],
        serde_json::json!(["source:one"])
    );
    assert_eq!(properties["claim_ids"]["minItems"], 1);
    assert_eq!(properties["claim_ids"]["maxItems"], 1);
    assert_eq!(properties["source_ids"]["minItems"], 1);
    assert_eq!(properties["source_ids"]["maxItems"], 1);
    assert_eq!(
        args["schema"]["required"],
        serde_json::json!(["section_id", "markdown", "claim_ids", "source_ids"])
    );
}

#[test]
fn outline_packet_preserves_question_evidence_claim_source_graph() {
    let mut question = Question::queued(
        "question:one",
        None,
        "What does the accepted evidence establish?",
    );
    question.status = QuestionStatus::Answered;
    question.answer = Some("The accepted claim is established.".to_string());
    question.evidence_ids = vec!["evidence:one".to_string()];
    let mut state = InquiryState {
        method: Some(ResearchMethod::Focused),
        questions: vec![question],
        ..InquiryState::default()
    };
    let reference = EvidenceRef::new(
        "evidence:one",
        vec!["claim:one".to_string()],
        vec!["source:one".to_string()],
    );
    state
        .evidence_catalog
        .insert("evidence:one".to_string(), reference);
    state.claim_catalog.insert("claim:one".to_string());
    state.source_catalog.insert("source:one".to_string());
    let context = state.outline_validation_context();

    let packet = closed_outline_packet("query", &state, &[sample_evidence()], &context).unwrap();
    let graph = &packet["evidence_graph"];
    assert_eq!(
        graph["question_evidence_bindings"][0],
        serde_json::json!({
            "question_id": "question:one",
            "obligation_ids": [],
            "evidence_ids": ["evidence:one"],
        })
    );
    assert_eq!(graph["evidence_items"][0]["id"], "evidence:one");
    assert_eq!(graph["evidence_items"][0]["claims"][0]["id"], "claim:one");
    assert_eq!(graph["evidence_items"][0]["sources"][0]["id"], "source:one");
    assert!(packet.get("claims").is_none());
    assert!(packet.get("sources").is_none());
}

#[test]
fn section_packet_keeps_claims_and_sources_grouped_by_evidence() {
    let section = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Answer the query".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string(), "claim:two".to_string()],
        source_ids: vec!["source:one".to_string(), "source:two".to_string()],
        composition_hint: "Lead with the finding".to_string(),
    };
    let args = section_generation_args(
        "query",
        &section,
        &InquiryState::default(),
        &[sample_evidence(), second_evidence()],
    )
    .unwrap();
    let prompt = args["prompt"].as_str().unwrap();
    assert_eq!(args["schema"]["properties"]["claim_ids"]["minItems"], 2);
    assert_eq!(args["schema"]["properties"]["source_ids"]["minItems"], 2);
    let packet: serde_json::Value = serde_json::from_str(
        prompt
            .split_once("CLOSED_SECTION_PACKET=")
            .map(|(_, packet)| packet)
            .unwrap(),
    )
    .unwrap();
    assert!(packet.get("claims").is_none());
    assert!(packet.get("sources").is_none());
    assert_eq!(
        packet["evidence_bindings"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(
        packet["evidence_bindings"][0]["claims"][0]["id"],
        "claim:one"
    );
    assert_eq!(
        packet["evidence_bindings"][0]["sources"][0]["id"],
        "source:one"
    );
    assert_eq!(
        packet["evidence_bindings"][1]["claims"][0]["id"],
        "claim:two"
    );
    assert_eq!(
        packet["evidence_bindings"][1]["sources"][0]["id"],
        "source:two"
    );
}

#[test]
fn section_packet_preserves_question_status_and_bound_reason() {
    let mut question = Question::queued(
        "question:bounded",
        None,
        "Which supporting detail remains unavailable?",
    );
    question.material = false;
    question.status = QuestionStatus::Bounded;
    question.bound_reason = Some("The closed evidence does not establish this detail.".to_string());
    let state = InquiryState {
        questions: vec![question],
        ..InquiryState::default()
    };
    let section = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Answer with the bounded uncertainty".to_string(),
        perspective_ids: Vec::new(),
        question_ids: vec!["question:bounded".to_string()],
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
        composition_hint: "State the supported answer and its bounded uncertainty".to_string(),
    };

    let packet = section_generation_packet("query", &section, &state, &[sample_evidence()])
        .expect("section packet");

    assert_eq!(packet["questions"][0]["status"], "bounded");
    assert_eq!(
        packet["questions"][0]["bound_reason"],
        "The closed evidence does not establish this detail."
    );
}

#[test]
fn section_cannot_omit_a_committed_outline_claim_or_source() {
    let planned = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Answer the query".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string(), "claim:two".to_string()],
        source_ids: vec!["source:one".to_string(), "source:two".to_string()],
        composition_hint: "Lead with the finding".to_string(),
    };
    let section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown:
            "The accepted claim is supported by [the primary source](https://example.com/source)."
                .to_string(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
    };

    let error = validate_section_obligation_coverage(&section, &planned)
        .expect_err("a writer must not silently drop committed outline evidence");
    assert!(error.contains("claim:two"), "{error}");
}

#[test]
fn section_citations_merge_and_deduplicate_claims_and_sources() {
    let section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown: "Draft".to_string(),
        claim_ids: vec![
            "claim:one".to_string(),
            "shared:id".to_string(),
            "claim:one".to_string(),
        ],
        source_ids: vec![
            "shared:id".to_string(),
            "source:one".to_string(),
            "source:one".to_string(),
        ],
    };
    assert_eq!(
        section.citation_ids(),
        vec!["claim:one", "shared:id", "source:one"]
    );
}

#[test]
fn section_body_passes_with_declared_claim_and_inline_source() {
    let section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown:
            "The accepted claim is supported by [the primary source](https://example.com/source)."
                .to_string(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
    };
    let resolved = audit_section_generation(&section, &[sample_evidence()]).unwrap();
    assert_eq!(resolved.claim_ids, ids(&["claim:one"]));
    assert_eq!(resolved.source_ids, ids(&["source:one"]));
}

#[test]
fn section_body_rejects_a_longer_link_that_only_contains_the_source_anchor() {
    let section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown: "The accepted claim cites [a different source](https://example.com/source-long)."
            .to_string(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
    };

    let error = audit_section_generation(&section, &[sample_evidence()])
        .expect_err("a source anchor substring must not count as an exact citation");
    assert!(error.contains("cites none"), "{error}");
}

#[test]
fn section_body_cannot_borrow_a_citation_from_the_global_source_ledger() {
    let section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown: "The accepted claim is supported by the retained evidence.".to_string(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
    };
    let mut document_with_ledger = section.markdown.clone();
    document_with_ledger
        .push_str("\n\n## Sources\n\n- [Primary source](https://example.com/source)");
    assert!(document_with_ledger.contains("https://example.com/source"));

    let error = audit_section_generation(&section, &[sample_evidence()]).unwrap_err();
    assert!(error.contains("cites none"), "{error}");
}

#[test]
fn section_audit_rejects_cross_evidence_claim_source_pairing() {
    let section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown:
            "The accepted claim is linked to [an unrelated source](https://example.com/second)."
                .to_string(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:two".to_string()],
    };

    let error = audit_section_generation(&section, &[sample_evidence(), second_evidence()])
        .expect_err("a source from another evidence item must not ground the claim");
    assert!(error.contains("same accepted evidence item"), "{error}");
}

#[test]
fn section_audit_requires_the_linked_source_to_appear_inline() {
    let section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown:
            "The accepted claim cites only [the unrelated source](https://example.com/second)."
                .to_string(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string(), "source:two".to_string()],
    };

    let error = audit_section_generation(&section, &[sample_evidence(), second_evidence()])
        .expect_err("a cited unrelated source must not satisfy the claim's linked source");
    assert!(
        error.contains("inline source from the same accepted evidence item"),
        "{error}"
    );
}

#[test]
fn section_audit_requires_every_declared_claim_to_be_covered() {
    let mut second = second_evidence();
    second.claims[0].text =
        "Orbital telemetry establishes a materially distinct finding.".to_string();
    let section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown: "The accepted claim cites [the primary source](https://example.com/source), while unrelated prose cites [the second source](https://example.com/second).".to_string(),
        claim_ids: vec!["claim:one".to_string(), "claim:two".to_string()],
        source_ids: vec!["source:one".to_string(), "source:two".to_string()],
    };

    let error = audit_section_generation(&section, &[sample_evidence(), second])
        .expect_err("declaring a claim must not replace covering it in the body");
    assert!(
        error.contains("claim ID `claim:two` is not covered"),
        "{error}"
    );
}

#[test]
fn section_audit_rejects_declared_but_uncited_sources() {
    let mut evidence = sample_evidence();
    evidence.sources.push(AcceptedSource {
        id: "source:backup".to_string(),
        anchor: "https://example.com/backup".to_string(),
        title: Some("Backup source".to_string()),
        date: None,
        reliability: Some("secondary".to_string()),
        quote_or_fact: Some("The backup source also supports the answer.".to_string()),
        tier: SourceTier::Secondary,
    });
    let section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown:
            "The accepted claim is supported by [the primary source](https://example.com/source)."
                .to_string(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string(), "source:backup".to_string()],
    };

    let error = audit_section_generation(&section, &[evidence])
        .expect_err("source declarations must match actual inline citations");
    assert!(
        error.contains("declared source ID `source:backup` but did not cite"),
        "{error}"
    );
}

#[test]
fn assembled_report_follows_committed_outline_order_and_has_source_ledger() {
    let outline = ResearchOutline {
        sections: vec![OutlineSection {
            id: "section:answer".to_string(),
            heading: "Answer".to_string(),
            purpose: "Answer the query".to_string(),
            perspective_ids: Vec::new(),
            question_ids: Vec::new(),
            claim_ids: vec!["claim:one".to_string()],
            source_ids: vec!["source:one".to_string()],
            composition_hint: "Lead with the finding".to_string(),
        }],
    };
    let mut state = InquiryState::default();
    state.drafts.insert(
        "section:answer".to_string(),
        SectionDraft {
            section_id: "section:answer".to_string(),
            content: "The accepted source supports the answer.".to_string(),
            citation_ids: vec!["source:one".to_string()],
        },
    );
    let frame = ReportFrame {
        report_title: "Useful report".to_string(),
        editorial: ReportEditorialPlan {
            thesis: "The accepted evidence directly supports the bounded answer.".to_string(),
            track_coverage: Vec::new(),
        },
        presentation: ReportPresentation::default(),
    };
    let evidence = vec![sample_evidence()];
    let assembled =
        assemble_markdown(&frame, &outline, &state, &ids(&["source:one"]), &evidence).unwrap();
    assert!(assembled.body.starts_with("# Useful report"));
    assert!(assembled.body.contains("## Answer"));
    assert!(!assembled.body.contains("## Sources"));
    assert!(assembled.markdown.contains("## Sources"));
    assert!(assembled.markdown.contains("https://example.com/source"));
}

#[test]
fn report_generation_uses_a_small_execution_adapter() {
    assert!(SECTION_WORKFLOW_SOURCE.contains("schedule_steps"));
    assert!(SECTION_WORKFLOW_SOURCE.contains("generate_object"));
    assert!(SECTION_WORKFLOW_SOURCE.len() < 4_000);
    assert!(!SECTION_WORKFLOW_SOURCE.contains("research_method"));
}

#[test]
fn durable_projection_must_extend_the_collected_workflow_prefix() {
    let workflow_events = vec![InquiryEvent::StrategySelected {
        method: ResearchMethod::Focused,
    }];
    let workflow_state = replay(&workflow_events, &InquiryLimits::default()).unwrap();
    let mut journal_events = workflow_events.clone();
    journal_events.push(InquiryEvent::BudgetExhausted {
        reason: "test terminal boundary".to_string(),
    });
    let journal_state = replay(&journal_events, &InquiryLimits::default()).unwrap();

    let (selected_events, selected_state) = recovery::select_projection(
        workflow_events.clone(),
        workflow_state.clone(),
        Some((journal_events.clone(), journal_state.clone())),
    )
    .unwrap();
    assert_eq!(selected_events, journal_events);
    assert_eq!(selected_state, journal_state);

    let stale = recovery::select_projection(
        workflow_events.clone(),
        workflow_state.clone(),
        Some((Vec::new(), InquiryState::default())),
    )
    .unwrap_err();
    assert!(stale.contains("does not extend"), "{stale}");

    let divergent_events = vec![InquiryEvent::StrategySelected {
        method: ResearchMethod::PerspectiveGuided,
    }];
    let divergent_state = replay(&divergent_events, &InquiryLimits::default()).unwrap();
    let divergent = recovery::select_projection(
        workflow_events,
        workflow_state,
        Some((divergent_events, divergent_state)),
    )
    .unwrap_err();
    assert!(divergent.contains("does not extend"), "{divergent}");
}

#[test]
fn partial_drafts_restore_without_reinventing_section_evidence_bindings() {
    let outline = ResearchOutline {
        sections: vec![
            OutlineSection {
                id: "section:one".to_string(),
                heading: "One".to_string(),
                purpose: "First answer".to_string(),
                perspective_ids: Vec::new(),
                question_ids: Vec::new(),
                claim_ids: vec!["claim:one".to_string()],
                source_ids: vec!["source:one".to_string()],
                composition_hint: "Lead with one".to_string(),
            },
            OutlineSection {
                id: "section:two".to_string(),
                heading: "Two".to_string(),
                purpose: "Second answer".to_string(),
                perspective_ids: Vec::new(),
                question_ids: Vec::new(),
                claim_ids: vec!["claim:two".to_string()],
                source_ids: vec!["source:two".to_string()],
                composition_hint: "Lead with two".to_string(),
            },
        ],
    };
    let mut state = InquiryState::default();
    state.drafts.insert(
        "section:one".to_string(),
        SectionDraft {
            section_id: "section:one".to_string(),
            content: "A durable first section.".to_string(),
            citation_ids: vec!["claim:one".to_string(), "source:one".to_string()],
        },
    );

    let restored = recovery::sections_from_drafts(&outline, &state).unwrap();
    assert_eq!(restored.len(), 1);
    let section = &restored["section:one"];
    assert_eq!(section.markdown, "A durable first section.");
    assert_eq!(section.claim_ids, vec!["claim:one"]);
    assert_eq!(section.source_ids, vec!["source:one"]);
    assert!(!restored.contains_key("section:two"));
    assert_eq!(
        recovery::missing_section_ids(&outline, &restored),
        vec!["section:two"]
    );
}

#[test]
fn failed_audit_and_redraft_form_one_replayable_revision_boundary() {
    let (_, _, mut events, completed_state) = sectioned_inquiry_snapshots();
    events.pop();
    let mut state = replay(&events, &InquiryLimits::default()).unwrap();
    assert_eq!(state.phase, InquiryPhase::Auditing);

    apply_event(
        &mut state,
        &mut events,
        InquiryEvent::AuditCompleted {
            passed: false,
            issues: vec!["structured audit failure".to_string()],
        },
    )
    .unwrap();
    assert_eq!(state.phase, InquiryPhase::Drafting);
    assert_eq!(
        recovery::resume_mode(&state).unwrap(),
        recovery::ReportResumeMode::RecoverFailedAudit
    );
    assert_eq!(recovery::restored_revision_rounds(&state), 0);
    apply_event(
        &mut state,
        &mut events,
        InquiryEvent::SectionRevisionStarted {
            round: 1,
            section_ids: vec!["section:answer".to_string()],
            input_digest: "revision-input-one".to_string(),
        },
    )
    .unwrap();
    assert_eq!(recovery::restored_revision_rounds(&state), 1);
    apply_event(
        &mut state,
        &mut events,
        InquiryEvent::SectionDrafted {
            section_id: "section:answer".to_string(),
            content: "The revised accepted claim is supported by the source.".to_string(),
            citation_ids: vec!["claim:one".to_string(), "source:one".to_string()],
        },
    )
    .unwrap();
    apply_event(
        &mut state,
        &mut events,
        InquiryEvent::SectionRevisionCommitted {
            round: 1,
            input_digest: "revision-input-one".to_string(),
        },
    )
    .unwrap();
    assert_eq!(state.phase, InquiryPhase::Auditing);
    assert_eq!(recovery::restored_revision_rounds(&state), 1);
    assert_eq!(replay(&events, &InquiryLimits::default()).unwrap(), state);

    assert_eq!(
        recovery::resume_mode(&completed_state).unwrap(),
        recovery::ReportResumeMode::VerifyCompleted
    );
    assert_eq!(recovery::restored_revision_rounds(&completed_state), 0);
}

#[test]
fn active_revision_reuses_identical_input_and_rejects_conflicts() {
    let (_, _, mut events, _) = sectioned_inquiry_snapshots();
    events.pop();
    events.pop();
    let mut state = replay(&events, &InquiryLimits::default()).unwrap();
    let targets = vec!["section:answer".to_string()];
    let start = revision::revision_start_event(
        &state,
        1,
        &targets,
        "stable-input-digest",
        "host validation failed",
    )
    .unwrap()
    .unwrap();
    apply_event(&mut state, &mut events, start).unwrap();

    assert_eq!(
        revision::revision_start_event(
            &state,
            1,
            &targets,
            "stable-input-digest",
            "host validation failed",
        )
        .unwrap(),
        None
    );
    let conflict = revision::revision_start_event(
        &state,
        1,
        &targets,
        "different-input-digest",
        "host validation failed",
    )
    .unwrap_err();
    assert!(
        conflict.contains("conflicts with recovered input"),
        "{conflict}"
    );
    assert_eq!(replay(&events, &InquiryLimits::default()).unwrap(), state);
}

#[test]
fn initial_validation_repairs_share_the_global_two_round_budget() {
    let (_, _, mut events, _) = sectioned_inquiry_snapshots();
    events.pop();
    events.pop();
    let mut state = replay(&events, &InquiryLimits::default()).unwrap();
    let targets = vec!["section:answer".to_string()];

    for round in 1..=2 {
        let digest = format!("validation-repair-{round}");
        let start = revision::revision_start_event(
            &state,
            round,
            &targets,
            &digest,
            "host validation failed",
        )
        .unwrap()
        .unwrap();
        apply_event(&mut state, &mut events, start).unwrap();
        apply_event(
            &mut state,
            &mut events,
            InquiryEvent::SectionDrafted {
                section_id: "section:answer".to_string(),
                content: format!(
                    "The accepted claim remains supported after validation repair {round}."
                ),
                citation_ids: vec!["claim:one".to_string(), "source:one".to_string()],
            },
        )
        .unwrap();
        apply_event(
            &mut state,
            &mut events,
            InquiryEvent::SectionRevisionCommitted {
                round,
                input_digest: digest,
            },
        )
        .unwrap();
    }

    assert_eq!(state.audit_attempts, 0);
    assert_eq!(recovery::restored_revision_rounds(&state), 2);
    assert_eq!(replay(&events, &InquiryLimits::default()).unwrap(), state);
    let exhausted = revision::revision_start_event(
        &state,
        3,
        &targets,
        "validation-repair-3",
        "host validation still failed",
    )
    .unwrap_err();
    assert!(
        exhausted.contains("after 2 targeted revision rounds"),
        "{exhausted}"
    );
}
