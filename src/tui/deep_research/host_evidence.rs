//! Validation and URL sanitization for structured DeepResearch evidence.

use super::*;

pub(super) fn deep_research_verified_structured_evidence(
    result: &serde_json::Value,
    structured: &serde_json::Value,
) -> Option<serde_json::Value> {
    if !is_deep_research_evidence_object(structured) {
        return None;
    }
    let observed = result
        .get("source_anchors")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|anchor| {
            let tool = anchor.get("tool").and_then(serde_json::Value::as_str)?;
            matches!(tool, "read" | "grep" | "web_search" | "web_fetch")
                .then(|| {
                    anchor
                        .get("url_or_path")
                        .and_then(serde_json::Value::as_str)
                })
                .flatten()
        })
        .filter_map(deep_research_safe_source_anchor)
        .collect::<std::collections::HashMap<_, _>>();
    if observed.is_empty() {
        return None;
    }

    let reported_count = structured
        .get("sources")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let sources = structured
        .get("sources")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .filter_map(|source| {
            let raw = source
                .get("url_or_path")
                .or_else(|| source.get("url"))
                .or_else(|| source.get("path"))
                .and_then(serde_json::Value::as_str)?;
            let safe = deep_research_reported_source_candidates(raw)
                .into_iter()
                .find_map(|(key, _)| observed.get(&key))?;
            let mut projected = serde_json::Map::new();
            if let Some(title) = first_string_field(source, &["title"]) {
                projected.insert("title".to_string(), title.into());
            }
            projected.insert("url_or_path".to_string(), safe.clone().into());
            for (key, aliases) in [
                ("date", &["date", "publication_date", "published_at"][..]),
                (
                    "quote_or_fact",
                    &["quote_or_fact", "evidence", "quote", "fact"][..],
                ),
                ("reliability", &["reliability", "publisher"][..]),
            ] {
                if let Some(value) = first_string_field(source, aliases) {
                    projected.insert(key.to_string(), value.into());
                }
            }
            Some(serde_json::Value::Object(projected))
        })
        .collect::<Vec<_>>();
    if sources.is_empty() {
        return None;
    }
    let omitted = reported_count.saturating_sub(sources.len());
    let mut map = serde_json::Map::new();
    for key in [
        "summary",
        "key_evidence",
        "contradictions",
        "confidence",
        "gaps",
    ] {
        if let Some(value) = structured.get(key) {
            map.insert(key.to_string(), value.clone());
        }
    }
    map.insert("sources".to_string(), serde_json::Value::Array(sources));
    if omitted > 0 {
        let gaps = map
            .entry("gaps".to_string())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if let Some(gaps) = gaps.as_array_mut() {
            gaps.push(serde_json::Value::String(format!(
                "{omitted} self-reported source(s) omitted because no successful research tool observed them."
            )));
        }
    }
    let mut verified = serde_json::Value::Object(map);
    deep_research_sanitize_evidence_urls(&mut verified);
    Some(verified)
}

pub(super) fn deep_research_safe_source_anchor(value: &str) -> Option<(String, String)> {
    let trimmed = value.trim();
    let safe = if let Ok(mut url) = reqwest::Url::parse(trimmed) {
        if !matches!(url.scheme(), "http" | "https") || url.host_str()?.is_empty() {
            return None;
        }
        url.set_username("").ok()?;
        url.set_password(None).ok()?;
        let safe_query = deep_research_safe_source_query(&url);
        url.set_query(None);
        if !safe_query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(safe_query.iter().map(|(key, value)| (key, value)));
        }
        url.set_fragment(None);
        url.to_string()
    } else {
        let mut path = trimmed.replace('\\', "/");
        if let Some(without_prefix) = path.strip_prefix("./") {
            path = without_prefix.to_string();
        }
        path
    };
    normalize_research_source_anchor(&safe)?;
    // URL parsing canonicalizes scheme and authority while retaining the
    // case-sensitive resource path. Local paths likewise remain case-sensitive.
    let key = safe.clone();
    Some((key, safe))
}

