use super::super::super::deep_research_evidence_ledger::{
    AcceptedClaim, AcceptedEvidence, AcceptedSource, SourceTier,
};
use super::super::{DeepResearchStateJournal, ResearchSpec};
use super::*;
use a3s::research::{
    EvidenceRef, InquiryEvent, InquiryLimits, InquiryState, OutlineSection, Perspective, Question,
    ResearchMethod, ResearchOutline,
};

fn inquiry_spec() -> ResearchSpec {
    ResearchSpec {
        query: "event-sourced inquiry".to_string(),
        current_date: "2026-07-17".to_string(),
        evidence_scope: "web".to_string(),
        required_claims: vec!["claim:root".to_string()],
        total_budget_ms: 60_000,
        finalization_reserve_ms: 9_000,
        host_pid: 0,
    }
}

fn inquiry_events() -> Vec<InquiryEvent> {
    vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::PerspectiveGuided,
        },
        InquiryEvent::ScoutCompleted {
            source_ids: vec!["source:scout".to_string()],
        },
        InquiryEvent::PerspectivesCommitted {
            perspectives: vec![Perspective::new(
                "perspective:risk",
                "Risk",
                "Test material risks",
                vec!["source:scout".to_string()],
            )],
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![Question::queued(
                "question:root",
                Some("perspective:risk".to_string()),
                "What does the evidence establish?",
            )],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                "evidence:accepted",
                vec!["claim:root".to_string()],
                vec!["source:accepted".to_string()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:root".to_string(),
            answer: "The evidence establishes the material finding.".to_string(),
            evidence_ids: vec!["evidence:accepted".to_string()],
        },
        InquiryEvent::OutlineCommitted {
            outline: ResearchOutline {
                sections: vec![OutlineSection {
                    id: "section:findings".to_string(),
                    heading: "Findings".to_string(),
                    purpose: "Explain the accepted finding.".to_string(),
                    perspective_ids: vec!["perspective:risk".to_string()],
                    question_ids: vec!["question:root".to_string()],
                    claim_ids: vec!["claim:root".to_string()],
                    source_ids: vec!["source:accepted".to_string()],
                    composition_hint: "Lead with the finding.".to_string(),
                }],
            },
        },
        InquiryEvent::SectionDrafted {
            section_id: "section:findings".to_string(),
            content: "A source-backed section draft.".to_string(),
            citation_ids: vec!["claim:root".to_string(), "source:accepted".to_string()],
        },
        InquiryEvent::AuditCompleted {
            passed: true,
            issues: Vec::new(),
        },
    ]
}

fn accepted_evidence() -> Vec<AcceptedEvidence> {
    vec![AcceptedEvidence {
        id: "evidence:accepted".to_string(),
        summary: "The accepted evidence establishes the material finding.".to_string(),
        confidence: Some("high".to_string()),
        sources: vec![AcceptedSource {
            id: "source:accepted".to_string(),
            anchor: "https://example.com/accepted".to_string(),
            title: Some("Accepted source".to_string()),
            date: Some("2026-07-17".to_string()),
            reliability: Some("authoritative".to_string()),
            quote_or_fact: Some("The material finding is established.".to_string()),
            tier: SourceTier::Authoritative,
        }],
        claims: vec![AcceptedClaim {
            id: "claim:root".to_string(),
            text: "The material finding is established.".to_string(),
        }],
        contradictions: Vec::new(),
        gaps: Vec::new(),
    }]
}

async fn recorded_inquiry(run_id: &str) -> (tempfile::TempDir, Vec<InquiryEvent>, InquiryState) {
    let temp = tempfile::tempdir().unwrap();
    DeepResearchStateJournal::create(temp.path(), run_id, inquiry_spec())
        .await
        .unwrap();
    let events = inquiry_events();
    let state = a3s::research::replay(&events, &InquiryLimits::default()).unwrap();
    let (first, concurrent) = tokio::join!(
        record_inquiry_state(temp.path(), run_id, &events, &state),
        record_inquiry_state(temp.path(), run_id, &events, &state),
    );
    first.unwrap();
    concurrent.unwrap();
    (temp, events, state)
}

