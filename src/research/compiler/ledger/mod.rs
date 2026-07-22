use super::CatalogError;
use serde::{Deserialize, Serialize};

mod admission;

pub(super) use admission::admit_claim_ledger;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ClaimPlacement {
    DirectAnswer,
    Finding,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ClaimKind {
    Fact,
    Inference,
    Recommendation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClaimEvidenceRef {
    pub(super) source_id: String,
    pub(super) chunk_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DerivationProposal {
    pub(super) method: String,
    pub(super) input_claim_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClaimProposal {
    pub(super) id: String,
    pub(super) dimension_id: String,
    pub(super) placement: ClaimPlacement,
    pub(super) kind: ClaimKind,
    pub(super) text: String,
    pub(super) evidence_refs: Vec<ClaimEvidenceRef>,
    pub(super) basis_claim_ids: Vec<String>,
    pub(super) derivation: Option<DerivationProposal>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ClaimRelationKind {
    Contradicts,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClaimRelationProposal {
    pub(super) id: String,
    pub(super) dimension_id: String,
    pub(super) kind: ClaimRelationKind,
    pub(super) claim_ids: [String; 2],
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GapProposal {
    pub(super) id: String,
    pub(super) dimension_id: String,
    pub(super) text: String,
    pub(super) attempted_query_ids: Vec<String>,
    pub(super) missing_source_target_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClaimLedgerProposal {
    pub(super) claims: Vec<ClaimProposal>,
    pub(super) relations: Vec<ClaimRelationProposal>,
    pub(super) gaps: Vec<GapProposal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AdmittedClaim {
    pub(super) id: String,
    pub(super) dimension_id: String,
    pub(super) placement: ClaimPlacement,
    pub(super) kind: ClaimKind,
    pub(super) text: String,
    pub(super) evidence_refs: Vec<ClaimEvidenceRef>,
    pub(super) basis_claim_ids: Vec<String>,
    pub(super) derivation: Option<DerivationProposal>,
}

impl From<ClaimProposal> for AdmittedClaim {
    fn from(claim: ClaimProposal) -> Self {
        Self {
            id: claim.id,
            dimension_id: claim.dimension_id,
            placement: claim.placement,
            kind: claim.kind,
            text: claim.text,
            evidence_refs: claim.evidence_refs,
            basis_claim_ids: claim.basis_claim_ids,
            derivation: claim.derivation,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AdmittedClaimRelation {
    pub(super) id: String,
    pub(super) dimension_id: String,
    pub(super) kind: ClaimRelationKind,
    pub(super) claim_ids: [String; 2],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GapOrigin {
    ModelProposed,
    Planning,
    HostMissingOutput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AdmittedGap {
    pub(super) id: String,
    pub(super) dimension_id: String,
    pub(super) text: String,
    pub(super) attempted_query_ids: Vec<String>,
    pub(super) missing_source_target_ids: Vec<String>,
    pub(super) origin: GapOrigin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RejectionReason {
    InvalidIdentity,
    DuplicateIdentity,
    UnknownDimension,
    InvalidText,
    InvalidClaimShape,
    InvalidEvidenceReference,
    EvidenceOutsideDimensionTargets,
    InvalidBasis,
    UnresolvableBasisGraph,
    InvalidDerivation,
    InvalidRelation,
    InvalidGapProvenance,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LedgerRejection {
    pub(super) item_id: String,
    pub(super) reason: RejectionReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AdmittedClaimLedger {
    pub(super) claims: Vec<AdmittedClaim>,
    pub(super) relations: Vec<AdmittedClaimRelation>,
    pub(super) gaps: Vec<AdmittedGap>,
    pub(super) rejections: Vec<LedgerRejection>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum LedgerAdmissionError {
    #[error("invalid source catalog: {0}")]
    InvalidCatalog(#[from] CatalogError),
}
