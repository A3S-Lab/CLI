use super::super::deep_research_evidence_ledger::{
    AcceptedClaim, AcceptedSource, AcceptedSourceExcerpt,
};
use super::*;
use a3s::research::{
    replay, EvidenceRef, InquiryEvent, InquiryLimits, OutlineSection, Question, QuestionStatus,
    ResearchMethod, ResearchOutline, SectionDraft,
};
use a3s_code_core::tools::{Tool, ToolContext, ToolExecutor, ToolOutput};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct RetryOnceReportGenerationFixture {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for RetryOnceReportGenerationFixture {
    fn name(&self) -> &str {
        "generate_object"
    }

    fn description(&self) -> &str {
        "Fails one report generation attempt before returning a structured object."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(ToolOutput::error(
                "generate_object timed out after the bounded attempt",
            ));
        }
        Ok(ToolOutput::success(
            serde_json::json!({
                "object": {"durable": true},
                "repair_rounds": 0,
                "mode_used": "fixture"
            })
            .to_string(),
        ))
    }
}

fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn sample_reader_labels() -> ReportReaderLabels {
    ReportReaderLabels {
        qualification_heading: "Evidence boundaries".to_string(),
        qualification_intro: "The following points remain bounded.".to_string(),
        sources_heading: "Sources".to_string(),
        decision_heading: "Decision guidance".to_string(),
        evidence_limitation: "Evidence limitation".to_string(),
        primary_source_support: "Primary-source support".to_string(),
        independent_corroboration: "Independent corroboration".to_string(),
        established_boundary: "The evidence establishes this point.".to_string(),
        qualified_boundary: "The evidence supports a qualified conclusion.".to_string(),
        unresolved_boundary: "The evidence does not establish this point.".to_string(),
    }
}

fn clear_semantic_checks_json() -> Value {
    serde_json::json!({
        "claim_granularity": "clear",
        "derived_quantities": "clear",
        "temporal_labels": "clear",
        "compatibility_scope": "clear",
        "maintenance_scope": "clear",
        "replacement_properties": "clear",
        "promotional_attribution": "clear",
        "sample_scope": "clear",
        "unknown_item_quantifiers": "clear",
        "evidence_gap_scope": "clear",
        "recommendation_support": "clear",
        "reader_language_and_internal_jargon": "clear"
    })
}

#[test]
fn report_budget_covers_the_unrevised_pipeline_and_bounds_revision_time() {
    let first_two_acceptance_passes =
        SECTION_STAGE_BUDGET_MS + FRAME_STAGE_BUDGET_MS + (2 * SEMANTIC_AUDIT_WORKFLOW_TIMEOUT_MS);

    assert_eq!(FRAME_TIMEOUT_MS, 360_000);
    assert_eq!(FRAME_PRESENTATION_TIMEOUT_MS, 240_000);
    assert_eq!(SEMANTIC_AUDIT_TIMEOUT_MS, 360_000);
    assert_eq!(
        FRAME_CONTENT_WORKFLOW_TIMEOUT_MS,
        (2 * FRAME_TIMEOUT_MS) + SECTION_WORKFLOW_GRACE_MS
    );
    assert_eq!(
        FRAME_PRESENTATION_WORKFLOW_TIMEOUT_MS,
        (2 * FRAME_PRESENTATION_TIMEOUT_MS) + SECTION_WORKFLOW_GRACE_MS
    );
    assert_eq!(
        FRAME_STAGE_BUDGET_MS,
        FRAME_CONTENT_WORKFLOW_TIMEOUT_MS + FRAME_PRESENTATION_WORKFLOW_TIMEOUT_MS
    );
    assert_eq!(
        SECTION_STAGE_BUDGET_MS,
        2 * SECTION_UNIT_WORKFLOW_TIMEOUT_MS
    );
    assert_eq!(
        SECTIONED_REPORT_BUDGET_MS,
        first_two_acceptance_passes
            + FINAL_TARGETED_REPAIR_RESERVE_MS
            + REPORT_FINALIZATION_RESERVE_MS
    );
    assert_eq!(FINAL_TARGETED_REPAIR_RESERVE_MS, 30 * 60 * 1000);
    const { assert!(SECTIONED_REPORT_BUDGET_MS < 97 * 60 * 1000) };
}

#[test]
fn active_report_orchestration_has_no_unbounded_audit_or_revision_loop() {
    let orchestration = include_str!("acceptance.rs");
    assert!(
        !orchestration.contains("loop {"),
        "active report acceptance must spell out its bounded audit and repair passes"
    );
    assert!(orchestration.contains("\"semantic_audit_1\""));
    assert!(orchestration.contains("\"semantic_audit_2\""));
    assert!(orchestration.contains("\"semantic_audit_3\""));
    assert!(orchestration.contains("audit_report_semantics_for_targets"));
    assert!(orchestration.contains("merge_reaudited_targets"));

    let revision = include_str!("revision.rs");
    let validation_revision = revision
        .split_once("pub(super) async fn revise_invalid_sections_once")
        .expect("section validation revision entrypoint must exist")
        .1
        .split_once("pub(super) async fn revise_targets")
        .expect("section revision boundary must exist")
        .0;
    assert!(
        !validation_revision.contains("loop {"),
        "section validation may request at most one targeted revision"
    );
}

#[test]
fn report_deadline_caps_each_tool_to_the_shared_remaining_budget() {
    let started_at = Instant::now();
    let deadline =
        ReportDeadline::new(started_at + Duration::from_millis(SECTIONED_REPORT_BUDGET_MS));

    assert_eq!(
        deadline
            .tool_timeout_ms(
                started_at,
                SECTION_UNIT_WORKFLOW_TIMEOUT_MS,
                "section generation",
            )
            .unwrap(),
        SECTION_UNIT_WORKFLOW_TIMEOUT_MS
    );

    let ten_seconds_of_active_budget = started_at
        + Duration::from_millis(
            SECTIONED_REPORT_BUDGET_MS - REPORT_FINALIZATION_RESERVE_MS - 10_000,
        );
    assert_eq!(
        deadline
            .tool_timeout_ms(
                ten_seconds_of_active_budget,
                FRAME_CONTENT_WORKFLOW_TIMEOUT_MS,
                "frame",
            )
            .unwrap(),
        10_000
    );

    let finalization_window = started_at
        + Duration::from_millis(SECTIONED_REPORT_BUDGET_MS - REPORT_FINALIZATION_RESERVE_MS);
    let error = deadline
        .tool_timeout_ms(
            finalization_window,
            SECTION_UNIT_WORKFLOW_TIMEOUT_MS,
            "revision",
        )
        .unwrap_err();
    assert!(error.contains("budget exhausted before revision"));
    assert!(error.contains("reserving"));
}

#[test]
fn resumed_report_deadline_consumes_elapsed_durable_time() {
    let monotonic_now = Instant::now();
    let elapsed_ms = 42_000;
    let started_at_ms = 1_000_000;
    let caller_deadline = monotonic_now + Duration::from_millis(SECTIONED_REPORT_BUDGET_MS * 2);
    let deadline = ReportDeadline::from_durable_start(
        caller_deadline,
        monotonic_now,
        started_at_ms + elapsed_ms,
        started_at_ms,
    )
    .unwrap();

    assert_eq!(
        deadline.expires_at.saturating_duration_since(monotonic_now),
        Duration::from_millis(SECTIONED_REPORT_BUDGET_MS - elapsed_ms)
    );
}

