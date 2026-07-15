//! DeepResearch evidence normalization and bounded prompt digests.

use std::collections::HashSet;

use super::report::deep_research_output_has_internal_leak;

const DEEP_RESEARCH_PROMPT_SUCCESS_OUTPUT_LIMIT: usize = 1200;
const DEEP_RESEARCH_PROMPT_TEXT_LIMIT: usize = 12_000;
const DEEP_RESEARCH_MAX_DIGEST_EVIDENCE: usize = 18;
const DEEP_RESEARCH_MAX_DIGEST_SOURCES: usize = 12;
const DEEP_RESEARCH_MAX_DIGEST_STRINGS: usize = 12;

pub(crate) fn deep_research_prompt_workflow_output(workflow_output: &str) -> String {
    let value = match serde_json::from_str::<serde_json::Value>(workflow_output) {
        Ok(value) => value,
        Err(_) => {
            if deep_research_output_has_internal_leak(workflow_output) {
                return "Research evidence was non-JSON and contained internal tool logs; raw text withheld from synthesis.".to_string();
            }
            return deep_research_truncate_chars(
                &workflow_output
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                DEEP_RESEARCH_PROMPT_TEXT_LIMIT,
            );
        }
    };
    let digest = deep_research_workflow_output_digest(&value);
    serde_json::to_string_pretty(&digest).unwrap_or_else(|_| {
        deep_research_truncate_chars(workflow_output, DEEP_RESEARCH_PROMPT_TEXT_LIMIT)
    })
}

pub(crate) fn deep_research_prompt_metadata(
    workflow_metadata: Option<&serde_json::Value>,
) -> String {
    workflow_metadata
        .map(deep_research_workflow_metadata_digest)
        .and_then(|metadata| serde_json::to_string_pretty(&metadata).ok())
        .unwrap_or_else(|| "{}".to_string())
}

pub(crate) fn deep_research_has_source_evidence(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> bool {
    let output_has_evidence = serde_json::from_str::<serde_json::Value>(workflow_output)
        .ok()
        .is_some_and(|value| {
            deep_research_collect_structured_evidence(&value)
                .into_iter()
                .any(|item| {
                    item.get("sources")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|sources| !sources.is_empty())
                })
        });
    output_has_evidence
        || workflow_metadata.is_some_and(|metadata| {
            deep_research_collect_structured_evidence(metadata)
                .into_iter()
                .any(|item| {
                    item.get("sources")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|sources| !sources.is_empty())
                })
        })
}

pub(crate) fn deep_research_sanitize_workflow_metadata(
    metadata: &serde_json::Value,
) -> serde_json::Value {
    let mut sanitized = metadata.clone();
    deep_research_sanitize_parallel_task_values(&mut sanitized);
    sanitized
}

pub(crate) fn deep_research_workflow_output_digest(value: &serde_json::Value) -> serde_json::Value {
    let mut digest = serde_json::Map::new();
    copy_json_field(&mut digest, value, "query");
    digest.insert(
        "collection_status".to_string(),
        serde_json::Value::String(deep_research_collection_status(value).to_string()),
    );
    if let Some(runtime_error) = value.get("runtime_error") {
        digest.insert(
            "collection_error".to_string(),
            serde_json::Value::String(deep_research_error_or_digest_text(runtime_error, 1000)),
        );
    }

    if let Some(research) = value.get("research") {
        if let Some(research) = research.as_object() {
            let mut compact = serde_json::Map::new();
            for key in [
                "algorithm",
                "status",
                "max_rounds",
                "completed_rounds",
                "stop_reason",
            ] {
                copy_json_field(
                    &mut compact,
                    &serde_json::Value::Object(research.clone()),
                    key,
                );
            }
            if let Some(complexity) = research.get("complexity") {
                compact.insert("complexity".to_string(), complexity.clone());
            }
            if let Some(metadata) = research.get("metadata") {
                compact.insert(
                    "counts".to_string(),
                    deep_research_compact_count_metadata(metadata),
                );
            }
            compact.insert(
                "rounds".to_string(),
                deep_research_compact_rounds(research.get("rounds")),
            );
            compact.insert(
                "evidence_items".to_string(),
                serde_json::Value::Array(deep_research_collect_structured_evidence(
                    research.get("runtime_output").unwrap_or(
                        research
                            .get("results")
                            .unwrap_or(research.get("rounds").unwrap_or(&serde_json::Value::Null)),
                    ),
                )),
            );
            if let Some(warnings) = research.get("warnings") {
                compact.insert(
                    "warnings".to_string(),
                    deep_research_compact_warnings(warnings),
                );
            }
            digest.insert("research".to_string(), serde_json::Value::Object(compact));
        } else {
            digest.insert(
                "research_summary".to_string(),
                serde_json::Value::String(deep_research_compact_json_text(
                    research,
                    DEEP_RESEARCH_PROMPT_SUCCESS_OUTPUT_LIMIT,
                )),
            );
        }
    }

    serde_json::Value::Object(digest)
}

