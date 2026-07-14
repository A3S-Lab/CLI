//! Bounded evidence and source digests used by DeepResearch synthesis.

use super::*;

pub(super) fn deep_research_collect_structured_evidence(
    root: &serde_json::Value,
) -> Vec<serde_json::Value> {
    deep_research_collect_structured_evidence_bounded(root).0
}

pub(super) fn deep_research_collect_structured_evidence_bounded(
    root: &serde_json::Value,
) -> (Vec<serde_json::Value>, usize) {
    fn walk(
        value: &serde_json::Value,
        round_hint: Option<u64>,
        out: &mut Vec<serde_json::Value>,
        omitted: &mut usize,
        seen: &mut HashSet<String>,
    ) {
        match value {
            serde_json::Value::Object(map) => {
                let round = map
                    .get("round")
                    .and_then(serde_json::Value::as_u64)
                    .or(round_hint);
                let has_structured_container = map.contains_key("structured");
                if let Some(structured) = map.get("structured") {
                    if let Some(compact) =
                        deep_research_compact_evidence_object(structured, round, seen)
                    {
                        if out.len() < DEEP_RESEARCH_MAX_DIGEST_EVIDENCE {
                            out.push(compact);
                        } else {
                            *omitted = omitted.saturating_add(1);
                        }
                    }
                } else if is_deep_research_evidence_object(value) {
                    if let Some(compact) = deep_research_compact_evidence_object(value, round, seen)
                    {
                        if out.len() < DEEP_RESEARCH_MAX_DIGEST_EVIDENCE {
                            out.push(compact);
                        } else {
                            *omitted = omitted.saturating_add(1);
                        }
                    }
                    // Evidence objects are terminal schema values. Recursing
                    // into free-form or extension fields could promote nested,
                    // unverified evidence-shaped JSON.
                    return;
                }
                for (key, child) in map {
                    if has_structured_container && key == "structured" {
                        continue;
                    }
                    if matches!(
                        key.as_str(),
                        "query"
                            | "input"
                            | "history"
                            | "prompt"
                            | "description"
                            | "error"
                            | "output_summary"
                            | "error_summary"
                    ) {
                        continue;
                    }
                    walk(child, round, out, omitted, seen);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, round_hint, out, omitted, seen);
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    let mut omitted = 0usize;
    let mut seen = HashSet::new();
    walk(root, None, &mut out, &mut omitted, &mut seen);
    (out, omitted)
}

pub(super) fn is_deep_research_evidence_object(value: &serde_json::Value) -> bool {
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

pub(super) fn deep_research_compact_evidence_object(
    evidence: &serde_json::Value,
    round: Option<u64>,
    seen: &mut HashSet<String>,
) -> Option<serde_json::Value> {
    let summary = evidence.get("summary")?.as_str()?.trim();
    if summary.is_empty() {
        return None;
    }
    let first_source = evidence
        .get("sources")
        .and_then(serde_json::Value::as_array)
        .and_then(|sources| {
            sources
                .iter()
                .find_map(deep_research_traceable_source_anchor)
        })
        .unwrap_or_default();
    let dedupe_key = format!(
        "{}|{}|{}",
        round.unwrap_or_default(),
        summary.to_ascii_lowercase(),
        first_source
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
    let source_values = evidence
        .get("sources")
        .and_then(serde_json::Value::as_array);
    let compact_sources = source_values
        .map(|sources| {
            sources
                .iter()
                .filter_map(deep_research_compact_source)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    compact.insert(
        "sources".to_string(),
        serde_json::Value::Array(
            compact_sources
                .iter()
                .take(DEEP_RESEARCH_MAX_DIGEST_SOURCES)
                .cloned()
                .collect(),
        ),
    );
    let omitted = compact_sources
        .len()
        .saturating_sub(DEEP_RESEARCH_MAX_DIGEST_SOURCES);
    if omitted > 0 {
        compact.insert(
            "sources_omitted".to_string(),
            serde_json::Value::Number(serde_json::Number::from(omitted as u64)),
        );
    }
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

pub(super) fn deep_research_compact_source(
    source: &serde_json::Value,
) -> Option<serde_json::Value> {
    let safe_anchor = deep_research_traceable_source_anchor(source)?;
    let mut compact = serde_json::Map::new();
    for (key, aliases, limit) in [
        ("title", &["title"][..], 220usize),
        (
            "date",
            &["date", "publication_date", "published_at"][..],
            120,
        ),
        (
            "quote_or_fact",
            &["quote_or_fact", "evidence", "quote", "fact"][..],
            450,
        ),
        ("reliability", &["reliability", "publisher"][..], 220),
    ] {
        if let Some(value) = first_string_field(source, aliases) {
            compact.insert(
                key.to_string(),
                serde_json::Value::String(deep_research_digest_text(value, limit)),
            );
        }
    }
    compact.insert(
        "url_or_path".to_string(),
        serde_json::Value::String(deep_research_digest_text(&safe_anchor, 500)),
    );
    Some(serde_json::Value::Object(compact))
}

pub(super) fn deep_research_traceable_source_anchor(source: &serde_json::Value) -> Option<String> {
    let raw_anchor = first_string_field(source, &["url_or_path", "url", "path"])?;
    let (_, safe_anchor) = deep_research_safe_source_anchor(raw_anchor)?;
    let has_traceable_context = [
        "title",
        "quote_or_fact",
        "evidence",
        "quote",
        "fact",
        "reliability",
        "publisher",
        "date",
        "publication_date",
        "published_at",
    ]
    .iter()
    .any(|key| {
        source
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    });
    if !has_traceable_context {
        return None;
    }
    Some(safe_anchor)
}

pub(super) fn first_string_field<'a>(
    value: &'a serde_json::Value,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
}

pub(super) fn deep_research_compact_string_array(
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

pub(super) fn deep_research_compact_json_text(value: &serde_json::Value, limit: usize) -> String {
    let text = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default());
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    deep_research_digest_text(&compact, limit)
}

pub(super) fn deep_research_digest_text(text: &str, limit: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return compact;
    }
    if deep_research_output_has_internal_leak(&compact) {
        return "Internal workflow/tool log text withheld from DeepResearch synthesis.".to_string();
    }
    deep_research_truncate_chars(&compact, limit)
}

pub(super) fn deep_research_error_or_digest_text(
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

pub(super) fn deep_research_failure_summary(value: &serde_json::Value) -> String {
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
    if deep_research_contains_workflow_store_reference(&lower)
        || lower.contains("● searched")
        || lower.contains("● ran")
        || lower.contains("● read")
        || lower.contains("• searched")
        || lower.contains("• ran")
        || lower.contains("• read")
        || text.contains('⎿')
    {
        return "Delegated task returned internal workflow/tool logs that were withheld from the report context.".to_string();
    }
    "Delegated task failed before returning usable evidence.".to_string()
}

pub(super) fn deep_research_truncate_chars(text: &str, limit: usize) -> String {
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
        output.push('…');
    }
    output
}
