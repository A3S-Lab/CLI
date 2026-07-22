use super::*;
use a3s_code_core::llm::structured::{generate_blocking, StructuredMode, StructuredRequest};
use futures::future::join_all;
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet, HashSet};

const C01_QUERY: &str = "截至2026年7月21日，比较 Tokio 与 async-std 的维护状态、HTTP 与数据库生态兼容性，以及新项目和存量项目的生产选型取舍。优先使用项目或库的官方资料，区分事实、判断与证据缺口。";

const C01_DECOMPOSED_QUERIES: [&str; 4] = [
    "Tokio async-std maintenance status official async-std discontinued RustSec advisory",
    "official Hyper Axum Tide HTTP runtime Tokio async-std requirements documentation",
    "official SQLx database runtime Tokio async-std requirements documentation",
    "Tokio async-std production migration official recommendation guidance",
];

const C01_PLANNED_SEARCH_COUNT: usize = 4;
const C01_PLANNED_DIMENSION_COUNT: usize = 6;
const C01_FETCH_BUDGET: usize = 8;
const C01_MAX_TARGETS_PER_SEARCH: usize = C01_FETCH_BUDGET / C01_PLANNED_SEARCH_COUNT;

#[tokio::test]
#[ignore = "live bounded-plan/four-query/eight-fetch DeepResearch acquisition measurement"]
async fn live_c01_bounded_plan_acquisition_probe() {
    let home = std::env::var_os("HOME").expect("HOME is required");
    let config_path = std::env::var_os("A3S_DEEP_RESEARCH_EVAL_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home).join(".a3s/config.acl"));
    let output_dir = std::env::var_os("A3S_DEEP_RESEARCH_EVAL_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/deep-research-eval/live/C01/bounded-plan")
        });
    let model = std::env::var("A3S_DEEP_RESEARCH_EVAL_MODEL")
        .unwrap_or_else(|_| "openai/gpt-4o-mini".to_string());
    std::fs::create_dir_all(&output_dir).expect("create live acquisition output directory");
    let config = CodeConfig::from_file(&config_path).expect("load live acquisition config");
    let (session, _) = build_deepresearch_session(
        output_dir.to_string_lossy().as_ref(),
        config.clone(),
        output_dir.join("memory"),
    )
    .await
    .expect("create live acquisition session");
    let options = SessionOptions::new()
        .with_model(model.clone())
        .with_llm_api_timeout(30_000);
    let llm = crate::session_llm::resolve_session_llm_client(
        &config,
        &options,
        &format!("deep-research-plan-eval-{}", std::process::id()),
    )
    .expect("resolve live query-planning model");

    let started = Instant::now();
    let plan_started = Instant::now();
    let plan_request = StructuredRequest {
        prompt: bounded_query_plan_prompt(C01_QUERY),
        system: Some(
            "You identify research dimensions and source-seeking web queries. Do not answer the research question. Return only the requested object."
                .to_string(),
        ),
        schema: bounded_query_plan_schema(),
        schema_name: "deep_research_bounded_query_plan".to_string(),
        schema_description: Some(
            "Material research dimensions and exactly four focused source-seeking queries"
                .to_string(),
        ),
        mode: StructuredMode::Auto,
        max_repair_attempts: 0,
    };
    let plan_result = match generate_blocking(llm.as_ref(), &plan_request).await {
        Ok(result) => result,
        Err(error) => {
            std::fs::write(
                output_dir.join("query-plan-generation-error.txt"),
                format!("{error:#}"),
            )
            .expect("persist query-plan generation failure");
            panic!("generate bounded live query plan: {error:#}");
        }
    };
    let plan_elapsed_ms = plan_started.elapsed().as_millis() as u64;
    std::fs::write(
        output_dir.join("query-plan.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "object": plan_result.object,
            "usage": plan_result.usage,
            "repair_rounds": plan_result.repair_rounds,
            "mode_used": format!("{:?}", plan_result.mode_used),
            "elapsed_ms": plan_elapsed_ms,
        }))
        .expect("encode bounded query-plan proposal"),
    )
    .expect("persist bounded query-plan proposal");
    if let Err(error) = validate_bounded_query_plan(&plan_result.object) {
        std::fs::write(output_dir.join("query-plan-validation-error.txt"), &error)
            .expect("persist query-plan validation failure");
        panic!("validate bounded live query plan: {error}");
    }
    let planned_queries = plan_result.object["searches"]
        .as_array()
        .expect("validated planned searches")
        .iter()
        .map(host_search_query)
        .collect::<Vec<_>>();

    let search_started = Instant::now();
    let mut search_groups = join_all(
        planned_queries
            .iter()
            .map(|query| search_group(&session, query)),
    )
    .await;
    for (group, search) in search_groups.iter_mut().zip(
        plan_result.object["searches"]
            .as_array()
            .expect("validated planned searches"),
    ) {
        if let Some(group) = group.as_object_mut() {
            group.insert(
                "source_family_id".to_string(),
                search["source_family_id"].clone(),
            );
            group.insert("dimension_ids".to_string(), search["dimension_ids"].clone());
            group.insert("targets".to_string(), search["targets"].clone());
            group.insert(
                "artifact_kind".to_string(),
                search["artifact_kind"].clone(),
            );
        }
    }
    let search_elapsed_ms = search_started.elapsed().as_millis() as u64;
    assert_eq!(
        search_groups.len(),
        4,
        "bounded plan must use four searches"
    );

    let round_robin_selected = round_robin_candidates(&search_groups, C01_FETCH_BUDGET);
    attach_canonical_target_seeds(&mut search_groups);
    let selected =
        target_balanced_candidates(&search_groups, &plan_result.object, C01_FETCH_BUDGET);
    assert!(!selected.is_empty(), "bounded search retained no candidate");
    assert!(
        selected
            .iter()
            .all(|candidate| candidate["matched_source_target"].is_string()),
        "every admitted candidate must identify one declared source target"
    );
    let target_admission = target_admission_coverage(&plan_result.object, &selected);
    let fetch_started = Instant::now();
    let fetches = join_all(selected.iter().map(|candidate| {
        session.tool(
            "web_fetch",
            serde_json::json!({
                "url": candidate["url"],
                "format": "markdown",
                "timeout": 20,
            }),
        )
    }))
    .await;
    let fetch_elapsed_ms = fetch_started.elapsed().as_millis() as u64;
    let fetched = fetched_sources(&selected, fetches);
    let fetched_source_count = successful_fetch_count(&fetched);
    let fetched_target_coverage = target_fetch_coverage(&plan_result.object, &fetched);

    let result = serde_json::json!({
        "schema": "a3s/deep-research-live-acquisition/v1",
        "case_id": "C01",
        "query": C01_QUERY,
        "strategy": "bounded-four-query-plan",
        "model": model,
        "query_plan": plan_result.object,
        "search_queries": planned_queries,
        "budgets": {
            "query_plan_generations": 1,
            "searches": 4,
            "fetches": C01_FETCH_BUDGET,
        },
        "timings_ms": {
            "query_plan": plan_elapsed_ms,
            "search": search_elapsed_ms,
            "fetch": fetch_elapsed_ms,
            "total": started.elapsed().as_millis() as u64,
        },
        "search_groups": search_groups,
        "round_robin_selected_candidates": round_robin_selected,
        "selected_candidates": selected,
        "target_admission": target_admission,
        "fetched_target_coverage": fetched_target_coverage,
        "fetched_sources": fetched,
        "fetched_source_count": fetched_source_count,
        "generated_at": chrono::Utc::now().to_rfc3339(),
    });
    let output = output_dir.join("acquisition.json");
    std::fs::write(
        &output,
        serde_json::to_vec_pretty(&result).expect("encode live acquisition result"),
    )
    .expect("write live acquisition result");
    eprintln!(
        "C01 bounded-plan acquisition fetched {fetched_source_count} sources in {:.2}s; artifact: {}",
        started.elapsed().as_secs_f64(),
        output.display()
    );
    assert!(fetched_source_count > 0, "bounded fetch retained no source");
}