pub(crate) fn deep_research_collection_status(value: &serde_json::Value) -> &'static str {
    let mode = value
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if mode.contains("failed") || value.get("error").is_some() {
        "failed"
    } else if value.get("runtime_error").is_some() || mode.contains("fallback") {
        "degraded"
    } else {
        "completed"
    }
}

pub(crate) fn deep_research_workflow_metadata_digest(
    metadata: &serde_json::Value,
) -> serde_json::Value {
    let sanitized = deep_research_sanitize_workflow_metadata(metadata);
    let Some(workflow) = sanitized.get("dynamic_workflow") else {
        let evidence_items = deep_research_collect_structured_evidence(&sanitized);
        return if evidence_items.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::json!({ "research_run": { "evidence_items": evidence_items } })
        };
    };
    let mut dynamic = serde_json::Map::new();
    copy_json_field(&mut dynamic, workflow, "status");
    copy_json_field(&mut dynamic, workflow, "last_sequence");

    if let Some(steps) = workflow
        .pointer("/snapshot/steps")
        .and_then(serde_json::Value::as_object)
    {
        let mut compact_steps = Vec::new();
        for (index, step) in steps.values().enumerate() {
            let mut compact = serde_json::Map::new();
            compact.insert(
                "step".to_string(),
                serde_json::Value::Number(serde_json::Number::from(index + 1)),
            );
            copy_json_field(&mut compact, step, "status");
            copy_json_field(&mut compact, step, "attempt");
            if let Some(output) = step.get("output") {
                if let Some(metadata) = output.get("metadata") {
                    compact.insert(
                        "counts".to_string(),
                        deep_research_compact_count_metadata(metadata),
                    );
                }
                if let Some(warnings) = output.get("warnings") {
                    compact.insert(
                        "warnings".to_string(),
                        deep_research_compact_warnings(warnings),
                    );
                }
            }
            compact_steps.push(serde_json::Value::Object(compact));
        }
        dynamic.insert("steps".to_string(), serde_json::Value::Array(compact_steps));
    }
    dynamic.insert(
        "evidence_items".to_string(),
        serde_json::Value::Array(deep_research_collect_structured_evidence(&sanitized)),
    );

    serde_json::json!({ "research_run": dynamic })
}

pub(crate) fn deep_research_sanitize_parallel_task_values(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let is_parallel_task = map
                .get("tool")
                .or_else(|| map.get("name"))
                .or_else(|| map.get("tool_name"))
                .and_then(serde_json::Value::as_str)
                == Some("parallel_task");
            if is_parallel_task {
                deep_research_sanitize_parallel_task_object(map);
            }
            for value in map.values_mut() {
                deep_research_sanitize_parallel_task_values(value);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                deep_research_sanitize_parallel_task_values(item);
            }
        }
        _ => {}
    }
}

