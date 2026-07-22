use super::{
    derive_coverage, validate_source_catalog, AdmittedClaim, AdmittedClaimLedger,
    AdmittedClaimRelation, AdmittedGap, CatalogError, ClaimEvidenceRef, ClaimKind, ClaimPlacement,
    ClaimRelationKind, DerivationProposal, GapOrigin, ResearchContract, SourceCatalog, SourceChunk,
    SourceRecord, StructuralCoverage,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReportDocumentKind {
    Claims,
    SourceBacked,
    NoEvidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReportGapOrigin {
    ModelProposed,
    Planning,
    HostMissingOutput,
    SourceBackedFallback,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReportDocument {
    pub(super) kind: ReportDocumentKind,
    pub(super) title: String,
    pub(super) language: String,
    pub(super) direct_answer_claims: Vec<ReportClaim>,
    pub(super) dimensions: Vec<ReportDimension>,
    pub(super) source_ledger: Vec<ReportSource>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReportDimension {
    pub(super) dimension_id: String,
    pub(super) heading: String,
    pub(super) material: bool,
    pub(super) coverage: StructuralCoverage,
    pub(super) claims: Vec<ReportClaim>,
    pub(super) relations: Vec<ReportRelation>,
    pub(super) gaps: Vec<ReportGap>,
    pub(super) source_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReportClaim {
    pub(super) id: String,
    pub(super) dimension_id: String,
    pub(super) kind: ClaimKind,
    pub(super) text: String,
    pub(super) evidence_refs: Vec<ClaimEvidenceRef>,
    pub(super) basis_claim_ids: Vec<String>,
    pub(super) derivation: Option<DerivationProposal>,
    pub(super) citation_numbers: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReportRelation {
    pub(super) id: String,
    pub(super) kind: ClaimRelationKind,
    pub(super) claim_ids: [String; 2],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReportGap {
    pub(super) id: String,
    pub(super) text: String,
    pub(super) attempted_query_ids: Vec<String>,
    pub(super) missing_source_target_ids: Vec<String>,
    pub(super) origin: ReportGapOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReportSource {
    pub(super) number: usize,
    pub(super) id: String,
    pub(super) title: String,
    pub(super) requested_anchor: String,
    pub(super) canonical_anchor: String,
    pub(super) captured_at: String,
    pub(super) chunks: Vec<SourceChunk>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum DocumentError {
    #[error("invalid source catalog: {0}")]
    InvalidCatalog(#[from] CatalogError),
    #[error("dimension `{dimension_id}` has no admitted claim or explicit gap")]
    MissingDimensionCoverage { dimension_id: String },
    #[error("claim `{claim_id}` references an unknown basis claim `{basis_id}`")]
    UnknownBasisClaim { claim_id: String, basis_id: String },
    #[error("claim `{claim_id}` references an unknown source `{source_id}`")]
    UnknownClaimSource { claim_id: String, source_id: String },
    #[error("claim basis graph contains a cycle at `{claim_id}`")]
    CyclicClaimBasis { claim_id: String },
    #[error("source-backed document requires at least one fetched source")]
    EmptySourceCatalog,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StructuralOutcome {
    Completed,
    Qualified,
    SourceBacked,
    Degraded,
}

pub(super) fn report_structural_outcome(document: &ReportDocument) -> StructuralOutcome {
    if document.kind == ReportDocumentKind::NoEvidence {
        return StructuralOutcome::Degraded;
    }
    if document.kind == ReportDocumentKind::SourceBacked {
        return if document.source_ledger.is_empty() {
            StructuralOutcome::Degraded
        } else {
            StructuralOutcome::SourceBacked
        };
    }
    let claim_count = document.direct_answer_claims.len()
        + document
            .dimensions
            .iter()
            .map(|dimension| dimension.claims.len())
            .sum::<usize>();
    if claim_count > 0
        && document.dimensions.iter().all(|dimension| {
            !dimension.material || dimension.coverage == StructuralCoverage::ClaimsOnly
        })
    {
        StructuralOutcome::Completed
    } else if claim_count > 0 {
        StructuralOutcome::Qualified
    } else if document.source_ledger.is_empty() {
        StructuralOutcome::Degraded
    } else {
        StructuralOutcome::SourceBacked
    }
}

pub(super) fn build_no_evidence_document(contract: &ResearchContract) -> ReportDocument {
    let chinese = contract.spec.language.eq_ignore_ascii_case("zh")
        || contract.spec.language.starts_with("zh-");
    let gap_text = if chinese {
        "本次运行未能获取可核查的来源，因此无法为该维度给出有证据支持的结论。"
    } else {
        "This run acquired no verifiable source, so no evidence-backed conclusion is available for this dimension."
    };
    ReportDocument {
        kind: ReportDocumentKind::NoEvidence,
        title: contract.spec.query.clone(),
        language: contract.spec.language.clone(),
        direct_answer_claims: Vec::new(),
        dimensions: contract
            .spec
            .dimensions
            .iter()
            .map(|dimension| ReportDimension {
                dimension_id: dimension.id.clone(),
                heading: dimension.question.clone(),
                material: dimension.material,
                coverage: StructuralCoverage::GapOnly,
                claims: Vec::new(),
                relations: Vec::new(),
                gaps: vec![ReportGap {
                    id: deterministic_document_id("no-evidence-gap", &dimension.id),
                    text: gap_text.to_string(),
                    attempted_query_ids: contract
                        .plan
                        .queries
                        .iter()
                        .filter(|query| query.dimension_ids.contains(&dimension.id))
                        .map(|query| query.id.clone())
                        .collect(),
                    missing_source_target_ids: dimension.source_target_ids.clone(),
                    origin: ReportGapOrigin::HostMissingOutput,
                }],
                source_ids: Vec::new(),
            })
            .collect(),
        source_ledger: Vec::new(),
    }
}

pub(super) fn build_report_document(
    contract: &ResearchContract,
    catalog: &SourceCatalog,
    ledger: &AdmittedClaimLedger,
) -> Result<ReportDocument, DocumentError> {
    validate_source_catalog(contract, catalog)?;
    let coverage = derive_coverage(contract, ledger);
    if let Some(missing) = coverage
        .dimensions
        .iter()
        .find(|dimension| dimension.structural == StructuralCoverage::Missing)
    {
        return Err(DocumentError::MissingDimensionCoverage {
            dimension_id: missing.dimension_id.clone(),
        });
    }

    let claims_by_id = ledger
        .claims
        .iter()
        .map(|claim| (claim.id.as_str(), claim))
        .collect::<BTreeMap<_, _>>();
    let sources_by_id = catalog
        .sources
        .iter()
        .map(|source| (source.id.as_str(), source))
        .collect::<BTreeMap<_, _>>();
    let ordered_claims = ordered_claims(contract, ledger);
    let mut claim_sources = BTreeMap::<String, Vec<String>>::new();
    let mut source_order = Vec::new();
    let mut seen_sources = BTreeSet::new();
    for claim in &ordered_claims {
        let mut sources = Vec::new();
        collect_claim_sources(
            claim,
            &claims_by_id,
            &sources_by_id,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            &mut sources,
        )?;
        for source_id in &sources {
            if seen_sources.insert(source_id.clone()) {
                source_order.push(source_id.clone());
            }
        }
        claim_sources.insert(claim.id.clone(), sources);
    }
    for source in &catalog.sources {
        if seen_sources.insert(source.id.clone()) {
            source_order.push(source.id.clone());
        }
    }

    let source_numbers = source_order
        .iter()
        .enumerate()
        .map(|(index, source_id)| (source_id.as_str(), index + 1))
        .collect::<BTreeMap<_, _>>();
    let source_ledger = report_sources(&source_order, &sources_by_id);
    let direct_answer_claims = ledger
        .claims
        .iter()
        .filter(|claim| claim.placement == ClaimPlacement::DirectAnswer)
        .map(|claim| report_claim(claim, &claim_sources, &source_numbers))
        .collect();
    let dimensions = contract
        .spec
        .dimensions
        .iter()
        .map(|dimension| ReportDimension {
            dimension_id: dimension.id.clone(),
            heading: dimension.question.clone(),
            material: dimension.material,
            coverage: coverage
                .dimension(&dimension.id)
                .expect("coverage is derived from every contract dimension")
                .structural,
            claims: ledger
                .claims
                .iter()
                .filter(|claim| {
                    claim.dimension_id == dimension.id && claim.placement == ClaimPlacement::Finding
                })
                .map(|claim| report_claim(claim, &claim_sources, &source_numbers))
                .collect(),
            relations: ledger
                .relations
                .iter()
                .filter(|relation| relation.dimension_id == dimension.id)
                .map(report_relation)
                .collect(),
            gaps: ledger
                .gaps
                .iter()
                .filter(|gap| gap.dimension_id == dimension.id)
                .map(report_gap)
                .collect(),
            source_ids: matching_source_ids(
                dimension.source_target_ids.as_slice(),
                &source_ledger,
                &sources_by_id,
            ),
        })
        .collect();

    Ok(ReportDocument {
        kind: ReportDocumentKind::Claims,
        title: contract.spec.query.clone(),
        language: contract.spec.language.clone(),
        direct_answer_claims,
        dimensions,
        source_ledger,
    })
}

pub(super) fn build_source_backed_document(
    contract: &ResearchContract,
    catalog: &SourceCatalog,
) -> Result<ReportDocument, DocumentError> {
    validate_source_catalog(contract, catalog)?;
    if catalog.sources.is_empty() {
        return Err(DocumentError::EmptySourceCatalog);
    }
    let source_order = catalog
        .sources
        .iter()
        .map(|source| source.id.clone())
        .collect::<Vec<_>>();
    let sources_by_id = catalog
        .sources
        .iter()
        .map(|source| (source.id.as_str(), source))
        .collect::<BTreeMap<_, _>>();
    let source_ledger = report_sources(&source_order, &sources_by_id);
    let attempted_query_ids = catalog
        .attempts
        .iter()
        .map(|attempt| attempt.query_id.as_str())
        .collect::<BTreeSet<_>>();
    let dimensions = contract
        .spec
        .dimensions
        .iter()
        .map(|dimension| {
            let source_ids = matching_source_ids(
                dimension.source_target_ids.as_slice(),
                &source_ledger,
                &sources_by_id,
            );
            let planning_gap = contract
                .plan
                .planning_gaps
                .iter()
                .find(|gap| gap.dimension_id == dimension.id);
            let gap = planning_gap.map_or_else(
                || ReportGap {
                    id: deterministic_document_id("source-backed-gap", &dimension.id),
                    text: "A verified answer is not available for this dimension. The retained source excerpts are provided without additional interpretation."
                        .to_string(),
                    attempted_query_ids: contract
                        .plan
                        .queries
                        .iter()
                        .filter(|query| {
                            query.dimension_ids.contains(&dimension.id)
                                && attempted_query_ids.contains(query.id.as_str())
                        })
                        .map(|query| query.id.clone())
                        .collect(),
                    missing_source_target_ids: dimension
                        .source_target_ids
                        .iter()
                        .filter(|target_id| {
                            !source_ids.iter().any(|source_id| {
                                sources_by_id
                                    .get(source_id.as_str())
                                    .is_some_and(|source| {
                                        source.provenance.iter().any(|edge| {
                                            edge.source_target_id.as_str() == target_id.as_str()
                                        })
                                    })
                            })
                        })
                        .cloned()
                        .collect(),
                    origin: ReportGapOrigin::SourceBackedFallback,
                },
                |gap| ReportGap {
                    id: deterministic_document_id("planning-gap", &dimension.id),
                    text: gap.reason.clone(),
                    attempted_query_ids: vec![],
                    missing_source_target_ids: gap.missing_source_target_ids.clone(),
                    origin: ReportGapOrigin::Planning,
                },
            );
            ReportDimension {
                dimension_id: dimension.id.clone(),
                heading: dimension.question.clone(),
                material: dimension.material,
                coverage: StructuralCoverage::GapOnly,
                claims: vec![],
                relations: vec![],
                gaps: vec![gap],
                source_ids,
            }
        })
        .collect();

    Ok(ReportDocument {
        kind: ReportDocumentKind::SourceBacked,
        title: contract.spec.query.clone(),
        language: contract.spec.language.clone(),
        direct_answer_claims: vec![],
        dimensions,
        source_ledger,
    })
}

fn ordered_claims<'a>(
    contract: &ResearchContract,
    ledger: &'a AdmittedClaimLedger,
) -> Vec<&'a AdmittedClaim> {
    let mut claims = ledger
        .claims
        .iter()
        .filter(|claim| claim.placement == ClaimPlacement::DirectAnswer)
        .collect::<Vec<_>>();
    for dimension in &contract.spec.dimensions {
        claims.extend(ledger.claims.iter().filter(|claim| {
            claim.dimension_id == dimension.id && claim.placement == ClaimPlacement::Finding
        }));
    }
    claims
}

fn collect_claim_sources(
    claim: &AdmittedClaim,
    claims_by_id: &BTreeMap<&str, &AdmittedClaim>,
    sources_by_id: &BTreeMap<&str, &SourceRecord>,
    visiting: &mut BTreeSet<String>,
    seen_sources: &mut BTreeSet<String>,
    output: &mut Vec<String>,
) -> Result<(), DocumentError> {
    if !visiting.insert(claim.id.clone()) {
        return Err(DocumentError::CyclicClaimBasis {
            claim_id: claim.id.clone(),
        });
    }
    for evidence in &claim.evidence_refs {
        if !sources_by_id.contains_key(evidence.source_id.as_str()) {
            return Err(DocumentError::UnknownClaimSource {
                claim_id: claim.id.clone(),
                source_id: evidence.source_id.clone(),
            });
        }
        if seen_sources.insert(evidence.source_id.clone()) {
            output.push(evidence.source_id.clone());
        }
    }
    for basis_id in &claim.basis_claim_ids {
        let Some(basis) = claims_by_id.get(basis_id.as_str()).copied() else {
            return Err(DocumentError::UnknownBasisClaim {
                claim_id: claim.id.clone(),
                basis_id: basis_id.clone(),
            });
        };
        collect_claim_sources(
            basis,
            claims_by_id,
            sources_by_id,
            visiting,
            seen_sources,
            output,
        )?;
    }
    visiting.remove(&claim.id);
    Ok(())
}

fn report_claim(
    claim: &AdmittedClaim,
    claim_sources: &BTreeMap<String, Vec<String>>,
    source_numbers: &BTreeMap<&str, usize>,
) -> ReportClaim {
    let mut citation_numbers = claim_sources
        .get(&claim.id)
        .into_iter()
        .flatten()
        .filter_map(|source_id| source_numbers.get(source_id.as_str()).copied())
        .collect::<Vec<_>>();
    citation_numbers.sort_unstable();
    citation_numbers.dedup();
    ReportClaim {
        id: claim.id.clone(),
        dimension_id: claim.dimension_id.clone(),
        kind: claim.kind,
        text: claim.text.clone(),
        evidence_refs: claim.evidence_refs.clone(),
        basis_claim_ids: claim.basis_claim_ids.clone(),
        derivation: claim.derivation.clone(),
        citation_numbers,
    }
}

fn report_relation(relation: &AdmittedClaimRelation) -> ReportRelation {
    ReportRelation {
        id: relation.id.clone(),
        kind: relation.kind,
        claim_ids: relation.claim_ids.clone(),
    }
}

fn report_gap(gap: &AdmittedGap) -> ReportGap {
    ReportGap {
        id: gap.id.clone(),
        text: match gap.origin {
            GapOrigin::HostMissingOutput => "This run did not produce a verified answer for this dimension; relevant retained sources remain listed below."
                .to_string(),
            GapOrigin::ModelProposed | GapOrigin::Planning => gap.text.clone(),
        },
        attempted_query_ids: gap.attempted_query_ids.clone(),
        missing_source_target_ids: gap.missing_source_target_ids.clone(),
        origin: match gap.origin {
            GapOrigin::ModelProposed => ReportGapOrigin::ModelProposed,
            GapOrigin::Planning => ReportGapOrigin::Planning,
            GapOrigin::HostMissingOutput => ReportGapOrigin::HostMissingOutput,
        },
    }
}

fn report_sources(
    source_order: &[String],
    sources_by_id: &BTreeMap<&str, &SourceRecord>,
) -> Vec<ReportSource> {
    source_order
        .iter()
        .enumerate()
        .filter_map(|(index, source_id)| {
            sources_by_id
                .get(source_id.as_str())
                .copied()
                .map(|source| ReportSource {
                    number: index + 1,
                    id: source.id.clone(),
                    title: source.title.clone(),
                    requested_anchor: source.requested_anchor.clone(),
                    canonical_anchor: source.canonical_anchor.clone(),
                    captured_at: source.captured_at.clone(),
                    chunks: source.chunks.clone(),
                })
        })
        .collect()
}

fn matching_source_ids(
    target_ids: &[String],
    source_ledger: &[ReportSource],
    sources_by_id: &BTreeMap<&str, &SourceRecord>,
) -> Vec<String> {
    source_ledger
        .iter()
        .filter(|report_source| {
            sources_by_id
                .get(report_source.id.as_str())
                .is_some_and(|source| {
                    source
                        .provenance
                        .iter()
                        .any(|edge| target_ids.contains(&edge.source_target_id))
                })
        })
        .map(|source| source.id.clone())
        .collect()
}

fn deterministic_document_id(prefix: &str, dimension_id: &str) -> String {
    let digest = Sha256::digest(format!("{prefix}\0{dimension_id}").as_bytes());
    format!("{prefix}-{:x}", digest)[..prefix.len() + 1 + 16].to_string()
}