#[test]
fn caller_can_shorten_but_never_extend_the_durable_report_deadline() {
    let monotonic_now = Instant::now();
    let short_caller_deadline = monotonic_now + Duration::from_secs(3);
    let shortened =
        ReportDeadline::from_durable_start(short_caller_deadline, monotonic_now, 50_000, 50_000)
            .unwrap();
    assert_eq!(shortened.expires_at, short_caller_deadline);

    let regressed_clock = ReportDeadline::from_durable_start(
        monotonic_now + Duration::from_secs(30),
        monotonic_now,
        49_999,
        50_000,
    )
    .unwrap();
    assert_eq!(regressed_clock.expires_at, monotonic_now);
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
            evidence_excerpts: Vec::new(),
        }],
        claims: vec![AcceptedClaim {
            id: "claim:one".to_string(),
            text: "The accepted claim".to_string(),
        }],
        source_coverage: Vec::new(),
        relevant_obligation_ids: Vec::new(),
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
            evidence_excerpts: Vec::new(),
        }],
        claims: vec![AcceptedClaim {
            id: "claim:two".to_string(),
            text: "The second accepted claim".to_string(),
        }],
        source_coverage: Vec::new(),
        relevant_obligation_ids: Vec::new(),
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
fn outline_schema_closes_active_reference_catalogs_without_perspectives() {
    let mut context = OutlineValidationContext {
        allowed_perspective_ids: ids(&["perspective:a", "perspective:b"]),
        allowed_question_ids: ids(&["question:a"]),
        allowed_claim_ids: ids(&["claim:a", "claim:b"]),
        allowed_source_ids: ids(&["source:a"]),
        ..OutlineValidationContext::default()
    };
    let schema = closed_outline_schema(&context, MAX_REPORT_SECTIONS).unwrap();
    assert_eq!(
        schema["properties"]["sections"]["maxItems"],
        MAX_REPORT_SECTIONS
    );
    let properties = &schema["properties"]["sections"]["items"]["properties"];
    assert!(properties.get("perspective_ids").is_none());
    for (field, allowed) in [
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
    context.allowed_question_ids.clear();
    let schema = closed_outline_schema(&context, MAX_REPORT_SECTIONS).unwrap();
    let properties = &schema["properties"]["sections"]["items"]["properties"];
    assert!(properties.get("perspective_ids").is_none());
    assert_eq!(properties["question_ids"]["maxItems"], 0);
    assert!(properties["question_ids"]["items"].get("enum").is_none());
}

#[test]
fn host_outline_uses_typed_obligation_and_evidence_edges() {
    let (_, collected_state, _, _) = sectioned_inquiry_snapshots();
    let context = collected_state.outline_validation_context();

    let outline = derive_outline("query", &collected_state, &context).unwrap();

    assert_eq!(outline.sections.len(), 1);
    assert_eq!(outline.sections[0].id, "section:1");
    assert_eq!(outline.sections[0].heading, "Evidence-backed answer");
    assert_eq!(outline.sections[0].question_ids, ["question:one"]);
    assert_eq!(outline.sections[0].claim_ids, ["claim:one"]);
    assert_eq!(outline.sections[0].source_ids, ["source:one"]);
    validate_research_outline(&outline, &context).unwrap();
}

#[test]
fn host_outline_does_not_give_unrelated_evidence_to_a_bounded_supporting_obligation() {
    let mut material = Question::queued(
        "question:material",
        None,
        "What does the accepted evidence establish?",
    );
    material.obligation_ids = vec!["obligation:material".to_string()];
    let mut supporting = Question::queued(
        "question:supporting",
        None,
        "What resource-constrained case study is available?",
    );
    supporting.material = false;
    supporting.obligation_ids = vec!["obligation:supporting".to_string()];
    let events = vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        InquiryEvent::ResearchObligationsCommitted {
            obligations: vec![
                a3s::research::ResearchObligation::new(
                    "obligation:material",
                    "Material finding",
                    "Report the accepted finding",
                    true,
                    vec!["The finding is traceable".to_string()],
                ),
                a3s::research::ResearchObligation::new(
                    "obligation:supporting",
                    "Resource case study",
                    "Bound the requested resource scenario",
                    false,
                    vec!["A resource case study is available or bounded".to_string()],
                ),
            ],
            stop_conditions: vec!["The material finding is traceable".to_string()],
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![material, supporting],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:one",
                vec!["claim:one".to_string()],
                vec!["source:one".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:material".to_string(),
            answer: "The accepted evidence establishes the finding.".to_string(),
            evidence_ids: vec!["evidence:one".to_string()],
        },
        InquiryEvent::QuestionBounded {
            question_id: "question:supporting".to_string(),
            reason: "No scoped case-study evidence was accepted.".to_string(),
        },
        InquiryEvent::ResearchContractAssessed {
            assessment: a3s::research::ResearchContractAssessment {
                obligations: vec![
                    a3s::research::ResearchObligationAssessment {
                        obligation_id: "obligation:material".to_string(),
                        criteria: vec![a3s::research::CompletionCriterionAssessment {
                            criterion_index: 0,
                            status: a3s::research::ContractAssessmentStatus::Satisfied,
                            rationale: "The material finding has accepted evidence.".to_string(),
                            evidence_ids: vec!["evidence:one".to_string()],
                        }],
                        primary_source: None,
                        independent_corroboration: None,
                    },
                    a3s::research::ResearchObligationAssessment {
                        obligation_id: "obligation:supporting".to_string(),
                        criteria: vec![a3s::research::CompletionCriterionAssessment {
                            criterion_index: 0,
                            status: a3s::research::ContractAssessmentStatus::Uncovered,
                            rationale: "The supporting case study remains bounded.".to_string(),
                            evidence_ids: Vec::new(),
                        }],
                        primary_source: None,
                        independent_corroboration: None,
                    },
                ],
                stop_conditions: vec![a3s::research::StopConditionAssessment {
                    condition_index: 0,
                    status: a3s::research::ContractAssessmentStatus::Satisfied,
                    rationale: "The material finding is traceable.".to_string(),
                    evidence_ids: vec!["evidence:one".to_string()],
                }],
                diagnostics: Vec::new(),
            },
        },
    ];
    let state = replay(&events, &InquiryLimits::default()).expect("qualified report state");
    let context = state.outline_validation_context();

    let outline = derive_outline("query", &state, &context).expect("host outline");

    assert_eq!(outline.sections.len(), 1);
    assert_eq!(outline.sections[0].heading, "Material finding");
    assert_eq!(
        outline.sections[0].question_ids,
        ["question:material", "question:supporting"]
    );
    assert_eq!(outline.sections[0].claim_ids, ["claim:one"]);
    assert_eq!(outline.sections[0].source_ids, ["source:one"]);
    validate_research_outline(&outline, &context).expect("bounded support needs no fake section");
}

#[test]
fn section_schema_leaves_committed_evidence_identity_to_the_host() {
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
    assert!(properties.get("claim_ids").is_none());
    assert!(properties.get("source_ids").is_none());
    assert_eq!(
        args["schema"]["required"],
        serde_json::json!(["section_id", "markdown"])
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
            "completion_criterion_indexes": [],
            "evidence_ids": ["evidence:one"],
        })
    );
    assert_eq!(graph["evidence_items"][0]["id"], "evidence:one");
    assert_eq!(graph["evidence_items"][0]["claims"][0]["id"], "claim:one");
    assert_eq!(graph["evidence_items"][0]["sources"][0]["id"], "source:one");
    assert_eq!(
        graph["evidence_items"][0]["sources"][0]["anchor"],
        "https://example.com/source"
    );
    assert!(graph["evidence_items"][0]["sources"][0]
        .get("quote_or_fact")
        .is_none());
    assert!(graph["evidence_items"][0]["sources"][0]
        .get("evidence_excerpts")
        .is_none());
    assert!(packet.get("claims").is_none());
    assert!(packet.get("sources").is_none());
    assert!(packet.get("research_method").is_none());
    assert!(packet.get("perspectives").is_none());
    assert!(packet.get("allowed_perspective_ids").is_none());
    assert!(packet.get("material_perspective_ids").is_none());
    assert!(packet["questions"][0].get("perspective_id").is_none());
    assert!(packet["questions"][0].get("parent_question_id").is_none());
    assert!(packet["questions"][0].get("round").is_none());
}