pub(crate) fn deep_research_sanitize_parallel_task_object(
    map: &mut serde_json::Map<String, serde_json::Value>,
) {
    let sanitized_results = map
        .get("metadata")
        .and_then(|metadata| metadata.get("results"))
        .and_then(serde_json::Value::as_array)
        .map(|results| {
            let mut successes = Vec::new();
            let mut failed_tasks = Vec::new();
            for result in results {
                let success = result
                    .get("success")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if success {
                    successes.push(deep_research_sanitize_parallel_result(result, true));
                } else {
                    failed_tasks.push(deep_research_sanitize_parallel_result(result, false));
                }
            }
            (successes, failed_tasks)
        });

    if let Some((successes, failed_tasks)) = sanitized_results {
        if let Some(metadata) = map
            .get_mut("metadata")
            .and_then(serde_json::Value::as_object_mut)
        {
            metadata.insert(
                "results".to_string(),
                serde_json::Value::Array(successes.clone()),
            );
        }
        if !failed_tasks.is_empty() {
            let warnings = map
                .entry("warnings".to_string())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if let Some(warnings) = warnings.as_object_mut() {
                warnings.insert(
                    "failed_tasks".to_string(),
                    serde_json::Value::Array(failed_tasks),
                );
            }
        }
        map.remove("output");
    } else if let Some(output) = map.remove("output") {
        map.insert(
            "output_summary".to_string(),
            serde_json::Value::String(deep_research_compact_json_text(
                &output,
                DEEP_RESEARCH_PROMPT_SUCCESS_OUTPUT_LIMIT,
            )),
        );
    }
}

pub(crate) fn deep_research_sanitize_parallel_result(
    result: &serde_json::Value,
    success: bool,
) -> serde_json::Value {
    let mut next = serde_json::Map::new();
    for key in [
        "task_id",
        "session_id",
        "agent",
        "success",
        "artifact_id",
        "artifact_uri",
        "output_bytes",
        "truncated_for_context",
        "structured_error",
    ] {
        if let Some(value) = result.get(key) {
            next.insert(key.to_string(), value.clone());
        }
    }

    if success {
        if let Some(structured) = result.get("structured") {
            next.insert("structured".to_string(), structured.clone());
        } else if let Some(output) = result.get("output") {
            next.insert(
                "output_summary".to_string(),
                serde_json::Value::String(deep_research_compact_json_text(
                    output,
                    DEEP_RESEARCH_PROMPT_SUCCESS_OUTPUT_LIMIT,
                )),
            );
        }
    } else {
        let summary = result
            .get("output")
            .or_else(|| result.get("error"))
            .map(deep_research_failure_summary)
            .unwrap_or_else(|| {
                "Delegated task failed before returning usable evidence.".to_string()
            });
        next.insert(
            "error_summary".to_string(),
            serde_json::Value::String(summary),
        );
    }

    serde_json::Value::Object(next)
}

pub(crate) fn copy_json_field(
    target: &mut serde_json::Map<String, serde_json::Value>,
    source: &serde_json::Value,
    key: &str,
) {
    if let Some(value) = source.get(key) {
        target.insert(key.to_string(), value.clone());
    }
}

pub(crate) fn deep_research_compact_count_metadata(
    metadata: &serde_json::Value,
) -> serde_json::Value {
    let mut counts = serde_json::Map::new();
    for key in [
        "task_count",
        "result_count",
        "success_count",
        "failed_count",
        "all_success",
        "partial_failure",
        "allow_partial_failure",
    ] {
        copy_json_field(&mut counts, metadata, key);
    }
    serde_json::Value::Object(counts)
}

