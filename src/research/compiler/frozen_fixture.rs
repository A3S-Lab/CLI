use super::*;
use a3s_acl::{Block, Value};
use std::path::{Path, PathBuf};

pub(super) struct FrozenReplay {
    pub(super) id: String,
    pub(super) required_behaviors: Vec<String>,
    pub(super) fault_stage: Option<String>,
    pub(super) fault_mode: Option<String>,
    pub(super) contract: ResearchContract,
    pub(super) catalog: SourceCatalog,
    pub(super) proposal: ClaimLedgerProposal,
    pub(super) forbidden_statements: Vec<String>,
}

struct FixtureCase {
    id: String,
    query: String,
    language: String,
    required_behaviors: Vec<String>,
    dimensions: Vec<FixtureDimension>,
    sources: Vec<FixtureSource>,
    claims: Vec<FixtureClaim>,
    relations: Vec<FixtureRelation>,
    fault: Option<FixtureFault>,
}

struct FixtureDimension {
    id: String,
    question: String,
    material: bool,
}

struct FixtureSource {
    id: String,
    title: String,
    url: String,
    requested_url: Option<String>,
    path: String,
    authority: String,
    captured_at: String,
}

struct FixtureClaim {
    id: String,
    disposition: String,
    dimension_id: String,
    kind: String,
    placement: String,
    statement: String,
    reader_statement: Option<String>,
    evidence: Vec<String>,
    basis: Vec<String>,
    derivation: Option<String>,
}

struct FixtureRelation {
    id: String,
    dimension_id: String,
    kind: String,
    claim_ids: Vec<String>,
}

struct FixtureFault {
    stage: String,
    target: Option<String>,
    mode: String,
}

pub(super) fn load_frozen_replays() -> Vec<FrozenReplay> {
    let root = fixture_root();
    let manifest =
        std::fs::read_to_string(root.join("frozen.acl")).expect("read frozen DeepResearch corpus");
    let document = a3s_acl::parse_acl(&manifest).expect("parse frozen DeepResearch corpus");
    let corpus = document.blocks.first().expect("frozen corpus block");
    corpus
        .blocks
        .iter()
        .filter(|block| block.name == "case")
        .map(parse_case)
        .map(|case| compile_case(&root, case))
        .collect()
}

fn parse_case(block: &Block) -> FixtureCase {
    FixtureCase {
        id: label(block),
        query: string(block, "query"),
        language: string(block, "report_language"),
        required_behaviors: string_list(block, "required_behaviors"),
        dimensions: child_blocks(block, "dimension")
            .map(|dimension| FixtureDimension {
                id: label(dimension),
                question: string(dimension, "question"),
                material: boolean(dimension, "material"),
            })
            .collect(),
        sources: child_blocks(block, "source")
            .map(|source| FixtureSource {
                id: label(source),
                title: string(source, "title"),
                url: string(source, "url"),
                requested_url: optional_string(source, "requested_url"),
                path: string(source, "path"),
                authority: string(source, "authority"),
                captured_at: string(source, "captured_at"),
            })
            .collect(),
        claims: child_blocks(block, "claim")
            .map(|claim| FixtureClaim {
                id: label(claim),
                disposition: string(claim, "disposition"),
                dimension_id: string(claim, "dimension"),
                kind: string(claim, "kind"),
                placement: string(claim, "placement"),
                statement: string(claim, "statement"),
                reader_statement: optional_string(claim, "reader_statement"),
                evidence: optional_string_list(claim, "evidence"),
                basis: string_list(claim, "basis"),
                derivation: optional_string(claim, "derivation"),
            })
            .collect(),
        relations: child_blocks(block, "relation")
            .map(|relation| FixtureRelation {
                id: label(relation),
                dimension_id: string(relation, "dimension"),
                kind: string(relation, "kind"),
                claim_ids: string_list(relation, "claims"),
            })
            .collect(),
        fault: child_blocks(block, "fault")
            .next()
            .map(|fault| FixtureFault {
                stage: string(fault, "stage"),
                target: optional_string(fault, "target"),
                mode: string(fault, "mode"),
            }),
    }
}

