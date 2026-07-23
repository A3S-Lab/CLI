use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

const COMPILER_SPEC_VERSION: u32 = 2;
const MAX_ID_CHARS: usize = 64;
const MAX_QUERY_CHARS: usize = 4_000;
const MAX_TEXT_CHARS: usize = 1_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EvidenceScope {
    Web,
    Workspace,
    WebAndWorkspace,
}

impl EvidenceScope {
    fn permits(self, transport: AcquisitionTransport) -> bool {
        matches!(
            (self, transport),
            (Self::Web, AcquisitionTransport::Web)
                | (Self::Workspace, AcquisitionTransport::Workspace)
                | (Self::WebAndWorkspace, _)
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AcquisitionTransport {
    Web,
    Workspace,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum QueryMode {
    Exact,
    Discovery,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SourceRole {
    Canonical,
    Official,
    Primary,
    Independent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub(super) enum SourceIdentity {
    Repository(String),
    Domain(String),
    Url(String),
    WorkspacePath(String),
}

impl SourceIdentity {
    pub(super) fn transport(&self) -> AcquisitionTransport {
        match self {
            Self::WorkspacePath(_) => AcquisitionTransport::Workspace,
            Self::Repository(_) | Self::Domain(_) | Self::Url(_) => AcquisitionTransport::Web,
        }
    }

    fn is_valid(&self) -> bool {
        match self {
            Self::Repository(value) => valid_repository_identity(value),
            Self::Domain(value) => valid_domain_identity(value),
            Self::Url(value) => valid_https_url(value),
            Self::WorkspacePath(value) => valid_workspace_path(value),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(super) enum TargetMatchPolicy {
    Named { identity: SourceIdentity },
    Exploratory { selection_goal: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceTarget {
    pub(super) id: String,
    pub(super) source_family_id: String,
    pub(super) role: SourceRole,
    pub(super) transport: AcquisitionTransport,
    pub(super) match_policy: TargetMatchPolicy,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResearchDimension {
    pub(super) id: String,
    pub(super) question: String,
    pub(super) material: bool,
    pub(super) source_target_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResearchBudget {
    pub(super) max_queries: usize,
    pub(super) max_fetches: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResearchSpec {
    pub(super) version: u32,
    pub(super) query: String,
    pub(super) language: String,
    pub(super) current_date: String,
    pub(super) evidence_scope: EvidenceScope,
    pub(super) dimensions: Vec<ResearchDimension>,
    pub(super) source_targets: Vec<SourceTarget>,
    pub(super) budget: ResearchBudget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResearchQuery {
    pub(super) id: String,
    pub(super) text: String,
    pub(super) transport: AcquisitionTransport,
    pub(super) mode: QueryMode,
    pub(super) dimension_ids: Vec<String>,
    pub(super) source_target_ids: Vec<String>,
    pub(super) fetch_slots: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlanningGap {
    pub(super) dimension_id: String,
    pub(super) missing_source_target_ids: Vec<String>,
    pub(super) reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryPlan {
    pub(super) spec_digest: String,
    pub(super) queries: Vec<ResearchQuery>,
    pub(super) planning_gaps: Vec<PlanningGap>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResearchContract {
    pub(super) spec: ResearchSpec,
    pub(super) plan: QueryPlan,
}

impl ResearchContract {
    pub(super) fn dimension(&self, id: &str) -> Option<&ResearchDimension> {
        self.spec
            .dimensions
            .iter()
            .find(|dimension| dimension.id == id)
    }

    pub(super) fn target(&self, id: &str) -> Option<&SourceTarget> {
        self.spec
            .source_targets
            .iter()
            .find(|target| target.id == id)
    }

    pub(super) fn query(&self, id: &str) -> Option<&ResearchQuery> {
        self.plan.queries.iter().find(|query| query.id == id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum ContractError {
    #[error("unsupported research spec version {observed}")]
    UnsupportedSpecVersion { observed: u32 },
    #[error("invalid research contract field `{field}`")]
    InvalidField { field: &'static str },
    #[error("duplicate dimension ID `{dimension_id}`")]
    DuplicateDimensionId { dimension_id: String },
    #[error("duplicate source target ID `{target_id}`")]
    DuplicateTargetId { target_id: String },
    #[error("dimension `{dimension_id}` references unknown target `{target_id}`")]
    UnknownDimensionTarget {
        dimension_id: String,
        target_id: String,
    },
    #[error("source target `{target_id}` has an invalid identity")]
    InvalidTargetIdentity { target_id: String },
    #[error("source target `{target_id}` identity conflicts with its transport")]
    TargetIdentityTransportMismatch { target_id: String },
    #[error("source target `{target_id}` is outside the evidence scope")]
    TargetOutsideEvidenceScope { target_id: String },
    #[error("query plan belongs to a different research spec")]
    SpecDigestMismatch,
    #[error("query count {observed} exceeds budget {maximum}")]
    QueryBudgetExceeded { observed: usize, maximum: usize },
    #[error("fetch allocation {observed} exceeds budget {maximum}")]
    FetchBudgetExceeded { observed: usize, maximum: usize },
    #[error("duplicate query ID `{query_id}`")]
    DuplicateQueryId { query_id: String },
    #[error("query `{query_id}` references unknown dimension `{dimension_id}`")]
    UnknownQueryDimension {
        query_id: String,
        dimension_id: String,
    },
    #[error("query `{query_id}` references unknown source target `{target_id}`")]
    UnknownQueryTarget { query_id: String, target_id: String },
    #[error("query `{query_id}` and target `{target_id}` use different transports")]
    QueryTargetTransportMismatch { query_id: String, target_id: String },
    #[error("exact query `{query_id}` references exploratory target `{target_id}`")]
    ExactQueryUsesExploratoryTarget { query_id: String, target_id: String },
    #[error(
        "query `{query_id}` has {named_target_count} named targets but only {fetch_slots} fetch slots"
    )]
    NamedTargetsExceedFetchAllocation {
        query_id: String,
        named_target_count: usize,
        fetch_slots: usize,
    },
    #[error("query `{query_id}` target `{target_id}` is unrelated to its dimensions")]
    QueryTargetNotDeclaredByDimension { query_id: String, target_id: String },
    #[error("query `{query_id}` dimension `{dimension_id}` has no target edge")]
    QueryDimensionHasNoTarget {
        query_id: String,
        dimension_id: String,
    },
    #[error("planning gap references unknown dimension `{dimension_id}`")]
    UnknownPlanningGapDimension { dimension_id: String },
    #[error("duplicate planning gap for dimension `{dimension_id}`")]
    DuplicatePlanningGap { dimension_id: String },
    #[error(
        "planning gap for dimension `{dimension_id}` references undeclared target `{target_id}`"
    )]
    PlanningGapTargetNotDeclared {
        dimension_id: String,
        target_id: String,
    },
    #[error("material dimension `{dimension_id}` has no query edge or planning gap")]
    MissingDimensionCoverage { dimension_id: String },
    #[error("dimension `{dimension_id}` target `{target_id}` has no query edge or planning gap")]
    MissingTargetCoverage {
        dimension_id: String,
        target_id: String,
    },
}

pub(super) fn research_spec_digest(spec: &ResearchSpec) -> String {
    let encoded = serde_json::to_vec(spec).expect("ResearchSpec serialization is infallible");
    format!("{:x}", Sha256::digest(encoded))
}

pub(super) fn validate_research_contract(
    spec: ResearchSpec,
    plan: QueryPlan,
) -> Result<ResearchContract, ContractError> {
    validate_spec(&spec)?;
    if plan.spec_digest != research_spec_digest(&spec) {
        return Err(ContractError::SpecDigestMismatch);
    }

    let dimensions = spec
        .dimensions
        .iter()
        .map(|dimension| (dimension.id.as_str(), dimension))
        .collect::<BTreeMap<_, _>>();
    let targets = spec
        .source_targets
        .iter()
        .map(|target| (target.id.as_str(), target))
        .collect::<BTreeMap<_, _>>();
    validate_plan(&spec, &plan, &dimensions, &targets)?;

    Ok(ResearchContract { spec, plan })
}

fn validate_spec(spec: &ResearchSpec) -> Result<(), ContractError> {
    if spec.version != COMPILER_SPEC_VERSION {
        return Err(ContractError::UnsupportedSpecVersion {
            observed: spec.version,
        });
    }
    for (field, value, maximum) in [
        ("query", spec.query.as_str(), MAX_QUERY_CHARS),
        ("language", spec.language.as_str(), 32),
        ("current_date", spec.current_date.as_str(), 32),
    ] {
        if !valid_text(value, maximum) {
            return Err(ContractError::InvalidField { field });
        }
    }
    if chrono::NaiveDate::parse_from_str(&spec.current_date, "%Y-%m-%d").is_err() {
        return Err(ContractError::InvalidField {
            field: "current_date",
        });
    }
    if spec.dimensions.is_empty() || spec.budget.max_queries == 0 || spec.budget.max_fetches == 0 {
        return Err(ContractError::InvalidField {
            field: "dimensions_or_budget",
        });
    }

    let mut target_ids = BTreeSet::new();
    for target in &spec.source_targets {
        if !stable_id(&target.id) || !stable_id(&target.source_family_id) {
            return Err(ContractError::InvalidField {
                field: "source_target_id",
            });
        }
        if !target_ids.insert(target.id.as_str()) {
            return Err(ContractError::DuplicateTargetId {
                target_id: target.id.clone(),
            });
        }
        if !spec.evidence_scope.permits(target.transport) {
            return Err(ContractError::TargetOutsideEvidenceScope {
                target_id: target.id.clone(),
            });
        }
        match &target.match_policy {
            TargetMatchPolicy::Named { identity } => {
                if !identity.is_valid() {
                    return Err(ContractError::InvalidTargetIdentity {
                        target_id: target.id.clone(),
                    });
                }
                if identity.transport() != target.transport {
                    return Err(ContractError::TargetIdentityTransportMismatch {
                        target_id: target.id.clone(),
                    });
                }
            }
            TargetMatchPolicy::Exploratory { selection_goal } => {
                if !valid_text(selection_goal, MAX_TEXT_CHARS) {
                    return Err(ContractError::InvalidField {
                        field: "selection_goal",
                    });
                }
            }
        }
    }

    let mut dimension_ids = BTreeSet::new();
    for dimension in &spec.dimensions {
        if !stable_id(&dimension.id) || !valid_text(&dimension.question, MAX_TEXT_CHARS) {
            return Err(ContractError::InvalidField {
                field: "research_dimension",
            });
        }
        if !dimension_ids.insert(dimension.id.as_str()) {
            return Err(ContractError::DuplicateDimensionId {
                dimension_id: dimension.id.clone(),
            });
        }
        let mut seen_targets = BTreeSet::new();
        for target_id in &dimension.source_target_ids {
            if !seen_targets.insert(target_id) {
                return Err(ContractError::InvalidField {
                    field: "dimension_source_target_ids",
                });
            }
            if !target_ids.contains(target_id.as_str()) {
                return Err(ContractError::UnknownDimensionTarget {
                    dimension_id: dimension.id.clone(),
                    target_id: target_id.clone(),
                });
            }
        }
    }
    Ok(())
}

fn validate_plan(
    spec: &ResearchSpec,
    plan: &QueryPlan,
    dimensions: &BTreeMap<&str, &ResearchDimension>,
    targets: &BTreeMap<&str, &SourceTarget>,
) -> Result<(), ContractError> {
    if plan.queries.len() > spec.budget.max_queries {
        return Err(ContractError::QueryBudgetExceeded {
            observed: plan.queries.len(),
            maximum: spec.budget.max_queries,
        });
    }
    let fetch_slots = plan
        .queries
        .iter()
        .try_fold(0usize, |total, query| total.checked_add(query.fetch_slots))
        .unwrap_or(usize::MAX);
    if fetch_slots > spec.budget.max_fetches {
        return Err(ContractError::FetchBudgetExceeded {
            observed: fetch_slots,
            maximum: spec.budget.max_fetches,
        });
    }

    let mut query_ids = BTreeSet::new();
    let mut queried_dimensions = BTreeSet::new();
    let mut queried_targets_by_dimension = BTreeMap::<&str, BTreeSet<&str>>::new();
    for query in &plan.queries {
        validate_query(
            query,
            dimensions,
            targets,
            &mut queried_dimensions,
            &mut queried_targets_by_dimension,
        )?;
        if !query_ids.insert(query.id.as_str()) {
            return Err(ContractError::DuplicateQueryId {
                query_id: query.id.clone(),
            });
        }
    }

    let mut gap_dimensions = BTreeSet::new();
    let mut gap_targets_by_dimension = BTreeMap::<&str, BTreeSet<&str>>::new();
    for gap in &plan.planning_gaps {
        let Some(dimension) = dimensions.get(gap.dimension_id.as_str()).copied() else {
            return Err(ContractError::UnknownPlanningGapDimension {
                dimension_id: gap.dimension_id.clone(),
            });
        };
        if !gap_dimensions.insert(gap.dimension_id.as_str()) {
            return Err(ContractError::DuplicatePlanningGap {
                dimension_id: gap.dimension_id.clone(),
            });
        }
        if !valid_text(&gap.reason, MAX_TEXT_CHARS)
            || has_duplicates(&gap.missing_source_target_ids)
        {
            return Err(ContractError::InvalidField {
                field: "planning_gap_reason",
            });
        }
        for target_id in &gap.missing_source_target_ids {
            if !dimension.source_target_ids.contains(target_id) {
                return Err(ContractError::PlanningGapTargetNotDeclared {
                    dimension_id: gap.dimension_id.clone(),
                    target_id: target_id.clone(),
                });
            }
            gap_targets_by_dimension
                .entry(dimension.id.as_str())
                .or_default()
                .insert(target_id.as_str());
        }
    }

    for dimension in &spec.dimensions {
        if dimension.material
            && !queried_dimensions.contains(dimension.id.as_str())
            && !gap_dimensions.contains(dimension.id.as_str())
        {
            return Err(ContractError::MissingDimensionCoverage {
                dimension_id: dimension.id.clone(),
            });
        }
        let queried_targets = queried_targets_by_dimension
            .get(dimension.id.as_str())
            .cloned()
            .unwrap_or_default();
        let gap_targets = gap_targets_by_dimension
            .get(dimension.id.as_str())
            .cloned()
            .unwrap_or_default();
        for target_id in &dimension.source_target_ids {
            if !queried_targets.contains(target_id.as_str())
                && !gap_targets.contains(target_id.as_str())
            {
                return Err(ContractError::MissingTargetCoverage {
                    dimension_id: dimension.id.clone(),
                    target_id: target_id.clone(),
                });
            }
        }
    }
    Ok(())
}

fn validate_query<'a>(
    query: &'a ResearchQuery,
    dimensions: &BTreeMap<&'a str, &'a ResearchDimension>,
    targets: &BTreeMap<&'a str, &'a SourceTarget>,
    queried_dimensions: &mut BTreeSet<&'a str>,
    queried_targets_by_dimension: &mut BTreeMap<&'a str, BTreeSet<&'a str>>,
) -> Result<(), ContractError> {
    if !stable_id(&query.id)
        || !valid_text(&query.text, MAX_QUERY_CHARS)
        || query.fetch_slots == 0
        || query.dimension_ids.is_empty()
        || query.source_target_ids.is_empty()
        || has_duplicates(&query.dimension_ids)
        || has_duplicates(&query.source_target_ids)
    {
        return Err(ContractError::InvalidField {
            field: "research_query",
        });
    }

    let mut query_dimensions = Vec::with_capacity(query.dimension_ids.len());
    for dimension_id in &query.dimension_ids {
        let Some(dimension) = dimensions.get(dimension_id.as_str()).copied() else {
            return Err(ContractError::UnknownQueryDimension {
                query_id: query.id.clone(),
                dimension_id: dimension_id.clone(),
            });
        };
        queried_dimensions.insert(dimension.id.as_str());
        query_dimensions.push(dimension);
    }

    let mut named_target_count = 0usize;
    for target_id in &query.source_target_ids {
        let Some(target) = targets.get(target_id.as_str()).copied() else {
            return Err(ContractError::UnknownQueryTarget {
                query_id: query.id.clone(),
                target_id: target_id.clone(),
            });
        };
        if target.transport != query.transport {
            return Err(ContractError::QueryTargetTransportMismatch {
                query_id: query.id.clone(),
                target_id: target.id.clone(),
            });
        }
        match target.match_policy {
            TargetMatchPolicy::Named { .. } => named_target_count += 1,
            TargetMatchPolicy::Exploratory { .. } if query.mode == QueryMode::Exact => {
                return Err(ContractError::ExactQueryUsesExploratoryTarget {
                    query_id: query.id.clone(),
                    target_id: target.id.clone(),
                });
            }
            TargetMatchPolicy::Exploratory { .. } => {}
        }
        if !query_dimensions
            .iter()
            .any(|dimension| dimension.source_target_ids.contains(&target.id))
        {
            return Err(ContractError::QueryTargetNotDeclaredByDimension {
                query_id: query.id.clone(),
                target_id: target.id.clone(),
            });
        }
    }
    if named_target_count > query.fetch_slots {
        return Err(ContractError::NamedTargetsExceedFetchAllocation {
            query_id: query.id.clone(),
            named_target_count,
            fetch_slots: query.fetch_slots,
        });
    }

    for dimension in query_dimensions {
        let edges = query
            .source_target_ids
            .iter()
            .filter(|target_id| dimension.source_target_ids.contains(*target_id))
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if edges.is_empty() {
            return Err(ContractError::QueryDimensionHasNoTarget {
                query_id: query.id.clone(),
                dimension_id: dimension.id.clone(),
            });
        }
        queried_targets_by_dimension
            .entry(dimension.id.as_str())
            .or_default()
            .extend(edges);
    }
    Ok(())
}

pub(super) fn stable_id(value: &str) -> bool {
    value.len() <= MAX_ID_CHARS
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && value.chars().skip(1).all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
}

pub(super) fn valid_text(value: &str, maximum_chars: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.chars().count() <= maximum_chars
        && !value
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
}

fn has_duplicates(values: &[String]) -> bool {
    let mut seen = BTreeSet::new();
    values.iter().any(|value| !seen.insert(value))
}

fn valid_repository_identity(value: &str) -> bool {
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    !owner.is_empty()
        && !repository.is_empty()
        && parts.next().is_none()
        && [owner, repository].iter().all(|part| {
            part.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
        })
}

fn valid_domain_identity(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && value.contains('.')
        && !value.contains('/')
        && !value.contains(':')
        && value.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
}

fn valid_https_url(value: &str) -> bool {
    value.starts_with("https://")
        && reqwest::Url::parse(value).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
}

fn valid_workspace_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && value == value.trim()
        && !path.is_absolute()
        && path.components().all(|component| {
            matches!(component, Component::Normal(_) | Component::CurDir)
                && component != Component::CurDir
        })
}