#[tokio::test]
#[ignore = "live four-query/eight-fetch DeepResearch acquisition measurement"]
async fn live_c01_decomposed_acquisition_probe() {
    let home = std::env::var_os("HOME").expect("HOME is required");
    let config_path = std::env::var_os("A3S_DEEP_RESEARCH_EVAL_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home).join(".a3s/config.acl"));
    let output_dir = std::env::var_os("A3S_DEEP_RESEARCH_EVAL_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/deep-research-eval/live/C01/decomposed")
        });
    std::fs::create_dir_all(&output_dir).expect("create live acquisition output directory");
    let config = CodeConfig::from_file(&config_path).expect("load live acquisition config");
    let (session, _) = build_deepresearch_session(
        output_dir.to_string_lossy().as_ref(),
        config,
        output_dir.join("memory"),
    )
    .await
    .expect("create live acquisition session");

    let started = Instant::now();
    let searches = join_all(C01_DECOMPOSED_QUERIES.iter().map(|query| {
        session.tool(
            "web_search",
            serde_json::json!({
                "query": query,
                "engines": ["anysearch", "tavily", "ddg"],
                "format": "json",
                "limit": 8,
                "timeout": 12,
            }),
        )
    }))
    .await;
    let search_elapsed_ms = started.elapsed().as_millis() as u64;

    let mut search_groups = Vec::new();
    for (query, result) in C01_DECOMPOSED_QUERIES.iter().zip(searches) {
        let result = result.unwrap_or_else(|error| panic!("search `{query}` failed: {error:#}"));
        assert_eq!(result.exit_code, 0, "search `{query}`: {}", result.output);
        let results = serde_json::from_str::<Vec<JsonValue>>(&result.output)
            .unwrap_or_else(|error| panic!("decode search `{query}`: {error}"));
        search_groups.push(serde_json::json!({
            "query": query,
            "results": results,
        }));
    }

    let selected = round_robin_candidates(&search_groups, 8);
    assert!(
        !selected.is_empty(),
        "decomposed search retained no candidate"
    );
    let fetch_started = Instant::now();
    let fetches = join_all(selected.iter().map(|candidate| {
        session.tool(
            "web_fetch",
            serde_json::json!({
                "url": candidate["url"],
                "format": "markdown",
                "timeout": 20,
            }),
        )
    }))
    .await;
    let fetch_elapsed_ms = fetch_started.elapsed().as_millis() as u64;

    let fetched = fetched_sources(&selected, fetches);
    let fetched_source_count = successful_fetch_count(&fetched);

    let result = serde_json::json!({
        "schema": "a3s/deep-research-live-acquisition/v1",
        "case_id": "C01",
        "query": C01_QUERY,
        "search_queries": C01_DECOMPOSED_QUERIES,
        "budgets": {
            "searches": 4,
            "fetches": 8,
        },
        "timings_ms": {
            "search": search_elapsed_ms,
            "fetch": fetch_elapsed_ms,
            "total": started.elapsed().as_millis() as u64,
        },
        "search_groups": search_groups,
        "selected_candidates": selected,
        "fetched_sources": fetched,
        "fetched_source_count": fetched_source_count,
        "generated_at": chrono::Utc::now().to_rfc3339(),
    });
    let output = output_dir.join("acquisition.json");
    std::fs::write(
        &output,
        serde_json::to_vec_pretty(&result).expect("encode live acquisition result"),
    )
    .expect("write live acquisition result");
    eprintln!(
        "C01 decomposed acquisition fetched {fetched_source_count} sources in {:.2}s; artifact: {}",
        started.elapsed().as_secs_f64(),
        output.display()
    );
    assert!(
        fetched_source_count > 0,
        "decomposed fetch retained no source"
    );
}