fn compile_case(root: &Path, case: FixtureCase) -> FrozenReplay {
    let source_targets = case
        .sources
        .iter()
        .map(|source| SourceTarget {
            id: target_id(&source.id),
            source_family_id: format!("fixture-{}", source.id),
            role: source_role(&source.authority),
            transport: source_transport(source),
            match_policy: TargetMatchPolicy::Named {
                identity: source_identity(source),
            },
        })
        .collect::<Vec<_>>();
    let dimensions = case
        .dimensions
        .iter()
        .map(|dimension| ResearchDimension {
            id: dimension.id.clone(),
            question: dimension.question.clone(),
            material: dimension.material,
            source_target_ids: case
                .sources
                .iter()
                .filter(|source| {
                    active_claims(&case).any(|claim| {
                        claim.dimension_id == dimension.id && claim.evidence.contains(&source.id)
                    })
                })
                .map(|source| target_id(&source.id))
                .collect(),
        })
        .collect::<Vec<_>>();
    let evidence_scope = match (
        case.sources
            .iter()
            .any(|source| source_transport(source) == AcquisitionTransport::Web),
        case.sources
            .iter()
            .any(|source| source_transport(source) == AcquisitionTransport::Workspace),
    ) {
        (true, true) => EvidenceScope::WebAndWorkspace,
        (true, false) => EvidenceScope::Web,
        (false, true) => EvidenceScope::Workspace,
        (false, false) => panic!("{}: fixture has no acquisition transport", case.id),
    };
    let spec = ResearchSpec {
        version: 2,
        query: case.query.clone(),
        language: case.language.clone(),
        current_date: "2026-07-21".to_string(),
        evidence_scope,
        dimensions,
        source_targets,
        budget: ResearchBudget {
            max_queries: case.sources.len(),
            max_fetches: case.sources.len(),
        },
    };
    let queries = case
        .sources
        .iter()
        .map(|source| {
            let target_id = target_id(&source.id);
            ResearchQuery {
                id: query_id(&source.id),
                text: format!("{} {}", case.query, source.title),
                transport: source_transport(source),
                mode: QueryMode::Exact,
                dimension_ids: spec
                    .dimensions
                    .iter()
                    .filter(|dimension| dimension.source_target_ids.contains(&target_id))
                    .map(|dimension| dimension.id.clone())
                    .collect(),
                source_target_ids: vec![target_id],
                fetch_slots: 1,
            }
        })
        .collect();
    let plan = QueryPlan {
        spec_digest: research_spec_digest(&spec),
        queries,
        planning_gaps: vec![],
    };
    let contract = validate_research_contract(spec, plan)
        .unwrap_or_else(|error| panic!("{}: compile frozen contract: {error}", case.id));
    let catalog = SourceCatalog {
        spec_digest: research_spec_digest(&contract.spec),
        attempts: case
            .sources
            .iter()
            .map(|source| AcquisitionAttempt {
                query_id: query_id(&source.id),
                source_target_ids: vec![target_id(&source.id)],
                outcome: AcquisitionOutcome::Fetched,
            })
            .collect(),
        sources: case
            .sources
            .iter()
            .map(|source| fixture_source_record(root, source))
            .collect(),
    };
    validate_source_catalog(&contract, &catalog)
        .unwrap_or_else(|error| panic!("{}: compile frozen catalog: {error}", case.id));

    let malformed_dimension = case.fault.as_ref().and_then(|fault| {
        (fault.mode == "malformed_target_result")
            .then(|| fault.target.clone())
            .flatten()
    });
    let claims = active_claims(&case)
        .map(|claim| {
            let kind = claim_kind(&claim.kind);
            ClaimProposal {
                id: claim.id.clone(),
                dimension_id: claim.dimension_id.clone(),
                placement: claim_placement(&claim.placement),
                kind,
                text: claim
                    .reader_statement
                    .clone()
                    .unwrap_or_else(|| claim.statement.clone()),
                evidence_refs: claim
                    .evidence
                    .iter()
                    .map(|source_id| ClaimEvidenceRef {
                        source_id: source_id.clone(),
                        chunk_ids: vec![if malformed_dimension.as_deref()
                            == Some(claim.dimension_id.as_str())
                        {
                            format!("{source_id}:chunk:missing")
                        } else {
                            chunk_id(source_id)
                        }],
                    })
                    .collect(),
                basis_claim_ids: claim.basis.clone(),
                derivation: (kind == ClaimKind::Inference)
                    .then_some(claim.derivation.as_ref())
                    .flatten()
                    .map(|method| DerivationProposal {
                        method: method.clone(),
                        input_claim_ids: claim.basis.clone(),
                    }),
            }
        })
        .collect();
    let relations = case
        .relations
        .iter()
        .map(|relation| ClaimRelationProposal {
            id: relation.id.clone(),
            dimension_id: relation.dimension_id.clone(),
            kind: match relation.kind.as_str() {
                "contradicts" => ClaimRelationKind::Contradicts,
                kind => panic!("{}: unknown relation kind `{kind}`", case.id),
            },
            claim_ids: relation
                .claim_ids
                .clone()
                .try_into()
                .unwrap_or_else(|_| panic!("{}: relation needs two claims", case.id)),
        })
        .collect();
    let forbidden_statements = case
        .claims
        .iter()
        .filter(|claim| claim.disposition == "forbidden")
        .map(|claim| claim.statement.clone())
        .collect();

    FrozenReplay {
        id: case.id,
        required_behaviors: case.required_behaviors,
        fault_stage: case.fault.as_ref().map(|fault| fault.stage.clone()),
        fault_mode: case.fault.as_ref().map(|fault| fault.mode.clone()),
        contract,
        catalog,
        proposal: ClaimLedgerProposal {
            claims,
            relations,
            gaps: vec![],
        },
        forbidden_statements,
    }
}

