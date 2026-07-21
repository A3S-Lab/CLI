use super::*;

fn legacy_perspective(id: &str, title: &str, focus: &str, source_ids: Vec<String>) -> Perspective {
    Perspective {
        id: id.to_string(),
        title: title.to_string(),
        focus: focus.to_string(),
        source_ids,
    }
}

fn legacy_follow_up(
    id: &str,
    perspective_id: Option<String>,
    parent_question_id: &str,
    round: u32,
    prompt: &str,
) -> Question {
    let mut question = Question::queued(id, perspective_id, prompt);
    question.parent_question_id = Some(parent_question_id.to_string());
    question.round = round;
    question
}

fn outline_section(id: &str, claim_id: &str, source_id: &str) -> OutlineSection {
    OutlineSection {
        id: id.to_string(),
        heading: format!("Findings for {id}"),
        purpose: "Explain the accepted evidence.".to_string(),
        perspective_ids: Vec::new(),
        question_ids: vec!["question:root".to_string()],
        claim_ids: vec![claim_id.to_string()],
        source_ids: vec![source_id.to_string()],
        composition_hint: "Lead with the finding and cite its source.".to_string(),
    }
}

fn drafting_state() -> InquiryState {
    let limits = InquiryLimits::default();
    let events = [
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
    replay(&events, &limits).expect("fixture events should be valid")
}

#[test]
fn outline_context_marks_only_perspectives_with_material_questions_as_material() {
    let mut supporting = Question::queued(
        "question:supporting",
        Some("perspective:supporting".to_string()),
        "Which supporting detail remains useful?",
    );
    supporting.material = false;
    let state = InquiryState {
        perspectives: vec![
            legacy_perspective(
                "perspective:core",
                "Core perspective",
                "Resolve the consequential conclusion",
                vec!["source:core".to_string()],
            ),
            legacy_perspective(
                "perspective:supporting",
                "Supporting perspective",
                "Add non-gating context",
                vec!["source:supporting".to_string()],
            ),
        ],
        questions: vec![
            Question::queued(
                "question:core",
                Some("perspective:core".to_string()),
                "What evidence determines the core conclusion?",
            ),
            supporting,
        ],
        ..InquiryState::default()
    };

    let context = state.outline_validation_context();
    assert_eq!(
        context.material_perspective_ids,
        std::collections::BTreeSet::from(["perspective:core".to_string()])
    );
}

#[test]
fn replay_rejects_answers_outside_the_accepted_evidence_catalog() {
    let limits = InquiryLimits::default();
    let events = [
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
                "evidence:accepted",
                vec!["claim:accepted".to_string()],
                vec!["source:accepted".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:root".to_string(),
            answer: "A forged answer must not enter replay.".to_string(),
            evidence_ids: vec!["evidence:forged".to_string()],
        },
    ];

    assert_eq!(
        replay(&events, &limits),
        Err(InquiryReplayError {
            event_index: 3,
            error: InquiryError::UnknownId {
                resource: "accepted evidence",
                id: "evidence:forged".to_string(),
            },
        })
    );
}

#[test]
fn replay_rejects_outline_ids_and_missing_material_coverage() {
    let limits = InquiryLimits::default();
    let prefix = vec![
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
                "evidence:accepted",
                vec!["claim:accepted".to_string()],
                vec!["source:accepted".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:root".to_string(),
            answer: "The accepted evidence supports the finding.".to_string(),
            evidence_ids: vec!["evidence:accepted".to_string()],
        },
    ];

    let mut forged = prefix.clone();
    forged.push(InquiryEvent::OutlineCommitted {
        outline: ResearchOutline {
            sections: vec![outline_section(
                "section:forged",
                "claim:forged",
                "source:accepted",
            )],
        },
    });
    let error = replay(&forged, &limits).expect_err("forged claim must be rejected");
    assert_eq!(error.event_index, 4);
    assert!(matches!(
        error.error,
        InquiryError::InvalidOutline { ref reason }
            if reason.contains("unknown claim id `claim:forged`")
    ));

    let mut uncovered = prefix;
    let mut section = outline_section("section:uncovered", "claim:accepted", "source:accepted");
    section.question_ids.clear();
    uncovered.push(InquiryEvent::OutlineCommitted {
        outline: ResearchOutline {
            sections: vec![section],
        },
    });
    let error = replay(&uncovered, &limits).expect_err("material question must be covered");
    assert_eq!(error.event_index, 4);
    assert!(matches!(
        error.error,
        InquiryError::InvalidOutline { ref reason }
            if reason.contains("material question id `question:root`")
    ));
}

#[test]
fn replay_rejects_cross_evidence_claim_source_pairing() {
    let events = vec![
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
            answer: "The first evidence item answers the inquiry.".to_string(),
            evidence_ids: vec!["evidence:first".to_string()],
        },
        InquiryEvent::OutlineCommitted {
            outline: ResearchOutline {
                sections: vec![outline_section(
                    "section:cross-item",
                    "claim:first",
                    "source:second",
                )],
            },
        },
    ];

    let error = replay(&events, &InquiryLimits::default())
        .expect_err("an unrelated accepted source must not ground a claim");
    assert_eq!(error.event_index, 5);
    assert!(matches!(
        error.error,
        InquiryError::InvalidOutline { ref reason }
            if reason.contains("same accepted evidence item")
    ));
}

