use super::*;

pub(super) fn budget(max_queries: usize, max_fetches: usize) -> ResearchBudget {
    ResearchBudget {
        max_queries,
        max_fetches,
    }
}

pub(super) fn dimension(id: &str, target_ids: &[&str]) -> ResearchDimension {
    ResearchDimension {
        id: id.to_string(),
        question: format!("What establishes {id}?"),
        material: true,
        source_target_ids: target_ids.iter().map(|id| (*id).to_string()).collect(),
    }
}

pub(super) fn named_target(
    id: &str,
    family: &str,
    role: SourceRole,
    identity: SourceIdentity,
) -> SourceTarget {
    let transport = identity.transport();
    SourceTarget {
        id: id.to_string(),
        source_family_id: family.to_string(),
        role,
        transport,
        match_policy: TargetMatchPolicy::Named { identity },
    }
}

pub(super) fn exploratory_target(id: &str, family: &str, goal: &str) -> SourceTarget {
    SourceTarget {
        id: id.to_string(),
        source_family_id: family.to_string(),
        role: SourceRole::Independent,
        transport: AcquisitionTransport::Web,
        match_policy: TargetMatchPolicy::Exploratory {
            selection_goal: goal.to_string(),
        },
    }
}

pub(super) fn spec(
    scope: EvidenceScope,
    dimensions: Vec<ResearchDimension>,
    source_targets: Vec<SourceTarget>,
    budget: ResearchBudget,
) -> ResearchSpec {
    ResearchSpec {
        version: 2,
        query: "Evaluate the requested decision from traceable evidence.".to_string(),
        language: "en".to_string(),
        current_date: "2026-07-21".to_string(),
        evidence_scope: scope,
        dimensions,
        source_targets,
        budget,
    }
}

pub(super) fn query(
    id: &str,
    transport: AcquisitionTransport,
    mode: QueryMode,
    dimensions: &[&str],
    targets: &[&str],
    fetch_slots: usize,
) -> ResearchQuery {
    ResearchQuery {
        id: id.to_string(),
        text: format!("source-seeking query for {id}"),
        transport,
        mode,
        dimension_ids: dimensions
            .iter()
            .map(|dimension| (*dimension).to_string())
            .collect(),
        source_target_ids: targets.iter().map(|target| (*target).to_string()).collect(),
        fetch_slots,
    }
}

pub(super) fn plan(
    spec: &ResearchSpec,
    queries: Vec<ResearchQuery>,
    gaps: Vec<PlanningGap>,
) -> QueryPlan {
    QueryPlan {
        spec_digest: research_spec_digest(spec),
        queries,
        planning_gaps: gaps,
    }
}
