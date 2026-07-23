use super::test_support::*;
use super::*;

fn projection_contract(include_bounded_query: bool) -> ResearchContract {
    let spec = spec(
        EvidenceScope::Web,
        vec![
            dimension("answer", &["answer-source"]),
            dimension("partial", &["partial-source"]),
            dimension("bounded", &["bounded-source"]),
        ],
        vec![
            named_target(
                "answer-source",
                "answer_records",
                SourceRole::Primary,
                SourceIdentity::Domain("answer.example.test".to_string()),
            ),
            named_target(
                "partial-source",
                "partial_records",
                SourceRole::Official,
                SourceIdentity::Domain("partial.example.test".to_string()),
            ),
            named_target(
                "bounded-source",
                "bounded_records",
                SourceRole::Independent,
                SourceIdentity::Domain("bounded.example.test".to_string()),
            ),
        ],
        budget(3, 3),
    );
    let mut queries = vec![
        query(
            "q-answer",
            AcquisitionTransport::Web,
            QueryMode::Exact,
            &["answer"],
            &["answer-source"],
            1,
        ),
        query(
            "q-partial",
            AcquisitionTransport::Web,
            QueryMode::Exact,
            &["partial"],
            &["partial-source"],
            1,
        ),
    ];
    let mut planning_gaps = Vec::new();
    if include_bounded_query {
        queries.push(query(
            "q-bounded",
            AcquisitionTransport::Web,
            QueryMode::Exact,
            &["bounded"],
            &["bounded-source"],
            1,
        ));
    } else {
        planning_gaps.push(PlanningGap {
            dimension_id: "bounded".to_string(),
            missing_source_target_ids: vec!["bounded-source".to_string()],
            reason: "The bounded source target did not fit the shared acquisition budget."
                .to_string(),
        });
    }
    let plan = plan(&spec, queries, planning_gaps);
    validate_research_contract(spec, plan).expect("projection contract")
}

fn projection_catalog(contract: &ResearchContract) -> SourceCatalog {
    let mut attempts = vec![
        attempt("q-answer", "answer-source", AcquisitionOutcome::Fetched),
        attempt("q-partial", "partial-source", AcquisitionOutcome::Fetched),
    ];
    if contract.query("q-bounded").is_some() {
        attempts.push(attempt(
            "q-bounded",
            "bounded-source",
            AcquisitionOutcome::NoCandidates,
        ));
    }
    SourceCatalog {
        spec_digest: research_spec_digest(&contract.spec),
        attempts,
        sources: vec![
            source(
                "source-answer",
                "q-answer",
                "answer-source",
                "The primary answer record establishes the observed result.",
            ),
            source(
                "source-partial",
                "q-partial",
                "partial-source",
                "The official partial record establishes a bounded premise.",
            ),
        ],
    }
}

fn attempt(query_id: &str, target_id: &str, outcome: AcquisitionOutcome) -> AcquisitionAttempt {
    AcquisitionAttempt {
        query_id: query_id.to_string(),
        source_target_ids: vec![target_id.to_string()],
        outcome,
    }
}

fn source(id: &str, query_id: &str, target_id: &str, text: &str) -> SourceRecord {
    let chunks = vec![SourceChunk {
        id: format!("{id}:chunk:1"),
        text: text.to_string(),
    }];
    SourceRecord {
        id: id.to_string(),
        title: format!("Source {id}"),
        requested_anchor: format!("https://{target_id}.example.test/{id}"),
        canonical_anchor: format!("https://{target_id}.example.test/{id}"),
        captured_at: "2026-07-21T00:00:00Z".to_string(),
        provenance: vec![SourceProvenance {
            query_id: query_id.to_string(),
            source_target_id: target_id.to_string(),
        }],
        content_digest: source_content_digest(&chunks),
        chunks,
    }
}

