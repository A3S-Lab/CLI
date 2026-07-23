use super::test_support::*;
use super::*;

fn two_dimension_contract() -> ResearchContract {
    let spec = spec(
        EvidenceScope::Web,
        vec![
            dimension("observed-fact", &["fact-source"]),
            dimension("decision", &["fact-source"]),
        ],
        vec![named_target(
            "fact-source",
            "primary_records",
            SourceRole::Primary,
            SourceIdentity::Domain("evidence.example.test".to_string()),
        )],
        budget(1, 1),
    );
    let plan = plan(
        &spec,
        vec![query(
            "q-primary",
            AcquisitionTransport::Web,
            QueryMode::Exact,
            &["observed-fact", "decision"],
            &["fact-source"],
            1,
        )],
        vec![],
    );
    validate_research_contract(spec, plan).expect("fixture contract")
}

fn catalog(contract: &ResearchContract) -> SourceCatalog {
    SourceCatalog {
        spec_digest: research_spec_digest(&contract.spec),
        attempts: vec![AcquisitionAttempt {
            query_id: "q-primary".to_string(),
            source_target_ids: vec!["fact-source".to_string()],
            outcome: AcquisitionOutcome::Fetched,
        }],
        sources: vec![source(
            "source-primary",
            "fact-source",
            &["The primary record establishes value 100 and revised value 80."],
        )],
    }
}

fn source(id: &str, target_id: &str, chunks: &[&str]) -> SourceRecord {
    source_for_query(id, target_id, "q-primary", chunks)
}

fn source_for_query(id: &str, target_id: &str, query_id: &str, chunks: &[&str]) -> SourceRecord {
    let chunks = chunks
        .iter()
        .enumerate()
        .map(|(index, text)| SourceChunk {
            id: format!("{id}:chunk:{}", index + 1),
            text: (*text).to_string(),
        })
        .collect::<Vec<_>>();
    SourceRecord {
        id: id.to_string(),
        title: format!("Source {id}"),
        requested_anchor: format!("https://evidence.example.test/{id}"),
        canonical_anchor: format!("https://evidence.example.test/{id}"),
        captured_at: "2026-07-21T00:00:00Z".to_string(),
        provenance: vec![SourceProvenance {
            query_id: query_id.to_string(),
            source_target_id: target_id.to_string(),
        }],
        content_digest: source_content_digest(&chunks),
        chunks,
    }
}

fn fact(id: &str, dimension_id: &str, source_id: &str, chunk_id: &str) -> ClaimProposal {
    ClaimProposal {
        id: id.to_string(),
        dimension_id: dimension_id.to_string(),
        placement: ClaimPlacement::Finding,
        kind: ClaimKind::Fact,
        text: format!("Admitted factual claim {id}."),
        evidence_refs: vec![ClaimEvidenceRef {
            source_id: source_id.to_string(),
            chunk_ids: vec![chunk_id.to_string()],
        }],
        basis_claim_ids: vec![],
        derivation: None,
    }
}

fn recommendation(id: &str, basis_claim_ids: &[&str]) -> ClaimProposal {
    ClaimProposal {
        id: id.to_string(),
        dimension_id: "decision".to_string(),
        placement: ClaimPlacement::Finding,
        kind: ClaimKind::Recommendation,
        text: "Prefer the bounded option while the observed constraint remains.".to_string(),
        evidence_refs: vec![],
        basis_claim_ids: basis_claim_ids.iter().map(|id| (*id).to_string()).collect(),
        derivation: None,
    }
}

fn proposal(claims: Vec<ClaimProposal>) -> ClaimLedgerProposal {
    ClaimLedgerProposal {
        claims,
        relations: vec![],
        gaps: vec![],
    }
}

#[test]
fn premise_admission_is_independent_of_proposal_order() {
    let contract = two_dimension_contract();
    let catalog = catalog(&contract);
    let proposal = proposal(vec![
        recommendation("bounded-advice", &["observed-value"]),
        fact(
            "observed-value",
            "observed-fact",
            "source-primary",
            "source-primary:chunk:1",
        ),
    ]);

    let ledger = admit_claim_ledger(&contract, &catalog, proposal).expect("ledger admission");

    assert_eq!(
        ledger
            .claims
            .iter()
            .map(|claim| claim.id.as_str())
            .collect::<Vec<_>>(),
        ["bounded-advice", "observed-value"]
    );
    assert!(ledger.rejections.is_empty());
}