#[tokio::test]
async fn inquiry_objects_replay_idempotently_with_final_state() {
    let (temp, events, state) = recorded_inquiry("run-inquiry-objects").await;
    let journal = DeepResearchStateJournal::open(temp.path(), "run-inquiry-objects")
        .await
        .unwrap()
        .unwrap();
    for (object_type, expected) in [
        (PERSPECTIVE_OBJECT_TYPE, 1),
        (QUESTION_OBJECT_TYPE, 1),
        (OUTLINE_SECTION_OBJECT_TYPE, 1),
        (SECTION_DRAFT_OBJECT_TYPE, 1),
    ] {
        assert_eq!(
            journal
                .runtime
                .graph()
                .objects()
                .filter(|object| object.object_type == object_type)
                .count(),
            expected
        );
    }
    let question = journal
        .runtime
        .graph()
        .object(&object_id(
            "run-inquiry-objects",
            "question",
            "question:root",
        ))
        .unwrap();
    assert_eq!(
        serde_json::from_value::<Question>(question.data.clone()).unwrap(),
        state.questions[0]
    );
    let event_count = journal.runtime.events().len();
    drop(journal);
    record_inquiry_state(temp.path(), "run-inquiry-objects", &events, &state)
        .await
        .unwrap();
    let reopened = DeepResearchStateJournal::open(temp.path(), "run-inquiry-objects")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reopened.runtime.events().len(), event_count);
}

#[tokio::test]
async fn inquiry_relations_replay_with_stable_direction_and_run_scope() {
    let (temp, _, _) = recorded_inquiry("run-inquiry-relations").await;
    let journal = DeepResearchStateJournal::open(temp.path(), "run-inquiry-relations")
        .await
        .unwrap()
        .unwrap();
    let run_id = "run-inquiry-relations";
    let perspective = object_id(run_id, "perspective", "perspective:risk");
    let question = object_id(run_id, "question", "question:root");
    let section = object_id(run_id, "outline-section", "section:findings");
    let draft = object_id(run_id, "section-draft", "section:findings");
    let relations = journal.runtime.graph().relations().collect::<Vec<_>>();
    for (relation_type, source, target) in [
        ("deep_research.frames_question", &perspective, &question),
        ("deep_research.covers_perspective", &section, &perspective),
        ("deep_research.covers_question", &section, &question),
        ("deep_research.has_section_draft", &section, &draft),
    ] {
        assert!(relations.iter().any(|relation| {
            relation.relation_type == relation_type
                && &relation.source == source
                && &relation.target == target
        }));
        assert!(!relations.iter().any(|relation| {
            relation.relation_type == relation_type
                && &relation.source == target
                && &relation.target == source
        }));
    }
    assert_ne!(
        perspective,
        object_id("another-run", "perspective", "perspective:risk")
    );
}

#[tokio::test]
async fn evidence_recorded_after_inquiry_connects_answers_outline_and_draft() {
    let run_id = "run-inquiry-then-evidence";
    let (temp, _, _) = recorded_inquiry(run_id).await;
    super::super::record_evidence_ledger(temp.path(), run_id, &accepted_evidence())
        .await
        .unwrap();

    assert_inquiry_evidence_relations(
        &DeepResearchStateJournal::open(temp.path(), run_id)
            .await
            .unwrap()
            .unwrap(),
        run_id,
    );
}

#[tokio::test]
async fn inquiry_recorded_after_evidence_connects_answers_outline_and_draft() {
    let temp = tempfile::tempdir().unwrap();
    let run_id = "run-evidence-then-inquiry";
    DeepResearchStateJournal::create(temp.path(), run_id, inquiry_spec())
        .await
        .unwrap();
    super::super::record_evidence_ledger(temp.path(), run_id, &accepted_evidence())
        .await
        .unwrap();
    let events = inquiry_events();
    let state = a3s::research::replay(&events, &InquiryLimits::default()).unwrap();
    record_inquiry_state(temp.path(), run_id, &events, &state)
        .await
        .unwrap();

    assert_inquiry_evidence_relations(
        &DeepResearchStateJournal::open(temp.path(), run_id)
            .await
            .unwrap()
            .unwrap(),
        run_id,
    );
}