fn fact(id: &str, dimension_id: &str, placement: ClaimPlacement, source_id: &str) -> ClaimProposal {
    ClaimProposal {
        id: id.to_string(),
        dimension_id: dimension_id.to_string(),
        placement,
        kind: ClaimKind::Fact,
        text: format!("Factual claim {id}."),
        evidence_refs: vec![ClaimEvidenceRef {
            source_id: source_id.to_string(),
            chunk_ids: vec![format!("{source_id}:chunk:1")],
        }],
        basis_claim_ids: vec![],
        derivation: None,
    }
}

fn gap(id: &str, dimension_id: &str, query_id: &str, target_id: &str) -> GapProposal {
    GapProposal {
        id: id.to_string(),
        dimension_id: dimension_id.to_string(),
        text: format!("The bounded acquisition did not establish all of {dimension_id}."),
        attempted_query_ids: vec![query_id.to_string()],
        missing_source_target_ids: vec![target_id.to_string()],
    }
}

fn admitted_projection_ledger(
    contract: &ResearchContract,
    catalog: &SourceCatalog,
) -> AdmittedClaimLedger {
    admit_claim_ledger(
        contract,
        catalog,
        ClaimLedgerProposal {
            claims: vec![
                fact(
                    "partial-direct",
                    "partial",
                    ClaimPlacement::DirectAnswer,
                    "source-partial",
                ),
                fact(
                    "answer-finding",
                    "answer",
                    ClaimPlacement::Finding,
                    "source-answer",
                ),
            ],
            relations: vec![],
            gaps: vec![
                gap("partial-gap", "partial", "q-partial", "partial-source"),
                gap("bounded-gap", "bounded", "q-bounded", "bounded-source"),
            ],
        },
    )
    .expect("projection ledger")
}

#[test]
fn coverage_distinguishes_complete_partial_and_bounded_dimensions() {
    let contract = projection_contract(true);
    let catalog = projection_catalog(&contract);
    let ledger = admitted_projection_ledger(&contract, &catalog);

    let matrix = derive_coverage(&contract, &ledger);

    assert_eq!(
        matrix.dimension("answer").map(|item| item.structural),
        Some(StructuralCoverage::ClaimsOnly)
    );
    assert_eq!(
        matrix.dimension("partial").map(|item| item.structural),
        Some(StructuralCoverage::ClaimsAndGap)
    );
    assert_eq!(
        matrix.dimension("bounded").map(|item| item.structural),
        Some(StructuralCoverage::GapOnly)
    );
}

#[test]
fn coverage_exposes_missing_dimensions_instead_of_treating_them_as_complete() {
    let contract = projection_contract(true);
    let empty = AdmittedClaimLedger {
        claims: vec![],
        relations: vec![],
        gaps: vec![],
        rejections: vec![],
    };

    let matrix = derive_coverage(&contract, &empty);

    assert!(matrix
        .dimensions
        .iter()
        .all(|item| item.structural == StructuralCoverage::Missing));
}