#[test]
fn malformed_claim_preserves_a_valid_sibling_and_creates_a_local_gap() {
    let contract = two_dimension_contract();
    let catalog = catalog(&contract);
    let valid = fact(
        "observed-value",
        "observed-fact",
        "source-primary",
        "source-primary:chunk:1",
    );
    let invalid = fact(
        "invalid-decision-fact",
        "decision",
        "source-primary",
        "source-primary:chunk:missing",
    );

    let ledger =
        admit_claim_ledger(&contract, &catalog, proposal(vec![valid, invalid])).expect("salvage");

    assert_eq!(ledger.claims.len(), 1);
    assert_eq!(ledger.claims[0].id, "observed-value");
    assert!(ledger
        .rejections
        .iter()
        .any(|rejection| rejection.item_id == "invalid-decision-fact"));
    assert!(ledger.gaps.iter().any(|gap| {
        gap.dimension_id == "decision" && gap.origin == GapOrigin::HostMissingOutput
    }));
}

#[test]
fn a_fact_cannot_borrow_evidence_from_another_dimension_target() {
    let spec = spec(
        EvidenceScope::Web,
        vec![
            dimension("alpha", &["alpha-target"]),
            dimension("beta", &["beta-target"]),
        ],
        vec![
            named_target(
                "alpha-target",
                "alpha_family",
                SourceRole::Primary,
                SourceIdentity::Domain("alpha.example.test".to_string()),
            ),
            named_target(
                "beta-target",
                "beta_family",
                SourceRole::Primary,
                SourceIdentity::Domain("beta.example.test".to_string()),
            ),
        ],
        budget(2, 2),
    );
    let plan = plan(
        &spec,
        vec![
            query(
                "q-alpha",
                AcquisitionTransport::Web,
                QueryMode::Exact,
                &["alpha"],
                &["alpha-target"],
                1,
            ),
            query(
                "q-beta",
                AcquisitionTransport::Web,
                QueryMode::Exact,
                &["beta"],
                &["beta-target"],
                1,
            ),
        ],
        vec![],
    );
    let contract = validate_research_contract(spec, plan).expect("fixture contract");
    let beta_source = source_for_query(
        "source-beta",
        "beta-target",
        "q-beta",
        &["Beta evidence only."],
    );
    let catalog = SourceCatalog {
        spec_digest: research_spec_digest(&contract.spec),
        attempts: vec![AcquisitionAttempt {
            query_id: "q-beta".to_string(),
            source_target_ids: vec!["beta-target".to_string()],
            outcome: AcquisitionOutcome::Fetched,
        }],
        sources: vec![beta_source],
    };
    let borrowed = fact(
        "alpha-from-beta",
        "alpha",
        "source-beta",
        "source-beta:chunk:1",
    );

    let ledger =
        admit_claim_ledger(&contract, &catalog, proposal(vec![borrowed])).expect("admission");

    assert!(ledger.claims.is_empty());
    assert!(ledger.rejections.iter().any(|rejection| {
        rejection.item_id == "alpha-from-beta"
            && rejection.reason == RejectionReason::EvidenceOutsideDimensionTargets
    }));
}

#[test]
fn contradiction_relation_preserves_both_independent_facts() {
    let contract = two_dimension_contract();
    let mut catalog = catalog(&contract);
    catalog.sources.push(source(
        "source-secondary",
        "fact-source",
        &["The second primary record establishes a conflicting value 90."],
    ));
    let proposal = ClaimLedgerProposal {
        claims: vec![
            fact(
                "record-a",
                "observed-fact",
                "source-primary",
                "source-primary:chunk:1",
            ),
            fact(
                "record-b",
                "observed-fact",
                "source-secondary",
                "source-secondary:chunk:1",
            ),
        ],
        relations: vec![ClaimRelationProposal {
            id: "conflict-a-b".to_string(),
            dimension_id: "observed-fact".to_string(),
            kind: ClaimRelationKind::Contradicts,
            claim_ids: ["record-a".to_string(), "record-b".to_string()],
        }],
        gaps: vec![],
    };

    let ledger = admit_claim_ledger(&contract, &catalog, proposal).expect("contradiction");

    assert_eq!(ledger.claims.len(), 2);
    assert_eq!(ledger.relations.len(), 1);
    assert_eq!(ledger.relations[0].id, "conflict-a-b");
}

#[test]
fn invalid_contradiction_relation_does_not_remove_factual_siblings() {
    let contract = two_dimension_contract();
    let catalog = catalog(&contract);
    let proposal = ClaimLedgerProposal {
        claims: vec![fact(
            "record-a",
            "observed-fact",
            "source-primary",
            "source-primary:chunk:1",
        )],
        relations: vec![ClaimRelationProposal {
            id: "invalid-conflict".to_string(),
            dimension_id: "observed-fact".to_string(),
            kind: ClaimRelationKind::Contradicts,
            claim_ids: ["record-a".to_string(), "missing-record".to_string()],
        }],
        gaps: vec![],
    };

    let ledger = admit_claim_ledger(&contract, &catalog, proposal).expect("sibling salvage");

    assert_eq!(ledger.claims.len(), 1);
    assert!(ledger.relations.is_empty());
    assert!(ledger
        .rejections
        .iter()
        .any(|rejection| rejection.item_id == "invalid-conflict"));
}

