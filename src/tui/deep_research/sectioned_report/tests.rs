use super::super::deep_research_evidence_ledger::{AcceptedClaim, AcceptedSource, SourceTier};
use super::*;
use a3s::research::{
    EvidenceRef, OutlineSection, Question, QuestionStatus, ResearchMethod, ResearchOutline,
    SectionDraft,
};

fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
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
