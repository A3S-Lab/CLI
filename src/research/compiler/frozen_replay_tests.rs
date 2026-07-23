use super::frozen_fixture::{load_frozen_replays, FrozenReplay};
use super::*;

fn replay(case_id: &str) -> FrozenReplay {
    load_frozen_replays()
        .into_iter()
        .find(|replay| replay.id == case_id)
        .unwrap_or_else(|| panic!("missing frozen replay `{case_id}`"))
}

fn admitted(replay: &FrozenReplay) -> AdmittedClaimLedger {
    admit_claim_ledger(&replay.contract, &replay.catalog, replay.proposal.clone())
        .unwrap_or_else(|error| panic!("{}: admit frozen ledger: {error}", replay.id))
}

fn document_claims(document: &ReportDocument) -> impl Iterator<Item = &ReportClaim> {
    document.direct_answer_claims.iter().chain(
        document
            .dimensions
            .iter()
            .flat_map(|dimension| dimension.claims.iter()),
    )
}

fn assert_forbidden_statements_absent(replay: &FrozenReplay, document: &ReportDocument) {
    let published_text = document_claims(document)
        .map(|claim| claim.text.as_str())
        .chain(
            document
                .dimensions
                .iter()
                .flat_map(|dimension| dimension.gaps.iter().map(|gap| gap.text.as_str())),
        )
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in &replay.forbidden_statements {
        assert!(
            !published_text.contains(forbidden),
            "{}: forbidden statement reached the document: {forbidden}",
            replay.id
        );
    }
}

#[test]
fn all_frozen_cases_compile_to_closed_contracts_and_catalogs() {
    let replays = load_frozen_replays();

    assert_eq!(
        replays
            .iter()
            .map(|replay| replay.id.as_str())
            .collect::<Vec<_>>(),
        ["F01", "F02", "F03", "F04", "F05", "F06", "F07", "F08"]
    );
    for replay in replays {
        assert!(!replay.required_behaviors.is_empty(), "{}", replay.id);
        validate_source_catalog(&replay.contract, &replay.catalog)
            .unwrap_or_else(|error| panic!("{}: invalid frozen catalog: {error}", replay.id));
    }
}

#[test]
fn f01_preserves_both_sides_of_the_contradiction_with_separate_citations() {
    let replay = replay("F01");
    let ledger = admitted(&replay);
    let document =
        build_report_document(&replay.contract, &replay.catalog, &ledger).expect("F01 document");

    assert_eq!(ledger.claims.len(), 2);
    assert_eq!(ledger.relations.len(), 1);
    assert_eq!(ledger.relations[0].kind, ClaimRelationKind::Contradicts);
    assert_eq!(document.direct_answer_claims[0].citation_numbers, [1]);
    assert_eq!(document.direct_answer_claims[1].citation_numbers, [2]);
    assert_eq!(document.dimensions[0].relations.len(), 1);
    assert_forbidden_statements_absent(&replay, &document);
}

#[test]
fn f02_keeps_the_reproducible_derivation_and_benchmark_boundary() {
    let replay = replay("F02");
    let ledger = admitted(&replay);
    let document =
        build_report_document(&replay.contract, &replay.catalog, &ledger).expect("F02 document");

    let derived = document_claims(&document)
        .find(|claim| claim.id == "derived-reduction")
        .expect("F02 derived claim");
    assert_eq!(derived.kind, ClaimKind::Inference);
    assert_eq!(derived.basis_claim_ids, ["observed-latencies"]);
    assert_eq!(
        derived
            .derivation
            .as_ref()
            .map(|derivation| derivation.method.as_str()),
        Some("(100 - 80) / 100 * 100")
    );
    assert_eq!(derived.citation_numbers, [1]);
    assert!(document_claims(&document).any(|claim| claim.id == "throughput-boundary"));
    assert_forbidden_statements_absent(&replay, &document);
}

