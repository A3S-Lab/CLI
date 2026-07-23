use super::{
    workspace_code_trace_pattern, AcquisitionTransport, QueryDiscovery, SelectedCandidate,
    SelectionEdge, SourceCandidate,
};
use crate::commands::code::research_runtime::tests::baseline::live::planning::{
    target_index, AcquisitionQuery, EvaluationStrategy, PlanningResult, PreferredSourceKind,
    SourcePreference,
};
use serde_json::Value as JsonValue;
use std::collections::BTreeSet;

pub(super) fn select_candidates(
    planning: &PlanningResult,
    discoveries: &[QueryDiscovery],
    maximum: usize,
) -> Vec<SelectedCandidate> {
    match planning.strategy {
        EvaluationStrategy::Minimal => select_minimal(planning, discoveries, maximum),
        EvaluationStrategy::Brief => select_brief(planning, discoveries, maximum),
        EvaluationStrategy::Compiler => select_compiler(planning, discoveries, maximum),
    }
}

fn select_minimal(
    planning: &PlanningResult,
    discoveries: &[QueryDiscovery],
    maximum: usize,
) -> Vec<SelectedCandidate> {
    if let Some(discovery) = discoveries.iter().find(|discovery| {
        discovery.query.id == "query.bootstrap"
            && discovery.query.transport == AcquisitionTransport::Workspace
            && workspace_code_trace_pattern(&discovery.query.text)
    }) {
        return rank_discovery_candidates(discovery)
            .into_iter()
            .take(discovery.query.fetch_slots.min(maximum))
            .map(|candidate| SelectedCandidate {
                edges: vec![SelectionEdge {
                    query_id: discovery.query.id.clone(),
                    source_target_id: None,
                    match_score: candidate_relevance_score(&candidate, &discovery.query),
                }],
                candidate,
            })
            .collect();
    }

    let ranked = discoveries
        .iter()
        .map(|discovery| rank_discovery_candidates_for_planning(discovery, planning))
        .collect::<Vec<_>>();
    let mut selected: Vec<SelectedCandidate> = Vec::new();
    let mut cursors = vec![0usize; discoveries.len()];
    let mut query_edges = vec![0usize; discoveries.len()];
    let mut made_progress = true;
    while selected.len() < maximum && made_progress {
        made_progress = false;
        for (index, discovery) in discoveries.iter().enumerate() {
            if query_edges[index] >= discovery.query.fetch_slots {
                continue;
            }
            if let Some(candidate) = ranked[index].get(cursors[index]) {
                cursors[index] += 1;
                let edge = SelectionEdge {
                    query_id: discovery.query.id.clone(),
                    source_target_id: None,
                    match_score: candidate_relevance_score(candidate, &discovery.query),
                };
                merge_or_push(&mut selected, candidate.clone(), edge, maximum);
                query_edges[index] += 1;
                made_progress = true;
            }
            if selected.len() >= maximum {
                break;
            }
        }
    }
    selected
}

fn select_compiler(
    planning: &PlanningResult,
    discoveries: &[QueryDiscovery],
    maximum: usize,
) -> Vec<SelectedCandidate> {
    let Some(spec) = planning.spec.as_ref() else {
        return Vec::new();
    };
    let targets = target_index(spec);
    let mut selected: Vec<SelectedCandidate> = Vec::new();
    let mut used_edges = BTreeSet::<(String, String, String)>::new();

    for discovery in discoveries {
        let mut allocated = 0usize;
        for target_id in &discovery.query.source_target_ids {
            if allocated >= discovery.query.fetch_slots {
                break;
            }
            let Some(target) = targets.get(target_id) else {
                continue;
            };
            let Some((candidate, score)) = best_candidate(
                &discovery.candidates,
                target,
                &used_edges,
                &discovery.query.id,
                target_id,
            ) else {
                continue;
            };
            used_edges.insert((
                discovery.query.id.clone(),
                target_id.clone(),
                candidate.anchor.clone(),
            ));
            merge_or_push(
                &mut selected,
                candidate,
                SelectionEdge {
                    query_id: discovery.query.id.clone(),
                    source_target_id: Some(target_id.clone()),
                    match_score: score,
                },
                maximum,
            );
            allocated += 1;
        }

        while allocated < discovery.query.fetch_slots && selected.len() < maximum {
            let best = discovery
                .query
                .source_target_ids
                .iter()
                .filter_map(|target_id| {
                    let target = targets.get(target_id)?;
                    best_candidate(
                        &discovery.candidates,
                        target,
                        &used_edges,
                        &discovery.query.id,
                        target_id,
                    )
                    .map(|(candidate, score)| (candidate, target_id.clone(), score))
                })
                .max_by_key(|(_, _, score)| *score);
            let Some((candidate, target_id, score)) = best else {
                break;
            };
            used_edges.insert((
                discovery.query.id.clone(),
                target_id.clone(),
                candidate.anchor.clone(),
            ));
            merge_or_push(
                &mut selected,
                candidate,
                SelectionEdge {
                    query_id: discovery.query.id.clone(),
                    source_target_id: Some(target_id),
                    match_score: score,
                },
                maximum,
            );
            allocated += 1;
        }
    }
    selected
}

fn select_brief(
    planning: &PlanningResult,
    discoveries: &[QueryDiscovery],
    maximum: usize,
) -> Vec<SelectedCandidate> {
    let ranked = discoveries
        .iter()
        .map(|discovery| {
            let mut candidates = discovery
                .candidates
                .iter()
                .filter(|candidate| {
                    candidate.transport != AcquisitionTransport::Web
                        || (web_candidate_relevance_score(candidate, &discovery.query.text)
                            .is_some()
                            && web_candidate_matches_planning_scope(
                                candidate,
                                &discovery.query,
                                planning,
                            ))
                })
                .cloned()
                .collect::<Vec<_>>();
            candidates.sort_by(|left, right| {
                brief_candidate_score(right, &discovery.query)
                    .cmp(&brief_candidate_score(left, &discovery.query))
                    .then_with(|| left.anchor.cmp(&right.anchor))
            });
            candidates
        })
        .collect::<Vec<_>>();
    let mut selected = Vec::new();
    let mut cursors = vec![0usize; ranked.len()];
    let mut made_progress = true;
    while selected.len() < maximum && made_progress {
        made_progress = false;
        for (index, discovery) in discoveries.iter().enumerate() {
            while let Some(candidate) = ranked[index].get(cursors[index]).cloned() {
                cursors[index] += 1;
                let edge = SelectionEdge {
                    query_id: discovery.query.id.clone(),
                    source_target_id: None,
                    match_score: brief_candidate_score(&candidate, &discovery.query),
                };
                let previous_len = selected.len();
                merge_or_push(&mut selected, candidate, edge, maximum);
                if selected.len() > previous_len {
                    made_progress = true;
                    break;
                }
            }
            if selected.len() >= maximum {
                break;
            }
        }
    }
    selected
}