fn round_robin_candidates(search_groups: &[JsonValue], maximum: usize) -> Vec<JsonValue> {
    let mut selected = Vec::<JsonValue>::new();
    let mut selected_urls = HashSet::new();
    let mut cursors = vec![0usize; search_groups.len()];
    let mut made_progress = true;
    while selected.len() < maximum && made_progress {
        made_progress = false;
        for (group_index, group) in search_groups.iter().enumerate() {
            let Some(results) = group["results"].as_array() else {
                continue;
            };
            while let Some(candidate) = results.get(cursors[group_index]) {
                cursors[group_index] += 1;
                let Some(url) = candidate["url"]
                    .as_str()
                    .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
                else {
                    continue;
                };
                if selected_urls.insert(url.to_string()) {
                    let mut selected_candidate = candidate.clone();
                    if let Some(object) = selected_candidate.as_object_mut() {
                        object.insert("query_index".to_string(), group_index.into());
                        object.insert("search_query".to_string(), group["query"].clone());
                    }
                    selected.push(selected_candidate);
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

async fn search_group(session: &AgentSession, query: &str) -> JsonValue {
    let result = session
        .tool(
            "web_search",
            serde_json::json!({
                "query": query,
                "engines": ["anysearch", "tavily", "ddg"],
                "format": "json",
                "limit": 8,
                "timeout": 12,
            }),
        )
        .await
        .unwrap_or_else(|error| panic!("search `{query}` failed: {error:#}"));
    assert_eq!(result.exit_code, 0, "search `{query}`: {}", result.output);
    let results = serde_json::from_str::<Vec<JsonValue>>(&result.output)
        .unwrap_or_else(|error| panic!("decode search `{query}`: {error}"));
    serde_json::json!({
        "query": query,
        "results": results,
    })
}

fn host_search_query(search: &JsonValue) -> String {
    let mut query = search["query"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();
    for target in search["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
    {
        if !query.to_ascii_lowercase().contains(&target.to_ascii_lowercase()) {
            query.push(' ');
            query.push_str(target);
        }
    }
    let artifact_terms = match search["artifact_kind"].as_str() {
        Some("maintenance_record") => "official releases changelog maintenance status",
        Some("documentation") => "official documentation README runtime support",
        Some("guidance") => "official migration guidance recommendation",
        _ => "official source",
    };
    query.push(' ');
    query.push_str(artifact_terms);
    query
}

fn attach_canonical_target_seeds(search_groups: &mut [JsonValue]) {
    for group in search_groups {
        let targets = group["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(JsonValue::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>();
        let Some(results) = group["results"].as_array_mut() else {
            continue;
        };
        let mut known_urls = results
            .iter()
            .filter_map(|candidate| candidate["url"].as_str())
            .map(canonical_candidate_url_key)
            .collect::<HashSet<_>>();
        for target in targets {
            let url = canonical_target_seed_url(&target);
            if known_urls.insert(canonical_candidate_url_key(&url)) {
                results.push(serde_json::json!({
                    "title": target,
                    "url": url,
                    "content": "",
                    "score": 0.0,
                    "candidate_origin": "canonical_target_seed",
                }));
            }
        }
    }
}

fn canonical_target_seed_url(target: &str) -> String {
    let authority = target.split('/').next().unwrap_or_default();
    if authority.contains('.') {
        format!("https://{target}")
    } else {
        format!("https://github.com/{target}")
    }
}

fn canonical_candidate_url_key(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn fetched_sources(
    selected: &[JsonValue],
    fetches: Vec<Result<ToolCallResult, a3s_code_core::CodeError>>,
) -> Vec<JsonValue> {
    selected
        .iter()
        .zip(fetches)
        .map(|(candidate, result)| match result {
            Ok(result) => serde_json::json!({
                "title": candidate["title"],
                "url": candidate["url"],
                "query_index": candidate["query_index"],
                "query_indices": candidate["query_indices"],
                "search_query": candidate["search_query"],
                "search_queries": candidate["search_queries"],
                "source_family_id": candidate["source_family_id"],
                "source_family_ids": candidate["source_family_ids"],
                "artifact_kinds": candidate["artifact_kinds"],
                "dimension_ids": candidate["dimension_ids"],
                "all_dimension_ids": candidate["all_dimension_ids"],
                "source_targets": candidate["source_targets"],
                "matched_source_target": candidate["matched_source_target"],
                "matched_source_targets": candidate["matched_source_targets"],
                "target_match_score": candidate["target_match_score"],
                "target_edges": candidate["target_edges"],
                "exit_code": result.exit_code,
                "content": result.output,
            }),
            Err(error) => serde_json::json!({
                "title": candidate["title"],
                "url": candidate["url"],
                "query_index": candidate["query_index"],
                "query_indices": candidate["query_indices"],
                "search_query": candidate["search_query"],
                "search_queries": candidate["search_queries"],
                "source_family_id": candidate["source_family_id"],
                "source_family_ids": candidate["source_family_ids"],
                "artifact_kinds": candidate["artifact_kinds"],
                "dimension_ids": candidate["dimension_ids"],
                "all_dimension_ids": candidate["all_dimension_ids"],
                "source_targets": candidate["source_targets"],
                "matched_source_target": candidate["matched_source_target"],
                "matched_source_targets": candidate["matched_source_targets"],
                "target_match_score": candidate["target_match_score"],
                "target_edges": candidate["target_edges"],
                "error": format!("{error:#}"),
            }),
        })
        .collect()
}

fn successful_fetch_count(fetched: &[JsonValue]) -> usize {
    fetched
        .iter()
        .filter(|source| {
            source["exit_code"].as_i64() == Some(0)
                && source["content"]
                    .as_str()
                    .is_some_and(|content| !content.trim().is_empty())
        })
        .count()
}

fn bounded_query_plan_prompt(query: &str) -> String {
    format!(
        "Decompose QUERY into exactly {C01_PLANNED_DIMENSION_COUNT} independently assessable material dimensions: current maintenance for both runtimes, official Tokio HTTP support, official async-std HTTP support, official database compatibility, the new-project decision, and the legacy-project migration decision. Preserve those separate action scenarios. Treat named libraries as source examples, not permission to invent extra middleware, connection-pool, performance, adoption, or implementation dimensions. Define exactly {C01_PLANNED_SEARCH_COUNT} stable ASCII source families. A source family is one coherent class of authoritative artifacts, not one publisher or project. Maintenance records, HTTP documentation, database documentation, and migration or choice guidance are different families. Assign every dimension exactly one declared source_family_id. Propose exactly {C01_PLANNED_SEARCH_COUNT} concise, non-overlapping web searches that together cover every dimension. Every search must repeat one declared source_family_id and may reference only dimensions with that family. Never combine unrelated families merely to reduce query count. Set artifact_kind=maintenance_record for the maintenance search, documentation for HTTP and database searches, and guidance for the new-project and legacy-project search. Write each search in the language most likely to retrieve its primary sources. Name one or two required canonical source targets per search so the targets fit the Host's two fetch slots. The HTTP search must target one canonical Tokio HTTP library and one canonical async-std HTTP library or adapter, not the runtime repositories themselves; tokio-rs/axum and http-rs/tide are representative identities. A canonical target may repeat across maintenance and guidance only when that authority genuinely serves both families; the Host will merge an identical selected URL and retain both provenance edges. Use stable non-URL identifiers: owner/repository when known, such as tokio-rs/tokio, async-rs/async-std, tokio-rs/axum, launchbadge/sqlx, or SeaQL/sea-orm; use an official domain such as rustsec.org only when no repository identity is appropriate. Do not use descriptions such as 'Tokio documentation' as target identifiers. Include the target identifiers and source-family terms in the search text. Prioritize targets needed for the explicit comparison and decisions. Do not answer QUERY, state conclusions, or invent facts.\n\nQUERY={query}"
    )
}

fn bounded_query_plan_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "source_families": {
                "type": "array",
                "minItems": C01_PLANNED_SEARCH_COUNT,
                "maxItems": C01_PLANNED_SEARCH_COUNT,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "id": { "type": "string", "pattern": "^[a-z][a-z0-9_]{0,63}$" },
                        "description": { "type": "string", "minLength": 4, "maxLength": 360 }
                    },
                    "required": ["id", "description"]
                }
            },
            "dimensions": {
                "type": "array",
                "minItems": C01_PLANNED_DIMENSION_COUNT,
                "maxItems": C01_PLANNED_DIMENSION_COUNT,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "id": { "type": "string", "pattern": "^d[1-6]$" },
                        "question": { "type": "string", "minLength": 4, "maxLength": 240 },
                        "source_family_id": { "type": "string", "pattern": "^[a-z][a-z0-9_]{0,63}$" },
                        "artifact_kind": {
                            "type": "string",
                            "enum": ["maintenance_record", "documentation", "guidance"]
                        },
                        "source_requirement": { "type": "string", "minLength": 4, "maxLength": 180 }
                    },
                    "required": ["id", "question", "source_family_id", "source_requirement"]
                }
            },
            "searches": {
                "type": "array",
                "minItems": C01_PLANNED_SEARCH_COUNT,
                "maxItems": C01_PLANNED_SEARCH_COUNT,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "query": { "type": "string", "minLength": 4, "maxLength": 240 },
                        "source_family_id": { "type": "string", "pattern": "^[a-z][a-z0-9_]{0,63}$" },
                        "dimension_ids": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": C01_PLANNED_DIMENSION_COUNT,
                            "uniqueItems": true,
                            "items": { "type": "string", "pattern": "^d[1-6]$" }
                        },
                        "targets": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": C01_MAX_TARGETS_PER_SEARCH,
                            "uniqueItems": true,
                            "items": {
                                "type": "string",
                                "minLength": 2,
                                "maxLength": 100,
                                "pattern": "^[A-Za-z0-9][A-Za-z0-9._-]*(/[A-Za-z0-9][A-Za-z0-9._-]*)?$"
                            }
                        },
                        "source_goal": { "type": "string", "minLength": 4, "maxLength": 180 }
                    },
                    "required": ["query", "source_family_id", "artifact_kind", "dimension_ids", "targets", "source_goal"]
                }
            }
        },
        "required": ["source_families", "dimensions", "searches"]
    })
}

