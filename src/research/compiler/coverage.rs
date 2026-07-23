use super::{AdmittedClaimLedger, ResearchContract};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StructuralCoverage {
    ClaimsOnly,
    ClaimsAndGap,
    GapOnly,
    Missing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DimensionCoverage {
    pub(super) dimension_id: String,
    pub(super) material: bool,
    pub(super) structural: StructuralCoverage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CoverageMatrix {
    pub(super) dimensions: Vec<DimensionCoverage>,
}

impl CoverageMatrix {
    pub(super) fn dimension(&self, id: &str) -> Option<&DimensionCoverage> {
        self.dimensions
            .iter()
            .find(|dimension| dimension.dimension_id == id)
    }
}

pub(super) fn derive_coverage(
    contract: &ResearchContract,
    ledger: &AdmittedClaimLedger,
) -> CoverageMatrix {
    CoverageMatrix {
        dimensions: contract
            .spec
            .dimensions
            .iter()
            .map(|dimension| {
                let has_claim = ledger
                    .claims
                    .iter()
                    .any(|claim| claim.dimension_id == dimension.id);
                let has_gap = ledger
                    .gaps
                    .iter()
                    .any(|gap| gap.dimension_id == dimension.id);
                let structural = match (has_claim, has_gap) {
                    (true, false) => StructuralCoverage::ClaimsOnly,
                    (true, true) => StructuralCoverage::ClaimsAndGap,
                    (false, true) => StructuralCoverage::GapOnly,
                    (false, false) => StructuralCoverage::Missing,
                };
                DimensionCoverage {
                    dimension_id: dimension.id.clone(),
                    material: dimension.material,
                    structural,
                }
            })
            .collect(),
    }
}