pub(super) fn rank_discovery_candidates(discovery: &QueryDiscovery) -> Vec<SourceCandidate> {
    let mut candidates = discovery.candidates.clone();
    if discovery.query.transport == AcquisitionTransport::Web {
        candidates.retain(|candidate| {
            web_candidate_relevance_score(candidate, &discovery.query.text).is_some()
        });
    }
    candidates.sort_by(|left, right| {
        candidate_relevance_score(right, &discovery.query)
            .cmp(&candidate_relevance_score(left, &discovery.query))
            .then_with(|| left.anchor.cmp(&right.anchor))
    });
    if discovery.query.transport == AcquisitionTransport::Workspace
        && workspace_code_trace_pattern(&discovery.query.text)
    {
        return rank_workspace_trace_candidates(candidates, &discovery.query);
    }
    candidates
}

fn rank_discovery_candidates_for_planning(
    discovery: &QueryDiscovery,
    planning: &PlanningResult,
) -> Vec<SourceCandidate> {
    let mut candidates = rank_discovery_candidates(discovery);
    if discovery.query.transport == AcquisitionTransport::Web {
        candidates.retain(|candidate| {
            web_candidate_matches_planning_scope(candidate, &discovery.query, planning)
        });
    }
    candidates
}

fn rank_workspace_trace_candidates(
    candidates: Vec<SourceCandidate>,
    query: &AcquisitionQuery,
) -> Vec<SourceCandidate> {
    const OWNER_ROLES: &[&[&str]] = &[
        &["submit", "submission", "submissionintent"],
        &["researchruntime", "parsedeepresearch", "deepresearchcli"],
        &[
            "appresearchworkflow",
            "startdeepresearchworkflow",
            "researchworkflow",
            "workflow",
            "launch",
        ],
        &[
            "inquiryruntime",
            "bootstrapacquisition",
            "runretrievalstage",
            "retrieval",
            "acceptedevidence",
            "admit",
            "evidenceledger",
        ],
        &["reportgeneration", "sectionedreport", "synthesis"],
        &["publication", "publish", "artifact"],
        &["browser", "openremoteview", "view"],
        &[
            "legacy",
            "inactive",
            "compat",
            "replay",
            "convergence",
            "hostreport",
        ],
    ];
    let mut ranked = Vec::with_capacity(candidates.len());
    let mut selected = BTreeSet::new();
    for role in OWNER_ROLES {
        let best = candidates
            .iter()
            .filter(|candidate| !selected.contains(&candidate.anchor))
            .filter(|candidate| workspace_candidate_matches_role(candidate, role))
            .max_by(|left, right| {
                workspace_candidate_source_priority(left)
                    .cmp(&workspace_candidate_source_priority(right))
                    .then_with(|| {
                        workspace_candidate_role_score(left, role)
                            .cmp(&workspace_candidate_role_score(right, role))
                    })
                    .then_with(|| {
                        candidate_relevance_score(left, query)
                            .cmp(&candidate_relevance_score(right, query))
                    })
                    .then_with(|| right.anchor.cmp(&left.anchor))
            });
        if let Some(candidate) = best {
            selected.insert(candidate.anchor.clone());
            ranked.push(candidate.clone());
        }
    }
    ranked.extend(
        candidates
            .into_iter()
            .filter(|candidate| selected.insert(candidate.anchor.clone())),
    );
    ranked
}

fn workspace_candidate_matches_role(candidate: &SourceCandidate, role: &[&str]) -> bool {
    workspace_candidate_role_score(candidate, role) > 0
}

fn workspace_candidate_role_score(candidate: &SourceCandidate, role: &[&str]) -> usize {
    let path = normalize(&candidate.anchor);
    let preview = normalize(&candidate.preview);
    role.iter()
        .map(|term| usize::from(path.contains(term)) * 2 + usize::from(preview.contains(term)))
        .sum()
}

fn workspace_candidate_source_priority(candidate: &SourceCandidate) -> usize {
    workspace_path_priority(&candidate.anchor.to_ascii_lowercase())
}

fn brief_candidate_score(candidate: &SourceCandidate, query: &AcquisitionQuery) -> usize {
    let preference = query
        .preferred_sources
        .iter()
        .filter_map(|preference| preference_match_score(candidate, preference))
        .max()
        .unwrap_or_default();
    preference * 100_000_000 + candidate_relevance_score(candidate, query)
}

fn candidate_relevance_score(candidate: &SourceCandidate, query: &AcquisitionQuery) -> usize {
    if candidate.transport != AcquisitionTransport::Workspace {
        return web_candidate_relevance_score(candidate, &query.text).unwrap_or_default();
    }

    let provider_score = bounded_provider_score(candidate.provider_score);
    let path = candidate.anchor.to_ascii_lowercase();
    let path_score = workspace_path_priority(&path);
    let terms = distinctive_terms(&query.text);
    let matched_context = candidate.preview.to_ascii_lowercase();
    let path_overlap = terms
        .iter()
        .filter(|term| text_matches_term(&path, term))
        .count()
        .min(6);
    let content_overlap = terms
        .iter()
        .filter(|term| text_matches_term(&matched_context, term))
        .count()
        .min(8);
    let ownership_signals = [
        "runtime",
        "workflow",
        "execution",
        "acquisition",
        "artifact",
        "publication",
        "report",
        "submit",
        "command",
        "launch",
        "browser",
        "inquiry",
        "planning",
        "synthesis",
        "render",
        "dispatch",
        "view",
    ]
    .into_iter()
    .filter(|signal| path.contains(signal))
    .count()
    .min(8);
    let call_site_score = [
        "fn ",
        "async fn ",
        "pub(crate) fn ",
        "pub(super) fn ",
        "spawn_",
        "write_",
        "open_",
    ]
    .into_iter()
    .filter(|signal| candidate.preview.contains(signal))
    .count()
    .min(4)
        * 50_000;
    let matched_context_score = candidate.preview.lines().count().min(8) * 10_000;

    path_score
        + path_overlap * 250_000
        + content_overlap * 400_000
        + ownership_signals * 150_000
        + call_site_score
        + matched_context_score
        + provider_score.min(99_999)
}

fn workspace_path_priority(path: &str) -> usize {
    if workspace_metadata_path(path) {
        0
    } else if workspace_test_path(path) {
        1_000_000
    } else if workspace_barrel_path(path) {
        5_000_000
    } else if path.starts_with("src/") && path.ends_with(".rs") {
        10_000_000
    } else if source_code_path(path) {
        7_000_000
    } else if path.starts_with("docs/") || path.contains("/docs/") {
        2_000_000
    } else {
        4_000_000
    }
}

fn workspace_barrel_path(path: &str) -> bool {
    matches!(path.rsplit('/').next().unwrap_or(path), "mod.rs" | "lib.rs")
}

