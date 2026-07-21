#[test]
fn accepted_evidence_rejects_empty_duplicate_and_conflicting_relationships() {
    let limits = InquiryLimits::default();
    let mut state = reduce(
        &InquiryState::default(),
        &InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        &limits,
    )
    .expect("strategy");
    let accepted = EvidenceRef::new(
        "evidence:one",
        vec!["claim:one".to_string()],
        vec!["source:one".to_string()],
    );
    state = reduce(
        &state,
        &InquiryEvent::EvidenceAccepted {
            evidence: accepted.clone(),
        },
        &limits,
    )
    .expect("evidence");

    assert_eq!(
        reduce(
            &state,
            &InquiryEvent::EvidenceAccepted {
                evidence: accepted.clone(),
            },
            &limits,
        ),
        Err(InquiryError::DuplicateId {
            resource: "evidence",
            id: "evidence:one".to_string(),
        })
    );
    assert_eq!(
        reduce(
            &state,
            &InquiryEvent::EvidenceAccepted {
                evidence: EvidenceRef::new(
                    "evidence:one",
                    vec!["claim:other".to_string()],
                    vec!["source:one".to_string()],
                ),
            },
            &limits,
        ),
        Err(InquiryError::ConflictingEvidence {
            id: "evidence:one".to_string(),
        })
    );
    assert_eq!(
        reduce(
            &state,
            &InquiryEvent::EvidenceAccepted {
                evidence: EvidenceRef::new(
                    "evidence:empty",
                    Vec::new(),
                    vec!["source:empty".to_string()],
                ),
            },
            &limits,
        ),
        Err(InquiryError::EmptyBatch {
            resource: "evidence claim ids",
        })
    );
}

#[test]
fn accepted_evidence_rejects_diagnostic_ids_reused_by_another_evidence_item() {
    let limits = InquiryLimits::default();
    let mut state = reduce(
        &InquiryState::default(),
        &InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        &limits,
    )
    .expect("strategy");
    let diagnostic = EvidenceDiagnostic::new(
        "diagnostic:shared",
        EvidenceDiagnosticKind::Gap,
        "A bounded gap remains.",
    );
    state = reduce(
        &state,
        &InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:first",
                vec!["claim:first".to_string()],
                vec!["source:first".to_string()],
            )
            .with_diagnostics(vec![diagnostic.clone()]),
        },
        &limits,
    )
    .expect("first diagnostic");

    assert_eq!(
        reduce(
            &state,
            &InquiryEvent::EvidenceAccepted {
                evidence: EvidenceRef::new(
                    "evidence:second",
                    vec!["claim:second".to_string()],
                    vec!["source:second".to_string()],
                )
                .with_diagnostics(vec![diagnostic]),
            },
            &limits,
        ),
        Err(InquiryError::DuplicateId {
            resource: "evidence diagnostic",
            id: "diagnostic:shared".to_string(),
        })
    );
}