#[test]
fn derived_claim_requires_admitted_inputs_and_keeps_its_method() {
    let contract = two_dimension_contract();
    let catalog = catalog(&contract);
    let derived = ClaimProposal {
        id: "derived-change".to_string(),
        dimension_id: "decision".to_string(),
        placement: ClaimPlacement::Finding,
        kind: ClaimKind::Inference,
        text: "The recorded value decreased by 20 percent.".to_string(),
        evidence_refs: vec![],
        basis_claim_ids: vec!["observed-value".to_string()],
        derivation: Some(DerivationProposal {
            method: "(100 - 80) / 100 * 100".to_string(),
            input_claim_ids: vec!["observed-value".to_string()],
        }),
    };
    let proposal = proposal(vec![
        derived,
        fact(
            "observed-value",
            "observed-fact",
            "source-primary",
            "source-primary:chunk:1",
        ),
    ]);

    let ledger = admit_claim_ledger(&contract, &catalog, proposal).expect("derivation");

    let admitted = ledger
        .claims
        .iter()
        .find(|claim| claim.id == "derived-change")
        .expect("derived claim");
    assert_eq!(
        admitted
            .derivation
            .as_ref()
            .map(|item| item.method.as_str()),
        Some("(100 - 80) / 100 * 100")
    );
}

#[test]
fn cyclic_inferences_are_rejected_without_erasing_a_valid_fact() {
    let contract = two_dimension_contract();
    let catalog = catalog(&contract);
    let inference = |id: &str, basis: &str| ClaimProposal {
        id: id.to_string(),
        dimension_id: "decision".to_string(),
        placement: ClaimPlacement::Finding,
        kind: ClaimKind::Inference,
        text: format!("Inference {id}."),
        evidence_refs: vec![],
        basis_claim_ids: vec![basis.to_string()],
        derivation: None,
    };
    let proposal = proposal(vec![
        fact(
            "observed-value",
            "observed-fact",
            "source-primary",
            "source-primary:chunk:1",
        ),
        inference("cycle-a", "cycle-b"),
        inference("cycle-b", "cycle-a"),
    ]);

    let ledger = admit_claim_ledger(&contract, &catalog, proposal).expect("cycle salvage");

    assert_eq!(ledger.claims.len(), 1);
    assert_eq!(ledger.claims[0].id, "observed-value");
    assert!(ledger
        .rejections
        .iter()
        .any(|rejection| rejection.item_id == "cycle-a"));
    assert!(ledger
        .rejections
        .iter()
        .any(|rejection| rejection.item_id == "cycle-b"));
}

#[test]
fn a_gap_must_reference_real_queries_and_dimension_targets() {
    let contract = two_dimension_contract();
    let catalog = catalog(&contract);
    let invalid_gap = GapProposal {
        id: "decision-gap".to_string(),
        dimension_id: "decision".to_string(),
        text: "The bounded acquisition did not establish deployment cost.".to_string(),
        attempted_query_ids: vec!["unknown-query".to_string()],
        missing_source_target_ids: vec!["fact-source".to_string()],
    };
    let proposal = ClaimLedgerProposal {
        claims: vec![fact(
            "observed-value",
            "observed-fact",
            "source-primary",
            "source-primary:chunk:1",
        )],
        relations: vec![],
        gaps: vec![invalid_gap],
    };

    let ledger = admit_claim_ledger(&contract, &catalog, proposal).expect("gap salvage");

    assert!(ledger
        .rejections
        .iter()
        .any(|rejection| rejection.item_id == "decision-gap"));
    assert!(ledger.gaps.iter().any(|gap| {
        gap.dimension_id == "decision" && gap.origin == GapOrigin::HostMissingOutput
    }));
}