fn bounded_provider_score(score: f64) -> usize {
    if !score.is_finite() || score <= 0.0 {
        return 0;
    }
    (score.min(1_000.0) * 1_000.0) as usize
}

fn workspace_metadata_path(path: &str) -> bool {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        file_name,
        "cargo.toml"
            | "cargo.lock"
            | "license"
            | "license.md"
            | "license.txt"
            | "readme"
            | "readme.md"
            | "changelog"
            | "changelog.md"
    )
}

fn workspace_test_path(path: &str) -> bool {
    path.starts_with("tests/")
        || path.contains("/tests/")
        || path.contains("/fixtures/")
        || path.ends_with("_test.rs")
        || path.ends_with("_tests.rs")
        || path.ends_with(".snap")
}

fn source_code_path(path: &str) -> bool {
    [".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go"]
        .into_iter()
        .any(|extension| path.ends_with(extension))
}

fn text_matches_term(text: &str, term: &str) -> bool {
    text.contains(term) || normalize(text).contains(&normalize(term))
}

fn preference_match_score(
    candidate: &SourceCandidate,
    preference: &SourcePreference,
) -> Option<usize> {
    match preference.kind {
        PreferredSourceKind::Repository => {
            named_identity_score(&candidate.anchor, "repository", &preference.value).map(|_| 3)
        }
        PreferredSourceKind::Domain => {
            named_identity_score(&candidate.anchor, "domain", &preference.value).map(|_| 2)
        }
        PreferredSourceKind::Url => {
            named_identity_score(&candidate.anchor, "url", &preference.value).map(|_| 4)
        }
        PreferredSourceKind::WorkspacePath => {
            named_identity_score(&candidate.anchor, "workspace_path", &preference.value).map(|_| 4)
        }
    }
}

fn best_candidate(
    candidates: &[SourceCandidate],
    target: &JsonValue,
    used_edges: &BTreeSet<(String, String, String)>,
    query_id: &str,
    target_id: &str,
) -> Option<(SourceCandidate, usize)> {
    candidates
        .iter()
        .filter(|candidate| {
            !used_edges.contains(&(
                query_id.to_string(),
                target_id.to_string(),
                candidate.anchor.clone(),
            ))
        })
        .filter_map(|candidate| {
            candidate_match_score(candidate, target).map(|score| (candidate.clone(), score))
        })
        .max_by_key(|(_, score)| *score)
}

fn candidate_match_score(candidate: &SourceCandidate, target: &JsonValue) -> Option<usize> {
    let policy = &target["match_policy"];
    let provider_score = (candidate.provider_score.max(0.0) * 1_000.0) as usize;
    match policy["kind"].as_str()? {
        "named" => {
            let identity = &policy["identity"];
            let value = identity["value"].as_str()?;
            let identity_score =
                named_identity_score(&candidate.anchor, identity["kind"].as_str()?, value)?;
            Some(identity_score * 1_000_000 + provider_score)
        }
        "exploratory" => {
            let goal = policy["selection_goal"].as_str()?;
            let searchable = format!(
                "{} {} {}",
                candidate.title, candidate.anchor, candidate.preview
            )
            .to_ascii_lowercase();
            let terms = distinctive_terms(goal);
            let overlap = terms
                .iter()
                .filter(|term| searchable.contains(term.as_str()))
                .count();
            Some((overlap + 1) * 100_000 + provider_score)
        }
        _ => None,
    }
}

fn named_identity_score(anchor: &str, kind: &str, value: &str) -> Option<usize> {
    let anchor_lower = anchor.trim().trim_end_matches('/').to_ascii_lowercase();
    let value_lower = value.trim().trim_end_matches('/').to_ascii_lowercase();
    match kind {
        "repository" => {
            let repository_path = format!("github.com/{value_lower}");
            if anchor_lower.contains(&repository_path) {
                Some(10)
            } else {
                let project = value_lower.split('/').next_back()?;
                registry_project(&anchor_lower)
                    .is_some_and(|candidate| normalize(candidate) == normalize(project))
                    .then_some(8)
            }
        }
        "domain" => {
            let host = anchor_host(&anchor_lower)?;
            (host == value_lower || host.ends_with(&format!(".{value_lower}"))).then_some(10)
        }
        "url" => (anchor_lower == value_lower
            || anchor_lower.starts_with(&format!("{value_lower}/")))
        .then_some(10),
        "workspace_path" => (anchor_lower == value_lower
            || anchor_lower.starts_with(&format!("{value_lower}/")))
        .then_some(10),
        _ => None,
    }
}

fn registry_project(anchor: &str) -> Option<&str> {
    let remainder = anchor.split_once("://").map(|(_, value)| value)?;
    let (host, path) = remainder.split_once('/')?;
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    match (host, parts.as_slice()) {
        ("docs.rs", ["crate", project, ..]) => Some(*project),
        ("docs.rs", [project, ..]) => Some(*project),
        ("crates.io", ["crates", project, ..]) => Some(*project),
        _ => None,
    }
}

fn anchor_host(anchor: &str) -> Option<String> {
    reqwest::Url::parse(anchor)
        .ok()?
        .host_str()
        .map(|host| host.trim_start_matches("www.").to_ascii_lowercase())
}