#[test]
fn report_document_preserves_dimension_order_and_places_every_claim_once() {
    let contract = projection_contract(true);
    let catalog = projection_catalog(&contract);
    let ledger = admitted_projection_ledger(&contract, &catalog);

    let document = build_report_document(&contract, &catalog, &ledger).expect("report document");

    assert_eq!(document.kind, ReportDocumentKind::Claims);
    assert_eq!(document.title, contract.spec.query);
    assert_eq!(document.language, contract.spec.language);
    assert_eq!(
        document
            .dimensions
            .iter()
            .map(|dimension| dimension.dimension_id.as_str())
            .collect::<Vec<_>>(),
        ["answer", "partial", "bounded"]
    );
    assert_eq!(document.direct_answer_claims[0].id, "partial-direct");
    assert_eq!(document.dimensions[0].claims[0].id, "answer-finding");

    let rendered_claim_ids = document
        .direct_answer_claims
        .iter()
        .chain(
            document
                .dimensions
                .iter()
                .flat_map(|dimension| dimension.claims.iter()),
        )
        .map(|claim| claim.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(rendered_claim_ids.len(), ledger.claims.len());
    assert_eq!(
        rendered_claim_ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        ledger.claims.len()
    );

    assert_eq!(document.source_ledger[0].id, "source-partial");
    assert_eq!(document.source_ledger[0].number, 1);
    assert_eq!(document.source_ledger[1].id, "source-answer");
    assert_eq!(document.source_ledger[1].number, 2);
    assert_eq!(document.direct_answer_claims[0].citation_numbers, [1]);
    assert_eq!(document.dimensions[0].claims[0].citation_numbers, [2]);
}

#[test]
fn report_document_retains_contradictions_derivations_and_planning_gaps() {
    let contract = projection_contract(false);
    let mut catalog = projection_catalog(&contract);
    catalog.sources.push(source(
        "source-answer-second",
        "q-answer",
        "answer-source",
        "A second primary record establishes a conflicting observed result.",
    ));
    let ledger = admit_claim_ledger(
        &contract,
        &catalog,
        ClaimLedgerProposal {
            claims: vec![
                fact(
                    "record-a",
                    "answer",
                    ClaimPlacement::Finding,
                    "source-answer",
                ),
                fact(
                    "record-b",
                    "answer",
                    ClaimPlacement::Finding,
                    "source-answer-second",
                ),
                ClaimProposal {
                    id: "derived-decision".to_string(),
                    dimension_id: "answer".to_string(),
                    placement: ClaimPlacement::DirectAnswer,
                    kind: ClaimKind::Inference,
                    text: "The two recorded inputs imply a bounded difference.".to_string(),
                    evidence_refs: vec![],
                    basis_claim_ids: vec!["record-a".to_string(), "record-b".to_string()],
                    derivation: Some(DerivationProposal {
                        method: "Compare the two admitted record values.".to_string(),
                        input_claim_ids: vec!["record-a".to_string(), "record-b".to_string()],
                    }),
                },
            ],
            relations: vec![ClaimRelationProposal {
                id: "record-conflict".to_string(),
                dimension_id: "answer".to_string(),
                kind: ClaimRelationKind::Contradicts,
                claim_ids: ["record-a".to_string(), "record-b".to_string()],
            }],
            gaps: vec![],
        },
    )
    .expect("complex ledger");

    let document = build_report_document(&contract, &catalog, &ledger).expect("complex document");

    assert_eq!(document.dimensions[0].relations[0].id, "record-conflict");
    assert_eq!(
        document.direct_answer_claims[0]
            .derivation
            .as_ref()
            .map(|derivation| derivation.method.as_str()),
        Some("Compare the two admitted record values.")
    );
    assert_eq!(document.direct_answer_claims[0].citation_numbers, [1, 2]);
    assert_eq!(document.dimensions[2].gaps.len(), 1);
    assert_eq!(
        document.dimensions[2].gaps[0].origin,
        ReportGapOrigin::Planning
    );
    assert_eq!(
        document.dimensions[2].gaps[0].text,
        "The bounded source target did not fit the shared acquisition budget."
    );
}

#[test]
fn source_backed_document_preserves_fetched_material_without_synthesis() {
    let contract = projection_contract(false);
    let catalog = projection_catalog(&contract);

    let document =
        build_source_backed_document(&contract, &catalog).expect("source-backed document");

    assert_eq!(document.kind, ReportDocumentKind::SourceBacked);
    assert!(document.direct_answer_claims.is_empty());
    assert!(document
        .dimensions
        .iter()
        .all(|dimension| dimension.coverage == StructuralCoverage::GapOnly));
    assert_eq!(document.dimensions[0].source_ids, ["source-answer"]);
    assert_eq!(document.dimensions[1].source_ids, ["source-partial"]);
    assert!(document.dimensions[2].source_ids.is_empty());
    assert_eq!(
        document.dimensions[2].gaps[0].origin,
        ReportGapOrigin::Planning
    );
    assert_eq!(document.source_ledger.len(), 2);
    assert_eq!(
        document.source_ledger[0].chunks[0].text,
        "The primary answer record establishes the observed result."
    );
    assert_eq!(
        document.dimensions[0].gaps[0].origin,
        ReportGapOrigin::SourceBackedFallback
    );
}
