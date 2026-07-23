use super::*;
use crate::research::compiler::{
    stable_id, valid_text, ResearchContract, SourceCatalog, SourceRecord,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const MAX_CLAIM_TEXT_CHARS: usize = 4_000;
const MAX_GAP_TEXT_CHARS: usize = 2_000;
const MAX_DERIVATION_CHARS: usize = 1_000;

pub(in crate::research::compiler) fn admit_claim_ledger(
    contract: &ResearchContract,
    catalog: &SourceCatalog,
    proposal: ClaimLedgerProposal,
) -> Result<AdmittedClaimLedger, LedgerAdmissionError> {
    crate::research::compiler::validate_source_catalog(contract, catalog)?;

    let sources = catalog
        .sources
        .iter()
        .map(|source| (source.id.as_str(), source))
        .collect::<BTreeMap<_, _>>();
    let claim_id_counts = identity_counts(proposal.claims.iter().map(|claim| claim.id.as_str()));
    let proposed_claim_kinds = proposal
        .claims
        .iter()
        .filter(|claim| claim_id_counts.get(claim.id.as_str()) == Some(&1))
        .map(|claim| (claim.id.as_str(), claim.kind))
        .collect::<BTreeMap<_, _>>();
    let mut admitted_by_id = BTreeMap::<String, AdmittedClaim>::new();
    let mut rejections = Vec::new();
    let mut pending = Vec::new();

    for claim in proposal.claims.iter().cloned() {
        if !stable_id(&claim.id) {
            reject(&mut rejections, &claim.id, RejectionReason::InvalidIdentity);
            continue;
        }
        if claim_id_counts.get(claim.id.as_str()) != Some(&1) {
            reject(
                &mut rejections,
                &claim.id,
                RejectionReason::DuplicateIdentity,
            );
            continue;
        }
        if let Err(reason) = validate_claim_common(contract, &sources, &claim) {
            reject(&mut rejections, &claim.id, reason);
            continue;
        }
        if claim.kind == ClaimKind::Fact {
            if !claim.basis_claim_ids.is_empty()
                || claim.derivation.is_some()
                || claim.evidence_refs.is_empty()
            {
                reject(
                    &mut rejections,
                    &claim.id,
                    RejectionReason::InvalidClaimShape,
                );
                continue;
            }
            admitted_by_id.insert(claim.id.clone(), claim.into());
        } else {
            if let Err(reason) = validate_dependent_claim_shape(&claim, &proposed_claim_kinds) {
                reject(&mut rejections, &claim.id, reason);
                continue;
            }
            pending.push(claim);
        }
    }

    while !pending.is_empty() {
        let mut made_progress = false;
        let mut unresolved = Vec::new();
        for claim in pending {
            if claim
                .basis_claim_ids
                .iter()
                .all(|basis_id| admitted_by_id.contains_key(basis_id))
            {
                let rejection = if !basis_kinds_are_valid(&claim, &admitted_by_id)
                    || !derivation_inputs_are_admitted(&claim, &admitted_by_id)
                {
                    Some(RejectionReason::InvalidBasis)
                } else if !basis_evidence_is_within_claim_dimension(
                    &claim,
                    contract,
                    &admitted_by_id,
                    &sources,
                ) {
                    Some(RejectionReason::EvidenceOutsideDimensionTargets)
                } else {
                    None
                };
                if let Some(reason) = rejection {
                    reject(&mut rejections, &claim.id, reason);
                } else {
                    admitted_by_id.insert(claim.id.clone(), claim.into());
                }
                made_progress = true;
            } else {
                unresolved.push(claim);
            }
        }
        if !made_progress {
            for claim in unresolved {
                reject(
                    &mut rejections,
                    &claim.id,
                    RejectionReason::UnresolvableBasisGraph,
                );
            }
            break;
        }
        pending = unresolved;
    }

    let claims = proposal
        .claims
        .iter()
        .filter_map(|claim| admitted_by_id.remove(&claim.id))
        .collect::<Vec<_>>();
    let admitted_claims = claims
        .iter()
        .map(|claim| (claim.id.as_str(), claim))
        .collect::<BTreeMap<_, _>>();
    let relations = admit_relations(
        contract,
        proposal.relations,
        &admitted_claims,
        &mut rejections,
    );
    let mut gaps = admit_gaps(contract, catalog, proposal.gaps, &mut rejections);
    insert_host_gaps(contract, &claims, &mut gaps);

    Ok(AdmittedClaimLedger {
        claims,
        relations,
        gaps,
        rejections,
    })
}

fn validate_claim_common(
    contract: &ResearchContract,
    sources: &BTreeMap<&str, &SourceRecord>,
    claim: &ClaimProposal,
) -> Result<(), RejectionReason> {
    let Some(dimension) = contract.dimension(&claim.dimension_id) else {
        return Err(RejectionReason::UnknownDimension);
    };
    if !valid_text(&claim.text, MAX_CLAIM_TEXT_CHARS)
        || has_duplicates(&claim.basis_claim_ids)
        || has_duplicate_evidence_sources(&claim.evidence_refs)
    {
        return Err(RejectionReason::InvalidText);
    }
    for evidence_ref in &claim.evidence_refs {
        let Some(source) = sources.get(evidence_ref.source_id.as_str()).copied() else {
            return Err(RejectionReason::InvalidEvidenceReference);
        };
        if evidence_ref.chunk_ids.is_empty() || has_duplicates(&evidence_ref.chunk_ids) {
            return Err(RejectionReason::InvalidEvidenceReference);
        }
        if !evidence_ref.chunk_ids.iter().all(|chunk_id| {
            source
                .chunks
                .iter()
                .any(|chunk| chunk.id.as_str() == chunk_id)
        }) {
            return Err(RejectionReason::InvalidEvidenceReference);
        }
        if !source
            .provenance
            .iter()
            .any(|edge| dimension.source_target_ids.contains(&edge.source_target_id))
        {
            return Err(RejectionReason::EvidenceOutsideDimensionTargets);
        }
    }
    Ok(())
}

fn validate_dependent_claim_shape(
    claim: &ClaimProposal,
    proposed_claim_kinds: &BTreeMap<&str, ClaimKind>,
) -> Result<(), RejectionReason> {
    if claim.basis_claim_ids.is_empty() {
        return Err(RejectionReason::InvalidBasis);
    }
    if claim
        .basis_claim_ids
        .iter()
        .any(|basis_id| !proposed_claim_kinds.contains_key(basis_id.as_str()))
    {
        return Err(RejectionReason::InvalidBasis);
    }
    match (&claim.kind, &claim.derivation) {
        (ClaimKind::Inference, Some(derivation)) => {
            if !valid_text(&derivation.method, MAX_DERIVATION_CHARS)
                || derivation.input_claim_ids.is_empty()
                || has_duplicates(&derivation.input_claim_ids)
                || derivation
                    .input_claim_ids
                    .iter()
                    .any(|id| !claim.basis_claim_ids.contains(id))
            {
                return Err(RejectionReason::InvalidDerivation);
            }
        }
        (ClaimKind::Recommendation, None) | (ClaimKind::Inference, None) => {}
        (ClaimKind::Recommendation, Some(_)) | (ClaimKind::Fact, _) => {
            return Err(RejectionReason::InvalidDerivation)
        }
    }
    Ok(())
}

fn basis_kinds_are_valid(
    claim: &ClaimProposal,
    admitted_by_id: &BTreeMap<String, AdmittedClaim>,
) -> bool {
    claim.basis_claim_ids.iter().all(|basis_id| {
        admitted_by_id
            .get(basis_id)
            .is_some_and(|basis| match claim.kind {
                ClaimKind::Fact => false,
                ClaimKind::Inference | ClaimKind::Recommendation => {
                    matches!(basis.kind, ClaimKind::Fact | ClaimKind::Inference)
                }
            })
    })
}

fn derivation_inputs_are_admitted(
    claim: &ClaimProposal,
    admitted_by_id: &BTreeMap<String, AdmittedClaim>,
) -> bool {
    claim.derivation.as_ref().is_none_or(|derivation| {
        derivation
            .input_claim_ids
            .iter()
            .all(|input_id| admitted_by_id.contains_key(input_id))
    })
}

fn basis_evidence_is_within_claim_dimension(
    claim: &ClaimProposal,
    contract: &ResearchContract,
    admitted_by_id: &BTreeMap<String, AdmittedClaim>,
    sources: &BTreeMap<&str, &SourceRecord>,
) -> bool {
    let Some(dimension) = contract.dimension(&claim.dimension_id) else {
        return false;
    };
    claim.basis_claim_ids.iter().all(|basis_id| {
        admitted_by_id.get(basis_id).is_some_and(|basis| {
            admitted_claim_evidence_is_within_targets(
                basis,
                &dimension.source_target_ids,
                admitted_by_id,
                sources,
                &mut BTreeSet::new(),
            )
        })
    })
}

fn admitted_claim_evidence_is_within_targets(
    claim: &AdmittedClaim,
    target_ids: &[String],
    admitted_by_id: &BTreeMap<String, AdmittedClaim>,
    sources: &BTreeMap<&str, &SourceRecord>,
    visiting: &mut BTreeSet<String>,
) -> bool {
    if !visiting.insert(claim.id.clone()) {
        return false;
    }
    let direct_evidence_is_valid = claim.evidence_refs.iter().all(|evidence| {
        sources
            .get(evidence.source_id.as_str())
            .is_some_and(|source| {
                source
                    .provenance
                    .iter()
                    .any(|edge| target_ids.contains(&edge.source_target_id))
            })
    });
    let basis_evidence_is_valid = claim.basis_claim_ids.iter().all(|basis_id| {
        admitted_by_id.get(basis_id).is_some_and(|basis| {
            admitted_claim_evidence_is_within_targets(
                basis,
                target_ids,
                admitted_by_id,
                sources,
                visiting,
            )
        })
    });
    visiting.remove(&claim.id);
    direct_evidence_is_valid && basis_evidence_is_valid
}

fn admit_relations(
    contract: &ResearchContract,
    proposals: Vec<ClaimRelationProposal>,
    claims: &BTreeMap<&str, &AdmittedClaim>,
    rejections: &mut Vec<LedgerRejection>,
) -> Vec<AdmittedClaimRelation> {
    let counts = identity_counts(proposals.iter().map(|relation| relation.id.as_str()));
    proposals
        .into_iter()
        .filter_map(|relation| {
            let valid_id = stable_id(&relation.id) && counts.get(relation.id.as_str()) == Some(&1);
            let valid_dimension = contract.dimension(&relation.dimension_id).is_some();
            let [left_id, right_id] = &relation.claim_ids;
            let claims_are_distinct = left_id != right_id;
            let endpoints = claims
                .get(left_id.as_str())
                .zip(claims.get(right_id.as_str()));
            let endpoints_are_valid = endpoints.is_some_and(|(left, right)| {
                left.dimension_id == relation.dimension_id
                    && right.dimension_id == relation.dimension_id
                    && left.kind == ClaimKind::Fact
                    && right.kind == ClaimKind::Fact
            });
            if !valid_id || !valid_dimension || !claims_are_distinct || !endpoints_are_valid {
                reject(
                    rejections,
                    &relation.id,
                    if valid_id {
                        RejectionReason::InvalidRelation
                    } else {
                        RejectionReason::InvalidIdentity
                    },
                );
                return None;
            }
            Some(AdmittedClaimRelation {
                id: relation.id,
                dimension_id: relation.dimension_id,
                kind: relation.kind,
                claim_ids: relation.claim_ids,
            })
        })
        .collect()
}

fn admit_gaps(
    contract: &ResearchContract,
    catalog: &SourceCatalog,
    proposals: Vec<GapProposal>,
    rejections: &mut Vec<LedgerRejection>,
) -> Vec<AdmittedGap> {
    let counts = identity_counts(proposals.iter().map(|gap| gap.id.as_str()));
    let attempted_query_ids = catalog
        .attempts
        .iter()
        .map(|attempt| attempt.query_id.as_str())
        .collect::<BTreeSet<_>>();
    let attempt_edges = catalog
        .attempts
        .iter()
        .flat_map(|attempt| {
            attempt
                .source_target_ids
                .iter()
                .map(move |target_id| (attempt.query_id.as_str(), target_id.as_str()))
        })
        .collect::<BTreeSet<_>>();
    proposals
        .into_iter()
        .filter_map(|gap| {
            let dimension = contract.dimension(&gap.dimension_id);
            let valid = stable_id(&gap.id)
                && counts.get(gap.id.as_str()) == Some(&1)
                && valid_text(&gap.text, MAX_GAP_TEXT_CHARS)
                && !gap.attempted_query_ids.is_empty()
                && !has_duplicates(&gap.attempted_query_ids)
                && !has_duplicates(&gap.missing_source_target_ids)
                && dimension.is_some_and(|dimension| {
                    gap.attempted_query_ids.iter().all(|query_id| {
                        contract.query(query_id).is_some_and(|query| {
                            query.dimension_ids.contains(&dimension.id)
                                && attempted_query_ids.contains(query_id.as_str())
                        })
                    }) && gap.missing_source_target_ids.iter().all(|target_id| {
                        dimension.source_target_ids.contains(target_id)
                            && gap.attempted_query_ids.iter().any(|query_id| {
                                attempt_edges.contains(&(query_id.as_str(), target_id.as_str()))
                            })
                    })
                });
            if !valid {
                reject(rejections, &gap.id, RejectionReason::InvalidGapProvenance);
                return None;
            }
            Some(AdmittedGap {
                id: gap.id,
                dimension_id: gap.dimension_id,
                text: gap.text,
                attempted_query_ids: gap.attempted_query_ids,
                missing_source_target_ids: gap.missing_source_target_ids,
                origin: GapOrigin::ModelProposed,
            })
        })
        .collect()
}

fn insert_host_gaps(
    contract: &ResearchContract,
    claims: &[AdmittedClaim],
    gaps: &mut Vec<AdmittedGap>,
) {
    for planning_gap in &contract.plan.planning_gaps {
        gaps.push(AdmittedGap {
            id: deterministic_host_id("planning-gap", &planning_gap.dimension_id),
            dimension_id: planning_gap.dimension_id.clone(),
            text: planning_gap.reason.clone(),
            attempted_query_ids: Vec::new(),
            missing_source_target_ids: planning_gap.missing_source_target_ids.clone(),
            origin: GapOrigin::Planning,
        });
    }
    for dimension in &contract.spec.dimensions {
        let has_claim = claims
            .iter()
            .any(|claim| claim.dimension_id == dimension.id);
        let has_gap = gaps.iter().any(|gap| gap.dimension_id == dimension.id);
        if !has_claim && !has_gap {
            gaps.push(AdmittedGap {
                id: deterministic_host_id("missing-output-gap", &dimension.id),
                dimension_id: dimension.id.clone(),
                text: "No admissible claim or model-proposed gap was returned for this dimension."
                    .to_string(),
                attempted_query_ids: contract
                    .plan
                    .queries
                    .iter()
                    .filter(|query| query.dimension_ids.contains(&dimension.id))
                    .map(|query| query.id.clone())
                    .collect(),
                missing_source_target_ids: dimension.source_target_ids.clone(),
                origin: GapOrigin::HostMissingOutput,
            });
        }
    }
}

fn deterministic_host_id(prefix: &str, value: &str) -> String {
    let digest = Sha256::digest(format!("{prefix}\0{value}").as_bytes());
    format!("{prefix}-{:x}", digest)[..prefix.len() + 1 + 16].to_string()
}

fn identity_counts<'a>(ids: impl Iterator<Item = &'a str>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for id in ids {
        *counts.entry(id.to_string()).or_insert(0) += 1;
    }
    counts
}

fn has_duplicates(values: &[String]) -> bool {
    let mut seen = BTreeSet::new();
    values.iter().any(|value| !seen.insert(value))
}

fn has_duplicate_evidence_sources(references: &[ClaimEvidenceRef]) -> bool {
    let mut seen = BTreeSet::new();
    references
        .iter()
        .any(|reference| !seen.insert(reference.source_id.as_str()))
}

fn reject(rejections: &mut Vec<LedgerRejection>, item_id: &str, reason: RejectionReason) {
    rejections.push(LedgerRejection {
        item_id: item_id.to_string(),
        reason,
    });
}