fn fixture_source_record(root: &Path, source: &FixtureSource) -> SourceRecord {
    let text = std::fs::read_to_string(root.join(&source.path))
        .unwrap_or_else(|error| panic!("{}: read frozen source: {error}", source.id));
    let chunks = vec![SourceChunk {
        id: chunk_id(&source.id),
        text: text.trim().to_string(),
    }];
    SourceRecord {
        id: source.id.clone(),
        title: source.title.clone(),
        requested_anchor: source
            .requested_url
            .clone()
            .unwrap_or_else(|| source.url.clone()),
        canonical_anchor: source.url.clone(),
        captured_at: format!("{}T00:00:00Z", source.captured_at),
        provenance: vec![SourceProvenance {
            query_id: query_id(&source.id),
            source_target_id: target_id(&source.id),
        }],
        content_digest: source_content_digest(&chunks),
        chunks,
    }
}

fn active_claims(case: &FixtureCase) -> impl Iterator<Item = &FixtureClaim> {
    case.claims
        .iter()
        .filter(|claim| claim.disposition != "forbidden")
}

fn source_transport(source: &FixtureSource) -> AcquisitionTransport {
    if source.url.starts_with("local://") {
        AcquisitionTransport::Workspace
    } else {
        AcquisitionTransport::Web
    }
}

fn source_identity(source: &FixtureSource) -> SourceIdentity {
    match source_transport(source) {
        AcquisitionTransport::Web => SourceIdentity::Url(source.url.clone()),
        AcquisitionTransport::Workspace => SourceIdentity::WorkspacePath(source.path.clone()),
    }
}

fn source_role(authority: &str) -> SourceRole {
    match authority {
        "primary" | "local_primary" => SourceRole::Primary,
        value => panic!("unknown frozen source authority `{value}`"),
    }
}

fn claim_kind(kind: &str) -> ClaimKind {
    match kind {
        "fact" => ClaimKind::Fact,
        "inference" => ClaimKind::Inference,
        "recommendation" => ClaimKind::Recommendation,
        value => panic!("unknown frozen claim kind `{value}`"),
    }
}

fn claim_placement(placement: &str) -> ClaimPlacement {
    match placement {
        "direct_answer" => ClaimPlacement::DirectAnswer,
        "finding" => ClaimPlacement::Finding,
        value => panic!("unknown frozen claim placement `{value}`"),
    }
}

fn target_id(source_id: &str) -> String {
    format!("target-{source_id}")
}

fn query_id(source_id: &str) -> String {
    format!("query-{source_id}")
}

fn chunk_id(source_id: &str) -> String {
    format!("{source_id}:chunk:1")
}

fn child_blocks<'a>(block: &'a Block, name: &'a str) -> impl Iterator<Item = &'a Block> {
    block.blocks.iter().filter(move |child| child.name == name)
}

fn label(block: &Block) -> String {
    assert_eq!(block.labels.len(), 1, "{} needs one label", block.name);
    block.labels[0].clone()
}

fn string(block: &Block, key: &str) -> String {
    block
        .attributes
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{} {:?} needs string `{key}`", block.name, block.labels))
        .to_string()
}

fn optional_string(block: &Block, key: &str) -> Option<String> {
    block
        .attributes
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn boolean(block: &Block, key: &str) -> bool {
    block
        .attributes
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("{} {:?} needs bool `{key}`", block.name, block.labels))
}

fn string_list(block: &Block, key: &str) -> Vec<String> {
    match block.attributes.get(key) {
        Some(Value::List(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .unwrap_or_else(|| panic!("{} `{key}` needs strings", block.name))
                    .to_string()
            })
            .collect(),
        _ => panic!("{} {:?} needs list `{key}`", block.name, block.labels),
    }
}

fn optional_string_list(block: &Block, key: &str) -> Vec<String> {
    if block.attributes.contains_key(key) {
        string_list(block, key)
    } else {
        Vec::new()
    }
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/deep_research_eval")
}