pub(super) fn deep_research_safe_source_query(url: &reqwest::Url) -> Vec<(String, String)> {
    let mut safe_query = url
        .query_pairs()
        .filter_map(|(key, value)| {
            let key = key.to_ascii_lowercase();
            let allowed_key = matches!(
                key.as_str(),
                "lang" | "seq_code" | "id" | "article_id" | "news_id"
            );
            let allowed_value = !value.is_empty()
                && value.len() <= 128
                && value
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
            (allowed_key && allowed_value).then(|| (key, value.into_owned()))
        })
        .collect::<Vec<_>>();
    safe_query.sort();
    safe_query.dedup();
    safe_query
}

pub(super) fn deep_research_reported_source_candidates(value: &str) -> Vec<(String, String)> {
    let Some(exact) = deep_research_safe_source_anchor(value) else {
        return Vec::new();
    };
    let is_http = reqwest::Url::parse(value.trim()).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some_and(|host| !host.is_empty())
    });
    if is_http {
        return vec![exact];
    }

    let mut candidates = vec![exact.clone()];
    let without_fragment = exact.1.split('#').next().unwrap_or(&exact.1);
    for candidate in [
        without_fragment,
        without_fragment
            .rsplit_once(':')
            .filter(|(_, suffix)| {
                !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit())
            })
            .map(|(path, _)| path)
            .unwrap_or(without_fragment),
    ] {
        let Some(candidate) = deep_research_safe_source_anchor(candidate) else {
            continue;
        };
        if !candidates.iter().any(|(key, _)| key == &candidate.0) {
            candidates.push(candidate);
        }
    }
    candidates
}

pub(super) fn deep_research_sanitize_evidence_urls(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            *text = deep_research_sanitize_evidence_text(text);
        }
        serde_json::Value::Array(items) => {
            for item in items {
                deep_research_sanitize_evidence_urls(item);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                deep_research_sanitize_evidence_urls(item);
            }
        }
        _ => {}
    }
}

pub(super) fn deep_research_sanitize_evidence_text(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;

    while cursor < text.len() {
        let next = ["http://", "https://"]
            .into_iter()
            .filter_map(|prefix| lower[cursor..].find(prefix).map(|index| cursor + index))
            .min();
        let Some(start) = next else {
            output.push_str(&text[cursor..]);
            break;
        };
        output.push_str(&text[cursor..start]);

        let token_end = text[start..]
            .char_indices()
            .find_map(|(offset, ch)| {
                (ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | '\'' | '`'))
                    .then_some(start + offset)
            })
            .unwrap_or(text.len());
        let mut candidate_end = token_end;
        while let Some((offset, ch)) = text[start..candidate_end].char_indices().next_back() {
            if matches!(ch, ')' | ']' | '}' | ',' | '.' | ';' | ':' | '!' | '?') {
                candidate_end = start + offset;
            } else {
                break;
            }
        }

        if let Some((_, safe)) = deep_research_safe_source_anchor(&text[start..candidate_end]) {
            output.push_str(&safe);
        }
        output.push_str(&text[candidate_end..token_end]);
        cursor = token_end;
    }

    output
}

pub(super) fn copy_json_field(
    target: &mut serde_json::Map<String, serde_json::Value>,
    source: &serde_json::Value,
    key: &str,
) {
    if let Some(value) = source.get(key) {
        target.insert(key.to_string(), value.clone());
    }
}

pub(super) fn deep_research_compact_count_metadata(
    metadata: &serde_json::Value,
) -> serde_json::Value {
    let mut counts = serde_json::Map::new();
    for key in [
        "task_count",
        "result_count",
        "search_count",
        "source_count",
        "host_count",
        "freshness_required",
        "dated_source_count",
        "query_term_count",
        "matched_query_term_count",
        "query_term_coverage",
        "fetched_query_term_count",
        "fetched_query_term_coverage",
        "query_terms_truncated",
        "fetch_count",
        "fetched_count",
        "fetched_host_count",
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

pub(super) fn deep_research_compact_rounds(
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

pub(super) fn deep_research_compact_warnings(warnings: &serde_json::Value) -> serde_json::Value {
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