#[test]
fn outline_packet_projects_out_raw_excerpts_and_bounds_claim_previews() {
    let mut evidence = sample_evidence();
    evidence.claims[0].text = format!(
        "{}CLAIM_PREVIEW_TAIL_MUST_NOT_SURVIVE",
        "material claim preview ".repeat(200)
    );
    evidence.sources[0].quote_or_fact = Some("RAW_SOURCE_FACT_MUST_NOT_ENTER_OUTLINE".repeat(200));
    evidence.sources[0].evidence_excerpts = vec![AcceptedSourceExcerpt {
        id: "excerpt:one".to_string(),
        focus: "Raw excerpt focus".to_string(),
        quote_or_fact: "RAW_EXCERPT_MUST_NOT_ENTER_OUTLINE".repeat(200),
    }];
    let mut state = InquiryState::default();
    state.evidence_catalog.insert(
        "evidence:one".to_string(),
        EvidenceRef::new(
            "evidence:one",
            vec!["claim:one".to_string()],
            vec!["source:one".to_string()],
        ),
    );
    state.claim_catalog.insert("claim:one".to_string());
    state.source_catalog.insert("source:one".to_string());
    let context = state.outline_validation_context();

    let packet = closed_outline_packet("query", &state, &[evidence], &context).unwrap();
    let encoded = serde_json::to_string(&packet).unwrap();

    assert!(encoded.chars().count() < MAX_OUTLINE_PACKET_CHARS);
    assert!(!encoded.contains("RAW_SOURCE_FACT_MUST_NOT_ENTER_OUTLINE"));
    assert!(!encoded.contains("RAW_EXCERPT_MUST_NOT_ENTER_OUTLINE"));
    assert!(!encoded.contains("CLAIM_PREVIEW_TAIL_MUST_NOT_SURVIVE"));
    assert!(
        packet["evidence_graph"]["evidence_items"][0]["claims"][0]["text"]
            .as_str()
            .is_some_and(|text| text.chars().count() <= 240)
    );
}

#[test]
fn section_packet_projects_out_raw_excerpts_and_bounds_claim_text() {
    let mut evidence = sample_evidence();
    evidence.claims[0].text = format!(
        "{}CLAIM_TEXT_TAIL_MUST_NOT_SURVIVE",
        "material section claim ".repeat(200)
    );
    evidence.sources[0].quote_or_fact = Some("RAW_SOURCE_FACT_MUST_NOT_ENTER_SECTION".repeat(200));
    evidence.sources[0].evidence_excerpts = vec![AcceptedSourceExcerpt {
        id: "excerpt:section".to_string(),
        focus: "Raw section excerpt focus".to_string(),
        quote_or_fact: "RAW_EXCERPT_MUST_NOT_ENTER_SECTION".repeat(200),
    }];
    let section = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Answer the query".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
        composition_hint: "Lead with the accepted finding".to_string(),
    };

    let packet =
        section_generation_packet("query", &section, &InquiryState::default(), &[evidence])
            .expect("closed section packet");
    let encoded = serde_json::to_string(&packet).unwrap();

    assert!(!encoded.contains("RAW_SOURCE_FACT_MUST_NOT_ENTER_SECTION"));
    assert!(!encoded.contains("RAW_EXCERPT_MUST_NOT_ENTER_SECTION"));
    assert!(!encoded.contains("CLAIM_TEXT_TAIL_MUST_NOT_SURVIVE"));
    assert!(packet["evidence_bindings"][0]["sources"][0]
        .get("quote_or_fact")
        .is_none());
    assert!(packet["evidence_bindings"][0]["sources"][0]
        .get("evidence_excerpts")
        .is_none());
    assert_eq!(
        packet["evidence_bindings"][0]["claims"][0]["id"],
        "claim:one"
    );
    assert!(packet["evidence_bindings"][0]["claims"][0]["text"]
        .as_str()
        .is_some_and(|text| text.chars().count() <= 500));
    assert_eq!(packet["evidence_bindings"][0]["must_cite_one_source"], true);
}

#[test]
fn report_frame_packet_is_a_bounded_host_projection() {
    let (_, _, _, mut state) = sectioned_inquiry_snapshots();
    let outline = state.outline.clone().expect("committed outline");
    state
        .drafts
        .get_mut("section:answer")
        .expect("committed draft")
        .content = format!(
        "{}DRAFT_TAIL_MUST_NOT_ENTER_FRAME",
        "bounded report draft ".repeat(300)
    );
    state.questions[0].answer = Some(format!(
        "{}ANSWER_TAIL_MUST_NOT_ENTER_FRAME",
        "bounded accepted answer ".repeat(200)
    ));
    state.questions[0].bound_reason = Some(format!(
        "{}REASON_TAIL_MUST_NOT_ENTER_FRAME",
        "bounded qualification ".repeat(200)
    ));
    state
        .evidence_catalog
        .get_mut("evidence:one")
        .expect("accepted evidence")
        .diagnostics
        .push(a3s::research::EvidenceDiagnostic::new(
            "diagnostic:gap",
            a3s::research::EvidenceDiagnosticKind::Gap,
            "The release history leaves one compatibility case unresolved.",
        ));
    state
        .contract_assessment
        .as_mut()
        .expect("contract assessment")
        .diagnostics
        .push(a3s::research::EvidenceDiagnosticAssessment {
            diagnostic_id: "diagnostic:gap".to_string(),
            disposition: a3s::research::DiagnosticDisposition::Bounded,
            obligation_ids: vec!["obligation:one".to_string()],
            rationale: "INTERNAL host evidence-path diagnostic rationale".to_string(),
            evidence_ids: vec!["evidence:one".to_string()],
        });

    let packet =
        composition::report_frame_packet("query", &outline, &state, &[sample_evidence()], None);
    let encoded = serde_json::to_string(&packet).unwrap();

    assert!(packet.get("plan").is_none());
    assert!(packet["outline"][0].get("claim_ids").is_none());
    assert!(packet["outline"][0].get("source_ids").is_none());
    assert!(packet["inquiry"]["accepted_questions"][0]
        .get("evidence_ids")
        .is_none());
    assert!(
        packet["inquiry"]["contract_assessment"]["obligations"][0]["criteria"][0]
            .get("rationale")
            .is_none()
    );
    assert!(!encoded.contains("DRAFT_TAIL_MUST_NOT_ENTER_FRAME"));
    assert!(!encoded.contains("ANSWER_TAIL_MUST_NOT_ENTER_FRAME"));
    assert!(!encoded.contains("REASON_TAIL_MUST_NOT_ENTER_FRAME"));
    assert!(!encoded.contains("INTERNAL host evidence-path diagnostic rationale"));
    assert!(encoded.contains("compatibility case unresolved"));
    assert!(packet["drafts"][0]["markdown"]
        .as_str()
        .is_some_and(|text| text.chars().count() <= 2_500));
}