pub(crate) fn deep_research_compact_rounds(
    rounds: Option<&serde_json::Value>,
) -> serde_json::Value {
    let items = rounds
        .and_then(serde_json::Value::as_array)
        .map(|rounds| {
            rounds
                .iter()
                .map(|round| {
                    let mut compact = serde_json::Map::new();
                    copy_json_field(&mut compact, round, "round");
                    copy_json_field(&mut compact, round, "status");
                    if let Some(metadata) = round.get("metadata") {
                        compact.insert(
                            "counts".to_string(),
                            deep_research_compact_count_metadata(metadata),
                        );
                    }
                    if let Some(warnings) = round.get("warnings") {
                        compact.insert(
                            "warnings".to_string(),
                            deep_research_compact_warnings(warnings),
                        );
                    }
                    serde_json::Value::Object(compact)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::Value::Array(items)
}

pub(crate) fn deep_research_compact_warnings(warnings: &serde_json::Value) -> serde_json::Value {
    let mut compact = serde_json::Map::new();
    if let Some(failed_tasks) = warnings
        .get("failed_tasks")
        .and_then(serde_json::Value::as_array)
    {
        compact.insert(
            "failed_tasks".to_string(),
            serde_json::Value::Array(
                failed_tasks
                    .iter()
                    .take(8)
                    .map(|item| {
                        let mut task = serde_json::Map::new();
                        copy_json_field(&mut task, item, "round");
                        copy_json_field(&mut task, item, "agent");
                        copy_json_field(&mut task, item, "task_id");
                        if let Some(summary) = item
                            .get("error_summary")
                            .or_else(|| item.get("error"))
                            .and_then(serde_json::Value::as_str)
                        {
                            task.insert(
                                "error_summary".to_string(),
                                serde_json::Value::String(deep_research_failure_summary(
                                    &serde_json::Value::String(summary.to_string()),
                                )),
                            );
                        }
                        serde_json::Value::Object(task)
                    })
                    .collect(),
            ),
        );
    }
    if let Some(failed_rounds) = warnings
        .get("failed_rounds")
        .and_then(serde_json::Value::as_array)
    {
        compact.insert(
            "failed_rounds".to_string(),
            serde_json::Value::Array(
                failed_rounds
                    .iter()
                    .take(4)
                    .map(|item| {
                        let mut round = serde_json::Map::new();
                        copy_json_field(&mut round, item, "round");
                        if let Some(error) = item.get("error").and_then(serde_json::Value::as_str) {
                            round.insert(
                                "error".to_string(),
                                serde_json::Value::String(deep_research_failure_summary(
                                    &serde_json::Value::String(error.to_string()),
                                )),
                            );
                        }
                        serde_json::Value::Object(round)
                    })
                    .collect(),
            ),
        );
    }
    serde_json::Value::Object(compact)
}

pub(crate) fn deep_research_collect_structured_evidence(
    root: &serde_json::Value,
) -> Vec<serde_json::Value> {
    fn walk(
        value: &serde_json::Value,
        round_hint: Option<u64>,
        out: &mut Vec<serde_json::Value>,
        seen: &mut HashSet<String>,
    ) {
        if out.len() >= DEEP_RESEARCH_MAX_DIGEST_EVIDENCE {
            return;
        }
        match value {
            serde_json::Value::Object(map) => {
                let round = map
                    .get("round")
                    .and_then(serde_json::Value::as_u64)
                    .or(round_hint);
                if let Some(structured) = map.get("structured") {
                    if let Some(compact) =
                        deep_research_compact_evidence_object(structured, round, seen)
                    {
                        out.push(compact);
                    }
                } else if is_deep_research_evidence_object(value) {
                    if let Some(compact) = deep_research_compact_evidence_object(value, round, seen)
                    {
                        out.push(compact);
                    }
                }
                for (key, child) in map {
                    if matches!(
                        key.as_str(),
                        "output_summary" | "error_summary" | "input" | "history"
                    ) || (key == "output" && !child.is_object() && !child.is_array())
                    {
                        continue;
                    }
                    walk(child, round, out, seen);
                    if out.len() >= DEEP_RESEARCH_MAX_DIGEST_EVIDENCE {
                        break;
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, round_hint, out, seen);
                    if out.len() >= DEEP_RESEARCH_MAX_DIGEST_EVIDENCE {
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    walk(root, None, &mut out, &mut seen);
    out
}

pub(crate) fn is_deep_research_evidence_object(value: &serde_json::Value) -> bool {
    value
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .is_some()
        && value
            .get("sources")
            .and_then(serde_json::Value::as_array)
            .is_some()
        && value
            .get("confidence")
            .and_then(serde_json::Value::as_str)
            .is_some()
}

pub(crate) fn deep_research_compact_evidence_object(
    evidence: &serde_json::Value,
    round: Option<u64>,
    seen: &mut HashSet<String>,
) -> Option<serde_json::Value> {
    let summary = evidence.get("summary")?.as_str()?.trim();
    if summary.is_empty() {
        return None;
    }
    let dedupe_key = format!(
        "{}|{}",
        round.unwrap_or_default(),
        summary.to_ascii_lowercase()
    );
    if !seen.insert(dedupe_key) {
        return None;
    }

    let mut compact = serde_json::Map::new();
    if let Some(round) = round {
        compact.insert(
            "round".to_string(),
            serde_json::Value::Number(serde_json::Number::from(round)),
        );
    }
    compact.insert(
        "summary".to_string(),
        serde_json::Value::String(deep_research_digest_text(summary, 700)),
    );
    compact.insert(
        "sources".to_string(),
        serde_json::Value::Array(
            evidence
                .get("sources")
                .and_then(serde_json::Value::as_array)
                .map(|sources| {
                    sources
                        .iter()
                        .take(DEEP_RESEARCH_MAX_DIGEST_SOURCES)
                        .map(deep_research_compact_source)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        ),
    );
    for key in ["key_evidence", "contradictions", "gaps"] {
        compact.insert(
            key.to_string(),
            serde_json::Value::Array(deep_research_compact_string_array(
                evidence.get(key),
                DEEP_RESEARCH_MAX_DIGEST_STRINGS,
                350,
            )),
        );
    }
    if let Some(confidence) = evidence
        .get("confidence")
        .and_then(serde_json::Value::as_str)
    {
        compact.insert(
            "confidence".to_string(),
            serde_json::Value::String(deep_research_digest_text(confidence, 350)),
        );
    }
    Some(serde_json::Value::Object(compact))
}

pub(crate) fn deep_research_compact_source(source: &serde_json::Value) -> serde_json::Value {
    let mut compact = serde_json::Map::new();
    for (key, limit) in [
        ("title", 220usize),
        ("url_or_path", 500),
        ("date", 120),
        ("quote_or_fact", 450),
        ("reliability", 220),
    ] {
        if let Some(value) = source.get(key).and_then(serde_json::Value::as_str) {
            compact.insert(
                key.to_string(),
                serde_json::Value::String(deep_research_digest_text(value, limit)),
            );
        }
    }
    serde_json::Value::Object(compact)
}

pub(crate) fn deep_research_compact_string_array(
    value: Option<&serde_json::Value>,
    max_items: usize,
    max_chars: usize,
) -> Vec<serde_json::Value> {
    let mut seen = HashSet::new();
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter_map(|item| {
                    let item = item.trim();
                    if item.is_empty() {
                        return None;
                    }
                    let key = item.to_ascii_lowercase();
                    if !seen.insert(key) {
                        return None;
                    }
                    Some(serde_json::Value::String(deep_research_digest_text(
                        item, max_chars,
                    )))
                })
                .take(max_items)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(crate) fn deep_research_compact_json_text(value: &serde_json::Value, limit: usize) -> String {
    let text = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default());
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    deep_research_digest_text(&compact, limit)
}

pub(crate) fn deep_research_digest_text(text: &str, limit: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return compact;
    }
    if deep_research_output_has_internal_leak(&compact) {
        return "Internal workflow/tool log text withheld from DeepResearch synthesis.".to_string();
    }
    deep_research_truncate_chars(&compact, limit)
}

pub(crate) fn deep_research_error_or_digest_text(
    value: &serde_json::Value,
    limit: usize,
) -> String {
    let text = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default());
    if deep_research_output_has_internal_leak(&text) {
        deep_research_failure_summary(&serde_json::Value::String(text))
    } else {
        deep_research_digest_text(&text, limit)
    }
}

pub(crate) fn deep_research_failure_summary(value: &serde_json::Value) -> String {
    let text = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default());
    let lower = text.to_ascii_lowercase();
    if lower.contains("permission denied: tool") || lower.contains("permission policy denied") {
        return "Delegated task could not use a requested tool because the permission policy denied it.".to_string();
    }
    if lower.contains("max tool rounds") || lower.contains("tool-round budget") {
        return "Delegated task exhausted its tool-round budget before returning usable evidence."
            .to_string();
    }
    if lower.contains("timed out") || lower.contains("[command timed out") {
        return "Delegated task timed out before returning usable evidence.".to_string();
    }
    if lower.contains("[tool output truncated")
        || lower.contains("full output artifact:")
        || lower.contains("a3s://tool-output")
    {
        return "Delegated task produced oversized tool output that was withheld from the report context.".to_string();
    }
    if lower.contains(".a3s-flow/dynamic-workflows")
        || lower.contains("● searched")
        || lower.contains("● ran")
        || lower.contains("● read")
        || text.contains('⎿')
    {
        return "Delegated task returned internal workflow/tool logs that were withheld from the report context.".to_string();
    }
    "Delegated task failed before returning usable evidence.".to_string()
}

pub(crate) fn deep_research_truncate_chars(text: &str, limit: usize) -> String {
    let mut output = String::new();
    let mut truncated = false;
    for (index, ch) in text.chars().enumerate() {
        if index >= limit {
            truncated = true;
            break;
        }
        output.push(ch);
    }
    if truncated {
        output.push_str(" ... [truncated]");
    }
    output
}

pub(crate) fn deep_research_loop_layers(query: &str, os_runtime: bool) -> usize {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return 0;
    }
    let local_only_markers = [
        "local only",
        "local workspace",
        "local evidence",
        "local files",
        "local file",
        "workspace evidence",
        "workspace only",
        "repository only",
        "locally",
        "no os",
        "without os",
        "不要 os",
        "不用 os",
        "不使用 os",
        "本地",
        "不要远程",
        "不用远程",
    ];
    if local_only_markers.iter().any(|marker| q.contains(marker)) {
        return 0;
    }

    let mut score = 0usize;
    let groups: &[&[&str]] = &[
        &["comprehensive", "deep dive", "全面", "深入", "调研", "研究"],
        &["compare", "comparison", "benchmark", "对比", "比较", "竞品"],
        &["latest", "recent", "timeline", "最新", "趋势", "时间线"],
        &[
            "market",
            "regulation",
            "policy",
            "paper",
            "papers",
            "市场",
            "法规",
            "政策",
            "论文",
        ],
        &["multi-source", "多来源", "大量", "并行"],
    ];
    for group in groups {
        if group.iter().any(|marker| q.contains(marker)) {
            score += 1;
        }
    }
    let words = q.split_whitespace().count();
    let chars = q.chars().count();
    if words >= 14 || chars >= 80 {
        score += 1;
    }
    if words >= 28 || chars >= 140 {
        score += 1;
    }
    if os_runtime {
        score += 1;
    }
    let narrow_official_lookup =
        (q.contains("latest") || q.contains("current") || q.contains("最新"))
            && (q.contains("version") || q.contains("release") || q.contains("版本"))
            && (q.contains("official") || q.contains("primary") || q.contains("官方"))
            && ![
                "compare",
                "comparison",
                "versus",
                "benchmark",
                "market",
                "regulation",
                "policy",
                "paper",
                "papers",
                "对比",
                "比较",
                "市场",
                "法规",
                "政策",
                "论文",
            ]
            .iter()
            .any(|marker| q.contains(marker));
    if narrow_official_lookup && score <= 2 {
        return 0;
    }

    match score {
        0 => 0,
        1 | 2 => 1,
        3 | 4 => 2,
        _ => 3,
    }
}