#[test]
fn replay_requires_material_question_answer_evidence_pairing() {
    let events = vec![
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
                "evidence:answer",
                vec!["claim:answer".to_string()],
                vec!["source:answer".to_string()],
            ),
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:unrelated",
                vec!["claim:unrelated".to_string()],
                vec!["source:unrelated".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:root".to_string(),
            answer: "Only the answer evidence item supports this response.".to_string(),
            evidence_ids: vec!["evidence:answer".to_string()],
        },
        InquiryEvent::OutlineCommitted {
            outline: ResearchOutline {
                sections: vec![outline_section(
                    "section:unrelated",
                    "claim:unrelated",
                    "source:unrelated",
                )],
            },
        },
    ];

    let error = replay(&events, &InquiryLimits::default())
        .expect_err("material coverage must use the question's answer evidence");
    assert_eq!(error.event_index, 5);
    assert!(matches!(
        error.error,
        InquiryError::InvalidOutline { ref reason }
            if reason.contains("material question id `question:root`")
                && reason.contains("claim id `claim:answer`")
                && reason.contains("answer evidence id `evidence:answer`")
    ));
}

#[test]
fn replay_rejects_outlining_when_every_material_question_is_bounded() {
    let events = vec![
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
        InquiryEvent::QuestionBounded {
            question_id: "question:root".to_string(),
            reason: "No retained evidence answers the material question.".to_string(),
        },
        InquiryEvent::OutlineCommitted {
            outline: ResearchOutline {
                sections: vec![outline_section(
                    "section:bounded",
                    "claim:missing",
                    "source:missing",
                )],
            },
        },
    ];
    let error = replay(&events, &InquiryLimits::default())
        .expect_err("an all-bounded inquiry must not publish a completed outline");
    assert_eq!(error.event_index, 3);
    assert!(matches!(
        error.error,
        InquiryError::InvalidOutline { ref reason }
            if reason.contains("every material research question")
    ));
}

#[test]
fn replay_rejects_outlining_when_any_material_question_is_bounded() {
    let mut second = Question::queued(
        "question:second",
        None,
        "Which consequential boundary also needs evidence?",
    );
    second.material = true;
    let mut section = outline_section("section:partial", "claim:accepted", "source:accepted");
    section.question_ids.push("question:second".to_string());
    let events = vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![
                Question::queued("question:root", None, "What evidence answers the inquiry?"),
                second,
            ],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:accepted",
                vec!["claim:accepted".to_string()],
                vec!["source:accepted".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:root".to_string(),
            answer: "The accepted evidence supports one material finding.".to_string(),
            evidence_ids: vec!["evidence:accepted".to_string()],
        },
        InquiryEvent::QuestionBounded {
            question_id: "question:second".to_string(),
            reason: "The consequential boundary remains unsupported.".to_string(),
        },
        InquiryEvent::OutlineCommitted {
            outline: ResearchOutline {
                sections: vec![section],
            },
        },
    ];

    let error = replay(&events, &InquiryLimits::default())
        .expect_err("a partially bounded material inquiry must not publish");
    assert_eq!(error.event_index, 5);
    assert!(matches!(
        error.error,
        InquiryError::InvalidOutline { ref reason }
            if reason.contains("1 remain bounded")
    ));
}