#[test]
fn section_packet_keeps_source_groups_and_includes_bounded_claim_previews() {
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
    assert!(prompt.contains("cite at least one exact source URL from every entry"));
    assert!(prompt.contains("Never construct, extend, shorten, or replace a source URL"));
    assert!(prompt.contains("include every supported partial answer"));
    assert!(prompt.contains("never mention what outside knowledge"));
    assert!(prompt.contains("dates and numerical literals against the bound claim excerpts"));
    assert!(prompt.contains("Write every prose sentence in the query language"));
    assert!(prompt.contains("Never mention the packet, evidence bindings"));
    assert!(prompt.contains(CLOSED_EVIDENCE_REASONING_GUARDRAILS));
    assert!(prompt.contains("never converted into a new interval, rate, density"));
    assert!(prompt.contains("limitation from this section must never be expanded"));
    assert!(args["schema"]["properties"].get("claim_ids").is_none());
    assert!(args["schema"]["properties"].get("source_ids").is_none());
    let packet: serde_json::Value = serde_json::from_str(
        prompt
            .split_once("CLOSED_SECTION_PACKET=")
            .map(|(_, packet)| packet)
            .unwrap(),
    )
    .unwrap();
    assert!(packet.get("claims").is_none());
    assert!(packet.get("sources").is_none());
    assert!(packet.get("perspectives").is_none());
    assert!(packet["section"].get("perspective_ids").is_none());
    assert!(packet["section"].get("claim_ids").is_none());
    assert!(packet["section"].get("source_ids").is_none());
    assert!(packet.get("allowed_claim_ids").is_none());
    assert!(packet.get("allowed_source_ids").is_none());
    assert!(packet.get("contract_assessment").is_none());
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
fn section_candidate_rejects_a_date_not_grounded_in_its_committed_claims() {
    let mut evidence = sample_evidence();
    evidence.claims[0].text =
        "async-std v1.12.0 was released on 2022-06-18; v1.13.2 followed on 2025-08-15.".to_string();
    let planned = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Report the accepted release dates".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
        composition_hint: "Copy dates exactly".to_string(),
    };
    let mut section = SectionGeneration {
        section_id: planned.id.clone(),
        markdown: "v1.12.0 发布于 202-06-18（[source](https://example.com/source)）。".to_string(),
        claim_ids: planned.claim_ids.clone(),
        source_ids: planned.source_ids.clone(),
    };

    let error = revision::validate_section_candidate(&mut section, &planned, &[evidence])
        .expect_err("a transcribed date absent from the accepted claims must fail closed");

    assert!(error.contains("202-06-18"), "{error}");
    assert!(error.contains("committed accepted claims"), "{error}");
}

#[test]
fn section_candidate_accepts_equivalent_iso_english_and_chinese_dates() {
    let mut evidence = sample_evidence();
    evidence.claims[0].text =
        "The release feed records 2022-06-18 and August 15th, 2025.".to_string();
    let planned = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Report the accepted release dates".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
        composition_hint: "Copy dates exactly".to_string(),
    };
    let mut section = SectionGeneration {
        section_id: planned.id.clone(),
        markdown: "首个版本发布于 2022 年 6 月 18 日，后续版本发布于 2025-08-15（[source](https://example.com/source)）。".to_string(),
        claim_ids: planned.claim_ids.clone(),
        source_ids: planned.source_ids.clone(),
    };

    revision::validate_section_candidate(&mut section, &planned, &[evidence])
        .expect("equivalent localized renderings of accepted dates should remain valid");
}