fn validate_bounded_query_plan(plan: &JsonValue) -> Result<(), String> {
    let source_families = plan["source_families"]
        .as_array()
        .ok_or_else(|| "bounded plan omitted source families".to_string())?;
    let dimensions = plan["dimensions"]
        .as_array()
        .ok_or_else(|| "bounded plan omitted dimensions".to_string())?;
    let searches = plan["searches"]
        .as_array()
        .ok_or_else(|| "bounded plan omitted searches".to_string())?;
    if source_families.len() != C01_PLANNED_SEARCH_COUNT {
        return Err(format!(
            "bounded plan must contain exactly {C01_PLANNED_SEARCH_COUNT} source families"
        ));
    }
    if dimensions.len() != C01_PLANNED_DIMENSION_COUNT {
        return Err(format!(
            "bounded plan must contain exactly {C01_PLANNED_DIMENSION_COUNT} dimensions"
        ));
    }
    if searches.len() != C01_PLANNED_SEARCH_COUNT {
        return Err(format!(
            "bounded plan must contain exactly {C01_PLANNED_SEARCH_COUNT} focused searches"
        ));
    }
    let declared_families = source_families
        .iter()
        .map(|family| {
            let id = family["id"]
                .as_str()
                .ok_or_else(|| "bounded source family omitted its ID".to_string())?;
            let description = family["description"]
                .as_str()
                .map(str::trim)
                .filter(|description| !description.is_empty())
                .ok_or_else(|| format!("bounded source family `{id}` omitted its description"))?;
            Ok((id, description))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    if declared_families.len() != source_families.len() {
        return Err("bounded plan repeated a source-family ID".to_string());
    }
    let dimension_families = dimensions
        .iter()
        .map(|dimension| {
            let id = dimension["id"]
                .as_str()
                .ok_or_else(|| "bounded dimension omitted its ID".to_string())?;
            let family = dimension["source_family_id"]
                .as_str()
                .ok_or_else(|| format!("bounded dimension `{id}` omitted its source family"))?;
            if !declared_families.contains_key(family) {
                return Err(format!(
                    "bounded dimension `{id}` referenced undeclared source family `{family}`"
                ));
            }
            Ok((id, family))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    if dimension_families.len() != dimensions.len() {
        return Err("bounded plan repeated a dimension ID".to_string());
    }
    let dimension_ids = dimension_families.keys().copied().collect::<BTreeSet<_>>();
    let expected_dimension_ids = (1..=C01_PLANNED_DIMENSION_COUNT)
        .map(|index| format!("d{index}"))
        .collect::<BTreeSet<_>>();
    if dimension_ids
        != expected_dimension_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
    {
        return Err("bounded plan dimension IDs must be the complete d1-d6 registry".to_string());
    }
    let mut covered = BTreeSet::new();
    let mut searched_families = BTreeSet::new();
    let mut queries = BTreeSet::new();
    for search in searches {
        let search_family = search["source_family_id"]
            .as_str()
            .ok_or_else(|| "bounded planned search omitted its source family".to_string())?;
        if !declared_families.contains_key(search_family) {
            return Err(format!(
                "bounded planned search referenced undeclared source family `{search_family}`"
            ));
        }
        searched_families.insert(search_family);
        let query = search["query"]
            .as_str()
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .ok_or_else(|| "bounded planned search omitted its query".to_string())?;
        if query == C01_QUERY || !queries.insert(query.to_ascii_lowercase()) {
            return Err("bounded planned searches repeated a query".to_string());
        }
        let search_dimension_ids = search["dimension_ids"]
            .as_array()
            .ok_or_else(|| "bounded planned search omitted dimension IDs".to_string())?;
        for id in search_dimension_ids {
            let id = id.as_str().ok_or_else(|| {
                "bounded planned search used a non-string dimension ID".to_string()
            })?;
            if !dimension_ids.contains(id) {
                return Err(format!(
                    "bounded planned search referenced unknown dimension `{id}`"
                ));
            }
            if dimension_families.get(id).copied() != Some(search_family) {
                return Err(format!(
                    "bounded planned search family `{search_family}` differs from dimension `{id}` family"
                ));
            }
            covered.insert(id);
        }
        let search_dimension_ids = search_dimension_ids
            .iter()
            .filter_map(JsonValue::as_str)
            .collect::<BTreeSet<_>>();
        let expected_artifact_kind = if search_dimension_ids.contains("d1") {
            "maintenance_record"
        } else if search_dimension_ids.contains("d5") || search_dimension_ids.contains("d6") {
            "guidance"
        } else {
            "documentation"
        };
        let artifact_kind = search["artifact_kind"]
            .as_str()
            .ok_or_else(|| "bounded planned search omitted its artifact kind".to_string())?;
        if artifact_kind != expected_artifact_kind {
            return Err(format!(
                "bounded planned search for dimensions {search_dimension_ids:?} requires artifact kind `{expected_artifact_kind}`, not `{artifact_kind}`"
            ));
        }
        let targets = search["targets"]
            .as_array()
            .ok_or_else(|| "bounded planned search omitted source targets".to_string())?;
        if targets.is_empty() || targets.len() > C01_MAX_TARGETS_PER_SEARCH {
            return Err(format!(
                "bounded planned search must name one to {C01_MAX_TARGETS_PER_SEARCH} source targets"
            ));
        }
        for target in targets {
            let target = target
                .as_str()
                .map(str::trim)
                .filter(|target| !target.is_empty())
                .ok_or_else(|| "bounded planned search used a blank source target".to_string())?;
            if target.contains("http://") || target.contains("https://") {
                return Err("bounded planned source targets must be names, not URLs".to_string());
            }
            if !is_stable_source_target_id(target) {
                return Err(format!(
                    "bounded planned source target `{target}` is not a canonical identifier"
                ));
            }
            if search_dimension_ids.contains("d2")
                && search_dimension_ids.contains("d3")
                && matches!(
                    target.to_ascii_lowercase().as_str(),
                    "tokio-rs/tokio" | "async-rs/async-std"
                )
            {
                return Err(format!(
                    "bounded HTTP source target `{target}` identifies a runtime, not an HTTP library or adapter"
                ));
            }
        }
    }
    if covered != dimension_ids {
        return Err("bounded planned searches do not cover every dimension".to_string());
    }
    let used_families = dimension_families
        .values()
        .copied()
        .collect::<BTreeSet<_>>();
    if searched_families != used_families || used_families.len() != declared_families.len() {
        return Err(
            "bounded source families must each own a dimension and a planned search".to_string(),
        );
    }
    Ok(())
}

fn target_balanced_candidates(
    search_groups: &[JsonValue],
    plan: &JsonValue,
    maximum: usize,
) -> Vec<JsonValue> {
    if search_groups.is_empty() || maximum == 0 {
        return Vec::new();
    }
    let Some(searches) = plan["searches"].as_array() else {
        return Vec::new();
    };
    let candidates = flattened_candidates(search_groups);
    let mut selected = Vec::<JsonValue>::new();
    let mut selected_urls = HashSet::new();
    let mut group_selected_urls = vec![HashSet::<String>::new(); search_groups.len()];
    let mut group_transport_families = vec![HashSet::<String>::new(); search_groups.len()];
    let base_allocation = maximum / search_groups.len();
    let remainder = maximum % search_groups.len();
    let allocations = (0..search_groups.len())
        .map(|index| base_allocation + usize::from(index < remainder))
        .collect::<Vec<_>>();

    // First allocate one fetch opportunity to every declared target. A high
    // ranked result for one target cannot displace another target in the same
    // bounded query.
    for group_index in 0..search_groups.len() {
        let targets = searches
            .get(group_index)
            .and_then(|search| search["targets"].as_array())
            .into_iter()
            .flatten()
            .filter_map(JsonValue::as_str)
            .collect::<Vec<_>>();
        for target in targets {
            if group_selected_urls[group_index].len() >= allocations[group_index] {
                break;
            }
            let Some((candidate, match_score)) = best_candidate_for_target(
                &candidates,
                group_index,
                target,
                &group_selected_urls[group_index],
                &group_transport_families[group_index],
            ) else {
                continue;
            };
            let Some(url) = admit_target_candidate(
                candidate,
                target,
                match_score,
                &mut selected,
                &mut selected_urls,
                maximum,
            ) else {
                continue;
            };
            group_selected_urls[group_index].insert(url.clone());
            group_transport_families[group_index].insert(transport_source_family(&url));
        }
    }

    // Fill only remaining per-query slots and only with another candidate that
    // resolves to a declared target. Unrelated results never consume budget
    // merely to make the selected count equal the global maximum.
    loop {
        let mut made_progress = false;
        for group_index in 0..search_groups.len() {
            if selected.len() >= maximum
                || group_selected_urls[group_index].len() >= allocations[group_index]
            {
                continue;
            }
            let targets = searches
                .get(group_index)
                .and_then(|search| search["targets"].as_array())
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_str)
                .collect::<Vec<_>>();
            let best = targets
                .iter()
                .filter_map(|target| {
                    best_candidate_for_target(
                        &candidates,
                        group_index,
                        target,
                        &selected_urls,
                        &group_transport_families[group_index],
                    )
                    .map(|(candidate, score)| (candidate, *target, score))
                })
                .max_by_key(|(_, _, score)| *score);
            let Some((candidate, target, match_score)) = best else {
                continue;
            };
            let Some(url) = admit_target_candidate(
                candidate,
                target,
                match_score,
                &mut selected,
                &mut selected_urls,
                maximum,
            ) else {
                continue;
            };
            group_selected_urls[group_index].insert(url.clone());
            group_transport_families[group_index].insert(transport_source_family(&url));
            made_progress = true;
        }
        if selected.len() >= maximum || !made_progress {
            break;
        }
    }
    selected
}

fn best_candidate_for_target(
    candidates: &[JsonValue],
    group_index: usize,
    target: &str,
    excluded_urls: &HashSet<String>,
    selected_transport_families: &HashSet<String>,
) -> Option<(JsonValue, usize)> {
    candidates
        .iter()
        .filter(|candidate| candidate["query_index"].as_u64() == Some(group_index as u64))
        .filter(|candidate| {
            candidate["url"]
                .as_str()
                .is_some_and(|url| !excluded_urls.contains(url))
        })
        .filter_map(|candidate| {
            candidate_target_rank(candidate, target, selected_transport_families)
                .map(|score| (candidate.clone(), score))
        })
        .max_by_key(|(_, score)| *score)
}

fn candidate_target_rank(
    candidate: &JsonValue,
    target: &str,
    selected_transport_families: &HashSet<String>,
) -> Option<usize> {
    let identity_match = candidate_target_identity_match(candidate, target)?;
    let artifact_kind = candidate["artifact_kind"].as_str()?;
    let artifact_fitness = candidate_artifact_fitness(candidate, artifact_kind);
    if artifact_fitness == 0 {
        return None;
    }
    let url = candidate["url"].as_str().unwrap_or_default();
    let target_terms = distinctive_target_terms(target);
    let authority = source_authority_score(url, &target_terms);
    let new_transport =
        usize::from(!selected_transport_families.contains(&transport_source_family(url)));
    let provider_score = candidate["score"].as_f64().unwrap_or_default().max(0.0);
    Some(
        artifact_fitness * 10_000_000
            + identity_match * 1_000_000
            + new_transport * 100_000
            + authority * 10_000
            + (provider_score * 1_000.0) as usize,
    )
}

fn candidate_artifact_fitness(candidate: &JsonValue, artifact_kind: &str) -> usize {
    let url = candidate["url"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let title = candidate["title"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let content = candidate["content"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let searchable = format!("{title} {url} {content}");
    let issue_like = ["/issues/", "/pull/", "/discussions/", "/topics/"]
        .iter()
        .any(|marker| url.contains(marker));
    let github_path_segments = url
        .split_once("github.com/")
        .map(|(_, path)| {
            path.split(['?', '#', '/'])
                .filter(|segment| !segment.is_empty())
                .count()
        })
        .unwrap_or_default();
    let repository_root = url.contains("github.com/") && github_path_segments == 2;
    let explicit_documentation = url.contains("docs.rs/")
        || url.contains("/docs/")
        || url.contains("/documentation/")
        || url.contains("/readme")
        || title.contains("documentation")
        || title.contains("database & async runtime");

    match artifact_kind {
        "maintenance_record" => {
            if searchable.contains("officially sunset")
                || searchable.contains("officially been discontinued")
            {
                10
            } else if url.contains("changelog")
                || (url.ends_with("/releases") || url.ends_with("/releases/"))
            {
                9
            } else if url.contains("/releases/tag/") {
                7
            } else if repository_root || url.ends_with("/issues") {
                6
            } else {
                4
            }
        }
        "documentation" => {
            if issue_like {
                0
            } else if explicit_documentation {
                10
            } else if repository_root {
                8
            } else {
                6
            }
        }
        "guidance" => {
            if issue_like {
                0
            } else if (searchable.contains("recommend") || searchable.contains("guidance"))
                && (searchable.contains("migrat")
                    || searchable.contains("switch")
                    || searchable.contains("discontinued"))
            {
                10
            } else if ["migration", "guide", "tutorial", "changelog", "readme", "blog"]
                .iter()
                .any(|marker| searchable.contains(marker))
            {
                9
            } else if repository_root {
                7
            } else {
                5
            }
        }
        _ => 0,
    }
}

fn admit_target_candidate(
    mut candidate: JsonValue,
    target: &str,
    match_score: usize,
    selected: &mut Vec<JsonValue>,
    selected_urls: &mut HashSet<String>,
    maximum: usize,
) -> Option<String> {
    let Some(url) = candidate["url"].as_str().map(str::to_string) else {
        return None;
    };
    let target_edge = serde_json::json!({
        "query_index": candidate["query_index"],
        "search_query": candidate["search_query"],
        "source_family_id": candidate["source_family_id"],
        "artifact_kind": candidate["artifact_kind"],
        "dimension_ids": candidate["dimension_ids"],
        "matched_source_target": target,
        "target_match_score": match_score,
    });
    if let Some(existing) = selected
        .iter_mut()
        .find(|selected| selected["url"].as_str() == Some(url.as_str()))
    {
        merge_candidate_target_edge(existing, &target_edge);
        return Some(url);
    }
    if selected.len() >= maximum {
        return None;
    }
    if let Some(object) = candidate.as_object_mut() {
        object.insert(
            "matched_source_target".to_string(),
            JsonValue::String(target.to_string()),
        );
        object.insert("target_match_score".to_string(), match_score.into());
        object.insert(
            "target_edges".to_string(),
            JsonValue::Array(vec![target_edge.clone()]),
        );
        object.insert(
            "query_indices".to_string(),
            JsonValue::Array(vec![target_edge["query_index"].clone()]),
        );
        object.insert(
            "search_queries".to_string(),
            JsonValue::Array(vec![target_edge["search_query"].clone()]),
        );
        object.insert(
            "source_family_ids".to_string(),
            JsonValue::Array(vec![target_edge["source_family_id"].clone()]),
        );
        object.insert(
            "artifact_kinds".to_string(),
            JsonValue::Array(vec![target_edge["artifact_kind"].clone()]),
        );
        object.insert(
            "matched_source_targets".to_string(),
            JsonValue::Array(vec![JsonValue::String(target.to_string())]),
        );
    }
    for dimension_id in target_edge["dimension_ids"]
        .as_array()
        .into_iter()
        .flatten()
    {
        push_unique_json_array(&mut candidate, "all_dimension_ids", dimension_id.clone());
    }
    selected_urls.insert(url.clone());
    selected.push(candidate);
    Some(url)
}

fn merge_candidate_target_edge(candidate: &mut JsonValue, edge: &JsonValue) {
    push_unique_json_array(candidate, "target_edges", edge.clone());
    push_unique_json_array(candidate, "query_indices", edge["query_index"].clone());
    push_unique_json_array(candidate, "search_queries", edge["search_query"].clone());
    push_unique_json_array(
        candidate,
        "source_family_ids",
        edge["source_family_id"].clone(),
    );
    push_unique_json_array(
        candidate,
        "artifact_kinds",
        edge["artifact_kind"].clone(),
    );
    push_unique_json_array(
        candidate,
        "matched_source_targets",
        edge["matched_source_target"].clone(),
    );
    for dimension_id in edge["dimension_ids"].as_array().into_iter().flatten() {
        push_unique_json_array(candidate, "all_dimension_ids", dimension_id.clone());
    }
}

fn push_unique_json_array(candidate: &mut JsonValue, field: &str, value: JsonValue) {
    let Some(object) = candidate.as_object_mut() else {
        return;
    };
    let values = object
        .entry(field.to_string())
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    let Some(values) = values.as_array_mut() else {
        return;
    };
    if !values.contains(&value) {
        values.push(value);
    }
}

fn transport_source_family(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    let remainder = lower
        .split_once("://")
        .map(|(_, remainder)| remainder)
        .unwrap_or(lower.as_str());
    let mut segments = remainder.split('/').filter(|segment| !segment.is_empty());
    let host = segments
        .next()
        .unwrap_or_default()
        .trim_start_matches("www.");
    let path = segments.collect::<Vec<_>>();
    match host {
        "github.com" if path.len() >= 2 => format!("github.com/{}/{}", path[0], path[1]),
        "docs.rs" if path.first() == Some(&"crate") && path.len() >= 2 => {
            format!("docs.rs/{}", path[1])
        }
        "docs.rs" if !path.is_empty() => format!("docs.rs/{}", path[0]),
        "crates.io" if path.first() == Some(&"crates") && path.len() >= 2 => {
            format!("crates.io/{}", path[1])
        }
        _ => host.to_string(),
    }
}

fn candidate_target_identity_match(candidate: &JsonValue, target: &str) -> Option<usize> {
    let url = candidate["url"].as_str()?.to_ascii_lowercase();
    let remainder = url
        .split_once("://")
        .map(|(_, remainder)| remainder)
        .unwrap_or(url.as_str());
    let (host, path) = remainder
        .split_once('/')
        .map_or((remainder, ""), |(host, path)| (host, path));
    let host = host.trim_start_matches("www.");
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let target = target.to_ascii_lowercase();
    let (authority, target_path) = target.split_once('/').unwrap_or((&target, ""));

    if authority.contains('.') {
        let host_matches = host == authority || host.ends_with(&format!(".{authority}"));
        let path_matches = target_path.is_empty()
            || normalized_identity(path).contains(&normalized_identity(target_path));
        return (host_matches && path_matches).then_some(9);
    }

    if target_path.is_empty() {
        return None;
    }
    let canonical_repository_path = format!("{authority}/{target_path}");
    if host == "github.com"
        && (path == canonical_repository_path
            || path.starts_with(&format!("{canonical_repository_path}/")))
    {
        return Some(9);
    }

    let path_segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let registry_project = match host {
        "docs.rs" => match path_segments.as_slice() {
            ["crate", project, ..] => Some(*project),
            [project, ..] => Some(*project),
            _ => None,
        },
        "crates.io" => match path_segments.as_slice() {
            ["crates", project, ..] => Some(*project),
            _ => None,
        },
        _ => None,
    };
    if registry_project
        .is_some_and(|project| normalized_identity(project) == normalized_identity(target_path))
    {
        return Some(8);
    }

    let owner = authority.strip_suffix("-rs").unwrap_or(authority);
    let host_identity = normalized_identity(host);
    let path_identity = normalized_identity(path);
    let owner_identity = normalized_identity(owner);
    let project_identity = normalized_identity(target_path);
    let host_looks_owned =
        host_identity.contains(&owner_identity) || host_identity.contains(&project_identity);
    let project_is_named =
        host_identity.contains(&project_identity) || path_identity.contains(&project_identity);
    (host_looks_owned && project_is_named).then_some(7)
}

fn normalized_identity(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn is_stable_source_target_id(target: &str) -> bool {
    if !(target.contains('/') || target.contains('.')) || target.contains("://") {
        return false;
    }
    let segments = target.split('/').collect::<Vec<_>>();
    if segments.is_empty() || segments.len() > 2 {
        return false;
    }
    segments.iter().all(|segment| {
        !segment.is_empty()
            && segment
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
            && segment
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    })
}

fn target_admission_coverage(plan: &JsonValue, selected: &[JsonValue]) -> JsonValue {
    let groups = plan["searches"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(query_index, search)| {
            let targets = search["targets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_str)
                .map(|target| {
                    let urls = selected
                        .iter()
                        .filter(|candidate| {
                            candidate["target_edges"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|edge| {
                                    edge["query_index"].as_u64() == Some(query_index as u64)
                                        && edge["matched_source_target"].as_str() == Some(target)
                                })
                        })
                        .filter_map(|candidate| candidate["url"].as_str())
                        .collect::<Vec<_>>();
                    serde_json::json!({
                        "target": target,
                        "status": if urls.is_empty() { "missing" } else { "selected" },
                        "selected_urls": urls,
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "query_index": query_index,
                "source_family_id": search["source_family_id"],
                "targets": targets,
            })
        })
        .collect::<Vec<_>>();
    let declared_target_count = groups
        .iter()
        .filter_map(|group| group["targets"].as_array())
        .map(Vec::len)
        .sum::<usize>();
    let selected_target_count = groups
        .iter()
        .filter_map(|group| group["targets"].as_array())
        .flatten()
        .filter(|target| target["status"].as_str() == Some("selected"))
        .count();
    serde_json::json!({
        "declared_target_count": declared_target_count,
        "selected_target_count": selected_target_count,
        "missing_target_count": declared_target_count.saturating_sub(selected_target_count),
        "groups": groups,
    })
}

fn flattened_candidates(search_groups: &[JsonValue]) -> Vec<JsonValue> {
    let mut candidates = Vec::new();
    for (group_index, group) in search_groups.iter().enumerate() {
        let mut group_urls = HashSet::new();
        let Some(results) = group["results"].as_array() else {
            continue;
        };
        for result in results {
            let Some(url) = result["url"]
                .as_str()
                .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
            else {
                continue;
            };
            if !group_urls.insert(url.to_string()) {
                continue;
            }
            let mut candidate = result.clone();
            if let Some(object) = candidate.as_object_mut() {
                object.insert("query_index".to_string(), group_index.into());
                object.insert("search_query".to_string(), group["query"].clone());
                if !group["source_family_id"].is_null() {
                    object.insert(
                        "source_family_id".to_string(),
                        group["source_family_id"].clone(),
                    );
                }
                if !group["dimension_ids"].is_null() {
                    object.insert("dimension_ids".to_string(), group["dimension_ids"].clone());
                }
                if !group["targets"].is_null() {
                    object.insert("source_targets".to_string(), group["targets"].clone());
                }
                if !group["artifact_kind"].is_null() {
                    object.insert("artifact_kind".to_string(), group["artifact_kind"].clone());
                }
            }
            candidates.push(candidate);
        }
    }
    candidates
}

fn distinctive_target_terms(target: &str) -> Vec<String> {
    const GENERIC: [&str; 18] = [
        "official",
        "project",
        "canonical",
        "documentation",
        "docs",
        "repository",
        "github",
        "source",
        "runtime",
        "rust",
        "crate",
        "registry",
        "maintainers",
        "release",
        "releases",
        "statement",
        "policy",
        "support",
    ];
    target
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
        .map(str::trim)
        .filter(|term| term.len() >= 3)
        .map(str::to_ascii_lowercase)
        .filter(|term| !GENERIC.contains(&term.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn source_authority_score(url: &str, target_terms: &[String]) -> usize {
    let lower = url.to_ascii_lowercase();
    let host = lower
        .split_once("://")
        .map(|(_, remainder)| remainder)
        .unwrap_or(lower.as_str())
        .split('/')
        .next()
        .unwrap_or_default()
        .trim_start_matches("www.");
    if host == "rustsec.org"
        || host.ends_with(".gov")
        || host.ends_with(".gov.cn")
        || host.ends_with(".europa.eu")
    {
        return 7;
    }
    if host == "rust-lang.github.io" {
        return 7;
    }
    if host.ends_with(".github.io") {
        return 5;
    }
    if host == "docs.rs" || host == "crates.io" {
        return 6;
    }
    if host == "github.com" {
        let path_segments = lower
            .split_once("github.com/")
            .map(|(_, path)| path.split('/').filter(|item| !item.is_empty()).count())
            .unwrap_or_default();
        return if lower.contains("/releases") || lower.contains("/blob/") {
            6
        } else if path_segments >= 2 {
            5
        } else {
            3
        };
    }
    if target_terms.iter().any(|term| {
        host.replace(['.', '-'], "")
            .contains(&term.replace('-', ""))
    }) {
        return 6;
    }
    if host.starts_with("docs.") || lower.contains("/docs/") {
        return 4;
    }
    1
}

fn bounded_plan_fixture() -> JsonValue {
    serde_json::json!({
        "source_families": [
            { "id": "runtime_maintenance", "description": "Official runtime maintenance records" },
            { "id": "http_runtime_support", "description": "Official HTTP library runtime documentation" },
            { "id": "database_runtime_support", "description": "Official database library runtime documentation" },
            { "id": "runtime_choice_guidance", "description": "Official runtime choice and migration guidance" }
        ],
        "dimensions": [
            { "id": "d1", "question": "What is the current maintenance status of both runtimes?", "source_family_id": "runtime_maintenance", "source_requirement": "Official maintenance records for both sides" },
            { "id": "d2", "question": "What official evidence establishes Tokio HTTP integration?", "source_family_id": "http_runtime_support", "source_requirement": "Official Tokio HTTP library documentation" },
            { "id": "d3", "question": "What official evidence establishes async-std HTTP integration?", "source_family_id": "http_runtime_support", "source_requirement": "Official async-std HTTP library documentation" },
            { "id": "d4", "question": "What do official database libraries say about both runtimes?", "source_family_id": "database_runtime_support", "source_requirement": "Official database library runtime documentation" },
            { "id": "d5", "question": "What bounded choice follows for a new project?", "source_family_id": "runtime_choice_guidance", "source_requirement": "Official choice guidance and admitted ecosystem facts" },
            { "id": "d6", "question": "What bounded migration guidance exists for a legacy project?", "source_family_id": "runtime_choice_guidance", "source_requirement": "Official migration or coexistence guidance" }
        ],
        "searches": [
            {
                "query": "tokio-rs/tokio rustsec.org maintenance releases advisory",
                "source_family_id": "runtime_maintenance",
                "artifact_kind": "maintenance_record",
                "dimension_ids": ["d1"],
                "targets": ["tokio-rs/tokio", "rustsec.org"],
                "source_goal": "Acquire official maintenance records for both runtimes"
            },
            {
                "query": "tokio-rs/axum http-rs/tide runtime support documentation",
                "source_family_id": "http_runtime_support",
                "artifact_kind": "documentation",
                "dimension_ids": ["d2", "d3"],
                "targets": ["tokio-rs/axum", "http-rs/tide"],
                "source_goal": "Acquire official HTTP runtime support documentation"
            },
            {
                "query": "launchbadge/sqlx SeaQL/sea-orm async runtime support",
                "source_family_id": "database_runtime_support",
                "artifact_kind": "documentation",
                "dimension_ids": ["d4"],
                "targets": ["launchbadge/sqlx", "SeaQL/sea-orm"],
                "source_goal": "Acquire official database runtime support documentation"
            },
            {
                "query": "async-rs/async-std smol-rs/smol migration guidance",
                "source_family_id": "runtime_choice_guidance",
                "artifact_kind": "guidance",
                "dimension_ids": ["d5", "d6"],
                "targets": ["async-rs/async-std", "smol-rs/smol"],
                "source_goal": "Acquire official runtime choice and migration guidance"
            }
        ]
    })
}

fn search_group_fixture(plan: &JsonValue, query_index: usize, results: JsonValue) -> JsonValue {
    let search = &plan["searches"][query_index];
    serde_json::json!({
        "query": search["query"],
        "source_family_id": search["source_family_id"],
        "artifact_kind": search["artifact_kind"],
        "dimension_ids": search["dimension_ids"],
        "targets": search["targets"],
        "results": results,
    })
}

#[test]
fn bounded_plan_requires_six_dimensions_and_budget_feasible_canonical_targets() {
    let plan = bounded_plan_fixture();
    validate_bounded_query_plan(&plan).expect("valid bounded C01 plan");

    let mut too_many_targets = plan.clone();
    too_many_targets["searches"][0]["targets"] =
        serde_json::json!(["tokio-rs/tokio", "rustsec.org", "async-rs/async-std"]);
    assert!(validate_bounded_query_plan(&too_many_targets)
        .expect_err("three targets cannot fit two Host fetch slots")
        .contains("one to 2 source targets"));

    let mut descriptive_target = plan.clone();
    descriptive_target["searches"][0]["targets"][0] =
        JsonValue::String("Tokio documentation".to_string());
    assert!(validate_bounded_query_plan(&descriptive_target)
        .expect_err("descriptive target must not pass canonical identity validation")
        .contains("not a canonical identifier"));

    let mut runtime_as_http_target = plan.clone();
    runtime_as_http_target["searches"][1]["targets"][1] =
        JsonValue::String("async-rs/async-std".to_string());
    assert!(validate_bounded_query_plan(&runtime_as_http_target)
        .expect_err("runtime repository must not stand in for an HTTP library target")
        .contains("not an HTTP library or adapter"));

    let mut expanded_dimensions = plan;
    expanded_dimensions["dimensions"]
        .as_array_mut()
        .expect("fixture dimensions")
        .push(serde_json::json!({
            "id": "d7",
            "question": "Unrequested middleware expansion",
            "source_family_id": "http_runtime_support",
            "source_requirement": "Unrequested source requirement"
        }));
    assert!(validate_bounded_query_plan(&expanded_dimensions)
        .expect_err("unrequested seventh dimension must be rejected")
        .contains("exactly 6 dimensions"));
}

#[test]
fn target_balanced_admission_resists_high_ranked_cross_target_noise() {
    let plan = bounded_plan_fixture();
    let groups = vec![
        search_group_fixture(
            &plan,
            0,
            serde_json::json!([
                { "title": "async_std wrapper", "url": "https://docs.rs/tokio-async-std/latest/async_std/", "score": 9.0 },
                { "title": "Tokio", "url": "https://github.com/tokio-rs/tokio", "score": 0.2 },
                { "title": "RUSTSEC-2025-0052", "url": "https://rustsec.org/advisories/RUSTSEC-2025-0052", "score": 0.1 }
            ]),
        ),
        search_group_fixture(
            &plan,
            1,
            serde_json::json!([
                { "title": "SQLx runtime issue", "url": "https://github.com/transact-rs/sqlx/issues/1669", "score": 9.0 },
                { "title": "Axum analysis", "url": "https://stackwise.info/tech/axum", "score": 8.0 },
                { "title": "Axum", "url": "https://github.com/tokio-rs/axum", "score": 0.2 },
                { "title": "Tide", "url": "https://github.com/http-rs/tide", "score": 0.1 }
            ]),
        ),
        search_group_fixture(
            &plan,
            2,
            serde_json::json!([
                { "title": "SQLx", "url": "https://github.com/launchbadge/sqlx", "score": 1.0 },
                { "title": "sqlx - Rust", "url": "https://docs.rs/sqlx/latest/sqlx", "score": 0.9 },
                { "title": "Database and Async Runtime", "url": "https://www.sea-ql.org/SeaORM/docs/install-and-config/database-and-async-runtime/", "score": 0.1 }
            ]),
        ),
        search_group_fixture(
            &plan,
            3,
            serde_json::json!([
                { "title": "Third-party runtime guide", "url": "https://microsoft.github.io/RustTraining/async-book/ch07-executors-and-runtimes.html", "score": 9.0 },
                { "title": "async-std CHANGELOG", "url": "https://github.com/async-rs/async-std/blob/main/CHANGELOG.md", "score": 0.2 },
                { "title": "smol", "url": "https://github.com/smol-rs/smol", "score": 0.1 }
            ]),
        ),
    ];

    let selected = target_balanced_candidates(&groups, &plan, C01_FETCH_BUDGET);
    assert_eq!(selected.len(), C01_FETCH_BUDGET);
    let matched_targets = selected
        .iter()
        .filter_map(|candidate| candidate["matched_source_target"].as_str())
        .collect::<BTreeSet<_>>();
    let expected_targets = plan["searches"]
        .as_array()
        .expect("fixture searches")
        .iter()
        .flat_map(|search| search["targets"].as_array().expect("fixture targets"))
        .filter_map(JsonValue::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(matched_targets, expected_targets);
    let urls = selected
        .iter()
        .filter_map(|candidate| candidate["url"].as_str())
        .collect::<Vec<_>>();
    for rejected in [
        "tokio-async-std",
        "transact-rs/sqlx",
        "stackwise.info",
        "microsoft.github.io",
        "docs.rs/sqlx",
    ] {
        assert!(
            urls.iter().all(|url| !url.contains(rejected)),
            "target-balanced admission retained unrelated or duplicate candidate `{rejected}`: {urls:?}"
        );
    }
    let coverage = target_admission_coverage(&plan, &selected);
    assert_eq!(coverage["declared_target_count"], 8);
    assert_eq!(coverage["selected_target_count"], 8);
    assert_eq!(coverage["missing_target_count"], 0);
}

#[test]
fn target_balanced_admission_leaves_unmatched_slots_unused() {
    let plan = bounded_plan_fixture();
    let groups = vec![
        search_group_fixture(
            &plan,
            0,
            serde_json::json!([
                { "title": "Tokio", "url": "https://github.com/tokio-rs/tokio", "score": 0.2 },
                { "title": "Unrelated authoritative crate", "url": "https://docs.rs/unrelated/latest/unrelated", "score": 9.0 }
            ]),
        ),
        search_group_fixture(&plan, 1, serde_json::json!([])),
        search_group_fixture(&plan, 2, serde_json::json!([])),
        search_group_fixture(&plan, 3, serde_json::json!([])),
    ];

    let selected = target_balanced_candidates(&groups, &plan, C01_FETCH_BUDGET);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0]["matched_source_target"], "tokio-rs/tokio");
    let coverage = target_admission_coverage(&plan, &selected);
    assert_eq!(coverage["declared_target_count"], 8);
    assert_eq!(coverage["selected_target_count"], 1);
    assert_eq!(coverage["missing_target_count"], 7);
}

#[test]
fn target_balanced_admission_merges_shared_url_provenance_before_fetch() {
    let mut plan = bounded_plan_fixture();
    plan["searches"][3]["targets"][0] = JsonValue::String("tokio-rs/tokio".to_string());
    validate_bounded_query_plan(&plan)
        .expect("one canonical target may serve distinct semantic query edges");
    let shared = serde_json::json!([
        { "title": "Tokio", "url": "https://github.com/tokio-rs/tokio", "score": 1.0 }
    ]);
    let groups = vec![
        search_group_fixture(&plan, 0, shared.clone()),
        search_group_fixture(&plan, 1, serde_json::json!([])),
        search_group_fixture(&plan, 2, serde_json::json!([])),
        search_group_fixture(&plan, 3, shared),
    ];

    let selected = target_balanced_candidates(&groups, &plan, C01_FETCH_BUDGET);
    assert_eq!(selected.len(), 1, "one URL must consume one fetch slot");
    assert_eq!(
        selected[0]["target_edges"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(selected[0]["query_indices"], serde_json::json!([0, 3]));
    assert_eq!(
        selected[0]["source_family_ids"],
        serde_json::json!(["runtime_maintenance", "runtime_choice_guidance"])
    );
    let coverage = target_admission_coverage(&plan, &selected);
    assert_eq!(coverage["declared_target_count"], 8);
    assert_eq!(coverage["selected_target_count"], 2);
    assert_eq!(coverage["missing_target_count"], 6);
}
