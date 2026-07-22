use super::test_support::*;
use super::*;

#[test]
fn one_valid_provenance_edge_cannot_hide_an_unmatched_source_target() {
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
    let contract = validate_research_contract(spec, plan).expect("two-target contract");
    let chunks = vec![SourceChunk {
        id: "source-alpha:chunk:1".to_string(),
        text: "Alpha evidence only.".to_string(),
    }];
    let catalog = SourceCatalog {
        spec_digest: research_spec_digest(&contract.spec),
        attempts: vec![AcquisitionAttempt {
            query_id: "q-alpha".to_string(),
            source_target_ids: vec!["alpha-target".to_string()],
            outcome: AcquisitionOutcome::Fetched,
        }],
        sources: vec![SourceRecord {
            id: "source-alpha".to_string(),
            title: "Alpha source".to_string(),
            requested_anchor: "https://alpha.example.test/source".to_string(),
            canonical_anchor: "https://alpha.example.test/source".to_string(),
            captured_at: "2026-07-21T00:00:00Z".to_string(),
            provenance: vec![
                SourceProvenance {
                    query_id: "q-alpha".to_string(),
                    source_target_id: "alpha-target".to_string(),
                },
                SourceProvenance {
                    query_id: "q-alpha".to_string(),
                    source_target_id: "beta-target".to_string(),
                },
            ],
            content_digest: source_content_digest(&chunks),
            chunks,
        }],
    };

    assert!(matches!(
        validate_source_catalog(&contract, &catalog),
        Err(CatalogError::QueryTargetMismatch { query_id, target_id })
            if query_id == "q-alpha" && target_id == "beta-target"
    ));
}

#[test]
fn source_provenance_requires_a_matching_fetched_attempt() {
    let spec = spec(
        EvidenceScope::Web,
        vec![dimension("alpha", &["alpha-target"])],
        vec![named_target(
            "alpha-target",
            "alpha_family",
            SourceRole::Primary,
            SourceIdentity::Domain("alpha.example.test".to_string()),
        )],
        budget(1, 1),
    );
    let plan = plan(
        &spec,
        vec![query(
            "q-alpha",
            AcquisitionTransport::Web,
            QueryMode::Exact,
            &["alpha"],
            &["alpha-target"],
            1,
        )],
        vec![],
    );
    let contract = validate_research_contract(spec, plan).expect("attempt contract");
    let chunks = vec![SourceChunk {
        id: "source-alpha:chunk:1".to_string(),
        text: "Alpha evidence only.".to_string(),
    }];
    let catalog = SourceCatalog {
        spec_digest: research_spec_digest(&contract.spec),
        attempts: vec![AcquisitionAttempt {
            query_id: "q-alpha".to_string(),
            source_target_ids: vec!["alpha-target".to_string()],
            outcome: AcquisitionOutcome::Failed {
                reason: "The fetch failed before source admission.".to_string(),
            },
        }],
        sources: vec![SourceRecord {
            id: "source-alpha".to_string(),
            title: "Alpha source".to_string(),
            requested_anchor: "https://alpha.example.test/source".to_string(),
            canonical_anchor: "https://alpha.example.test/source".to_string(),
            captured_at: "2026-07-21T00:00:00Z".to_string(),
            provenance: vec![SourceProvenance {
                query_id: "q-alpha".to_string(),
                source_target_id: "alpha-target".to_string(),
            }],
            content_digest: source_content_digest(&chunks),
            chunks,
        }],
    };

    assert!(matches!(
        validate_source_catalog(&contract, &catalog),
        Err(CatalogError::MissingFetchedAttempt {
            source_id,
            query_id,
            target_id,
        }) if source_id == "source-alpha"
            && query_id == "q-alpha"
            && target_id == "alpha-target"
    ));
}