#[test]
fn revision_packet_names_every_uncited_evidence_binding() {
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
    let current = SectionGeneration {
        section_id: planned.id.clone(),
        markdown: "The first supported statement cites [Source one](https://example.com/source)."
            .to_string(),
        claim_ids: planned.claim_ids.clone(),
        source_ids: planned.source_ids.clone(),
    };

    let args = revision::section_revision_args(
        "query",
        &planned,
        &current,
        &InquiryState::default(),
        &[sample_evidence(), second_evidence()],
        &[serde_json::json!({"kind": "section_evidence_audit_failed"})],
        1,
    )
    .expect("revision args");
    let prompt = args["prompt"].as_str().expect("revision prompt");
    let packet: Value = serde_json::from_str(
        prompt
            .split_once("CLOSED_SECTION_REVISION_PACKET=")
            .map(|(_, packet)| packet)
            .expect("revision packet"),
    )
    .unwrap();

    assert!(prompt.contains("every missing_binding_citations entry"));
    assert!(prompt.contains("required_binding_citations is the complete citation requirement"));
    assert!(prompt.contains("Never construct, extend, shorten, or replace those URLs"));
    assert!(prompt.contains("Include every supported partial answer"));
    assert!(prompt.contains("mention outside knowledge even as a disclaimer"));
    assert!(prompt.contains("Bound claim excerpts control every version, date"));
    assert!(prompt.contains("Write every prose sentence in the query language"));
    assert!(prompt.contains("Never mention the packet, evidence bindings"));
    assert!(prompt.contains(CLOSED_EVIDENCE_REASONING_GUARDRAILS));
    assert!(prompt.contains("remove calculated date/count intervals"));
    assert!(prompt.contains("report-wide absence inferred from this section's local gap"));
    assert_eq!(
        packet["missing_binding_citations"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        packet["required_binding_citations"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        packet["missing_binding_citations"][0]["accepted_sources"][0]["anchor"],
        "https://example.com/second"
    );
    assert_eq!(
        packet["required_binding_citations"][0]["accepted_sources"][0]["anchor"],
        "https://example.com/source"
    );
    assert_eq!(
        packet["required_binding_citations"][1]["accepted_sources"][0]["anchor"],
        "https://example.com/second"
    );
}

#[test]
fn report_frame_prompt_preserves_closed_evidence_scope() {
    let packet = serde_json::json!({
        "query": "Compare two choices",
        "drafts": [],
        "inquiry": {},
        "outline": [],
    });
    let editorial = composition::report_editorial_frame_prompt(&packet);
    let guidance = composition::report_guidance_frame_prompt(&packet);
    let presentation = composition::report_presentation_frame_prompt(&packet);

    assert!(editorial.contains(CLOSED_EVIDENCE_REASONING_GUARDRAILS));
    assert!(editorial.contains("must not widen sampled examples"));
    assert!(editorial.contains("Write every reader-facing value"));
    assert!(editorial.contains("Host renders reader_labels verbatim"));
    assert!(editorial.contains("remove derived date/count intervals"));
    assert!(guidance.contains(CLOSED_EVIDENCE_REASONING_GUARDRAILS));
    assert!(guidance.contains("Cover every distinct action"));
    assert!(guidance.contains("normative recommendation"));
    assert!(guidance.contains("workload-specific test"));
    assert!(presentation.contains("ordered_headings array verbatim"));
    assert!(presentation.contains("information relationships"));
}

#[test]
fn independent_semantic_audit_covers_every_target_and_routes_a_section_issue() {
    let outline = ResearchOutline {
        sections: vec![OutlineSection {
            id: "section:answer".to_string(),
            heading: "Answer".to_string(),
            purpose: "Answer the query".to_string(),
            perspective_ids: Vec::new(),
            question_ids: Vec::new(),
            claim_ids: vec!["claim:one".to_string()],
            source_ids: vec!["source:one".to_string()],
            composition_hint: "Lead with exact observations".to_string(),
        }],
    };
    let sections = BTreeMap::from([(
        "section:answer".to_string(),
        SectionGeneration {
            section_id: "section:answer".to_string(),
            markdown: "The dates are three days apart, proving fast response. [Source](https://example.com/source)".to_string(),
            claim_ids: vec!["claim:one".to_string()],
            source_ids: vec!["source:one".to_string()],
        },
    )]);
    let frame = ReportFrame {
        report_title: "Bounded report".to_string(),
        reader_labels: sample_reader_labels(),
        decision_guidance: Vec::new(),
        editorial: ReportEditorialPlan {
            thesis: "The accepted evidence supports a bounded answer.".to_string(),
            track_coverage: Vec::new(),
        },
        presentation: ReportPresentation::default(),
    };
    let mut section_checks = clear_semantic_checks_json();
    section_checks["derived_quantities"] = Value::String("issue".to_string());
    let review = serde_json::from_value::<semantic_audit::SemanticReportReview>(
        serde_json::json!({
            "reviews": [
                {
                    "target_id": "frame",
                    "checks": clear_semantic_checks_json(),
                    "issues": []
                },
                {
                    "target_id": "section:answer",
                    "checks": section_checks,
                    "issues": [{
                        "category": "derived_quantities",
                        "excerpt": "three days apart",
                        "detail": "The accepted claims list observations but do not calculate this interval."
                    }]
                }
            ]
        }),
    )
    .expect("semantic review fixture");

    semantic_audit::validate_semantic_review(
        &review,
        &outline,
        &sections,
        &frame,
        &InquiryState::default(),
    )
    .expect("closed semantic target coverage");
    assert!(!review.passed());
    let merged = semantic_audit::merge_semantic_audit(
        ReportAudit {
            passed: true,
            accepted_sources: 1,
            cited_sources: 1,
            issues: Vec::new(),
            reason: "report citations resolve to exact anchors".to_string(),
        },
        &review,
    );
    assert!(!merged.passed);
    assert!(matches!(
        merged.issues.as_slice(),
        [ReportAuditIssue::SemanticBoundaryViolation { target_id, category, .. }]
            if target_id == "section:answer" && category == "derived_quantities"
    ));
    let resolved = resolve_evidence_ids(
        &ids(&["claim:one"]),
        &ids(&["source:one"]),
        &[sample_evidence()],
    )
    .expect("resolved fixture evidence");
    let targets = revision::target_sections_for_audit(&merged, &resolved, &outline)
        .expect("semantic section target");
    assert_eq!(
        targets.keys().cloned().collect::<Vec<_>>(),
        ["section:answer"]
    );
    assert_eq!(
        targets["section:answer"][0]["kind"],
        "semantic_boundary_violation"
    );
}

#[test]
fn final_semantic_reaudit_replaces_only_changed_targets() {
    let outline = ResearchOutline {
        sections: vec![OutlineSection {
            id: "section:answer".to_string(),
            heading: "Answer".to_string(),
            purpose: "Answer the query".to_string(),
            perspective_ids: Vec::new(),
            question_ids: Vec::new(),
            claim_ids: vec!["claim:one".to_string()],
            source_ids: vec!["source:one".to_string()],
            composition_hint: "Lead with exact observations".to_string(),
        }],
    };
    let sections = BTreeMap::from([(
        "section:answer".to_string(),
        SectionGeneration {
            section_id: "section:answer".to_string(),
            markdown: "The accepted observation is stated without a broader activity conclusion. [Source](https://example.com/source)".to_string(),
            claim_ids: vec!["claim:one".to_string()],
            source_ids: vec!["source:one".to_string()],
        },
    )]);
    let frame = ReportFrame {
        report_title: "Bounded report".to_string(),
        reader_labels: sample_reader_labels(),
        decision_guidance: Vec::new(),
        editorial: ReportEditorialPlan {
            thesis: "The accepted evidence supports a bounded answer.".to_string(),
            track_coverage: Vec::new(),
        },
        presentation: ReportPresentation::default(),
    };
    let mut issue_checks = clear_semantic_checks_json();
    issue_checks["claim_granularity"] = Value::String("issue".to_string());
    let baseline = serde_json::from_value::<semantic_audit::SemanticReportReview>(
        serde_json::json!({
            "reviews": [
                {
                    "target_id": "frame",
                    "checks": clear_semantic_checks_json(),
                    "issues": []
                },
                {
                    "target_id": "section:answer",
                    "checks": issue_checks,
                    "issues": [{
                        "category": "claim_granularity",
                        "excerpt": "broader activity conclusion",
                        "detail": "The prior prose inferred activity beyond the accepted observation."
                    }]
                }
            ]
        }),
    )
    .expect("baseline semantic review");
    assert_eq!(
        baseline.issue_target_ids(),
        BTreeSet::from(["section:answer".to_string()])
    );
    assert_eq!(
        baseline
            .revision_context_for_target("section:answer")
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        baseline
            .revision_context_for_target("frame")
            .as_array()
            .map(Vec::len),
        Some(0)
    );

    let replacement =
        serde_json::from_value::<semantic_audit::SemanticReportReview>(serde_json::json!({
            "reviews": [{
                "target_id": "section:answer",
                "checks": clear_semantic_checks_json(),
                "issues": []
            }]
        }))
        .expect("targeted semantic replacement");
    let merged = semantic_audit::merge_reaudited_targets(
        baseline,
        replacement,
        &outline,
        &sections,
        &frame,
        &InquiryState::default(),
    )
    .expect("merge exact changed target review");

    assert!(merged.passed());
    assert_eq!(
        serde_json::to_value(&merged).unwrap()["reviews"]
            .as_array()
            .unwrap()
            .iter()
            .map(|review| review["target_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["frame", "section:answer"]
    );
}

#[test]
fn semantic_audit_rejects_issue_text_when_the_corresponding_check_is_clear() {
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
    let sections = BTreeMap::from([(
        "section:answer".to_string(),
        SectionGeneration {
            section_id: "section:answer".to_string(),
            markdown: "A bounded statement cites [Source](https://example.com/source).".to_string(),
            claim_ids: vec!["claim:one".to_string()],
            source_ids: vec!["source:one".to_string()],
        },
    )]);
    let frame = ReportFrame {
        report_title: "Bounded report".to_string(),
        reader_labels: sample_reader_labels(),
        decision_guidance: Vec::new(),
        editorial: ReportEditorialPlan {
            thesis: "The accepted evidence supports a bounded answer.".to_string(),
            track_coverage: Vec::new(),
        },
        presentation: ReportPresentation::default(),
    };
    let review =
        serde_json::from_value::<semantic_audit::SemanticReportReview>(serde_json::json!({
            "reviews": [
                {"target_id": "frame", "checks": clear_semantic_checks_json(), "issues": []},
                {
                    "target_id": "section:answer",
                    "checks": clear_semantic_checks_json(),
                    "issues": [{
                        "category": "claim_granularity",
                        "excerpt": "bounded statement",
                        "detail": "An issue cannot be hidden behind a clear check."
                    }]
                }
            ]
        }))
        .expect("semantic review fixture");

    let error = semantic_audit::validate_semantic_review(
        &review,
        &outline,
        &sections,
        &frame,
        &InquiryState::default(),
    )
    .expect_err("check and issue categories must agree exactly");
    assert!(
        error.contains("do not match its issue categories"),
        "{error}"
    );
}

#[test]
fn semantic_audit_prompt_is_an_independent_closed_evidence_gate() {
    let prompt = semantic_audit::semantic_audit_prompt(&serde_json::json!({"targets": []}));
    assert!(prompt.contains("accepted answers, accepted report context, and prior frame"));
    assert!(prompt.contains("one exact reader-facing target"));
    assert!(prompt.contains("Check derived_quantities"));
    assert!(prompt.contains("Check unknown_item_quantifiers"));
    assert!(prompt.contains("question- or section-local gap"));
    assert!(prompt.contains("failing to give useful bounded guidance"));
    assert!(prompt.contains("reader-facing prose outside the query language"));
}

#[test]
fn semantic_audit_shards_frame_and_sections_into_one_target_packets() {
    let outline = ResearchOutline {
        sections: vec![OutlineSection {
            id: "section:answer".to_string(),
            heading: "Answer".to_string(),
            purpose: "Answer the query".to_string(),
            perspective_ids: Vec::new(),
            question_ids: Vec::new(),
            claim_ids: vec!["claim:one".to_string()],
            source_ids: vec!["source:one".to_string()],
            composition_hint: "Lead with exact observations".to_string(),
        }],
    };
    let sections = BTreeMap::from([(
        "section:answer".to_string(),
        SectionGeneration {
            section_id: "section:answer".to_string(),
            markdown: "A bounded statement cites [Source](https://example.com/source).".to_string(),
            claim_ids: vec!["claim:one".to_string()],
            source_ids: vec!["source:one".to_string()],
        },
    )]);
    let frame = ReportFrame {
        report_title: "Bounded report".to_string(),
        reader_labels: sample_reader_labels(),
        decision_guidance: Vec::new(),
        editorial: ReportEditorialPlan {
            thesis: "The accepted evidence supports a bounded answer.".to_string(),
            track_coverage: Vec::new(),
        },
        presentation: ReportPresentation::default(),
    };

    let packets = semantic_audit::semantic_audit_packets(
        "Compare two choices",
        &outline,
        &InquiryState::default(),
        &sections,
        &frame,
        &[sample_evidence()],
    )
    .expect("target audit packets");

    assert_eq!(
        packets
            .iter()
            .map(|(target_id, _)| target_id.as_str())
            .collect::<Vec<_>>(),
        ["frame", "section:answer"]
    );
    for (target_id, packet) in packets {
        assert_eq!(packet["target"]["target_id"], target_id);
        assert!(packet["accepted_report_context"].get("drafts").is_none());
        assert!(packet["accepted_report_context"]["accepted_claims"].is_array());
        assert!(packet["accepted_report_context"]["source_context"].is_array());
        assert!(
            semantic_audit::semantic_audit_prompt(&packet)
                .chars()
                .count()
                < 64_000
        );
        let schema = semantic_audit::semantic_audit_schema(std::slice::from_ref(&target_id));
        assert_eq!(schema["properties"]["reviews"]["minItems"], 1);
        assert_eq!(schema["properties"]["reviews"]["maxItems"], 1);
    }
}

#[test]
fn source_ledger_omits_internal_fetch_review_notes() {
    assert_eq!(
        composition::reader_facing_source_reliability(
            "Fetched source text discovered via AnySearch; authority and claim fit require closed-evidence review.",
        ),
        None
    );
    assert_eq!(
        composition::reader_facing_source_reliability("Official project documentation"),
        Some("Official project documentation")
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
fn partial_answer_limitation_is_a_host_derived_qualified_report_disclosure() {
    let limits = InquiryLimits::default();
    let mut question = Question::queued(
        "question:partial",
        None,
        "Which ecosystem facts are supported and what remains unknown?",
    );
    question.obligation_ids = vec!["obligation:ecosystem".to_string()];
    let mut events = vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        InquiryEvent::ResearchObligationsCommitted {
            obligations: vec![a3s::research::ResearchObligation::new(
                "obligation:ecosystem",
                "Ecosystem comparison",
                "Retain the supported comparison and its consequential gap",
                true,
                vec!["The comparison is traceable and transparently qualified".to_string()],
            )],
            stop_conditions: vec![
                "The material comparison has a traceable qualified answer".to_string()
            ],
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![question],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:partial",
                vec!["claim:partial".to_string()],
                vec!["source:partial".to_string()],
            ),
        },
        InquiryEvent::QuestionPartiallyAnswered {
            question_id: "question:partial".to_string(),
            answer: "The accepted evidence establishes the dominant ecosystem path.".to_string(),
            limitation: "The retained sources do not establish every named compatibility case."
                .to_string(),
            evidence_ids: vec!["evidence:partial".to_string()],
        },
    ];
    let state = replay(&events, &limits).expect("partial report prefix");
    let assessment =
        a3s::research::derive_research_contract_assessment(&state).expect("partial assessment");
    events.push(
        a3s::research::research_contract_assessment_event(&state, assessment)
            .expect("partial assessment event"),
    );
    let mut state = replay(&events, &limits).expect("qualified partial state");
    let assessment = state
        .contract_assessment
        .as_mut()
        .expect("qualified assessment");
    assessment.obligations[0].criteria[0].rationale =
        "INTERNAL structurally linked question material evidence floor".to_string();
    assessment.stop_conditions[0].rationale =
        "INTERNAL host will not infer this from keywords".to_string();

    assert_eq!(
        a3s::research::research_contract_outcome(&state),
        Some(a3s::research::ResearchContractOutcome::Qualified)
    );
    let disclosures = composition::qualification_disclosures(&state, &sample_reader_labels())
        .expect("partial disclosures");
    assert!(disclosures.iter().any(|disclosure| {
        disclosure
            .detail
            .contains("do not establish every named compatibility case")
    }));
    let rendered = disclosures
        .iter()
        .map(|disclosure| format!("{} {}", disclosure.label, disclosure.detail))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!rendered.contains("structurally linked"), "{rendered}");
    assert!(!rendered.contains("material evidence floor"), "{rendered}");
    assert!(!rendered.contains("host will not infer"), "{rendered}");
    assert!(!rendered.contains("keywords"), "{rendered}");
}

#[test]
fn section_cannot_omit_a_committed_outline_claim() {
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
        .expect_err("a writer must not silently drop a committed outline claim");
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
fn section_candidate_derives_actual_sources_without_requiring_every_alternative() {
    let mut evidence = sample_evidence();
    evidence.sources.push(AcceptedSource {
        id: "source:backup".to_string(),
        anchor: "https://example.com/backup".to_string(),
        title: Some("Backup source".to_string()),
        date: None,
        reliability: Some("secondary".to_string()),
        quote_or_fact: Some("The backup source supports the same accepted claim.".to_string()),
        evidence_excerpts: Vec::new(),
    });
    let planned = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Answer the query".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string(), "source:backup".to_string()],
        composition_hint: "Lead with the finding".to_string(),
    };
    let mut section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown:
            "The accepted claim is supported by [the primary source](https://example.com/source)."
                .to_string(),
        claim_ids: planned.claim_ids.clone(),
        source_ids: planned.source_ids.clone(),
    };

    let resolved = revision::validate_section_candidate(
        &mut section,
        &planned,
        std::slice::from_ref(&evidence),
    )
    .expect("one cited source from the claim binding is sufficient");

    assert_eq!(section.source_ids, ["source:one"]);
    assert_eq!(resolved.source_ids, ids(&["source:one"]));
}

#[test]
fn section_candidate_canonicalizes_an_exact_bracketed_source_url() {
    let planned = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Answer the query".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
        composition_hint: "Lead with the finding".to_string(),
    };
    let mut section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown: "The accepted claim uses the provider's citation-shaped URL [https://example.com/source].\n\n`[https://example.com/source]` remains code.\n\n```text\n[https://example.com/source]\n```".to_string(),
        claim_ids: planned.claim_ids.clone(),
        source_ids: planned.source_ids.clone(),
    };

    revision::validate_section_candidate(&mut section, &planned, &[sample_evidence()])
        .expect("an exact accepted bracketed URL should become a structural citation");

    assert!(section
        .markdown
        .contains("URL <https://example.com/source>."));
    assert!(section
        .markdown
        .contains("`[https://example.com/source]` remains code."));
    assert!(section
        .markdown
        .contains("```text\n[https://example.com/source]\n```"));
    assert_eq!(section.source_ids, ["source:one"]);
}

