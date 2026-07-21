#[test]
fn section_draft_citations_are_nonempty_scoped_and_source_backed() {
    let limits = InquiryLimits::default();
    let mut state = drafting_state();
    let unchanged = state.clone();

    let empty = InquiryEvent::SectionDrafted {
        section_id: "section:first".to_string(),
        content: "A supported first-section draft.".to_string(),
        citation_ids: Vec::new(),
    };
    assert_eq!(
        state.apply(&empty, &limits),
        Err(InquiryError::EmptyBatch {
            resource: "section citation ids"
        })
    );
    assert_eq!(state, unchanged);

    let claim_only = InquiryEvent::SectionDrafted {
        section_id: "section:first".to_string(),
        content: "A claim-only first-section draft.".to_string(),
        citation_ids: vec!["claim:first".to_string()],
    };
    assert_eq!(
        state.apply(&claim_only, &limits),
        Err(InquiryError::MissingSourceCitation {
            section_id: "section:first".to_string()
        })
    );
    assert_eq!(state, unchanged);

    let cross_section = InquiryEvent::SectionDrafted {
        section_id: "section:first".to_string(),
        content: "A cross-section citation draft.".to_string(),
        citation_ids: vec!["source:second".to_string()],
    };
    assert_eq!(
        state.apply(&cross_section, &limits),
        Err(InquiryError::UnknownId {
            resource: "outline section citation",
            id: "source:second".to_string()
        })
    );
    assert_eq!(state, unchanged);

    let valid = InquiryEvent::SectionDrafted {
        section_id: "section:first".to_string(),
        content: "A supported first-section draft.".to_string(),
        citation_ids: vec!["claim:first".to_string(), "source:first".to_string()],
    };
    state
        .apply(&valid, &limits)
        .expect("a section-local source citation should be accepted");
    assert_eq!(
        state.drafts["section:first"].citation_ids,
        ["claim:first", "source:first"]
    );
}

#[test]
fn section_revision_rounds_are_explicit_and_replayable() {
    let limits = InquiryLimits::default();
    let mut events = vec![InquiryEvent::SectionRevisionStarted {
        round: 1,
        section_ids: vec!["section:first".to_string()],
        input_digest: "digest:first".to_string(),
    }];
    let mut state = drafting_state();
    for event in &events {
        state.apply(event, &limits).unwrap();
    }
    assert_eq!(state.active_section_revision().unwrap().round, 1);

    events.extend([
        InquiryEvent::SectionDrafted {
            section_id: "section:first".to_string(),
            content: "A revised, source-backed first section.".to_string(),
            citation_ids: vec!["claim:first".to_string(), "source:first".to_string()],
        },
        InquiryEvent::SectionRevisionCommitted {
            round: 1,
            input_digest: "digest:first".to_string(),
        },
    ]);
    for event in &events[1..] {
        state.apply(event, &limits).unwrap();
    }
    assert!(state.active_section_revision().is_none());
    assert_eq!(state.section_revisions.len(), 1);
    assert!(state.section_revisions[0].committed);
    assert_eq!(
        state.section_revisions[0].drafted_section_ids,
        ["section:first"]
    );

    let mut full_history = vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![Question::queued(
                "question:root",
                None,
                "What evidence answers the inquiry?",
            )],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:first",
                vec!["claim:first".to_string()],
                vec!["source:first".to_string()],
            ),
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:second",
                vec!["claim:second".to_string()],
                vec!["source:second".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:root".to_string(),
            answer: "The accepted evidence supports the finding.".to_string(),
            evidence_ids: vec!["evidence:first".to_string(), "evidence:second".to_string()],
        },
        InquiryEvent::OutlineCommitted {
            outline: ResearchOutline {
                sections: vec![
                    outline_section("section:first", "claim:first", "source:first"),
                    outline_section("section:second", "claim:second", "source:second"),
                ],
            },
        },
    ];
    full_history.extend(events);
    assert_eq!(replay(&full_history, &limits).unwrap(), state);
}

#[test]
fn section_revision_replay_rejects_conflicts_and_incomplete_commits() {
    let limits = InquiryLimits::default();
    let mut state = drafting_state();
    state
        .apply(
            &InquiryEvent::SectionRevisionStarted {
                round: 1,
                section_ids: vec!["section:first".to_string()],
                input_digest: "digest:first".to_string(),
            },
            &limits,
        )
        .unwrap();

    let duplicate = state
        .apply(
            &InquiryEvent::SectionRevisionStarted {
                round: 1,
                section_ids: vec!["section:first".to_string()],
                input_digest: "digest:first".to_string(),
            },
            &limits,
        )
        .unwrap_err();
    assert!(matches!(
        duplicate,
        InquiryError::InvalidSectionRevision { ref reason }
            if reason.contains("still active")
    ));

    let untargeted = state
        .apply(
            &InquiryEvent::SectionDrafted {
                section_id: "section:second".to_string(),
                content: "An untargeted replacement.".to_string(),
                citation_ids: vec!["claim:second".to_string(), "source:second".to_string()],
            },
            &limits,
        )
        .unwrap_err();
    assert!(matches!(
        untargeted,
        InquiryError::InvalidSectionRevision { ref reason }
            if reason.contains("untargeted section")
    ));

    let incomplete = state
        .apply(
            &InquiryEvent::SectionRevisionCommitted {
                round: 1,
                input_digest: "digest:first".to_string(),
            },
            &limits,
        )
        .unwrap_err();
    assert!(matches!(
        incomplete,
        InquiryError::InvalidSectionRevision { ref reason }
            if reason.contains("no replacement draft")
    ));

    state
        .apply(
            &InquiryEvent::SectionDrafted {
                section_id: "section:first".to_string(),
                content: "A revised, source-backed first section.".to_string(),
                citation_ids: vec!["claim:first".to_string(), "source:first".to_string()],
            },
            &limits,
        )
        .unwrap();
    let mismatched = state
        .apply(
            &InquiryEvent::SectionRevisionCommitted {
                round: 1,
                input_digest: "digest:other".to_string(),
            },
            &limits,
        )
        .unwrap_err();
    assert!(matches!(
        mismatched,
        InquiryError::InvalidSectionRevision { ref reason }
            if reason.contains("does not match")
    ));
}

#[test]
fn section_revision_budget_counts_started_rounds_across_replay() {
    let limits = InquiryLimits::default();
    let mut state = drafting_state();
    for round in 1..=limits.max_section_revision_rounds {
        let digest = format!("digest:{round}");
        state
            .apply(
                &InquiryEvent::SectionRevisionStarted {
                    round,
                    section_ids: vec!["section:first".to_string()],
                    input_digest: digest.clone(),
                },
                &limits,
            )
            .unwrap();
        state
            .apply(
                &InquiryEvent::SectionDrafted {
                    section_id: "section:first".to_string(),
                    content: format!("A source-backed revision from round {round}."),
                    citation_ids: vec!["claim:first".to_string(), "source:first".to_string()],
                },
                &limits,
            )
            .unwrap();
        state
            .apply(
                &InquiryEvent::SectionRevisionCommitted {
                    round,
                    input_digest: digest,
                },
                &limits,
            )
            .unwrap();
    }

    let error = state
        .apply(
            &InquiryEvent::SectionRevisionStarted {
                round: limits.max_section_revision_rounds + 1,
                section_ids: vec!["section:first".to_string()],
                input_digest: "digest:excess".to_string(),
            },
            &limits,
        )
        .unwrap_err();
    assert_eq!(
        error,
        InquiryError::HardLimitExceeded {
            resource: "section revision rounds",
            limit: 2,
            actual: 3,
        }
    );
}