fn distinctive_terms(value: &str) -> Vec<String> {
    const GENERIC: [&str; 12] = [
        "official",
        "source",
        "documentation",
        "document",
        "primary",
        "evidence",
        "repository",
        "workspace",
        "research",
        "current",
        "information",
        "material",
    ];
    value
        .split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|term| term.chars().count() >= 3)
        .map(str::to_ascii_lowercase)
        .filter(|term| !GENERIC.contains(&term.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn web_candidate_matches_planning_scope(
    candidate: &SourceCandidate,
    query: &AcquisitionQuery,
    planning: &PlanningResult,
) -> bool {
    let mut scopes = planning
        .brief
        .as_ref()
        .into_iter()
        .flat_map(|brief| brief.dimensions.iter())
        .filter(|dimension| query.dimension_ids.contains(&dimension.id))
        .map(|dimension| dimension.request_scope())
        .collect::<Vec<_>>();
    if scopes.is_empty() {
        scopes.push(planning.planner_input.query.clone());
    }
    if scopes
        .iter()
        .any(|scope| web_candidate_relevance_score(candidate, scope).is_some())
    {
        return true;
    }
    query_requests_canonical_project_record(&query.text)
        && canonical_web_project(&candidate.anchor).is_some_and(|project| {
            let project = normalize(&project);
            scopes
                .iter()
                .any(|scope| web_query_terms(scope).iter().any(|term| term == &project))
        })
}

fn query_requests_canonical_project_record(query: &str) -> bool {
    let normalized = normalize(query);
    [
        "cargo",
        "documentation",
        "docs",
        "lts",
        "msrv",
        "readme",
        "release",
        "repository",
        "source",
    ]
    .into_iter()
    .any(|signal| normalized.contains(signal))
}

fn canonical_web_project(anchor: &str) -> Option<String> {
    if let Some(project) = registry_project(anchor) {
        return Some(project.to_string());
    }
    let url = reqwest::Url::parse(anchor).ok()?;
    if url.host_str()?.trim_start_matches("www.") != "github.com" {
        return None;
    }
    url.path_segments()?
        .filter(|segment| !segment.is_empty())
        .nth(1)
        .map(str::to_string)
}

fn web_candidate_relevance_score(candidate: &SourceCandidate, query: &str) -> Option<usize> {
    let ordered_terms = web_query_terms(query);
    let unique_terms = ordered_terms.iter().cloned().collect::<BTreeSet<_>>();
    let searchable = normalize(&format!(
        "{} {} {}",
        candidate.title, candidate.anchor, candidate.preview
    ));
    let matched_terms = unique_terms
        .iter()
        .filter(|term| searchable.contains(term.as_str()))
        .count();
    let phrase_match = ordered_terms.windows(2).any(|terms| {
        let [left, right] = terms else {
            return false;
        };
        searchable.contains(&format!("{left}{right}"))
    });
    let strict_gate = unique_terms.len() >= 5
        && !query
            .chars()
            .any(|character| character.is_alphabetic() && !character.is_ascii());
    let minimum_matches = unique_terms.len().div_ceil(3).clamp(3, 5);
    // A phrase hit improves ranking but cannot admit a source on its own. Name
    // collisions and split-topic pages often repeat one requested phrase while
    // omitting the other evidence facets.
    if strict_gate && matched_terms < minimum_matches {
        return None;
    }

    Some(
        usize::from(phrase_match) * 100_000_000
            + matched_terms * 1_000_000
            + bounded_provider_score(candidate.provider_score),
    )
}

fn web_query_terms(value: &str) -> Vec<String> {
    const GENERIC: &[&str] = &[
        "about",
        "and",
        "are",
        "as",
        "at",
        "be",
        "by",
        "can",
        "canonical",
        "compare",
        "cover",
        "current",
        "date",
        "determine",
        "distinguish",
        "does",
        "each",
        "evaluation",
        "find",
        "for",
        "from",
        "how",
        "identify",
        "in",
        "information",
        "into",
        "is",
        "official",
        "of",
        "on",
        "or",
        "page",
        "recommend",
        "report",
        "research",
        "source",
        "sources",
        "state",
        "that",
        "the",
        "this",
        "to",
        "use",
        "what",
        "when",
        "where",
        "which",
        "with",
    ];
    value
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '-' | '_' | '.')
        })
        .map(normalize)
        .filter(|term| term.len() >= 3)
        .filter(|term| !GENERIC.contains(&term.as_str()))
        .collect()
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn merge_or_push(
    selected: &mut Vec<SelectedCandidate>,
    candidate: SourceCandidate,
    edge: SelectionEdge,
    maximum: usize,
) {
    if let Some(existing) = selected
        .iter_mut()
        .find(|selected| selected.candidate.anchor == candidate.anchor)
    {
        if !existing.edges.contains(&edge) {
            existing.edges.push(edge);
        }
        return;
    }
    if selected.len() < maximum {
        selected.push(SelectedCandidate {
            candidate,
            edges: vec![edge],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::code::research_runtime::tests::baseline::live::corpus::{
        AcquisitionTransport, EvidenceScope, PlannerBudget, PlannerInput,
    };
    use crate::commands::code::research_runtime::tests::baseline::live::planning::{
        AcquisitionQuery, BriefDimension, EvaluationStrategy, PlanningResult, PreferredSourceKind,
        ResearchBrief, SourcePreference,
    };

    fn planning(spec: JsonValue) -> PlanningResult {
        PlanningResult {
            strategy: EvaluationStrategy::Compiler,
            planner_input: PlannerInput {
                schema: "test".to_string(),
                query: "test".to_string(),
                report_language: "en".to_string(),
                current_date: "2026-07-21".to_string(),
                display_utc_offset: "+08:00".to_string(),
                evidence_scope: EvidenceScope::Web,
                budget: PlannerBudget {
                    max_queries: 1,
                    max_acquired_sources: 2,
                },
            },
            prompt: String::new(),
            proposal: JsonValue::Null,
            brief: None,
            spec: Some(spec),
            plan: None,
            queries: vec![AcquisitionQuery {
                id: "q1".to_string(),
                text: "project".to_string(),
                transport: AcquisitionTransport::Web,
                path: String::new(),
                glob: String::new(),
                dimension_ids: vec!["d1".to_string()],
                source_target_ids: vec!["t1".to_string()],
                preferred_sources: Vec::new(),
                fetch_slots: 2,
            }],
            elapsed_ms: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            repair_rounds: 0,
            mode_used: "test".to_string(),
        }
    }

    fn brief_planning(
        transport: AcquisitionTransport,
        preferences: Vec<SourcePreference>,
        fetch_slots: usize,
    ) -> PlanningResult {
        let query = AcquisitionQuery {
            id: "q1".to_string(),
            text: "official project runtime behavior".to_string(),
            transport,
            path: String::new(),
            glob: String::new(),
            dimension_ids: vec!["d1".to_string()],
            source_target_ids: Vec::new(),
            preferred_sources: preferences,
            fetch_slots,
        };
        PlanningResult {
            strategy: EvaluationStrategy::Brief,
            planner_input: PlannerInput {
                schema: "test".to_string(),
                query: "test".to_string(),
                report_language: "en".to_string(),
                current_date: "2026-07-21".to_string(),
                display_utc_offset: "+08:00".to_string(),
                evidence_scope: match transport {
                    AcquisitionTransport::Web => EvidenceScope::Web,
                    AcquisitionTransport::Workspace => EvidenceScope::Workspace,
                },
                budget: PlannerBudget {
                    max_queries: 2,
                    max_acquired_sources: 2,
                },
            },
            prompt: String::new(),
            proposal: JsonValue::Null,
            brief: Some(ResearchBrief {
                dimensions: vec![BriefDimension {
                    id: "d1".to_string(),
                    question: "What does the official project establish?".to_string(),
                    request_basis: vec!["test".to_string()],
                    material: true,
                }],
                queries: vec![query.clone()],
                planning_gaps: Vec::new(),
                normalization_notes: Vec::new(),
            }),
            spec: None,
            plan: None,
            queries: vec![query],
            elapsed_ms: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            repair_rounds: 0,
            mode_used: "test".to_string(),
        }
    }

    #[test]
    fn named_target_selection_rejects_high_ranked_cross_project_noise() {
        let spec = serde_json::json!({
            "source_targets": [{
                "id": "t1",
                "match_policy": {
                    "kind": "named",
                    "identity": { "kind": "repository", "value": "owner/project" }
                }
            }]
        });
        let query = planning(spec.clone()).queries[0].clone();
        let discoveries = vec![QueryDiscovery {
            query,
            candidates: vec![
                SourceCandidate {
                    title: "Noise".to_string(),
                    anchor: "https://example.test/noise".to_string(),
                    preview: String::new(),
                    provider_score: 100.0,
                    transport: AcquisitionTransport::Web,
                },
                SourceCandidate {
                    title: "Project".to_string(),
                    anchor: "https://github.com/owner/project".to_string(),
                    preview: String::new(),
                    provider_score: 0.1,
                    transport: AcquisitionTransport::Web,
                },
            ],
            error: None,
            elapsed_ms: 0,
        }];
        let selected = select_candidates(&planning(spec), &discoveries, 2);
        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].candidate.anchor,
            "https://github.com/owner/project"
        );
        assert_eq!(selected[0].edges[0].source_target_id.as_deref(), Some("t1"));
    }

    #[test]
    fn brief_preference_beats_high_ranked_unrelated_noise() {
        let planning = brief_planning(
            AcquisitionTransport::Web,
            vec![SourcePreference {
                kind: PreferredSourceKind::Repository,
                value: "owner/project".to_string(),
            }],
            1,
        );
        let discoveries = vec![QueryDiscovery {
            query: planning.queries[0].clone(),
            candidates: vec![
                SourceCandidate {
                    title: "Best unrelated project".to_string(),
                    anchor: "https://example.test/noise".to_string(),
                    preview: "official project runtime behavior".to_string(),
                    provider_score: 100.0,
                    transport: AcquisitionTransport::Web,
                },
                SourceCandidate {
                    title: "Project".to_string(),
                    anchor: "https://github.com/owner/project".to_string(),
                    preview: String::new(),
                    provider_score: 0.01,
                    transport: AcquisitionTransport::Web,
                },
            ],
            error: None,
            elapsed_ms: 0,
        }];
        let selected = select_candidates(&planning, &discoveries, 1);
        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].candidate.anchor,
            "https://github.com/owner/project"
        );
    }

    #[test]
    fn domain_hint_ranks_matching_source_without_dropping_other_candidates() {
        let mut planning = brief_planning(
            AcquisitionTransport::Web,
            vec![SourcePreference {
                kind: PreferredSourceKind::Domain,
                value: "tokio.rs".to_string(),
            }],
            2,
        );
        planning.queries[0].text = "Tokio LTS release policy".to_string();
        let discoveries = vec![QueryDiscovery {
            query: planning.queries[0].clone(),
            candidates: vec![
                SourceCandidate {
                    title: "Historical Tokio announcement".to_string(),
                    anchor: "https://tokio.rs/blog/old".to_string(),
                    preview: "Tokio release policy".to_string(),
                    provider_score: 1.0,
                    transport: AcquisitionTransport::Web,
                },
                SourceCandidate {
                    title: "Tokio releases".to_string(),
                    anchor: "https://github.com/tokio-rs/tokio/releases".to_string(),
                    preview: "Tokio LTS release policy".to_string(),
                    provider_score: 0.1,
                    transport: AcquisitionTransport::Web,
                },
            ],
            error: None,
            elapsed_ms: 0,
        }];
        let selected = select_candidates(&planning, &discoveries, 2);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].candidate.anchor, "https://tokio.rs/blog/old");
        assert!(selected
            .iter()
            .any(|selected| selected.candidate.anchor
                == "https://github.com/tokio-rs/tokio/releases"));
    }

    #[test]
    fn canonical_repository_hint_ranks_canonical_source_without_hard_filtering() {
        let mut planning = brief_planning(
            AcquisitionTransport::Web,
            vec![SourcePreference {
                kind: PreferredSourceKind::Repository,
                value: "tokio-rs/tokio".to_string(),
            }],
            2,
        );
        planning.queries[0].text = "Tokio LTS release policy".to_string();
        let discoveries = vec![QueryDiscovery {
            query: planning.queries[0].clone(),
            candidates: vec![
                SourceCandidate {
                    title: "Forked Tokio policy".to_string(),
                    anchor: "https://github.com/dfoxfranke/tokio/blob/doc-links/ROADMAP.md"
                        .to_string(),
                    preview: "Tokio LTS release policy".to_string(),
                    provider_score: 100.0,
                    transport: AcquisitionTransport::Web,
                },
                SourceCandidate {
                    title: "Canonical Tokio policy".to_string(),
                    anchor: "https://github.com/tokio-rs/tokio/blob/master/README.md".to_string(),
                    preview: "Tokio LTS release policy".to_string(),
                    provider_score: 0.1,
                    transport: AcquisitionTransport::Web,
                },
            ],
            error: None,
            elapsed_ms: 0,
        }];
        let selected = select_candidates(&planning, &discoveries, 2);
        assert_eq!(selected.len(), 2);
        assert_eq!(
            selected[0].candidate.anchor,
            "https://github.com/tokio-rs/tokio/blob/master/README.md"
        );
    }

    #[test]
    fn brief_duplicate_candidate_is_merged_and_backfilled() {
        let mut planning = brief_planning(AcquisitionTransport::Web, Vec::new(), 1);
        let mut q2 = planning.queries[0].clone();
        q2.id = "q2".to_string();
        planning.queries.push(q2.clone());
        planning
            .brief
            .as_mut()
            .expect("brief")
            .queries
            .push(q2.clone());
        let shared = SourceCandidate {
            title: "Official shared record".to_string(),
            anchor: "https://example.test/shared".to_string(),
            preview: "official project runtime behavior".to_string(),
            provider_score: 1.0,
            transport: AcquisitionTransport::Web,
        };
        let discoveries = vec![
            QueryDiscovery {
                query: planning.queries[0].clone(),
                candidates: vec![shared.clone()],
                error: None,
                elapsed_ms: 0,
            },
            QueryDiscovery {
                query: q2,
                candidates: vec![
                    shared,
                    SourceCandidate {
                        title: "Second official record".to_string(),
                        anchor: "https://example.test/second".to_string(),
                        preview: "official project runtime behavior".to_string(),
                        provider_score: 0.5,
                        transport: AcquisitionTransport::Web,
                    },
                ],
                error: None,
                elapsed_ms: 0,
            },
        ];
        let selected = select_candidates(&planning, &discoveries, 2);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].edges.len(), 2);
        assert!(selected
            .iter()
            .any(|selected| selected.candidate.anchor == "https://example.test/second"));
    }

    #[test]
    fn brief_workspace_selection_prefers_owning_source_over_docs() {
        let planning = brief_planning(
            AcquisitionTransport::Workspace,
            vec![SourcePreference {
                kind: PreferredSourceKind::WorkspacePath,
                value: "src".to_string(),
            }],
            1,
        );
        let discoveries = vec![QueryDiscovery {
            query: planning.queries[0].clone(),
            candidates: vec![
                SourceCandidate {
                    title: "docs/design.md".to_string(),
                    anchor: "docs/design.md".to_string(),
                    preview: "official project runtime behavior".to_string(),
                    provider_score: 100.0,
                    transport: AcquisitionTransport::Workspace,
                },
                SourceCandidate {
                    title: "src/runtime.rs".to_string(),
                    anchor: "src/runtime.rs".to_string(),
                    preview: "fn runtime_behavior()".to_string(),
                    provider_score: 0.01,
                    transport: AcquisitionTransport::Workspace,
                },
            ],
            error: None,
            elapsed_ms: 0,
        }];
        let selected = select_candidates(&planning, &discoveries, 1);
        assert_eq!(selected[0].candidate.anchor, "src/runtime.rs");
    }

    #[test]
    fn web_selection_requires_query_facets_to_coexist_in_one_source() {
        let query = "Find verified private production incident rates for Tokio and async-std in Chinese financial institutions and determine which runtime causes fewer incidents.";
        let candidate =
            |title: &str, anchor: &str, preview: &str, provider_score: f64| SourceCandidate {
                title: title.to_string(),
                anchor: anchor.to_string(),
                preview: preview.to_string(),
                provider_score,
                transport: AcquisitionTransport::Web,
            };
        for strategy in [EvaluationStrategy::Minimal, EvaluationStrategy::Brief] {
            let mut planning = brief_planning(AcquisitionTransport::Web, Vec::new(), 4);
            planning.strategy = strategy;
            planning.queries[0].text = query.to_string();
            let discoveries = vec![QueryDiscovery {
                query: planning.queries[0].clone(),
                candidates: vec![
                    candidate(
                        "Tokio vs async-std",
                        "https://www.youtube.com/watch",
                        "Rust async runtimes and production choices",
                        100.0,
                    ),
                    candidate(
                        "China private manufacturing survey",
                        "https://news.example/china-private-survey",
                        "Private production activity at Chinese manufacturers",
                        99.0,
                    ),
                    candidate(
                        "Rust Async Programming: Tokio, async-std, and Patterns",
                        "https://blog.example/rust-async-patterns",
                        "Learn how to choose between Tokio and async-std for Rust async programming",
                        98.0,
                    ),
                    candidate(
                        "Verified Tokio and async-std incident rates",
                        "https://bank.example/runtime-incidents",
                        "Verified production incident rates for Tokio and async-std in Chinese financial institutions",
                        0.01,
                    ),
                ],
                error: None,
                elapsed_ms: 0,
            }];

            let selected = select_candidates(&planning, &discoveries, 4);
            assert_eq!(
                selected
                    .iter()
                    .map(|selected| selected.candidate.anchor.as_str())
                    .collect::<Vec<_>>(),
                ["https://bank.example/runtime-incidents"],
                "{strategy:?}"
            );
        }
    }

    #[test]
    fn web_ranking_keeps_canonical_release_identity_and_drops_name_collisions() {
        let mut query = brief_planning(AcquisitionTransport::Web, Vec::new(), 2).queries[0].clone();
        query.text =
            "Tokio stable releases page newest non-LTS version tokio-rs/tokio releases".to_string();
        let ranked = rank_discovery_candidates(&QueryDiscovery {
            query,
            candidates: vec![
                SourceCandidate {
                    title: "Tokio (band)".to_string(),
                    anchor: "https://en.wikipedia.org/wiki/Tokio_(band)".to_string(),
                    preview: "A Japanese rock and pop band".to_string(),
                    provider_score: 100.0,
                    transport: AcquisitionTransport::Web,
                },
                SourceCandidate {
                    title: "Releases · tokio-rs/tokio".to_string(),
                    anchor: "https://github.com/tokio-rs/tokio/releases".to_string(),
                    preview: "Tokio v1.53.1 Latest".to_string(),
                    provider_score: 0.01,
                    transport: AcquisitionTransport::Web,
                },
            ],
            error: None,
            elapsed_ms: 0,
        });
        assert_eq!(ranked.len(), 1);
        assert_eq!(
            ranked[0].anchor,
            "https://github.com/tokio-rs/tokio/releases"
        );
    }

    #[test]
    fn web_selection_keeps_the_root_scope_across_refinement_queries() {
        let root = "Find verified private production incident rates for Tokio and async-std in Chinese financial institutions and determine which runtime causes fewer incidents.";
        let candidate =
            |title: &str, anchor: &str, preview: &str, provider_score: f64| SourceCandidate {
                title: title.to_string(),
                anchor: anchor.to_string(),
                preview: preview.to_string(),
                provider_score,
                transport: AcquisitionTransport::Web,
            };
        for strategy in [EvaluationStrategy::Minimal, EvaluationStrategy::Brief] {
            let mut planning = brief_planning(AcquisitionTransport::Web, Vec::new(), 4);
            planning.strategy = strategy;
            planning.planner_input.query = root.to_string();
            let brief = planning.brief.as_mut().expect("root brief");
            brief.dimensions[0].question = root.to_string();
            brief.dimensions[0].request_basis = vec![root.to_string()];
            let mut tokio_query = planning.queries[0].clone();
            tokio_query.id = "q1".to_string();
            tokio_query.text =
                "Tokio runtime production incidents Rust Chinese financial institutions case study"
                    .to_string();
            let mut async_std_query = tokio_query.clone();
            async_std_query.id = "q2".to_string();
            async_std_query.text =
                "async-std Rust runtime reliability incident report China banking fintech production"
                    .to_string();
            planning.queries = vec![tokio_query.clone(), async_std_query.clone()];
            let discoveries = vec![
                QueryDiscovery {
                    query: tokio_query,
                    candidates: vec![candidate(
                        "Rust in Production: Fintech API Case Study",
                        "https://example.test/rust-fintech-case-study",
                        "Rust migration case study for a fintech production API",
                        1.0,
                    )],
                    error: None,
                    elapsed_ms: 0,
                },
                QueryDiscovery {
                    query: async_std_query,
                    candidates: vec![candidate(
                        "The End of async-std",
                        "https://example.test/end-of-async-std",
                        "Rust async runtime guidance for production users",
                        0.5,
                    )],
                    error: None,
                    elapsed_ms: 0,
                },
            ];

            assert!(
                select_candidates(&planning, &discoveries, 4).is_empty(),
                "{strategy:?}"
            );
        }
    }

    #[test]
    fn web_selection_allows_a_canonical_project_page_to_cover_one_root_facet() {
        let root = "As of the evaluation date, what Tokio LTS branches are supported, when does each support window end, and what MSRV does each branch declare? Use the canonical Tokio source and distinguish LTS information from the newest non-LTS release.";
        for strategy in [EvaluationStrategy::Minimal, EvaluationStrategy::Brief] {
            let mut planning = brief_planning(AcquisitionTransport::Web, Vec::new(), 2);
            planning.strategy = strategy;
            planning.planner_input.query = root.to_string();
            let brief = planning.brief.as_mut().expect("root brief");
            brief.dimensions[0].question = root.to_string();
            brief.dimensions[0].request_basis = vec![root.to_string()];
            planning.queries[0].text =
                "Tokio stable releases page newest non-LTS version tokio-rs/tokio releases"
                    .to_string();
            let discoveries = vec![QueryDiscovery {
                query: planning.queries[0].clone(),
                candidates: vec![
                    SourceCandidate {
                        title: "Tokio (band)".to_string(),
                        anchor: "https://en.wikipedia.org/wiki/Tokio_(band)".to_string(),
                        preview: "A Japanese rock and pop band".to_string(),
                        provider_score: 100.0,
                        transport: AcquisitionTransport::Web,
                    },
                    SourceCandidate {
                        title: "Tokyo".to_string(),
                        anchor: "https://en.wikipedia.org/wiki/Tokyo".to_string(),
                        preview: "Capital and most populous city in Japan".to_string(),
                        provider_score: 99.0,
                        transport: AcquisitionTransport::Web,
                    },
                    SourceCandidate {
                        title: "Tokio.".to_string(),
                        anchor: "https://www.tokiotokio.com/".to_string(),
                        preview: "A creative studio".to_string(),
                        provider_score: 98.0,
                        transport: AcquisitionTransport::Web,
                    },
                    SourceCandidate {
                        title: "Home | Tokyo Tokyo Official Website".to_string(),
                        anchor: "https://tokyotokyo.jp/home".to_string(),
                        preview: "Travel information for Tokyo".to_string(),
                        provider_score: 97.0,
                        transport: AcquisitionTransport::Web,
                    },
                    SourceCandidate {
                        title: "TOKIO - Updated July 2026".to_string(),
                        anchor: "https://www.yelp.com/biz/tokio-denver-3".to_string(),
                        preview: "Restaurant reviews and photos".to_string(),
                        provider_score: 96.0,
                        transport: AcquisitionTransport::Web,
                    },
                    SourceCandidate {
                        title: "Releases · tokio-rs/tokio".to_string(),
                        anchor: "https://github.com/tokio-rs/tokio/releases".to_string(),
                        preview: "Tokio v1.53.1 Latest".to_string(),
                        provider_score: 0.01,
                        transport: AcquisitionTransport::Web,
                    },
                ],
                error: None,
                elapsed_ms: 0,
            }];

            let selected = select_candidates(&planning, &discoveries, 2);
            assert_eq!(selected.len(), 1, "{strategy:?}");
            assert_eq!(
                selected[0].candidate.anchor, "https://github.com/tokio-rs/tokio/releases",
                "{strategy:?}"
            );
        }
    }

    #[test]
    fn mixed_language_web_query_ranks_context_without_hard_filtering() {
        let mut query = brief_planning(AcquisitionTransport::Web, Vec::new(), 1).queries[0].clone();
        query.text = "比较 Tokio 与 async-std 的 HTTP 生态兼容性".to_string();
        let ranked = rank_discovery_candidates(&QueryDiscovery {
            query,
            candidates: vec![SourceCandidate {
                title: "Official runtime compatibility".to_string(),
                anchor: "https://example.test/runtime-compatibility".to_string(),
                preview: "Tokio async-std HTTP compatibility".to_string(),
                provider_score: 0.0,
                transport: AcquisitionTransport::Web,
            }],
            error: None,
            elapsed_ms: 0,
        });
        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn workspace_selection_ranks_production_owners_above_metadata_and_tests() {
        for strategy in [EvaluationStrategy::Minimal, EvaluationStrategy::Brief] {
            let mut planning = brief_planning(AcquisitionTransport::Workspace, Vec::new(), 2);
            planning.strategy = strategy;
            planning.queries[0].text = "deep research submission artifact publication".to_string();
            let discoveries = vec![QueryDiscovery {
                query: planning.queries[0].clone(),
                candidates: vec![
                    SourceCandidate {
                        title: "Cargo.toml".to_string(),
                        anchor: "Cargo.toml".to_string(),
                        preview: "description = deep research submission".to_string(),
                        provider_score: 100.0,
                        transport: AcquisitionTransport::Workspace,
                    },
                    SourceCandidate {
                        title: "LICENSE".to_string(),
                        anchor: "LICENSE".to_string(),
                        preview: "research software".to_string(),
                        provider_score: 99.0,
                        transport: AcquisitionTransport::Workspace,
                    },
                    SourceCandidate {
                        title: "tests/deep_research_tests.rs".to_string(),
                        anchor: "tests/deep_research_tests.rs".to_string(),
                        preview: "fn submit_and_publish_report()".to_string(),
                        provider_score: 98.0,
                        transport: AcquisitionTransport::Workspace,
                    },
                    SourceCandidate {
                        title: "src/tui/app/submit.rs".to_string(),
                        anchor: "src/tui/app/submit.rs".to_string(),
                        preview: "fn submit_deep_research()".to_string(),
                        provider_score: 0.01,
                        transport: AcquisitionTransport::Workspace,
                    },
                    SourceCandidate {
                        title: "src/tui/deep_research/artifacts/publication.rs".to_string(),
                        anchor: "src/tui/deep_research/artifacts/publication.rs".to_string(),
                        preview: "fn write_research_report_pair()".to_string(),
                        provider_score: 0.001,
                        transport: AcquisitionTransport::Workspace,
                    },
                ],
                error: None,
                elapsed_ms: 0,
            }];

            let selected = select_candidates(&planning, &discoveries, 2);
            let anchors = selected
                .iter()
                .map(|selected| selected.candidate.anchor.as_str())
                .collect::<Vec<_>>();
            assert!(anchors.contains(&"src/tui/app/submit.rs"), "{anchors:?}");
            assert!(
                anchors.contains(&"src/tui/deep_research/artifacts/publication.rs"),
                "{anchors:?}"
            );
        }
    }

    #[test]
    fn workspace_ranking_prefers_matched_transition_code_over_path_only_inventory() {
        let planning = brief_planning(AcquisitionTransport::Workspace, Vec::new(), 1);
        let mut query = planning.queries[0].clone();
        query.text = "deep research report publication browser opening inactive legacy".to_string();
        let ranked = rank_discovery_candidates(&QueryDiscovery {
            query,
            candidates: vec![
                SourceCandidate {
                    title: "src/tui/deep_research/report_generation.rs".to_string(),
                    anchor: "src/tui/deep_research/report_generation.rs".to_string(),
                    preview: String::new(),
                    provider_score: 100.0,
                    transport: AcquisitionTransport::Workspace,
                },
                SourceCandidate {
                    title: "src/tui/app/view.rs".to_string(),
                    anchor: "src/tui/app/view.rs".to_string(),
                    preview: "pub(super) fn open_pending_deep_research_report_view() {\n    open_remote_view_in_browser();\n}".to_string(),
                    provider_score: 0.01,
                    transport: AcquisitionTransport::Workspace,
                },
            ],
            error: None,
            elapsed_ms: 0,
        });
        assert_eq!(ranked[0].anchor, "src/tui/app/view.rs");
    }

    #[test]
    fn workspace_trace_ranking_reserves_one_owner_for_each_transition_role() {
        let planning = brief_planning(AcquisitionTransport::Workspace, Vec::new(), 8);
        let mut query = planning.queries[0].clone();
        query.text = "deep[_-]?research|cli|tui|submission|acquisition|evidence|report|artifact|publication|browser|opening|inactive|legacy".to_string();
        let candidate = |anchor: &str, preview: &str| SourceCandidate {
            title: anchor.to_string(),
            anchor: anchor.to_string(),
            preview: preview.to_string(),
            provider_score: 0.0,
            transport: AcquisitionTransport::Workspace,
        };
        let ranked = rank_discovery_candidates(&QueryDiscovery {
            query,
            candidates: vec![
                candidate(
                    "src/tui/mod.rs",
                    "submission workflow evidence report publication browser legacy",
                ),
                candidate("src/tui/app/submit.rs", "enum SubmissionIntent"),
                candidate(
                    "src/commands/code/research_runtime.rs",
                    "fn parse_deepresearch_args()",
                ),
                candidate(
                    "src/tui/deep_research/host_workflow.rs",
                    "fn deep_research_workflow_source()",
                ),
                candidate(
                    "src/tui/app/research_workflow.rs",
                    "fn start_deep_research_workflow()",
                ),
                candidate(
                    "src/commands/code/research_runtime/tests/baseline/live/acquisition/mod.rs",
                    "bootstrap_acquisition inquiry_runtime retrieval accepted_evidence admit evidence_ledger",
                ),
                candidate(
                    "src/tui/deep_research/inquiry_runtime/execution/tools.rs",
                    "fn prepare_question_evidence_packet()",
                ),
                candidate(
                    "src/tui/deep_research/report_generation.rs",
                    "fn start_report_generation()",
                ),
                candidate(
                    "src/tui/deep_research/artifacts/publication.rs",
                    "fn publish_report_artifacts()",
                ),
                candidate("src/tui/app/view.rs", "fn open_remote_view_in_browser()"),
                candidate(
                    "src/tui/deep_research/host_report.rs",
                    "legacy checked-loop compatibility",
                ),
            ],
            error: None,
            elapsed_ms: 0,
        });
        assert_eq!(
            ranked
                .iter()
                .take(8)
                .map(|candidate| candidate.anchor.as_str())
                .collect::<Vec<_>>(),
            vec![
                "src/tui/app/submit.rs",
                "src/commands/code/research_runtime.rs",
                "src/tui/app/research_workflow.rs",
                "src/tui/deep_research/inquiry_runtime/execution/tools.rs",
                "src/tui/deep_research/report_generation.rs",
                "src/tui/deep_research/artifacts/publication.rs",
                "src/tui/app/view.rs",
                "src/tui/deep_research/host_report.rs",
            ]
        );
    }

    #[test]
    fn minimal_workspace_trace_root_closes_budget_before_followup_noise() {
        let mut planning = brief_planning(AcquisitionTransport::Workspace, Vec::new(), 8);
        planning.strategy = EvaluationStrategy::Minimal;
        planning.planner_input.budget.max_acquired_sources = 8;

        let mut root_query = planning.queries[0].clone();
        root_query.id = "query.bootstrap".to_string();
        root_query.text = "deep[_-]?research|cli|tui|submission|acquisition|evidence|report|artifact|publication|browser|opening|inactive|legacy".to_string();
        root_query.fetch_slots = 8;

        let candidate = |anchor: &str, preview: &str, provider_score: f64| SourceCandidate {
            title: anchor.to_string(),
            anchor: anchor.to_string(),
            preview: preview.to_string(),
            provider_score,
            transport: AcquisitionTransport::Workspace,
        };
        let root_candidates = vec![
            candidate("src/tui/app/submit.rs", "enum SubmissionIntent", 0.0),
            candidate(
                "src/commands/code/research_runtime.rs",
                "fn parse_deepresearch_args()",
                0.0,
            ),
            candidate(
                "src/tui/app/research_workflow.rs",
                "fn start_deep_research_workflow()",
                0.0,
            ),
            candidate(
                "src/tui/deep_research/inquiry_runtime/execution/tools.rs",
                "fn prepare_question_evidence_packet()",
                0.0,
            ),
            candidate(
                "src/tui/deep_research/report_generation.rs",
                "fn start_report_generation()",
                0.0,
            ),
            candidate(
                "src/tui/deep_research/artifacts/publication.rs",
                "fn publish_report_artifacts()",
                0.0,
            ),
            candidate(
                "src/tui/app/view.rs",
                "fn open_pending_deep_research_report_view()",
                0.0,
            ),
            candidate(
                "src/tui/deep_research/host_report.rs",
                "legacy checked-loop compatibility",
                0.0,
            ),
        ];

        let mut followup_query = planning.queries[0].clone();
        followup_query.id = "query.followup".to_string();
        followup_query.text = "narrow model-authored follow-up".to_string();
        followup_query.fetch_slots = 8;
        let followup_candidates = (1..=8)
            .map(|index| {
                candidate(
                    &format!("src/noise/high-score-{index}.rs"),
                    "fn unrelated_but_highly_ranked()",
                    1_000.0,
                )
            })
            .collect::<Vec<_>>();

        let selected = select_candidates(
            &planning,
            &[
                QueryDiscovery {
                    query: root_query,
                    candidates: root_candidates,
                    error: None,
                    elapsed_ms: 0,
                },
                QueryDiscovery {
                    query: followup_query,
                    candidates: followup_candidates,
                    error: None,
                    elapsed_ms: 0,
                },
            ],
            8,
        );

        assert_eq!(
            selected
                .iter()
                .map(|selected| selected.candidate.anchor.as_str())
                .collect::<Vec<_>>(),
            vec![
                "src/tui/app/submit.rs",
                "src/commands/code/research_runtime.rs",
                "src/tui/app/research_workflow.rs",
                "src/tui/deep_research/inquiry_runtime/execution/tools.rs",
                "src/tui/deep_research/report_generation.rs",
                "src/tui/deep_research/artifacts/publication.rs",
                "src/tui/app/view.rs",
                "src/tui/deep_research/host_report.rs",
            ]
        );
        assert!(selected.iter().all(|selected| selected
            .edges
            .iter()
            .all(|edge| edge.query_id == "query.bootstrap")));
    }
}