#[test]
fn section_candidate_canonicalizes_a_strict_descendant_to_the_committed_source() {
    let mut evidence = sample_evidence();
    evidence.sources[0].anchor = "https://example.com/releases".to_string();
    let planned = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Answer the query".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
        composition_hint: "Lead with the finding".to_string(),
    };
    let mut section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown: "The accepted claim cites [v1](https://example.com/releases/tag/v1). `https://example.com/releases/tag/v1` remains code."
            .to_string(),
        claim_ids: planned.claim_ids.clone(),
        source_ids: planned.source_ids.clone(),
    };

    revision::validate_section_candidate(&mut section, &planned, &[evidence])
        .expect("a strict descendant citation should resolve to its committed parent source");

    assert!(section
        .markdown
        .contains("[v1](https://example.com/releases)"));
    assert!(section
        .markdown
        .contains("`https://example.com/releases/tag/v1` remains code"));
    assert_eq!(section.source_ids, ["source:one"]);
}

#[test]
fn section_candidate_canonicalizes_adjacent_exact_bracketed_source_urls() {
    let mut evidence = sample_evidence();
    evidence.sources.push(AcceptedSource {
        id: "source:alternative".to_string(),
        anchor: "https://example.com/alternative".to_string(),
        title: Some("Alternative source".to_string()),
        date: None,
        reliability: Some("independent".to_string()),
        quote_or_fact: Some("The alternative source supports the same finding.".to_string()),
        evidence_excerpts: Vec::new(),
    });
    let planned = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Answer the query".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string(), "source:alternative".to_string()],
        composition_hint: "Lead with the finding".to_string(),
    };
    let mut section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown: "The finding cites adjacent provider-shaped URLs [https://example.com/source][https://example.com/alternative], while [https://example.com/source][reference] remains reference-style syntax.".to_string(),
        claim_ids: planned.claim_ids.clone(),
        source_ids: planned.source_ids.clone(),
    };

    revision::validate_section_candidate(&mut section, &planned, &[evidence])
        .expect("adjacent exact accepted URLs should both become structural citations");

    assert!(section
        .markdown
        .contains("<https://example.com/source><https://example.com/alternative>"));
    assert!(section
        .markdown
        .contains("[https://example.com/source][reference]"));
    assert_eq!(section.source_ids, ["source:one", "source:alternative"]);
}