#[tokio::test]
async fn redrafted_section_replaces_stale_claim_and_source_relations() {
    let temp = tempfile::tempdir().unwrap();
    let run_id = "run-redraft-replaces-citations";
    DeepResearchStateJournal::create(temp.path(), run_id, inquiry_spec())
        .await
        .unwrap();

    let mut evidence = accepted_evidence();
    evidence[0].claims.push(AcceptedClaim {
        id: "claim:replacement".to_string(),
        text: "The replacement finding is established.".to_string(),
    });
    evidence[0].sources.push(AcceptedSource {
        id: "source:replacement".to_string(),
        anchor: "https://example.com/replacement".to_string(),
        title: Some("Replacement source".to_string()),
        date: Some("2026-07-17".to_string()),
        reliability: Some("authoritative".to_string()),
        quote_or_fact: Some("The replacement finding is established.".to_string()),
        tier: SourceTier::Authoritative,
    });
    super::super::record_evidence_ledger(temp.path(), run_id, &evidence)
        .await
        .unwrap();

    let mut events = inquiry_events();
    let InquiryEvent::EvidenceAccepted {
        evidence: reference,
    } = &mut events[4]
    else {
        panic!("fixture event 4 must accept evidence");
    };
    reference.claim_ids.push("claim:replacement".to_string());
    reference.source_ids.push("source:replacement".to_string());
    let InquiryEvent::OutlineCommitted { outline } = &mut events[6] else {
        panic!("fixture event 6 must commit the outline");
    };
    outline.sections[0]
        .claim_ids
        .push("claim:replacement".to_string());
    outline.sections[0]
        .source_ids
        .push("source:replacement".to_string());
    let InquiryEvent::AuditCompleted { passed, issues } = &mut events[8] else {
        panic!("fixture event 8 must complete the audit");
    };
    *passed = false;
    issues.push("replace the draft evidence".to_string());

    let limits = InquiryLimits::default();
    let initial_state = a3s::research::replay(&events, &limits).unwrap();
    record_inquiry_state(temp.path(), run_id, &events, &initial_state)
        .await
        .unwrap();

    events.push(InquiryEvent::SectionDrafted {
        section_id: "section:findings".to_string(),
        content: "A replacement source-backed section draft.".to_string(),
        citation_ids: vec![
            "claim:replacement".to_string(),
            "source:replacement".to_string(),
        ],
    });
    let redrafted_state = a3s::research::replay(&events, &limits).unwrap();
    record_inquiry_state(temp.path(), run_id, &events, &redrafted_state)
        .await
        .unwrap();

    let journal = DeepResearchStateJournal::open(temp.path(), run_id)
        .await
        .unwrap()
        .unwrap();
    let draft = object_id(run_id, "section-draft", "section:findings");
    let citation_relations = journal
        .runtime
        .graph()
        .relations()
        .filter(|relation| {
            relation.source == draft
                && matches!(
                    relation.relation_type.as_str(),
                    "deep_research.cites_claim" | "deep_research.cites_source"
                )
        })
        .map(|relation| (relation.relation_type.clone(), relation.target.clone()))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        citation_relations,
        [
            (
                "deep_research.cites_claim".to_string(),
                "claim:replacement".to_string(),
            ),
            (
                "deep_research.cites_source".to_string(),
                "source:replacement".to_string(),
            ),
        ]
        .into_iter()
        .collect()
    );
    GraphRuntime::strict_replay(journal.runtime.events()).unwrap();
}

fn assert_inquiry_evidence_relations(journal: &DeepResearchStateJournal, run_id: &str) {
    let question = object_id(run_id, "question", "question:root");
    let section = object_id(run_id, "outline-section", "section:findings");
    let draft = object_id(run_id, "section-draft", "section:findings");
    let relations = journal.runtime.graph().relations().collect::<Vec<_>>();
    for (relation_type, source, target) in [
        (
            "deep_research.answered_by",
            question.as_str(),
            "evidence:accepted",
        ),
        ("deep_research.covers_claim", section.as_str(), "claim:root"),
        (
            "deep_research.covers_source",
            section.as_str(),
            "source:accepted",
        ),
        ("deep_research.cites_claim", draft.as_str(), "claim:root"),
        (
            "deep_research.cites_source",
            draft.as_str(),
            "source:accepted",
        ),
    ] {
        assert!(
            relations.iter().any(|relation| {
                relation.relation_type == relation_type
                    && relation.source == source
                    && relation.target == target
            }),
            "missing {relation_type}: {source} -> {target}"
        );
    }
    GraphRuntime::strict_replay(journal.runtime.events()).unwrap();
}