#[test]
fn accepted_evidence_validates_closed_source_coverage_edges() {
    let limits = InquiryLimits::default();
    let obligation = ResearchObligation::new(
        "obligation:coverage",
        "Coverage",
        "Validate typed source coverage",
        true,
        vec!["The first criterion is supported".to_string()],
    )
    .with_evidence_requirements(EvidenceQualityRequirements {
        primary_source_required: true,
        independent_corroboration_required: false,
    });
    let state = replay(
        &[
            InquiryEvent::StrategySelected {
                method: ResearchMethod::Focused,
            },
            InquiryEvent::ResearchObligationsCommitted {
                obligations: vec![obligation],
                stop_conditions: vec!["The typed edge is valid".to_string()],
            },
        ],
        &limits,
    )
    .expect("coverage contract");

    let unknown_source = EvidenceRef::new(
        "evidence:unknown-source",
        vec!["claim:coverage".to_string()],
        vec!["source:coverage".to_string()],
    )
    .with_source_coverage(vec![SourceCoverageBinding::new(
        "source:not-accepted",
        "obligation:coverage",
        vec![0],
        vec![SourceEvidenceRole::Supporting],
    )]);
    assert!(matches!(
        reduce(
            &state,
            &InquiryEvent::EvidenceAccepted {
                evidence: unknown_source
            },
            &limits,
        ),
        Err(InquiryError::UnknownId {
            resource: "evidence source",
            ..
        })
    ));

    let unrequested_role = EvidenceRef::new(
        "evidence:unrequested-role",
        vec!["claim:coverage".to_string()],
        vec!["source:coverage".to_string()],
    )
    .with_source_coverage(vec![SourceCoverageBinding::new(
        "source:coverage",
        "obligation:coverage",
        vec![0],
        vec![
            SourceEvidenceRole::Supporting,
            SourceEvidenceRole::Independent,
        ],
    )]);
    let error = reduce(
        &state,
        &InquiryEvent::EvidenceAccepted {
            evidence: unrequested_role,
        },
        &limits,
    )
    .expect_err("undeclared independent role");
    assert!(error.to_string().contains("unrequested independent role"));

    let accepted = EvidenceRef::new(
        "evidence:coverage",
        vec!["claim:coverage".to_string()],
        vec!["source:coverage".to_string()],
    )
    .with_source_coverage(vec![SourceCoverageBinding::new(
        "source:coverage",
        "obligation:coverage",
        vec![0],
        vec![SourceEvidenceRole::Supporting, SourceEvidenceRole::Primary],
    )]);
    let next = reduce(
        &state,
        &InquiryEvent::EvidenceAccepted { evidence: accepted },
        &limits,
    )
    .expect("closed typed source coverage");
    assert_eq!(
        next.evidence_catalog["evidence:coverage"]
            .source_coverage
            .len(),
        1
    );
}

#[test]
fn legacy_multi_wave_evidence_catalog_round_trips_and_replays() {
    let limits = InquiryLimits::default();
    let events = vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![Question::queued(
                "question:first",
                None,
                "What does the first wave establish?",
            )],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:first",
                vec!["claim:first".to_string()],
                vec!["source:first".to_string()],
            ),
        },
        InquiryEvent::QuestionDeferred {
            question_id: "question:first".to_string(),
            reason: "The first wave establishes a baseline but leaves a bounded gap.".to_string(),
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![legacy_follow_up(
                "question:second",
                None,
                "question:first",
                1,
                "What does the second wave add?",
            )],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:second",
                vec!["claim:second".to_string()],
                vec!["source:second".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:first".to_string(),
            answer: "The two retained waves establish the baseline.".to_string(),
            evidence_ids: vec!["evidence:first".to_string(), "evidence:second".to_string()],
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:second".to_string(),
            answer: "The second wave adds independent confirmation.".to_string(),
            evidence_ids: vec!["evidence:second".to_string()],
        },
        InquiryEvent::OutlineCommitted {
            outline: ResearchOutline {
                sections: vec![OutlineSection {
                    id: "section:findings".to_string(),
                    heading: "Findings".to_string(),
                    purpose: "Synthesize both accepted waves.".to_string(),
                    perspective_ids: Vec::new(),
                    question_ids: vec!["question:first".to_string(), "question:second".to_string()],
                    claim_ids: vec!["claim:first".to_string(), "claim:second".to_string()],
                    source_ids: vec!["source:first".to_string(), "source:second".to_string()],
                    composition_hint: "Compare the independently sourced findings.".to_string(),
                }],
            },
        },
    ];

    let encoded = serde_json::to_vec(&events).expect("events should serialize");
    let decoded: Vec<InquiryEvent> =
        serde_json::from_slice(&encoded).expect("events should deserialize");
    let state = replay(&decoded, &limits).expect("accepted multi-wave inquiry should replay");

    assert_eq!(decoded, events);
    assert_eq!(state.phase, InquiryPhase::Drafting);
    assert_eq!(state.evidence_catalog.len(), 2);
    assert_eq!(
        state.claim_catalog,
        ["claim:first".to_string(), "claim:second".to_string()]
            .into_iter()
            .collect()
    );
    assert_eq!(
        state.source_catalog,
        ["source:first".to_string(), "source:second".to_string()]
            .into_iter()
            .collect()
    );
}