#[test]
fn f03_rejects_only_the_malformed_beta_claim_and_retains_alpha() {
    let replay = replay("F03");
    assert_eq!(replay.fault_stage.as_deref(), Some("evidence_extraction"));
    assert_eq!(
        replay.fault_mode.as_deref(),
        Some("malformed_target_result")
    );
    let ledger = admitted(&replay);
    let document =
        build_report_document(&replay.contract, &replay.catalog, &ledger).expect("F03 document");

    assert!(ledger.claims.iter().any(|claim| claim.id == "alpha-window"));
    assert!(!ledger.claims.iter().any(|claim| claim.id == "beta-window"));
    assert!(ledger
        .rejections
        .iter()
        .any(|rejection| rejection.item_id == "beta-window"));
    assert_eq!(
        document.dimensions[0].coverage,
        StructuralCoverage::ClaimsOnly
    );
    assert_eq!(document.dimensions[1].coverage, StructuralCoverage::GapOnly);
    assert_eq!(
        document.dimensions[1].gaps[0].origin,
        ReportGapOrigin::HostMissingOutput
    );
    assert_forbidden_statements_absent(&replay, &document);
}

#[test]
fn f04_keeps_chinese_reader_prose_over_english_source_material() {
    let replay = replay("F04");
    let ledger = admitted(&replay);
    let document =
        build_report_document(&replay.contract, &replay.catalog, &ledger).expect("F04 document");

    assert_eq!(document.language, "zh");
    assert!(document_claims(&document).all(|claim| claim
        .text
        .chars()
        .any(|character| ('\u{4e00}'..='\u{9fff}').contains(&character))));
    assert!(document.source_ledger[0].chunks[0]
        .text
        .contains("supports Linux and macOS"));
}

#[test]
fn f05_keeps_prompt_injection_as_source_data_not_a_document_claim() {
    let replay = replay("F05");
    let ledger = admitted(&replay);
    let document =
        build_report_document(&replay.contract, &replay.catalog, &ledger).expect("F05 document");

    assert!(document.source_ledger[0].chunks[0]
        .text
        .contains("SYSTEM INSTRUCTION"));
    assert_eq!(document_claims(&document).count(), 1);
    assert!(document_claims(&document)
        .all(|claim| !claim.text.to_ascii_lowercase().contains("unbreakable")));
    assert_forbidden_statements_absent(&replay, &document);
}

#[test]
fn f06_report_timeout_selects_a_source_backed_document_without_evidence_loss() {
    let replay = replay("F06");
    assert_eq!(replay.fault_stage.as_deref(), Some("report_generation"));
    assert_eq!(replay.fault_mode.as_deref(), Some("timeout"));

    let document =
        build_source_backed_document(&replay.contract, &replay.catalog).expect("F06 fallback");

    assert_eq!(document.kind, ReportDocumentKind::SourceBacked);
    assert!(document.source_ledger[0].chunks[0]
        .text
        .contains("30 September 2027"));
    assert_eq!(document.dimensions[0].source_ids, ["maintenance-policy"]);
    assert_eq!(
        document.dimensions[0].gaps[0].origin,
        ReportGapOrigin::SourceBackedFallback
    );
}

#[test]
fn f07_keeps_one_canonical_source_and_its_original_request_anchor() {
    let replay = replay("F07");
    let ledger = admitted(&replay);
    let document =
        build_report_document(&replay.contract, &replay.catalog, &ledger).expect("F07 document");

    assert_eq!(document.source_ledger.len(), 1);
    assert_eq!(
        document.source_ledger[0].requested_anchor,
        "https://legacy.example.test/docs/latest"
    );
    assert_eq!(
        document.source_ledger[0].canonical_anchor,
        "https://docs.example.test/juniper/4/support"
    );
    assert_forbidden_statements_absent(&replay, &document);
}

#[test]
fn f08_expands_recommendation_citations_from_both_admitted_premises() {
    let replay = replay("F08");
    assert_eq!(
        replay.contract.spec.evidence_scope,
        EvidenceScope::WebAndWorkspace
    );
    let ledger = admitted(&replay);
    let document =
        build_report_document(&replay.contract, &replay.catalog, &ledger).expect("F08 document");

    let advice = document_claims(&document)
        .find(|claim| claim.id == "bounded-advice")
        .expect("F08 recommendation");
    assert_eq!(advice.kind, ClaimKind::Recommendation);
    assert_eq!(advice.basis_claim_ids, ["cedar-msrv", "team-pin"]);
    assert_eq!(advice.citation_numbers, [1, 2]);
    assert_forbidden_statements_absent(&replay, &document);
}