#[test]
fn replay_allows_outlining_when_only_a_supporting_question_is_bounded() {
    let mut supporting = Question::queued(
        "question:supporting",
        None,
        "Which non-gating context remains useful?",
    );
    supporting.material = false;
    let mut qualified_section =
        outline_section("section:qualified", "claim:accepted", "source:accepted");
    qualified_section
        .question_ids
        .push("question:supporting".to_string());
    let events = vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![
                Question::queued(
                    "question:root",
                    None,
                    "What evidence determines the core conclusion?",
                ),
                supporting,
            ],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:accepted",
                vec!["claim:accepted".to_string()],
                vec!["source:accepted".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:root".to_string(),
            answer: "The accepted evidence supports the core conclusion.".to_string(),
            evidence_ids: vec!["evidence:accepted".to_string()],
        },
        InquiryEvent::QuestionBounded {
            question_id: "question:supporting".to_string(),
            reason: "The supporting context remains unavailable.".to_string(),
        },
        InquiryEvent::OutlineCommitted {
            outline: ResearchOutline {
                sections: vec![qualified_section],
            },
        },
    ];

    let state = replay(&events, &InquiryLimits::default())
        .expect("a bounded supporting obligation must not block the core report");
    assert_eq!(state.phase, InquiryPhase::Drafting);
}

#[test]
fn legacy_replay_rejects_materiality_escalation_in_supporting_follow_ups() {
    let limits = InquiryLimits::default();
    let mut supporting = Question::queued(
        "question:supporting",
        None,
        "Which non-gating context remains useful?",
    );
    supporting.material = false;
    let mut state = replay(
        &[
            InquiryEvent::StrategySelected {
                method: ResearchMethod::Focused,
            },
            InquiryEvent::QuestionsQueued {
                questions: vec![supporting],
            },
        ],
        &limits,
    )
    .expect("supporting parent should be queued");
    state = reduce(
        &state,
        &InquiryEvent::QuestionDeferred {
            question_id: "question:supporting".to_string(),
            reason: "The completed wave left this supporting detail bounded.".to_string(),
        },
        &limits,
    )
    .expect("supporting parent should be deferred before refinement");

    let promoted = legacy_follow_up(
        "question:promoted-follow-up",
        None,
        "question:supporting",
        1,
        "Can the non-gating context become a publication blocker?",
    );
    let error = reduce(
        &state,
        &InquiryEvent::QuestionsQueued {
            questions: vec![promoted],
        },
        &limits,
    )
    .expect_err("a supporting follow-up must not become material");
    assert!(matches!(
        error,
        InquiryError::InvalidResearchPlan { ref reason }
            if reason.contains("must inherit material=false")
                && reason.contains("question:supporting")
    ));

    let mut inherited = legacy_follow_up(
        "question:supporting-follow-up",
        None,
        "question:supporting",
        1,
        "Which bounded supporting detail can still be verified?",
    );
    inherited.material = false;
    let next = reduce(
        &state,
        &InquiryEvent::QuestionsQueued {
            questions: vec![inherited],
        },
        &limits,
    )
    .expect("a supporting follow-up with inherited materiality should be accepted");
    let follow_up = next
        .questions
        .iter()
        .find(|question| question.id == "question:supporting-follow-up")
        .expect("accepted follow-up should be projected");
    assert!(!follow_up.material);
}

#[test]
fn legacy_replay_rejects_follow_up_for_an_answered_parent() {
    let limits = InquiryLimits::default();
    let state = replay(
        &[
            InquiryEvent::StrategySelected {
                method: ResearchMethod::Focused,
            },
            InquiryEvent::QuestionsQueued {
                questions: vec![Question::queued(
                    "question:answered-parent",
                    None,
                    "What does the retained evidence establish?",
                )],
            },
            InquiryEvent::EvidenceAccepted {
                evidence: EvidenceRef::new(
                    "evidence:accepted",
                    vec!["claim:accepted".to_string()],
                    vec!["source:accepted".to_string()],
                ),
            },
            InquiryEvent::QuestionAnswered {
                question_id: "question:answered-parent".to_string(),
                answer: "The retained evidence answers the question.".to_string(),
                evidence_ids: vec!["evidence:accepted".to_string()],
            },
        ],
        &limits,
    )
    .expect("answered parent state");

    let error = reduce(
        &state,
        &InquiryEvent::QuestionsQueued {
            questions: vec![legacy_follow_up(
                "question:invalid-follow-up",
                None,
                "question:answered-parent",
                1,
                "Which optional detail could extend the answered question?",
            )],
        },
        &limits,
    )
    .expect_err("an answered parent must not gain a follow-up");

    assert!(matches!(
        error,
        InquiryError::InvalidResearchPlan { ref reason }
            if reason.contains("may refine only a deferred bounded parent")
                && reason.contains("question:answered-parent")
    ));
}

include!("tests/evidence_catalog.rs");
include!("tests/drafting.rs");
