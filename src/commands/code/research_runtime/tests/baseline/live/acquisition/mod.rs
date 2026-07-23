mod selection;

use super::corpus::{AcquisitionTransport, EvidenceScope, LiveBudget, PlannerInput};
use super::planning::{
    AcquisitionQuery, EvaluationStrategy, PlanningResult, PreferredSourceKind, SourcePreference,
};
use a3s_code_core::{AgentSession, ToolCallResult};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::io::AsyncWriteExt;

const MAX_SOURCE_CONTENT_CHARS: usize = 16_000;
const MAX_SOURCE_CHUNK_CHARS: usize = 4_000;
const BOOTSTRAP_SOURCE_ATTEMPTS: usize = 2;
static SOURCE_TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct SourceCandidate {
    pub(super) title: String,
    pub(super) anchor: String,
    pub(super) preview: String,
    pub(super) provider_score: f64,
    pub(super) transport: AcquisitionTransport,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct SelectionEdge {
    pub(super) query_id: String,
    pub(super) source_target_id: Option<String>,
    pub(super) match_score: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct SelectedCandidate {
    pub(super) candidate: SourceCandidate,
    pub(super) edges: Vec<SelectionEdge>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct QueryDiscovery {
    pub(super) query: AcquisitionQuery,
    pub(super) candidates: Vec<SourceCandidate>,
    pub(super) error: Option<String>,
    pub(super) elapsed_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct AcquiredSource {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) requested_anchor: String,
    pub(super) canonical_anchor: String,
    pub(super) transport: AcquisitionTransport,
    pub(super) captured_at: String,
    pub(super) provenance: Vec<SelectionEdge>,
    pub(super) chunks: Vec<JsonValue>,
    pub(super) fetch_completed_ms: u64,
    pub(super) persisted_ms: Option<u64>,
}

impl AcquiredSource {
    pub(super) fn content(&self) -> String {
        self.chunks
            .iter()
            .filter_map(|chunk| chunk["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct AcquisitionFailure {
    pub(super) anchor: String,
    pub(super) edges: Vec<SelectionEdge>,
    pub(super) reason: String,
    pub(super) failed_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct AcquisitionResult {
    pub(super) strategy: EvaluationStrategy,
    pub(super) discoveries: Vec<QueryDiscovery>,
    pub(super) selected_candidates: Vec<SelectedCandidate>,
    pub(super) sources: Vec<AcquiredSource>,
    pub(super) failures: Vec<AcquisitionFailure>,
    pub(super) compiler_catalog: Option<JsonValue>,
    pub(super) query_call_count: usize,
    pub(super) source_call_count: usize,
    pub(super) discovery_elapsed_ms: u64,
    pub(super) source_elapsed_ms: u64,
    pub(super) phase_elapsed_ms: u64,
    pub(super) first_source_fetched_ms: Option<u64>,
    pub(super) first_source_persisted_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct BootstrapAcquisition {
    pub(super) discovery: QueryDiscovery,
    pub(super) selected_candidates: Vec<SelectedCandidate>,
    pub(super) sources: Vec<AcquiredSource>,
    pub(super) failures: Vec<AcquisitionFailure>,
    pub(super) discovery_elapsed_ms: u64,
    pub(super) source_elapsed_ms: u64,
}

pub(super) async fn acquire(
    session: &AgentSession,
    planning: &PlanningResult,
    budget: &LiveBudget,
    bootstrap: Option<BootstrapAcquisition>,
    output_dir: &Path,
    run_started: Instant,
) -> Result<AcquisitionResult, String> {
    let bootstrap_elapsed_ms = bootstrap
        .as_ref()
        .map(|bootstrap| bootstrap.discovery_elapsed_ms)
        .unwrap_or_default();
    let bootstrap_source_elapsed_ms = bootstrap
        .as_ref()
        .map(|bootstrap| bootstrap.source_elapsed_ms)
        .unwrap_or_default();
    let discovery_started = Instant::now();
    let followup_discoveries = join_all(
        planning
            .queries
            .iter()
            .filter(|query| query.id != "query.bootstrap")
            .cloned()
            .map(|query| discover_query(session, query, budget)),
    )
    .await;
    let discovery_elapsed_ms =
        bootstrap_elapsed_ms.saturating_add(discovery_started.elapsed().as_millis() as u64);
    let mut discoveries = bootstrap
        .as_ref()
        .map(|bootstrap| bootstrap.discovery.clone())
        .into_iter()
        .collect::<Vec<_>>();
    discoveries.extend(followup_discoveries);
    attach_preferred_candidates(&mut discoveries);
    let mut selected_candidates = bootstrap
        .as_ref()
        .map(|bootstrap| bootstrap.selected_candidates.clone())
        .unwrap_or_default();
    let mut attempted_anchors = selected_candidates
        .iter()
        .map(|selected| canonical_anchor_key(&selected.candidate.anchor))
        .collect::<BTreeSet<_>>();
    let ranked_candidates =
        selection::select_candidates(planning, &discoveries, budget.max_acquired_sources)
            .into_iter()
            .filter(|selected| {
                attempted_anchors.insert(canonical_anchor_key(&selected.candidate.anchor))
            })
            .collect::<Vec<_>>();
    let source_started = Instant::now();
    let source_goal = selected_candidates
        .len()
        .saturating_add(ranked_candidates.len())
        .min(budget.max_acquired_sources);
    let mut sources = bootstrap
        .as_ref()
        .map(|bootstrap| bootstrap.sources.clone())
        .unwrap_or_default();
    let mut failures = bootstrap
        .as_ref()
        .map(|bootstrap| bootstrap.failures.clone())
        .unwrap_or_default();
    let mut candidate_cursor = 0usize;
    while sources.len() < source_goal
        && selected_candidates.len() < budget.max_acquired_sources
        && candidate_cursor < ranked_candidates.len()
    {
        let needed = source_goal.saturating_sub(sources.len());
        let remaining_attempts = budget
            .max_acquired_sources
            .saturating_sub(selected_candidates.len());
        let available = ranked_candidates.len().saturating_sub(candidate_cursor);
        let batch_size = needed.min(remaining_attempts).min(available);
        if batch_size == 0 {
            break;
        }
        let batch = ranked_candidates
            [candidate_cursor..candidate_cursor.saturating_add(batch_size)]
            .to_vec();
        candidate_cursor = candidate_cursor.saturating_add(batch_size);
        let first_source_ordinal = selected_candidates.len().saturating_add(1);
        selected_candidates.extend(batch.iter().cloned());
        let fetched = join_all(batch.into_iter().enumerate().map(|(offset, selected)| {
            fetch_selected(
                session,
                selected,
                budget,
                format!("source-{}", first_source_ordinal.saturating_add(offset)),
                output_dir,
                run_started,
            )
        }))
        .await;
        for fetched in fetched {
            match fetched {
                Ok(source) => sources.push(source),
                Err(failure) => failures.push(failure),
            }
        }
    }
    if matches!(
        planning.strategy,
        EvaluationStrategy::Minimal | EvaluationStrategy::Brief
    ) && selected_candidates.len() < budget.max_acquired_sources
    {
        let remaining = budget
            .max_acquired_sources
            .saturating_sub(selected_candidates.len());
        let follow_candidates = github_readme_follow_candidates(&sources, &selected_candidates)
            .into_iter()
            .take(remaining)
            .collect::<Vec<_>>();
        let first_source_ordinal = selected_candidates.len().saturating_add(1);
        selected_candidates.extend(follow_candidates.iter().cloned());
        let followed = join_all(follow_candidates.into_iter().enumerate().map(
            |(offset, selected)| {
                fetch_selected(
                    session,
                    selected,
                    budget,
                    format!("source-{}", first_source_ordinal.saturating_add(offset)),
                    output_dir,
                    run_started,
                )
            },
        ))
        .await;
        for followed in followed {
            match followed {
                Ok(source) => sources.push(source),
                Err(failure) => failures.push(failure),
            }
        }
    }
    let source_elapsed_ms =
        bootstrap_source_elapsed_ms.saturating_add(source_started.elapsed().as_millis() as u64);
    sources.sort_by_key(|source| source.fetch_completed_ms);
    let compiler_catalog = if planning.strategy == EvaluationStrategy::Compiler {
        Some(build_compiler_catalog(
            planning,
            &discoveries,
            &sources,
            &failures,
        )?)
    } else {
        None
    };
    let first_source_fetched_ms = sources.iter().map(|source| source.fetch_completed_ms).min();
    let first_source_persisted_ms = sources
        .iter()
        .filter_map(|source| source.persisted_ms)
        .min();
    Ok(AcquisitionResult {
        strategy: planning.strategy,
        query_call_count: discoveries.len(),
        source_call_count: selected_candidates.len(),
        discoveries,
        selected_candidates,
        sources,
        failures,
        compiler_catalog,
        discovery_elapsed_ms,
        source_elapsed_ms,
        phase_elapsed_ms: run_started.elapsed().as_millis() as u64,
        first_source_fetched_ms,
        first_source_persisted_ms,
    })
}

fn attach_preferred_candidates(discoveries: &mut [QueryDiscovery]) {
    for discovery in discoveries {
        let mut preferred_urls = discovery
            .query
            .preferred_sources
            .iter()
            .filter(|preference| preference.kind == PreferredSourceKind::Url)
            .map(|preference| preference.value.trim())
            .filter(|anchor| safe_https_anchor(anchor))
            .map(str::to_string)
            .collect::<Vec<_>>();
        preferred_urls.extend(
            discovery
                .query
                .preferred_sources
                .iter()
                .filter(|preference| preference.kind == PreferredSourceKind::Repository)
                .filter_map(|preference| {
                    let (owner, repository) = preference.value.split_once('/')?;
                    github_blob_anchor(owner, repository, "HEAD", "README.md")
                }),
        );
        if discovery
            .query
            .text
            .split(|character: char| !character.is_alphanumeric())
            .any(|term| matches!(term.to_ascii_lowercase().as_str(), "release" | "releases"))
        {
            preferred_urls.extend(
                discovery
                    .query
                    .preferred_sources
                    .iter()
                    .filter(|preference| preference.kind == PreferredSourceKind::Repository)
                    .filter_map(|preference| {
                        let (owner, repository) = preference.value.split_once('/')?;
                        github_releases_anchor(owner, repository)
                    }),
            );
        }
        let mut known = discovery
            .candidates
            .iter()
            .map(|candidate| canonical_anchor_key(&candidate.anchor))
            .collect::<BTreeSet<_>>();
        for anchor in preferred_urls {
            if !known.insert(canonical_anchor_key(&anchor)) {
                continue;
            }
            discovery.candidates.push(SourceCandidate {
                title: preferred_url_title(&anchor),
                anchor,
                preview: discovery.query.text.clone(),
                provider_score: 0.0,
                transport: AcquisitionTransport::Web,
            });
        }
    }
}

fn preferred_url_title(anchor: &str) -> String {
    reqwest::Url::parse(anchor)
        .ok()
        .and_then(|url| {
            url.path_segments()?
                .rfind(|segment| !segment.is_empty())
                .map(str::to_string)
        })
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| anchor.to_string())
}

fn github_readme_follow_candidates(
    sources: &[AcquiredSource],
    selected: &[SelectedCandidate],
) -> Vec<SelectedCandidate> {
    let mut known = selected
        .iter()
        .map(|selected| canonical_anchor_key(&selected.candidate.anchor))
        .collect::<BTreeSet<_>>();
    sources
        .iter()
        .filter_map(|source| {
            let (owner, repository) = github_repository_root(&source.canonical_anchor)
                .or_else(|| github_repository_root(&source.requested_anchor))?;
            let content = source.content();
            if !content.contains("README.md") {
                return None;
            }
            let default_branch = extract_json_string(&content, "\"refInfo\":{\"name\":\"")?;
            let anchor = github_blob_anchor(&owner, &repository, &default_branch, "README.md")?;
            if !known.insert(canonical_anchor_key(&anchor)) {
                return None;
            }
            Some(SelectedCandidate {
                candidate: SourceCandidate {
                    title: "README.md".to_string(),
                    anchor,
                    preview:
                        "Repository README followed from the fetched canonical repository root"
                            .to_string(),
                    provider_score: 0.0,
                    transport: AcquisitionTransport::Web,
                },
                edges: source.provenance.clone(),
            })
        })
        .collect()
}

fn github_repository_root(anchor: &str) -> Option<(String, String)> {
    let url = reqwest::Url::parse(anchor).ok()?;
    if url.host_str()? != "github.com" {
        return None;
    }
    let segments = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    match segments.as_slice() {
        [owner, repository] => Some((owner.to_string(), repository.to_string())),
        _ => None,
    }
}

fn extract_json_string(content: &str, marker: &str) -> Option<String> {
    let remainder = content.split_once(marker)?.1;
    let value = remainder.split_once('"')?.0;
    (!value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '/')
        }))
    .then(|| value.to_string())
}

fn github_blob_anchor(
    owner: &str,
    repository: &str,
    reference: &str,
    path: &str,
) -> Option<String> {
    let mut url = reqwest::Url::parse("https://github.com").ok()?;
    url.path_segments_mut()
        .ok()?
        .push(owner)
        .push(repository)
        .push("blob")
        .push(reference);
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        url.path_segments_mut().ok()?.push(segment);
    }
    Some(url.to_string())
}

fn github_releases_anchor(owner: &str, repository: &str) -> Option<String> {
    let mut url = reqwest::Url::parse("https://github.com").ok()?;
    url.path_segments_mut()
        .ok()?
        .push(owner)
        .push(repository)
        .push("releases");
    Some(url.to_string())
}

pub(super) async fn discover_bootstrap(
    session: &AgentSession,
    input: &PlannerInput,
    budget: &LiveBudget,
) -> QueryDiscovery {
    discover_query(session, bootstrap_query(input), budget).await
}

pub(super) async fn acquire_bootstrap(
    session: &AgentSession,
    input: &PlannerInput,
    budget: &LiveBudget,
    output_dir: &Path,
    run_started: Instant,
) -> BootstrapAcquisition {
    let discovery_started = Instant::now();
    let mut discovery = discover_bootstrap(session, input, budget).await;
    discovery.query.fetch_slots = input.budget.max_acquired_sources;
    let discovery_elapsed_ms = discovery_started.elapsed().as_millis() as u64;
    let mut known = BTreeSet::new();
    let selected_candidates = selection::rank_discovery_candidates(&discovery)
        .into_iter()
        .filter(|candidate| known.insert(canonical_anchor_key(&candidate.anchor)))
        .take(
            BOOTSTRAP_SOURCE_ATTEMPTS
                .min(budget.max_acquired_sources)
                .min(discovery.candidates.len()),
        )
        .map(|candidate| SelectedCandidate {
            candidate,
            edges: vec![SelectionEdge {
                query_id: discovery.query.id.clone(),
                source_target_id: None,
                match_score: 0,
            }],
        })
        .collect::<Vec<_>>();
    let source_started = Instant::now();
    let fetched = join_all(selected_candidates.iter().cloned().enumerate().map(
        |(index, selected)| {
            fetch_selected(
                session,
                selected,
                budget,
                format!("source-{}", index + 1),
                output_dir,
                run_started,
            )
        },
    ))
    .await;
    let source_elapsed_ms = source_started.elapsed().as_millis() as u64;
    let mut sources = Vec::new();
    let mut failures = Vec::new();
    for result in fetched {
        match result {
            Ok(source) => sources.push(source),
            Err(failure) => failures.push(failure),
        }
    }
    sources.sort_by_key(|source| source.fetch_completed_ms);
    BootstrapAcquisition {
        discovery,
        selected_candidates,
        sources,
        failures,
        discovery_elapsed_ms,
        source_elapsed_ms,
    }
}

pub(super) fn bootstrap_observation(discovery: &QueryDiscovery) -> JsonValue {
    serde_json::json!({
        "attempt": {
            "id": discovery.query.id,
            "text": discovery.query.text,
            "transport": discovery.query.transport,
            "status": if discovery.error.is_some() { "failed" } else { "completed" },
            "error": discovery.error,
        },
        "candidates": discovery.candidates.iter().map(|candidate| serde_json::json!({
            "title": candidate.title,
            "anchor": candidate.anchor,
            "preview": candidate.preview,
            "provider_score": candidate.provider_score,
        })).collect::<Vec<_>>(),
    })
}

pub(super) fn bind_persisted_bootstrap(
    planning: &mut PlanningResult,
    discovery: &mut QueryDiscovery,
) -> Result<(), String> {
    if !matches!(
        planning.strategy,
        EvaluationStrategy::Minimal | EvaluationStrategy::Brief
    ) {
        return Err("bootstrap binding requires a persisted-evidence evaluator".to_string());
    }
    let brief = planning.brief.as_mut().ok_or_else(|| {
        "persisted planning result omitted its root research contract".to_string()
    })?;
    let dimension_ids = brief
        .dimensions
        .iter()
        .map(|dimension| dimension.id.clone())
        .collect::<Vec<_>>();
    if dimension_ids.is_empty() {
        return Err("persisted bootstrap has no root research dimension".to_string());
    }
    discovery.query.id = "query.bootstrap".to_string();
    discovery.query.dimension_ids = dimension_ids;
    discovery.query.preferred_sources.clear();

    planning
        .queries
        .retain(|query| query.id != discovery.query.id);
    let remaining = planning.planner_input.budget.max_queries.saturating_sub(1);
    planning.queries.truncate(remaining);
    planning.queries.insert(0, discovery.query.clone());
    brief.queries.retain(|query| query.id != discovery.query.id);
    brief.queries.truncate(remaining);
    brief.queries.insert(0, discovery.query.clone());
    brief.planning_gaps.retain(|gap| !gap.host_generated);
    brief
        .normalization_notes
        .push("Host attached bootstrap discovery to every research dimension without widening query-local source preferences".to_string());
    Ok(())
}

fn bootstrap_query(input: &PlannerInput) -> AcquisitionQuery {
    let transport = match input.evidence_scope {
        EvidenceScope::Workspace => AcquisitionTransport::Workspace,
        EvidenceScope::Web | EvidenceScope::WebAndWorkspace => AcquisitionTransport::Web,
    };
    let text = match transport {
        AcquisitionTransport::Web => input.query.clone(),
        AcquisitionTransport::Workspace => workspace_bootstrap_pattern(&input.query),
    };
    let (path, glob) = if transport == AcquisitionTransport::Workspace
        && workspace_code_ownership_request(&input.query)
    {
        ("src".to_string(), "*.rs".to_string())
    } else {
        (String::new(), String::new())
    };
    AcquisitionQuery {
        id: "query.bootstrap".to_string(),
        text,
        transport,
        path,
        glob,
        dimension_ids: Vec::new(),
        source_target_ids: Vec::new(),
        preferred_sources: Vec::new(),
        fetch_slots: 0,
    }
}

fn workspace_code_ownership_request(request: &str) -> bool {
    let lower = request.to_ascii_lowercase();
    let repository_scope = lower.contains("repository")
        || lower.contains("codebase")
        || request.contains("仓库")
        || request.contains("代码库");
    let code_signal = [
        "cli",
        "tui",
        "call site",
        "call path",
        "entrypoint",
        "handler",
        "runtime",
        "local files",
    ]
    .into_iter()
    .any(|term| lower.contains(term))
        || ["调用", "入口", "实现", "源码"]
            .into_iter()
            .any(|term| request.contains(term));
    repository_scope && code_signal
}

fn workspace_bootstrap_pattern(request: &str) -> String {
    const GENERIC: &[&str] = &[
        "and",
        "are",
        "cite",
        "does",
        "for",
        "its",
        "the",
        "this",
        "that",
        "from",
        "through",
        "with",
        "into",
        "repository",
        "trace",
        "active",
        "path",
        "paths",
        "local",
        "file",
        "files",
        "own",
        "owns",
        "each",
        "every",
        "transition",
        "transitions",
        "identify",
        "which",
        "where",
        "what",
        "when",
        "code",
    ];
    let mut seen = BTreeSet::new();
    let terms = request
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter_map(|term| {
            let normalized = term.trim_matches('_');
            if normalized.chars().count() < 3
                || GENERIC.contains(&normalized.to_ascii_lowercase().as_str())
            {
                return None;
            }
            let pattern = camel_case_pattern(normalized);
            seen.insert(pattern.clone()).then_some(pattern)
        })
        .take(24)
        .collect::<Vec<_>>();
    if terms.is_empty() {
        ".+".to_string()
    } else {
        terms.join("|")
    }
}

fn camel_case_pattern(value: &str) -> String {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_lowercase = false;
    for character in value.chars() {
        if character == '_' || character == '-' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_lowercase = false;
            continue;
        }
        if character.is_uppercase() && previous_lowercase && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        previous_lowercase = character.is_lowercase();
        current.push(character.to_ascii_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .iter()
        .map(|word| regex::escape(word))
        .collect::<Vec<_>>()
        .join("[_-]?")
}

async fn discover_query(
    session: &AgentSession,
    query: AcquisitionQuery,
    budget: &LiveBudget,
) -> QueryDiscovery {
    let started = Instant::now();
    let outcome = match query.transport {
        AcquisitionTransport::Web => discover_web(session, &query, budget).await,
        AcquisitionTransport::Workspace => discover_workspace(session, &query, budget).await,
    };
    match outcome {
        Ok(candidates) => QueryDiscovery {
            query,
            candidates,
            error: None,
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
        Err(error) => QueryDiscovery {
            query,
            candidates: Vec::new(),
            error: Some(bounded_error(&error)),
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
    }
}

async fn discover_web(
    session: &AgentSession,
    query: &AcquisitionQuery,
    budget: &LiveBudget,
) -> Result<Vec<SourceCandidate>, String> {
    let timeout_seconds = budget.search_timeout_ms.div_ceil(1_000).max(1);
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(budget.search_timeout_ms.saturating_add(2_000)),
        session.tool(
            "web_search",
            serde_json::json!({
                "query": query.text,
                "engines": ["anysearch", "tavily", "ddg"],
                "format": "json",
                "limit": 8,
                "timeout": timeout_seconds,
            }),
        ),
    )
    .await
    .map_err(|_| format!("web search `{}` exceeded the Host timeout", query.id))?
    .map_err(|error| format!("web search `{}` failed: {error:#}", query.id))?;
    if result.exit_code != 0 {
        return Err(format!("web search `{}`: {}", query.id, result.output));
    }
    let decoded = serde_json::from_str::<JsonValue>(&result.output)
        .map_err(|error| format!("decode web search `{}`: {error}", query.id))?;
    let values = decoded
        .as_array()
        .or_else(|| decoded["results"].as_array())
        .ok_or_else(|| format!("web search `{}` returned a non-array result", query.id))?;
    let mut seen = BTreeSet::new();
    Ok(values
        .iter()
        .filter_map(|value| {
            let anchor = value["url"].as_str()?.trim();
            if !safe_https_anchor(anchor) || !seen.insert(canonical_anchor_key(anchor)) {
                return None;
            }
            Some(SourceCandidate {
                title: bounded_text(
                    value["title"]
                        .as_str()
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or(anchor),
                    500,
                ),
                anchor: anchor.to_string(),
                preview: bounded_text(
                    value["content"]
                        .as_str()
                        .or_else(|| value["snippet"].as_str())
                        .unwrap_or_default(),
                    1_000,
                ),
                provider_score: value["score"].as_f64().unwrap_or_default(),
                transport: AcquisitionTransport::Web,
            })
        })
        .collect())
}

async fn discover_workspace(
    session: &AgentSession,
    query: &AcquisitionQuery,
    budget: &LiveBudget,
) -> Result<Vec<SourceCandidate>, String> {
    let grep = join_all(
        workspace_content_search_patterns(&query.text)
            .into_iter()
            .map(|pattern| {
                let mut args = serde_json::json!({
                    "pattern": pattern,
                    "path": if query.path.is_empty() { "." } else { query.path.as_str() },
                    "context": 0,
                    "-i": true,
                });
                if !query.glob.is_empty() {
                    args["glob"] = JsonValue::String(query.glob.clone());
                }
                async move {
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(
                            budget.search_timeout_ms.saturating_add(2_000),
                        ),
                        session.tool("grep", args),
                    )
                    .await
                    {
                        Ok(Ok(result)) if result.exit_code == 0 => Ok(result),
                        Ok(Ok(result)) => Err(format!(
                            "workspace search `{}`: {}",
                            query.id, result.output
                        )),
                        Ok(Err(error)) => {
                            Err(format!("workspace search `{}` failed: {error:#}", query.id))
                        }
                        Err(_) => Err(format!(
                            "workspace search `{}` exceeded the Host timeout",
                            query.id
                        )),
                    }
                }
            }),
    );
    let (inventory_path, inventory_pattern) = workspace_inventory_request(&query.path, &query.glob);
    let inventory = tokio::time::timeout(
        std::time::Duration::from_millis(budget.search_timeout_ms.saturating_add(2_000)),
        session.tool(
            "glob",
            serde_json::json!({
                "pattern": inventory_pattern,
                "path": inventory_path,
                "limit": 1_000,
            }),
        ),
    );
    let (grep, inventory) = tokio::join!(grep, inventory);
    let inventory = match inventory {
        Ok(Ok(result)) if result.exit_code == 0 => Ok(result),
        Ok(Ok(result)) => Err(format!(
            "workspace inventory `{}`: {}",
            query.id, result.output
        )),
        Ok(Err(error)) => Err(format!(
            "workspace inventory `{}` failed: {error:#}",
            query.id
        )),
        Err(_) => Err(format!(
            "workspace inventory `{}` exceeded the Host timeout",
            query.id
        )),
    };
    if grep.iter().all(Result::is_err) && inventory.is_err() {
        let mut errors = grep.into_iter().filter_map(Result::err).collect::<Vec<_>>();
        errors.push(inventory.expect_err("checked inventory error"));
        return Err(errors.join("; "));
    }

    let mut candidate_indices = BTreeMap::<String, usize>::new();
    let mut candidates: Vec<SourceCandidate> = Vec::new();
    for result in grep.into_iter().filter_map(Result::ok) {
        let anchors = result
            .metadata
            .as_ref()
            .and_then(|metadata| metadata["source_anchors"].as_array())
            .into_iter()
            .flatten()
            .filter_map(JsonValue::as_str)
            .map(str::trim)
            .filter(|path| safe_workspace_anchor(path))
            .collect::<Vec<_>>();
        for (index, anchor) in anchors.into_iter().enumerate() {
            let preview = workspace_candidate_preview(&result.output, anchor);
            let provider_score = 1.0 / (index + 1) as f64;
            if let Some(candidate_index) = candidate_indices.get(anchor).copied() {
                let candidate = &mut candidates[candidate_index];
                candidate.provider_score = candidate.provider_score.max(provider_score);
                merge_workspace_candidate_preview(&mut candidate.preview, &preview);
            } else {
                candidate_indices.insert(anchor.to_string(), candidates.len());
                candidates.push(SourceCandidate {
                    title: anchor.to_string(),
                    anchor: anchor.to_string(),
                    preview,
                    provider_score,
                    transport: AcquisitionTransport::Workspace,
                });
            }
        }
    }
    if let Ok(result) = inventory {
        candidates.extend(
            workspace_inventory_paths(&result.output)
                .into_iter()
                .filter(|path| !candidate_indices.contains_key(path))
                .map(|path| SourceCandidate {
                    title: path.clone(),
                    anchor: path,
                    preview: String::new(),
                    provider_score: 0.0,
                    transport: AcquisitionTransport::Workspace,
                }),
        );
    }
    Ok(candidates)
}

fn workspace_content_search_patterns(pattern: &str) -> Vec<String> {
    if workspace_code_trace_pattern(pattern) {
        return [
            "SubmissionIntent|parse_deep_research_tui_query|execute_deepresearch|parse_deepresearch_args",
            "start_deep_research_workflow|run_deepresearch_inquiry|spawn_deep_research|deep_research_workflow",
            "bootstrap_acquisition|inquiry_runtime|retrieval|accepted_evidence|admit.*evidence|evidence_ledger",
            "report_generation|sectioned_report|materialize.*report|generate.*report",
            "publication|publish|write_research_report|replace_staged",
            "stage_deep_research_report|open_pending_deep_research_report_view|open_remote_view|open_window_with|local_file_view",
            "LegacyCheckedLoop|legacy|compatibility|replay",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
    }
    vec![focused_workspace_search_pattern(pattern)]
}

fn workspace_code_trace_pattern(pattern: &str) -> bool {
    const SIGNALS: &[&str] = &[
        "submission",
        "acquisition",
        "evidence",
        "report",
        "publication",
        "browser",
        "legacy",
    ];
    let alternatives = top_level_regex_alternatives(pattern)
        .into_iter()
        .map(normalize_workspace_pattern)
        .collect::<BTreeSet<_>>();
    SIGNALS
        .iter()
        .filter(|signal| alternatives.contains(**signal))
        .count()
        >= 5
}

fn merge_workspace_candidate_preview(existing: &mut String, additional: &str) {
    if additional.is_empty() {
        return;
    }
    let mut lines = existing.lines().map(str::to_string).collect::<Vec<_>>();
    for line in additional.lines() {
        if !lines.iter().any(|existing| existing == line) {
            lines.push(line.to_string());
        }
    }
    *existing = lines.join("\n").chars().take(1_000).collect::<String>();
}

fn focused_workspace_search_pattern(pattern: &str) -> String {
    let alternatives = top_level_regex_alternatives(pattern);
    const BOUNDARY_PRIORITY: &[&str] = &[
        "legacy",
        "inactive",
        "deprecated",
        "compatibility",
        "browser",
        "opening",
        "publication",
        "publish",
        "submission",
        "settlement",
        "settle",
    ];
    let boundaries = BOUNDARY_PRIORITY
        .iter()
        .filter_map(|boundary| {
            alternatives
                .iter()
                .find(|alternative| normalize_workspace_pattern(alternative) == *boundary)
                .copied()
        })
        .take(6)
        .collect::<Vec<_>>();
    if boundaries.len() >= 2 {
        return boundaries.join("|");
    }

    let identifiers = alternatives
        .iter()
        .filter(|alternative| {
            alternative.contains("[_-]?") || alternative.contains('_') || alternative.contains('-')
        })
        .map(|alternative| normalize_workspace_pattern(alternative))
        .collect::<BTreeSet<_>>();
    if !identifiers.is_empty() {
        let selected = alternatives
            .iter()
            .filter(|alternative| identifiers.contains(&normalize_workspace_pattern(alternative)))
            .copied()
            .collect::<Vec<_>>();
        if !selected.is_empty() {
            return selected.join("|");
        }
    }

    const GENERIC: &[&str] = &[
        "active",
        "cli",
        "code",
        "evidence",
        "file",
        "files",
        "generation",
        "handling",
        "inactive",
        "legacy",
        "open",
        "report",
        "research",
        "source",
        "sources",
        "submission",
        "tui",
    ];
    let focused = alternatives
        .iter()
        .copied()
        .filter(|alternative| !GENERIC.contains(&normalize_workspace_pattern(alternative).as_str()))
        .collect::<Vec<_>>();
    if focused.is_empty() {
        pattern.to_string()
    } else {
        focused.join("|")
    }
}

fn top_level_regex_alternatives(pattern: &str) -> Vec<&str> {
    let mut alternatives = Vec::new();
    let mut start = 0usize;
    let mut parenthesis_depth = 0usize;
    let mut in_character_class = false;
    let mut escaped = false;
    for (index, character) in pattern.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '[' => in_character_class = true,
            ']' => in_character_class = false,
            '(' if !in_character_class => parenthesis_depth += 1,
            ')' if !in_character_class => parenthesis_depth = parenthesis_depth.saturating_sub(1),
            '|' if !in_character_class && parenthesis_depth == 0 => {
                let alternative = pattern[start..index].trim();
                if !alternative.is_empty() {
                    alternatives.push(alternative);
                }
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    let alternative = pattern[start..].trim();
    if !alternative.is_empty() {
        alternatives.push(alternative);
    }
    alternatives
}

fn normalize_workspace_pattern(pattern: &str) -> String {
    pattern
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn workspace_inventory_pattern(glob: &str) -> String {
    let glob = glob.trim();
    if glob.is_empty() {
        "**/*".to_string()
    } else if glob.contains('/') {
        glob.to_string()
    } else {
        format!("**/{glob}")
    }
}

fn workspace_inventory_request(path: &str, glob: &str) -> (String, String) {
    let path = path.trim();
    if path.is_empty() {
        return (".".to_string(), workspace_inventory_pattern(glob));
    }
    let path = Path::new(path);
    if path.extension().is_none() {
        return (
            path.to_string_lossy().into_owned(),
            workspace_inventory_pattern(glob),
        );
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| workspace_inventory_pattern(glob));
    (parent, file_name)
}

fn workspace_inventory_paths(output: &str) -> Vec<String> {
    if output
        .trim_start()
        .starts_with("No files found matching pattern:")
    {
        return Vec::new();
    }
    output
        .lines()
        .map(str::trim)
        .take_while(|line| !line.is_empty())
        .filter(|path| safe_workspace_anchor(path))
        .map(str::to_string)
        .collect()
}

fn workspace_candidate_preview(output: &str, anchor: &str) -> String {
    let prefix = format!(">{anchor}:");
    output
        .lines()
        .filter(|line| line.starts_with(&prefix))
        .take(8)
        .map(|line| line.chars().take(240).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(1_000)
        .collect()
}

async fn fetch_selected(
    session: &AgentSession,
    selected: SelectedCandidate,
    budget: &LiveBudget,
    source_id: String,
    output_dir: &Path,
    run_started: Instant,
) -> Result<AcquiredSource, AcquisitionFailure> {
    let focused_web_anchor = (selected.candidate.transport == AcquisitionTransport::Web)
        .then(|| github_raw_blob_anchor(&selected.candidate.anchor))
        .flatten();
    let result = match selected.candidate.transport {
        AcquisitionTransport::Web => {
            let timeout_seconds = budget.fetch_timeout_ms.div_ceil(1_000).max(1);
            tokio::time::timeout(
                std::time::Duration::from_millis(budget.fetch_timeout_ms.saturating_add(2_000)),
                session.tool(
                    "web_fetch",
                    serde_json::json!({
                        "url": focused_web_anchor
                            .as_deref()
                            .unwrap_or(&selected.candidate.anchor),
                        "format": "markdown",
                        "timeout": timeout_seconds,
                        "max_chars": MAX_SOURCE_CONTENT_CHARS,
                    }),
                ),
            )
            .await
        }
        AcquisitionTransport::Workspace => {
            tokio::time::timeout(
                std::time::Duration::from_millis(budget.fetch_timeout_ms.saturating_add(2_000)),
                session.tool(
                    "read",
                    serde_json::json!({
                        "file_path": selected.candidate.anchor,
                        "offset": 0,
                        "limit": 2000,
                    }),
                ),
            )
            .await
        }
    };
    let fetch_completed_ms = run_started.elapsed().as_millis() as u64;
    let result = match result {
        Err(_) => {
            return Err(AcquisitionFailure {
                anchor: selected.candidate.anchor,
                edges: selected.edges,
                reason: "source read exceeded the Host timeout".to_string(),
                failed_ms: fetch_completed_ms,
            });
        }
        Ok(Err(error)) => {
            return Err(AcquisitionFailure {
                anchor: selected.candidate.anchor,
                edges: selected.edges,
                reason: bounded_error(&format!("source read failed: {error:#}")),
                failed_ms: fetch_completed_ms,
            });
        }
        Ok(Ok(result)) => result,
    };
    if result.exit_code != 0 || result.output.trim().is_empty() {
        return Err(AcquisitionFailure {
            anchor: selected.candidate.anchor,
            edges: selected.edges,
            reason: bounded_error(&format!("source read returned: {}", result.output)),
            failed_ms: fetch_completed_ms,
        });
    }
    let canonical_anchor = if focused_web_anchor.is_some() {
        selected.candidate.anchor.clone()
    } else {
        canonical_source_anchor(&selected.candidate, &result)
    };
    let source_content = fetched_source_content(&selected.candidate, &result.output);
    let mut chunks = source_chunks(&source_content);
    if chunks.is_empty() {
        return Err(AcquisitionFailure {
            anchor: selected.candidate.anchor,
            edges: selected.edges,
            reason: "source read returned no substantive text".to_string(),
            failed_ms: fetch_completed_ms,
        });
    }
    for (chunk_index, chunk) in chunks.iter_mut().enumerate() {
        chunk["id"] = JsonValue::String(format!("{source_id}:chunk-{}", chunk_index + 1));
    }
    let mut source = AcquiredSource {
        id: source_id,
        title: bounded_text(&selected.candidate.title, 500),
        requested_anchor: selected.candidate.anchor,
        canonical_anchor,
        transport: selected.candidate.transport,
        captured_at: chrono::Utc::now().to_rfc3339(),
        provenance: selected.edges,
        chunks,
        fetch_completed_ms,
        persisted_ms: None,
    };
    if let Err(reason) = persist_source_record(output_dir, &mut source, run_started).await {
        return Err(AcquisitionFailure {
            anchor: source.requested_anchor,
            edges: source.provenance,
            reason,
            failed_ms: run_started.elapsed().as_millis() as u64,
        });
    }
    Ok(source)
}

fn fetched_source_content(candidate: &SourceCandidate, fetched: &str) -> String {
    if candidate.transport != AcquisitionTransport::Workspace || candidate.preview.trim().is_empty()
    {
        return fetched.to_string();
    }
    format!(
        "Matched source excerpts:\n{}\n\nSource file read:\n{}",
        candidate.preview, fetched
    )
}

async fn persist_source_record(
    output_dir: &Path,
    source: &mut AcquiredSource,
    run_started: Instant,
) -> Result<(), String> {
    let source_dir = output_dir.join("source-records");
    tokio::fs::create_dir_all(&source_dir)
        .await
        .map_err(|error| format!("create durable source directory: {error}"))?;
    let path = source_dir.join(format!("{}.json", source.id));
    let content_bytes = serde_json::to_vec_pretty(source)
        .map_err(|error| format!("encode durable source `{}`: {error}", source.id))?;
    write_bytes_atomic(&path, &content_bytes)
        .await
        .map_err(|error| format!("persist durable source `{}`: {error}", source.id))?;
    source.persisted_ms = Some(run_started.elapsed().as_millis() as u64);
    let committed_bytes = serde_json::to_vec_pretty(source)
        .map_err(|error| format!("encode committed source `{}`: {error}", source.id))?;
    write_bytes_atomic(&path, &committed_bytes)
        .await
        .map_err(|error| format!("commit durable source metadata `{}`: {error}", source.id))
}

async fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path has no parent: {}", path.display()),
        )
    })?;
    tokio::fs::create_dir_all(parent).await?;
    let temporary = temporary_source_path(path);
    let result = async {
        let mut options = tokio::fs::OpenOptions::new();
        options.create_new(true).write(true);
        let mut file = options.open(&temporary).await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        replace_file(&temporary, path).await?;
        #[cfg(unix)]
        tokio::fs::File::open(parent).await?.sync_all().await?;
        Ok::<(), std::io::Error>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

fn temporary_source_path(path: &Path) -> PathBuf {
    let sequence = SOURCE_TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("source.json");
    path.with_file_name(format!(
        ".{file_name}.tmp-{}-{sequence}",
        std::process::id()
    ))
}

async fn replace_file(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    #[cfg(windows)]
    if tokio::fs::try_exists(destination).await? {
        tokio::fs::remove_file(destination).await?;
    }
    tokio::fs::rename(source, destination).await
}

#[cfg(test)]
mod source_persistence_tests {
    use super::{persist_source_record, AcquiredSource, AcquisitionTransport, SelectionEdge};
    use std::time::{Duration, Instant};

    fn source(id: &str) -> AcquiredSource {
        AcquiredSource {
            id: id.to_string(),
            title: "Alpha policy".to_string(),
            requested_anchor: "https://example.test/alpha".to_string(),
            canonical_anchor: "https://example.test/alpha".to_string(),
            transport: AcquisitionTransport::Web,
            captured_at: "2026-07-22T00:00:00Z".to_string(),
            provenance: vec![SelectionEdge {
                query_id: "query.bootstrap".to_string(),
                source_target_id: None,
                match_score: 1,
            }],
            chunks: vec![serde_json::json!({
                "id": format!("{id}:chunk-1"),
                "text": "Alpha 2.x is supported through 2027."
            })],
            fetch_completed_ms: 10,
            persisted_ms: None,
        }
    }

    #[tokio::test]
    async fn source_records_are_independent_atomic_effects() {
        let output = tempfile::tempdir().expect("temporary evaluator output");
        let run_started = Instant::now() - Duration::from_millis(25);
        let mut first = source("source-1");
        persist_source_record(output.path(), &mut first, run_started)
            .await
            .expect("persist first source");

        let first_path = output.path().join("source-records/source-1.json");
        let persisted = serde_json::from_slice::<AcquiredSource>(
            &tokio::fs::read(&first_path)
                .await
                .expect("read persisted first source"),
        )
        .expect("decode persisted first source");
        assert_eq!(persisted.id, "source-1");
        assert_eq!(persisted.chunks[0]["id"], "source-1:chunk-1");
        assert_eq!(persisted.persisted_ms, first.persisted_ms);
        assert!(persisted.persisted_ms.is_some_and(|elapsed| elapsed >= 25));

        let blocked_path = output.path().join("source-records/source-2.json");
        tokio::fs::create_dir(&blocked_path)
            .await
            .expect("block second source destination");
        let mut second = source("source-2");
        let error = persist_source_record(output.path(), &mut second, run_started)
            .await
            .expect_err("second source persistence must fail locally");
        assert!(error.contains("source-2"), "{error}");
        assert!(first_path.is_file(), "the first source must survive");

        let mut entries = tokio::fs::read_dir(output.path().join("source-records"))
            .await
            .expect("read source record directory");
        while let Some(entry) = entries.next_entry().await.expect("read source entry") {
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(!name.contains(".tmp-"), "temporary file leaked: {name}");
        }
    }
}

fn github_raw_blob_anchor(anchor: &str) -> Option<String> {
    let url = reqwest::Url::parse(anchor).ok()?;
    if url.host_str()? != "github.com" {
        return None;
    }
    let segments = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let [owner, repository, blob, reference, path @ ..] = segments.as_slice() else {
        return None;
    };
    if *blob != "blob" || path.is_empty() {
        return None;
    }
    let mut raw = reqwest::Url::parse("https://raw.githubusercontent.com").ok()?;
    raw.path_segments_mut()
        .ok()?
        .push(owner)
        .push(repository)
        .push(reference);
    for segment in path {
        raw.path_segments_mut().ok()?.push(segment);
    }
    Some(raw.to_string())
}

fn canonical_source_anchor(candidate: &SourceCandidate, result: &ToolCallResult) -> String {
    result
        .metadata
        .as_ref()
        .and_then(|metadata| metadata["source_anchors"].as_array())
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .rfind(|anchor| match candidate.transport {
            AcquisitionTransport::Web => safe_https_anchor(anchor),
            AcquisitionTransport::Workspace => safe_workspace_anchor(anchor),
        })
        .map(str::to_string)
        .unwrap_or_else(|| candidate.anchor.clone())
}

fn source_chunks(content: &str) -> Vec<JsonValue> {
    let bounded = content
        .chars()
        .take(MAX_SOURCE_CONTENT_CHARS)
        .collect::<String>();
    let characters = bounded.chars().collect::<Vec<_>>();
    characters
        .chunks(MAX_SOURCE_CHUNK_CHARS)
        .filter_map(|chunk| {
            let text = chunk.iter().collect::<String>();
            let text = text.trim();
            (!text.is_empty()).then(|| serde_json::json!({ "id": "pending", "text": text }))
        })
        .collect()
}

#[cfg(test)]
mod brief_workspace_tests {
    use super::{
        attach_preferred_candidates, bootstrap_query, fetched_source_content,
        focused_workspace_search_pattern, github_raw_blob_anchor, github_readme_follow_candidates,
        source_chunks, top_level_regex_alternatives, workspace_bootstrap_pattern,
        workspace_candidate_preview, workspace_code_trace_pattern,
        workspace_content_search_patterns, workspace_inventory_paths, workspace_inventory_pattern,
        workspace_inventory_request, AcquiredSource, AcquisitionQuery, AcquisitionTransport,
        EvidenceScope, PlannerInput, PreferredSourceKind, QueryDiscovery, SelectedCandidate,
        SelectionEdge, SourceCandidate, SourcePreference,
    };
    use crate::commands::code::research_runtime::tests::baseline::live::corpus::PlannerBudget;

    #[test]
    fn workspace_bootstrap_keeps_late_ownership_terms_and_normalizes_camel_case() {
        let pattern = workspace_bootstrap_pattern(
            "In this repository, trace the active DeepResearch path from CLI or TUI submission through acquisition, evidence handling, report generation, artifact publication, browser opening, and inactive legacy paths.",
        );
        assert!(pattern.contains("deep[_-]?research"), "{pattern}");
        assert!(pattern.contains("acquisition"), "{pattern}");
        assert!(pattern.contains("publication"), "{pattern}");
        assert!(pattern.contains("browser"), "{pattern}");
        assert!(pattern.contains("legacy"), "{pattern}");
        assert!(!pattern.contains("repository"), "{pattern}");
        assert!(!pattern.contains("|this|"), "{pattern}");
    }

    #[test]
    fn workspace_preview_retains_only_the_candidate_match_context() {
        let output = ">src/active.rs:10:fn active_path() {}\n>docs/design.md:4:active path\n>src/active.rs:20:active_path();\n2 match(es) in 2 file(s)";
        let preview = workspace_candidate_preview(output, "src/active.rs");
        assert!(preview.contains("fn active_path"));
        assert!(preview.contains("active_path();"));
        assert!(!preview.contains("docs/design.md"));
        assert_eq!(preview.lines().count(), 2);
    }

    #[test]
    fn workspace_preview_retains_complete_late_browser_transition() {
        let output = [
            ">src/tui/app/view.rs:238: fn stage_deep_research_report()",
            ">src/tui/app/view.rs:248: remote_ui::local_file_view(&artifacts.html)",
            ">src/tui/app/view.rs:264: self.open_remote_view(&spec)",
            ">src/tui/app/view.rs:271: fn open_pending_deep_research_report_view()",
            ">src/tui/app/view.rs:277: self.open_remote_view(&spec)",
            ">src/tui/app/view.rs:284: fn open_remote_view()",
            ">src/tui/app/view.rs:286: remote_ui::open_window_with(&spec)",
        ]
        .join("\n");
        let preview = workspace_candidate_preview(&output, "src/tui/app/view.rs");
        assert!(preview.contains("stage_deep_research_report"), "{preview}");
        assert!(
            preview.contains("open_pending_deep_research_report_view"),
            "{preview}"
        );
        assert!(preview.contains("open_window_with"), "{preview}");
    }

    #[test]
    fn workspace_fetch_preserves_late_matched_lines_before_the_file_head_cap() {
        let candidate = SourceCandidate {
            title: "src/tui/app/view.rs".to_string(),
            anchor: "src/tui/app/view.rs".to_string(),
            preview: ">src/tui/app/view.rs:267: self.open_remote_view(&spec);".to_string(),
            provider_score: 1.0,
            transport: AcquisitionTransport::Workspace,
        };
        let fetched = "file head\n".repeat(4_000);
        let retained = source_chunks(&fetched_source_content(&candidate, &fetched))
            .into_iter()
            .filter_map(|chunk| chunk["text"].as_str().map(str::to_string))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(retained.contains("open_remote_view"), "{retained}");
    }

    #[test]
    fn focused_workspace_pattern_preserves_grouped_alternation() {
        let pattern = "evidence|acqui(sition|re)|admit";
        assert_eq!(
            top_level_regex_alternatives(pattern),
            vec!["evidence", "acqui(sition|re)", "admit"]
        );
        assert_eq!(
            focused_workspace_search_pattern(pattern),
            "acqui(sition|re)|admit"
        );
    }

    #[test]
    fn focused_workspace_pattern_prioritizes_rare_transition_boundaries() {
        assert_eq!(
            focused_workspace_search_pattern(
                "deep[_-]?research|submission|acquisition|publication|browser|opening|inactive|legacy"
            ),
            "legacy|inactive|browser|opening|publication|submission"
        );
    }

    #[test]
    fn focused_workspace_pattern_keeps_identifiers_without_multiple_boundaries() {
        assert_eq!(
            focused_workspace_search_pattern(
                "parse_deep_research_tui_query|start_deep_research_workflow|submit|launch"
            ),
            "parse_deep_research_tui_query|start_deep_research_workflow"
        );
    }

    #[test]
    fn workspace_code_trace_searches_each_owner_role_independently() {
        let pattern = "deep[_-]?research|cli|tui|submission|acquisition|evidence|report|artifact|publication|browser|opening|inactive|legacy";
        assert!(workspace_code_trace_pattern(pattern));
        assert_eq!(
            workspace_content_search_patterns(pattern),
            vec![
                "SubmissionIntent|parse_deep_research_tui_query|execute_deepresearch|parse_deepresearch_args",
                "start_deep_research_workflow|run_deepresearch_inquiry|spawn_deep_research|deep_research_workflow",
                "bootstrap_acquisition|inquiry_runtime|retrieval|accepted_evidence|admit.*evidence|evidence_ledger",
                "report_generation|sectioned_report|materialize.*report|generate.*report",
                "publication|publish|write_research_report|replace_staged",
                "stage_deep_research_report|open_pending_deep_research_report_view|open_remote_view|open_window_with|local_file_view",
                "LegacyCheckedLoop|legacy|compatibility|replay",
            ]
        );
        assert!(!workspace_code_trace_pattern(
            "report|artifact|publish|browser|open"
        ));
    }

    #[test]
    fn focused_workspace_pattern_drops_only_high_volume_generic_terms() {
        assert_eq!(
            focused_workspace_search_pattern("report|artifact|publish|browser|open"),
            "browser|publish"
        );
        assert_eq!(
            focused_workspace_search_pattern("evidence|acqui(sition|re)|admit"),
            "acqui(sition|re)|admit"
        );
    }

    #[test]
    fn workspace_inventory_uses_recursive_globs_and_ignores_page_footer() {
        assert_eq!(workspace_inventory_pattern(""), "**/*");
        assert_eq!(workspace_inventory_pattern("*.rs"), "**/*.rs");
        assert_eq!(workspace_inventory_pattern("src/**/*.rs"), "src/**/*.rs");
        assert_eq!(
            workspace_inventory_request("", ""),
            (".".to_string(), "**/*".to_string())
        );
        assert_eq!(
            workspace_inventory_request("src/tui/app", "*.rs"),
            ("src/tui/app".to_string(), "**/*.rs".to_string())
        );
        assert_eq!(
            workspace_inventory_request("src/tui/app/submit.rs", "*.rs"),
            ("src/tui/app".to_string(), "submit.rs".to_string())
        );

        let output = "src/commands/code/research_runtime.rs\nsrc/tui/app/submit.rs\nsrc/tui/deep_research/artifacts/publication.rs\n\n3 of 3 file(s) shown";
        assert_eq!(
            workspace_inventory_paths(output),
            vec![
                "src/commands/code/research_runtime.rs",
                "src/tui/app/submit.rs",
                "src/tui/deep_research/artifacts/publication.rs",
            ]
        );
        assert!(workspace_inventory_paths("No files found matching pattern: **/*.rs").is_empty());
    }

    #[test]
    fn workspace_code_bootstrap_inventories_only_production_rust_sources() {
        let query = bootstrap_query(&PlannerInput {
            schema: "test".to_string(),
            query: "In this repository, trace the active DeepResearch path from CLI or TUI submission through acquisition, evidence handling, report generation, artifact publication, and browser opening.".to_string(),
            report_language: "en".to_string(),
            current_date: "2026-07-22".to_string(),
            display_utc_offset: "+08:00".to_string(),
            evidence_scope: EvidenceScope::Workspace,
            budget: PlannerBudget {
                max_queries: 4,
                max_acquired_sources: 8,
            },
        });
        assert_eq!(query.transport, AcquisitionTransport::Workspace);
        assert_eq!(query.path, "src");
        assert_eq!(query.glob, "*.rs");
        assert!(query.text.contains("deep[_-]?research"), "{}", query.text);
    }

    #[test]
    fn safe_preferred_url_becomes_a_fetchable_candidate() {
        let mut discoveries = vec![QueryDiscovery {
            query: AcquisitionQuery {
                id: "q1".to_string(),
                text: "Tokio canonical LTS README policy".to_string(),
                transport: AcquisitionTransport::Web,
                path: String::new(),
                glob: String::new(),
                dimension_ids: vec!["policy".to_string()],
                source_target_ids: Vec::new(),
                preferred_sources: vec![SourcePreference {
                    kind: PreferredSourceKind::Url,
                    value: "https://github.com/tokio-rs/tokio/blob/master/README.md".to_string(),
                }],
                fetch_slots: 0,
            },
            candidates: Vec::new(),
            error: None,
            elapsed_ms: 1,
        }];
        attach_preferred_candidates(&mut discoveries);
        assert_eq!(discoveries[0].candidates.len(), 1);
        assert_eq!(
            discoveries[0].candidates[0].anchor,
            "https://github.com/tokio-rs/tokio/blob/master/README.md"
        );
    }

    #[test]
    fn repository_identity_gets_a_bounded_default_readme_candidate() {
        let mut discoveries = vec![QueryDiscovery {
            query: AcquisitionQuery {
                id: "q1".to_string(),
                text: "Tokio canonical security policy".to_string(),
                transport: AcquisitionTransport::Web,
                path: String::new(),
                glob: String::new(),
                dimension_ids: vec!["policy".to_string()],
                source_target_ids: Vec::new(),
                preferred_sources: vec![SourcePreference {
                    kind: PreferredSourceKind::Repository,
                    value: "tokio-rs/tokio".to_string(),
                }],
                fetch_slots: 0,
            },
            candidates: Vec::new(),
            error: None,
            elapsed_ms: 1,
        }];
        attach_preferred_candidates(&mut discoveries);
        assert_eq!(discoveries[0].candidates.len(), 1);
        assert_eq!(
            discoveries[0].candidates[0].anchor,
            "https://github.com/tokio-rs/tokio/blob/HEAD/README.md"
        );
    }

    #[test]
    fn release_query_with_repository_identity_gets_a_releases_candidate() {
        let mut discoveries = vec![QueryDiscovery {
            query: AcquisitionQuery {
                id: "q1".to_string(),
                text: "Tokio newest release version".to_string(),
                transport: AcquisitionTransport::Web,
                path: String::new(),
                glob: String::new(),
                dimension_ids: vec!["release".to_string()],
                source_target_ids: Vec::new(),
                preferred_sources: vec![SourcePreference {
                    kind: PreferredSourceKind::Repository,
                    value: "tokio-rs/tokio".to_string(),
                }],
                fetch_slots: 0,
            },
            candidates: Vec::new(),
            error: Some("search unavailable".to_string()),
            elapsed_ms: 1,
        }];
        attach_preferred_candidates(&mut discoveries);
        assert!(discoveries[0]
            .candidates
            .iter()
            .any(|candidate| candidate.anchor == "https://github.com/tokio-rs/tokio/releases"));
    }

    #[test]
    fn github_blob_fetch_uses_focused_raw_content() {
        assert_eq!(
            github_raw_blob_anchor(
                "https://github.com/tokio-rs/tokio/blob/tokio-1.47.x/tokio/Cargo.toml"
            )
            .as_deref(),
            Some("https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.47.x/tokio/Cargo.toml")
        );
    }

    #[test]
    fn fetched_repository_root_yields_default_branch_readme_follow() {
        let root = "https://github.com/tokio-rs/tokio";
        let edge = SelectionEdge {
            query_id: "q1".to_string(),
            source_target_id: None,
            match_score: 1,
        };
        let sources = vec![AcquiredSource {
            id: String::new(),
            title: "Tokio".to_string(),
            requested_anchor: root.to_string(),
            canonical_anchor: root.to_string(),
            transport: AcquisitionTransport::Web,
            captured_at: "2026-07-22T00:00:00Z".to_string(),
            provenance: vec![edge.clone()],
            chunks: vec![serde_json::json!({
                "id": "pending",
                "text": "{\"refInfo\":{\"name\":\"master\"},\"path\":\"README.md\"}"
            })],
            fetch_completed_ms: 1,
            persisted_ms: Some(2),
        }];
        let selected = vec![SelectedCandidate {
            candidate: SourceCandidate {
                title: "Tokio".to_string(),
                anchor: root.to_string(),
                preview: String::new(),
                provider_score: 1.0,
                transport: AcquisitionTransport::Web,
            },
            edges: vec![edge],
        }];
        let follows = github_readme_follow_candidates(&sources, &selected);
        assert_eq!(follows.len(), 1);
        assert_eq!(
            follows[0].candidate.anchor,
            "https://github.com/tokio-rs/tokio/blob/master/README.md"
        );
    }
}

fn build_compiler_catalog(
    planning: &PlanningResult,
    discoveries: &[QueryDiscovery],
    sources: &[AcquiredSource],
    failures: &[AcquisitionFailure],
) -> Result<JsonValue, String> {
    let plan = planning
        .plan
        .as_ref()
        .ok_or_else(|| "compiler acquisition omitted its query plan".to_string())?;
    let spec = planning
        .spec
        .as_ref()
        .ok_or_else(|| "compiler acquisition omitted its research spec".to_string())?;
    let successful_edges = sources
        .iter()
        .flat_map(|source| source.provenance.iter())
        .filter_map(|edge| {
            edge.source_target_id
                .as_ref()
                .map(|target| (edge.query_id.clone(), target.clone()))
        })
        .collect::<BTreeSet<_>>();
    let failed_edges = failures
        .iter()
        .flat_map(|failure| {
            failure.edges.iter().filter_map(move |edge| {
                edge.source_target_id.as_ref().map(|target| {
                    (
                        (edge.query_id.clone(), target.clone()),
                        failure.reason.clone(),
                    )
                })
            })
        })
        .collect::<BTreeMap<_, _>>();
    let discovery_errors = discoveries
        .iter()
        .filter_map(|discovery| {
            discovery
                .error
                .as_ref()
                .map(|error| (discovery.query.id.as_str(), error.as_str()))
        })
        .collect::<BTreeMap<_, _>>();
    let attempts = planning
        .queries
        .iter()
        .flat_map(|query| {
            query.source_target_ids.iter().map(|target_id| {
                let edge = (query.id.clone(), target_id.clone());
                let outcome = if successful_edges.contains(&edge) {
                    serde_json::json!({ "status": "fetched" })
                } else if let Some(reason) = failed_edges.get(&edge) {
                    serde_json::json!({ "status": "failed", "reason": bounded_error(reason) })
                } else if let Some(reason) = discovery_errors.get(query.id.as_str()) {
                    serde_json::json!({ "status": "failed", "reason": bounded_error(reason) })
                } else {
                    serde_json::json!({ "status": "no_candidates" })
                };
                serde_json::json!({
                    "query_id": query.id,
                    "source_target_ids": [target_id],
                    "outcome": outcome,
                })
            })
        })
        .collect::<Vec<_>>();
    let records = sources
        .iter()
        .map(|source| {
            let provenance = source
                .provenance
                .iter()
                .filter_map(|edge| {
                    edge.source_target_id.as_ref().map(|target_id| {
                        serde_json::json!({
                            "query_id": edge.query_id,
                            "source_target_id": target_id,
                        })
                    })
                })
                .collect::<Vec<_>>();
            let chunks = JsonValue::Array(source.chunks.clone());
            let digest = a3s::research::compiler::evidence_source_content_digest(&chunks)
                .map_err(|error| error.to_string())?;
            Ok(serde_json::json!({
                "id": source.id,
                "title": source.title,
                "requested_anchor": source.requested_anchor,
                "canonical_anchor": source.canonical_anchor,
                "captured_at": source.captured_at,
                "provenance": provenance,
                "chunks": chunks,
                "content_digest": digest,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let catalog = serde_json::json!({
        "spec_digest": plan["spec_digest"],
        "attempts": attempts,
        "sources": records,
    });
    a3s::research::compiler::validate_evidence_catalog(spec, plan, &catalog)
        .map_err(|error| error.to_string())?;
    Ok(catalog)
}

fn safe_https_anchor(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn safe_workspace_anchor(value: &str) -> bool {
    let path = std::path::Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

fn canonical_anchor_key(value: &str) -> String {
    let normalized = value.trim().trim_end_matches('/').to_ascii_lowercase();
    normalized.replace("https://redirect.github.com/", "https://github.com/")
}

fn bounded_text(value: &str, maximum: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(maximum)
        .collect()
}

fn bounded_error(value: &str) -> String {
    let value = bounded_text(value, 1_000);
    if value.is_empty() {
        "acquisition failed without a diagnostic".to_string()
    } else {
        value
    }
}