#[test]
fn a_gap_query_must_have_attempted_each_reported_missing_target() {
    let spec = spec(
        EvidenceScope::Web,
        vec![dimension("availability", &["alpha-target", "beta-target"])],
        vec![
            named_target(
                "alpha-target",
                "alpha_family",
                SourceRole::Primary,
                SourceIdentity::Domain("alpha.example.test".to_string()),
            ),
            named_target(
                "beta-target",
                "beta_family",
                SourceRole::Primary,
                SourceIdentity::Domain("beta.example.test".to_string()),
            ),
        ],
        budget(2, 2),
    );
    let plan = plan(
        &spec,
        vec![
            query(
                "q-alpha",
                AcquisitionTransport::Web,
                QueryMode::Exact,
                &["availability"],
                &["alpha-target"],
                1,
            ),
            query(
                "q-beta",
                AcquisitionTransport::Web,
                QueryMode::Exact,
                &["availability"],
                &["beta-target"],
                1,
            ),
        ],
        vec![],
    );
    let contract = validate_research_contract(spec, plan).expect("gap provenance contract");
    let catalog = SourceCatalog {
        spec_digest: research_spec_digest(&contract.spec),
        attempts: vec![AcquisitionAttempt {
            query_id: "q-alpha".to_string(),
            source_target_ids: vec!["alpha-target".to_string()],
            outcome: AcquisitionOutcome::NoCandidates,
        }],
        sources: vec![],
    };
    let proposed_gap = GapProposal {
        id: "beta-gap".to_string(),
        dimension_id: "availability".to_string(),
        text: "The bounded acquisition did not establish the Beta target.".to_string(),
        attempted_query_ids: vec!["q-alpha".to_string()],
        missing_source_target_ids: vec!["beta-target".to_string()],
    };

    let ledger = admit_claim_ledger(
        &contract,
        &catalog,
        ClaimLedgerProposal {
            claims: vec![],
            relations: vec![],
            gaps: vec![proposed_gap],
        },
    )
    .expect("gap admission");

    assert!(ledger.rejections.iter().any(|rejection| {
        rejection.item_id == "beta-gap" && rejection.reason == RejectionReason::InvalidGapProvenance
    }));
    assert!(ledger.gaps.iter().any(|gap| {
        gap.dimension_id == "availability" && gap.origin == GapOrigin::HostMissingOutput
    }));
}

#[test]
fn a_recommendation_cannot_use_basis_evidence_outside_its_dimension_targets() {
    let spec = spec(
        EvidenceScope::Web,
        vec![
            dimension("alpha", &["alpha-target"]),
            dimension("beta-decision", &["beta-target"]),
        ],
        vec![
            named_target(
                "alpha-target",
                "alpha_family",
                SourceRole::Primary,
                SourceIdentity::Domain("alpha.example.test".to_string()),
            ),
            named_target(
                "beta-target",
                "beta_family",
                SourceRole::Primary,
                SourceIdentity::Domain("beta.example.test".to_string()),
            ),
        ],
        budget(2, 2),
    );
    let plan = plan(
        &spec,
        vec![
            query(
                "q-alpha",
                AcquisitionTransport::Web,
                QueryMode::Exact,
                &["alpha"],
                &["alpha-target"],
                1,
            ),
            query(
                "q-beta",
                AcquisitionTransport::Web,
                QueryMode::Exact,
                &["beta-decision"],
                &["beta-target"],
                1,
            ),
        ],
        vec![],
    );
    let contract = validate_research_contract(spec, plan).expect("basis-target contract");
    let alpha_source = source_for_query(
        "source-alpha",
        "alpha-target",
        "q-alpha",
        &["Alpha evidence only."],
    );
    let catalog = SourceCatalog {
        spec_digest: research_spec_digest(&contract.spec),
        attempts: vec![
            AcquisitionAttempt {
                query_id: "q-alpha".to_string(),
                source_target_ids: vec!["alpha-target".to_string()],
                outcome: AcquisitionOutcome::Fetched,
            },
            AcquisitionAttempt {
                query_id: "q-beta".to_string(),
                source_target_ids: vec!["beta-target".to_string()],
                outcome: AcquisitionOutcome::NoCandidates,
            },
        ],
        sources: vec![alpha_source],
    };
    let alpha_fact = fact(
        "alpha-fact",
        "alpha",
        "source-alpha",
        "source-alpha:chunk:1",
    );
    let beta_recommendation = ClaimProposal {
        id: "beta-advice".to_string(),
        dimension_id: "beta-decision".to_string(),
        placement: ClaimPlacement::DirectAnswer,
        kind: ClaimKind::Recommendation,
        text: "Choose the Beta option.".to_string(),
        evidence_refs: vec![],
        basis_claim_ids: vec!["alpha-fact".to_string()],
        derivation: None,
    };

    let ledger = admit_claim_ledger(
        &contract,
        &catalog,
        proposal(vec![alpha_fact, beta_recommendation]),
    )
    .expect("basis-target admission");

    assert!(ledger.rejections.iter().any(|rejection| {
        rejection.item_id == "beta-advice"
            && rejection.reason == RejectionReason::EvidenceOutsideDimensionTargets
    }));
    assert!(ledger.gaps.iter().any(|gap| {
        gap.dimension_id == "beta-decision" && gap.origin == GapOrigin::HostMissingOutput
    }));
}
