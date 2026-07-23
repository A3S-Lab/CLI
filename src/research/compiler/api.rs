use super::{
    admit_claim_ledger, build_no_evidence_document, build_report_document,
    build_source_backed_document, render_report_document, report_structural_outcome,
    research_spec_digest, source_content_digest, validate_research_contract,
    validate_source_catalog, ClaimLedgerProposal, QueryPlan, RejectionReason, ReportDocument,
    ResearchContract, ResearchSpec, SourceCatalog, StructuralCoverage, StructuralOutcome,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceCompilerOutcome {
    Completed,
    Qualified,
    SourceBacked,
    Degraded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompilerCoverage {
    pub dimension_id: String,
    pub material: bool,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompilerRejection {
    pub item_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledEvidenceReport {
    pub outcome: EvidenceCompilerOutcome,
    pub markdown: String,
    pub html: String,
    pub coverage: Vec<CompilerCoverage>,
    pub accepted_claim_count: usize,
    pub accepted_gap_count: usize,
    pub rejected_item_count: usize,
    pub rejections: Vec<CompilerRejection>,
    pub source_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EvidenceCompilerError {
    #[error("invalid {artifact} JSON: {reason}")]
    InvalidJson {
        artifact: &'static str,
        reason: String,
    },
    #[error("invalid research contract: {0}")]
    InvalidContract(String),
    #[error("invalid source catalog: {0}")]
    InvalidCatalog(String),
    #[error("claim-ledger admission failed: {0}")]
    LedgerAdmission(String),
    #[error("report-document construction failed: {0}")]
    Document(String),
}

pub fn evidence_spec_digest(spec: &Value) -> Result<String, EvidenceCompilerError> {
    let spec = parse::<ResearchSpec>("research spec", spec)?;
    Ok(research_spec_digest(&spec))
}

pub fn evidence_source_content_digest(chunks: &Value) -> Result<String, EvidenceCompilerError> {
    let chunks = parse::<Vec<super::SourceChunk>>("source chunks", chunks)?;
    Ok(source_content_digest(&chunks))
}

pub fn validate_evidence_contract(spec: &Value, plan: &Value) -> Result<(), EvidenceCompilerError> {
    validated_contract(spec, plan).map(|_| ())
}

pub fn validate_evidence_catalog(
    spec: &Value,
    plan: &Value,
    catalog: &Value,
) -> Result<(), EvidenceCompilerError> {
    let contract = validated_contract(spec, plan)?;
    let catalog = parse::<SourceCatalog>("source catalog", catalog)?;
    validate_source_catalog(&contract, &catalog)
        .map_err(|error| EvidenceCompilerError::InvalidCatalog(error.to_string()))
}

pub fn compile_evidence_report(
    spec: &Value,
    plan: &Value,
    catalog: &Value,
    proposal: Option<&Value>,
) -> Result<CompiledEvidenceReport, EvidenceCompilerError> {
    let contract = validated_contract(spec, plan)?;
    let catalog = parse::<SourceCatalog>("source catalog", catalog)?;
    validate_source_catalog(&contract, &catalog)
        .map_err(|error| EvidenceCompilerError::InvalidCatalog(error.to_string()))?;

    let (document, accepted_claim_count, accepted_gap_count, rejections) = match proposal {
        Some(proposal) => {
            let proposal = parse::<ClaimLedgerProposal>("claim ledger", proposal)?;
            let ledger = admit_claim_ledger(&contract, &catalog, proposal)
                .map_err(|error| EvidenceCompilerError::LedgerAdmission(error.to_string()))?;
            let document = build_report_document(&contract, &catalog, &ledger)
                .map_err(|error| EvidenceCompilerError::Document(error.to_string()))?;
            let rejections = ledger
                .rejections
                .iter()
                .map(|rejection| CompilerRejection {
                    item_id: rejection.item_id.clone(),
                    reason: rejection_label(rejection.reason).to_string(),
                })
                .collect::<Vec<_>>();
            (document, ledger.claims.len(), ledger.gaps.len(), rejections)
        }
        None => (
            if catalog.sources.is_empty() {
                build_no_evidence_document(&contract)
            } else {
                build_source_backed_document(&contract, &catalog)
                    .map_err(|error| EvidenceCompilerError::Document(error.to_string()))?
            },
            0,
            contract.spec.dimensions.len(),
            Vec::new(),
        ),
    };

    let outcome = compiler_outcome(report_structural_outcome(&document));
    let coverage = document_coverage(&document);
    let rendered = render_report_document(&document);
    Ok(CompiledEvidenceReport {
        outcome,
        markdown: rendered.markdown,
        html: rendered.html,
        coverage,
        accepted_claim_count,
        accepted_gap_count,
        rejected_item_count: rejections.len(),
        rejections,
        source_count: catalog.sources.len(),
    })
}

fn validated_contract(
    spec: &Value,
    plan: &Value,
) -> Result<ResearchContract, EvidenceCompilerError> {
    let spec = parse::<ResearchSpec>("research spec", spec)?;
    let plan = parse::<QueryPlan>("query plan", plan)?;
    validate_research_contract(spec, plan)
        .map_err(|error| EvidenceCompilerError::InvalidContract(error.to_string()))
}

fn parse<T>(artifact: &'static str, value: &Value) -> Result<T, EvidenceCompilerError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(value.clone()).map_err(|error| EvidenceCompilerError::InvalidJson {
        artifact,
        reason: error.to_string(),
    })
}

fn compiler_outcome(outcome: StructuralOutcome) -> EvidenceCompilerOutcome {
    match outcome {
        StructuralOutcome::Completed => EvidenceCompilerOutcome::Completed,
        StructuralOutcome::Qualified => EvidenceCompilerOutcome::Qualified,
        StructuralOutcome::SourceBacked => EvidenceCompilerOutcome::SourceBacked,
        StructuralOutcome::Degraded => EvidenceCompilerOutcome::Degraded,
    }
}

fn document_coverage(document: &ReportDocument) -> Vec<CompilerCoverage> {
    document
        .dimensions
        .iter()
        .map(|dimension| CompilerCoverage {
            dimension_id: dimension.dimension_id.clone(),
            material: dimension.material,
            status: match dimension.coverage {
                StructuralCoverage::ClaimsOnly => "claims_only",
                StructuralCoverage::ClaimsAndGap => "claims_and_gap",
                StructuralCoverage::GapOnly => "gap_only",
                StructuralCoverage::Missing => "missing",
            }
            .to_string(),
        })
        .collect()
}

fn rejection_label(reason: RejectionReason) -> &'static str {
    match reason {
        RejectionReason::InvalidIdentity => "invalid_identity",
        RejectionReason::DuplicateIdentity => "duplicate_identity",
        RejectionReason::UnknownDimension => "unknown_dimension",
        RejectionReason::InvalidText => "invalid_text",
        RejectionReason::InvalidClaimShape => "invalid_claim_shape",
        RejectionReason::InvalidEvidenceReference => "invalid_evidence_reference",
        RejectionReason::EvidenceOutsideDimensionTargets => "evidence_outside_dimension_targets",
        RejectionReason::InvalidBasis => "invalid_basis",
        RejectionReason::UnresolvableBasisGraph => "unresolvable_basis_graph",
        RejectionReason::InvalidDerivation => "invalid_derivation",
        RejectionReason::InvalidRelation => "invalid_relation",
        RejectionReason::InvalidGapProvenance => "invalid_gap_provenance",
    }
}