#[test]
fn section_candidate_requires_one_actual_citation_for_each_claim_binding() {
    let planned = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Answer both accepted findings".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string(), "claim:two".to_string()],
        source_ids: vec!["source:one".to_string(), "source:two".to_string()],
        composition_hint: "Compare the findings".to_string(),
    };
    let mut section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown: "Only the first finding cites [its accepted source](https://example.com/source)."
            .to_string(),
        claim_ids: planned.claim_ids.clone(),
        source_ids: planned.source_ids.clone(),
    };

    let error = revision::validate_section_candidate(
        &mut section,
        &planned,
        &[sample_evidence(), second_evidence()],
    )
    .expect_err("the second claim has no actually cited source from its evidence binding");

    assert!(error.contains("claim:two"), "{error}");
    assert!(error.contains("same accepted evidence item"), "{error}");
}

#[test]
fn one_actual_source_can_cover_multiple_claims_in_the_same_evidence_binding() {
    let mut evidence = sample_evidence();
    evidence.claims.push(AcceptedClaim {
        id: "claim:two".to_string(),
        text: "A second claim retained in the same reviewed evidence item".to_string(),
    });
    evidence.sources.push(AcceptedSource {
        id: "source:alternative".to_string(),
        anchor: "https://example.com/alternative".to_string(),
        title: Some("Alternative source".to_string()),
        date: None,
        reliability: Some("secondary".to_string()),
        quote_or_fact: Some("An alternative source in the same evidence binding.".to_string()),
        evidence_excerpts: Vec::new(),
    });
    let planned = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Explain both accepted claims".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string(), "claim:two".to_string()],
        source_ids: vec!["source:one".to_string(), "source:alternative".to_string()],
        composition_hint: "Synthesize the reviewed evidence item".to_string(),
    };
    let mut section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown:
            "Both accepted claims are grounded in the same reviewed evidence binding [https://example.com/source]."
                .to_string(),
        claim_ids: planned.claim_ids.clone(),
        source_ids: planned.source_ids.clone(),
    };

    let resolved = revision::validate_section_candidate(&mut section, &planned, &[evidence])
        .expect("one actual source from the shared binding covers both committed claims");

    assert_eq!(section.source_ids, ["source:one"]);
    assert_eq!(resolved.claim_source_ids["claim:one"], ids(&["source:one"]));
    assert_eq!(resolved.claim_source_ids["claim:two"], ids(&["source:one"]));
}

#[test]
fn section_candidate_deterministically_demotes_nested_h1_and_h2_headings() {
    let planned = OutlineSection {
        id: "section:answer".to_string(),
        heading: "Answer".to_string(),
        purpose: "Answer the query".to_string(),
        perspective_ids: Vec::new(),
        question_ids: Vec::new(),
        claim_ids: vec!["claim:one".to_string()],
        source_ids: vec!["source:one".to_string()],
        composition_hint: "Lead with the finding".to_string(),
    };
    let mut section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown: "# Model title\n\n## Nested finding\n\nThe accepted claim is supported by [the source](https://example.com/source).\n\nSetext detail\n---\n\n```markdown\n## This is code, not a heading\n```".to_string(),
        claim_ids: planned.claim_ids.clone(),
        source_ids: planned.source_ids.clone(),
    };

    revision::validate_section_candidate(&mut section, &planned, &[sample_evidence()])
        .expect("Host heading normalization should preserve a valid section");

    assert!(section.markdown.starts_with("### Model title"));
    assert!(section.markdown.contains("\n### Nested finding"));
    assert!(section.markdown.contains("\n### Setext detail"));
    assert!(section
        .markdown
        .contains("```markdown\n## This is code, not a heading\n```"));
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
    assert!(error.contains("does not cite every source"), "{error}");
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
    assert!(error.contains("does not cite every source"), "{error}");
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
    assert!(error.contains("does not cite every source"), "{error}");
}

#[test]
fn section_audit_validates_exact_claim_source_bindings_without_matching_prose() {
    let mut second = second_evidence();
    second.claims[0].text =
        "Orbital telemetry establishes a materially distinct finding.".to_string();
    let section = SectionGeneration {
        section_id: "section:answer".to_string(),
        markdown: "The accepted claim cites [the primary source](https://example.com/source), while unrelated prose cites [the second source](https://example.com/second).".to_string(),
        claim_ids: vec!["claim:one".to_string(), "claim:two".to_string()],
        source_ids: vec!["source:one".to_string(), "source:two".to_string()],
    };

    let resolved = audit_section_generation(&section, &[sample_evidence(), second])
        .expect("closed IDs and exact citations are sufficient for structural audit");
    assert!(
        resolved.claim_source_ids["claim:one"].contains("source:one"),
        "{resolved:#?}"
    );
    assert!(
        resolved.claim_source_ids["claim:two"].contains("source:two"),
        "{resolved:#?}"
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
        evidence_excerpts: Vec::new(),
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
    assert!(error.contains("does not cite every source"), "{error}");
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
        reader_labels: sample_reader_labels(),
        decision_guidance: Vec::new(),
        editorial: ReportEditorialPlan {
            thesis: "The accepted evidence directly supports the bounded answer.".to_string(),
            track_coverage: Vec::new(),
        },
        presentation: ReportPresentation::default(),
    };
    let mut evidence_item = sample_evidence();
    evidence_item.sources.push(AcceptedSource {
        id: "source:unused".to_string(),
        anchor: "https://example.com/unused".to_string(),
        title: Some("Unused alternative".to_string()),
        date: None,
        reliability: Some("secondary".to_string()),
        quote_or_fact: Some("An alternative source that the report body did not cite.".to_string()),
        evidence_excerpts: Vec::new(),
    });
    let evidence = vec![evidence_item];
    let assembled =
        assemble_markdown(&frame, &outline, &state, &ids(&["source:one"]), &evidence).unwrap();
    assert!(assembled.body.starts_with("# Useful report"));
    assert!(assembled.body.contains("## Answer"));
    assert!(!assembled.body.contains("## Sources"));
    assert!(assembled.markdown.contains("## Sources"));
    assert!(assembled.markdown.contains("https://example.com/source"));
    assert!(!assembled.markdown.contains("https://example.com/unused"));
}

