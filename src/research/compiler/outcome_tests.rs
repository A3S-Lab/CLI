use super::test_support::*;
use super::*;

fn one_dimension_fixture(
    outcome: AcquisitionOutcome,
    with_source: bool,
) -> (ResearchContract, SourceCatalog) {
    let spec = spec(
        EvidenceScope::Web,
        vec![dimension("answer", &["answer-target"])],
        vec![named_target(
            "answer-target",
            "answer_family",
            SourceRole::Primary,
            SourceIdentity::Domain("answer.example.test".to_string()),
        )],
        budget(1, 1),
    );
    let plan = plan(
        &spec,
        vec![query(
            "q-answer",
            AcquisitionTransport::Web,
            QueryMode::Exact,
            &["answer"],
            &["answer-target"],
            1,
        )],
        vec![],
    );
    let contract = validate_research_contract(spec, plan).expect("outcome contract");
    let sources = with_source
        .then(|| {
            let chunks = vec![SourceChunk {
                id: "source-answer:chunk:1".to_string(),
                text: "The primary record establishes the answer.".to_string(),
            }];
            SourceRecord {
                id: "source-answer".to_string(),
                title: "Primary answer".to_string(),
                requested_anchor: "https://answer.example.test/record".to_string(),
                canonical_anchor: "https://answer.example.test/record".to_string(),
                captured_at: "2026-07-21T00:00:00Z".to_string(),
                provenance: vec![SourceProvenance {
                    query_id: "q-answer".to_string(),
                    source_target_id: "answer-target".to_string(),
                }],
                content_digest: source_content_digest(&chunks),
                chunks,
            }
        })
        .into_iter()
        .collect();
    let catalog = SourceCatalog {
        spec_digest: research_spec_digest(&contract.spec),
        attempts: vec![AcquisitionAttempt {
            query_id: "q-answer".to_string(),
            source_target_ids: vec!["answer-target".to_string()],
            outcome,
        }],
        sources,
    };
    (contract, catalog)
}

#[test]
fn completed_requires_claims_only_for_every_material_dimension() {
    let (contract, catalog) = one_dimension_fixture(AcquisitionOutcome::Fetched, true);
    let ledger = admit_claim_ledger(
        &contract,
        &catalog,
        ClaimLedgerProposal {
            claims: vec![ClaimProposal {
                id: "answer-fact".to_string(),
                dimension_id: "answer".to_string(),
                placement: ClaimPlacement::DirectAnswer,
                kind: ClaimKind::Fact,
                text: "The primary record establishes the answer.".to_string(),
                evidence_refs: vec![ClaimEvidenceRef {
                    source_id: "source-answer".to_string(),
                    chunk_ids: vec!["source-answer:chunk:1".to_string()],
                }],
                basis_claim_ids: vec![],
                derivation: None,
            }],
            relations: vec![],
            gaps: vec![],
        },
    )
    .expect("completed ledger");
    let document = build_report_document(&contract, &catalog, &ledger).expect("completed document");

    assert_eq!(
        report_structural_outcome(&document),
        StructuralOutcome::Completed
    );
}

#[test]
fn a_useful_claim_with_a_material_gap_is_qualified_not_completed() {
    let replay = super::frozen_fixture::load_frozen_replays()
        .into_iter()
        .find(|replay| replay.id == "F03")
        .expect("F03 replay");
    let ledger = admit_claim_ledger(&replay.contract, &replay.catalog, replay.proposal)
        .expect("qualified ledger");
    let document = build_report_document(&replay.contract, &replay.catalog, &ledger)
        .expect("qualified document");

    assert_eq!(
        report_structural_outcome(&document),
        StructuralOutcome::Qualified
    );
}

#[test]
fn preserved_sources_without_claims_are_source_backed() {
    let (contract, catalog) = one_dimension_fixture(AcquisitionOutcome::Fetched, true);
    let document =
        build_source_backed_document(&contract, &catalog).expect("source-backed document");

    assert_eq!(
        report_structural_outcome(&document),
        StructuralOutcome::SourceBacked
    );
}

#[test]
fn an_honest_gap_without_claims_or_sources_is_degraded() {
    let (contract, catalog) = one_dimension_fixture(AcquisitionOutcome::NoCandidates, false);
    let ledger = admit_claim_ledger(
        &contract,
        &catalog,
        ClaimLedgerProposal {
            claims: vec![],
            relations: vec![],
            gaps: vec![GapProposal {
                id: "answer-gap".to_string(),
                dimension_id: "answer".to_string(),
                text: "The bounded acquisition did not find a usable source.".to_string(),
                attempted_query_ids: vec!["q-answer".to_string()],
                missing_source_target_ids: vec!["answer-target".to_string()],
            }],
        },
    )
    .expect("degraded ledger");
    let document = build_report_document(&contract, &catalog, &ledger).expect("degraded document");

    assert_eq!(
        report_structural_outcome(&document),
        StructuralOutcome::Degraded
    );
}