#[test]
fn assembled_report_renders_query_language_labels_and_bounded_decision_guidance() {
    let outline = ResearchOutline {
        sections: vec![OutlineSection {
            id: "section:answer".to_string(),
            heading: "证据结论".to_string(),
            purpose: "回答问题".to_string(),
            perspective_ids: Vec::new(),
            question_ids: Vec::new(),
            claim_ids: vec!["claim:one".to_string()],
            source_ids: vec!["source:one".to_string()],
            composition_hint: "先给结论".to_string(),
        }],
    };
    let mut state = InquiryState::default();
    state.drafts.insert(
        "section:answer".to_string(),
        SectionDraft {
            section_id: "section:answer".to_string(),
            content: "已接受来源支持这个有界结论。".to_string(),
            citation_ids: vec!["source:one".to_string()],
        },
    );
    let mut labels = sample_reader_labels();
    labels.sources_heading = "来源".to_string();
    labels.decision_heading = "生产选型建议".to_string();
    let frame = ReportFrame {
        report_title: "有界研究报告".to_string(),
        reader_labels: labels,
        decision_guidance: vec![ReportDecisionGuidance {
            scenario: "新项目".to_string(),
            recommendation: "建议优先采用已被当前依赖明确支持的方案。".to_string(),
            basis_obligation_ids: vec!["obligation:one".to_string()],
            boundary: "资源消耗仍需用实际工作负载验证。".to_string(),
        }],
        editorial: ReportEditorialPlan {
            thesis: "现有证据支持一项有边界的生产决策。".to_string(),
            track_coverage: Vec::new(),
        },
        presentation: ReportPresentation::default(),
    };

    let assembled = assemble_markdown(
        &frame,
        &outline,
        &state,
        &ids(&["source:one"]),
        &[sample_evidence()],
    )
    .expect("localized assembled report");

    assert!(assembled.body.contains("## 生产选型建议"));
    assert!(assembled.body.contains("**新项目**"));
    assert!(assembled.body.contains("资源消耗仍需用实际工作负载验证"));
    assert!(assembled.markdown.contains("## 来源"));
    assert!(!assembled.markdown.contains("## Sources"));
}

#[test]
fn assembled_report_normalizes_exact_bracketed_citation_from_durable_draft() {
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
            content: "The durable finding remains traceable [https://example.com/source]."
                .to_string(),
            citation_ids: vec!["source:one".to_string()],
        },
    );
    let frame = ReportFrame {
        report_title: "Recovered report".to_string(),
        reader_labels: sample_reader_labels(),
        decision_guidance: Vec::new(),
        editorial: ReportEditorialPlan {
            thesis: "The durable evidence supports the recovered answer.".to_string(),
            track_coverage: Vec::new(),
        },
        presentation: ReportPresentation::default(),
    };
    let evidence = vec![sample_evidence()];

    let assembled =
        assemble_markdown(&frame, &outline, &state, &ids(&["source:one"]), &evidence).unwrap();

    assert!(assembled
        .body
        .contains("traceable <https://example.com/source>."));
    assert!(!assembled.body.contains("[https://example.com/source]"));
}

#[test]
fn report_generation_uses_a_small_execution_adapter() {
    assert!(SECTION_WORKFLOW_SOURCE.contains("schedule_steps"));
    assert!(SECTION_WORKFLOW_SOURCE.contains("generate_object"));
    assert!(SECTION_WORKFLOW_SOURCE.contains("max_attempts: 2"));
    assert!(SECTION_WORKFLOW_SOURCE.contains("exitCode !== 0"));
    assert!(SECTION_WORKFLOW_SOURCE.len() < 4_000);
    assert!(!SECTION_WORKFLOW_SOURCE.contains("research_method"));
}

#[tokio::test]
async fn report_generation_failure_retries_only_its_durable_step() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let calls = Arc::new(AtomicUsize::new(0));
    executor.register_dynamic_tool(Arc::new(RetryOnceReportGenerationFixture {
        calls: Arc::clone(&calls),
    }));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());
    let args = serde_json::json!({
        "source": SECTION_WORKFLOW_SOURCE,
        "input": {
            "sections": [{
                "step_id": "outline",
                "section_id": "outline",
                "generation_args": {
                    "schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {"durable": {"type": "boolean"}},
                        "required": ["durable"]
                    },
                    "prompt": "Return the closed fixture object.",
                    "timeout_ms": 1_000
                }
            }]
        },
        "run_id": "durable-report-generation-retry",
        "limits": {
            "timeoutMs": 10_000,
            "maxToolCalls": 4,
            "maxOutputBytes": 1024 * 1024
        }
    });

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("durable report generation workflow");

    assert_eq!(result.exit_code, 0, "{}", result.output);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let steps = result
        .metadata
        .as_ref()
        .and_then(|metadata| metadata["dynamic_workflow"]["snapshot"]["steps"].as_object())
        .expect("durable report generation steps");
    assert_eq!(steps["outline"]["attempt"], 2);
    assert_eq!(steps["outline"]["status"], "completed");
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
                source_ids: vec!["source:one".to_string(), "source:backup".to_string()],
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
fn structural_and_semantic_acceptance_share_two_bounded_report_repairs() {
    let (_, _, mut events, _) = sectioned_inquiry_snapshots();
    events.pop();
    events.pop();
    let mut state = replay(&events, &InquiryLimits::default()).unwrap();
    let targets = vec!["section:answer".to_string()];

    let digest = "validation-revision-1".to_string();
    let start =
        revision::revision_start_event(&state, 1, &targets, &digest, "host validation failed")
            .unwrap()
            .unwrap();
    apply_event(&mut state, &mut events, start).unwrap();
    apply_event(
        &mut state,
        &mut events,
        InquiryEvent::SectionDrafted {
            section_id: "section:answer".to_string(),
            content: "The accepted claim remains supported after the one validation revision."
                .to_string(),
            citation_ids: vec!["claim:one".to_string(), "source:one".to_string()],
        },
    )
    .unwrap();
    apply_event(
        &mut state,
        &mut events,
        InquiryEvent::SectionRevisionCommitted {
            round: 1,
            input_digest: digest,
        },
    )
    .unwrap();

    assert_eq!(state.audit_attempts, 0);
    assert_eq!(recovery::restored_revision_rounds(&state), 1);
    assert_eq!(replay(&events, &InquiryLimits::default()).unwrap(), state);
    apply_event(
        &mut state,
        &mut events,
        InquiryEvent::AuditCompleted {
            passed: false,
            issues: vec!["independent semantic acceptance failed".to_string()],
        },
    )
    .unwrap();
    let second_digest = "semantic-revision-2".to_string();
    let second = revision::revision_start_event(
        &state,
        2,
        &targets,
        &second_digest,
        "semantic acceptance failed",
    )
    .unwrap()
    .expect("a second targeted semantic repair remains available");
    apply_event(&mut state, &mut events, second).unwrap();
    apply_event(
        &mut state,
        &mut events,
        InquiryEvent::SectionDrafted {
            section_id: "section:answer".to_string(),
            content: "The accepted claim remains supported after semantic repair.".to_string(),
            citation_ids: vec!["claim:one".to_string(), "source:one".to_string()],
        },
    )
    .unwrap();
    apply_event(
        &mut state,
        &mut events,
        InquiryEvent::SectionRevisionCommitted {
            round: 2,
            input_digest: second_digest,
        },
    )
    .unwrap();

    assert_eq!(recovery::restored_revision_rounds(&state), 2);
    let exhausted = revision::revision_start_event(
        &state,
        3,
        &targets,
        "semantic-revision-3",
        "semantic acceptance still failed",
    )
    .unwrap_err();
    assert!(
        exhausted.contains("after 2 targeted revision rounds"),
        "{exhausted}"
    );
}
