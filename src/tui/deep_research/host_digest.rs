//! Bounded evidence and source digests used by DeepResearch synthesis.

use super::*;

pub(super) fn deep_research_collect_structured_evidence(
    root: &serde_json::Value,
) -> Vec<serde_json::Value> {
    deep_research_collect_structured_evidence_bounded(root).0
}

/// Preserve complete bounded facts for the durable evidence ledger. The UI
/// and recovery digests intentionally remain shorter; reasoning must not use
/// those display excerpts as its source of truth.
pub(super) fn deep_research_collect_structured_evidence_for_ledger(
    root: &serde_json::Value,
) -> Vec<serde_json::Value> {
    collect_structured_evidence(root, EvidenceProjection::Ledger).0
}

pub(super) fn deep_research_collect_structured_evidence_bounded(
    root: &serde_json::Value,
) -> (Vec<serde_json::Value>, usize) {
    collect_structured_evidence(root, EvidenceProjection::Display)
}

#[derive(Clone, Copy)]
enum EvidenceProjection {
    Display,
    Ledger,
}

fn collect_structured_evidence(
    root: &serde_json::Value,
    projection: EvidenceProjection,
) -> (Vec<serde_json::Value>, usize) {
    fn walk(
        value: &serde_json::Value,
        round_hint: Option<u64>,
        out: &mut Vec<serde_json::Value>,
        omitted: &mut usize,
        seen: &mut HashSet<String>,
        projection: EvidenceProjection,
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
                        compact_evidence_object(structured, round, seen, projection)
                    {
                        if out.len() < DEEP_RESEARCH_MAX_DIGEST_EVIDENCE {
                            out.push(compact);
                        } else {
                            *omitted = omitted.saturating_add(1);
                        }
                    }
                } else if is_deep_research_evidence_object(value) {
                    if let Some(compact) = compact_evidence_object(value, round, seen, projection) {
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
                    walk(child, round, out, omitted, seen, projection);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item, round_hint, out, omitted, seen, projection);
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    let mut omitted = 0usize;
    let mut seen = HashSet::new();
    walk(root, None, &mut out, &mut omitted, &mut seen, projection);
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
    compact_evidence_object(evidence, round, seen, EvidenceProjection::Display)
}

fn compact_evidence_object(
    evidence: &serde_json::Value,
    round: Option<u64>,
    seen: &mut HashSet<String>,
    projection: EvidenceProjection,
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
                .filter_map(|source| compact_source(source, projection))
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
        let maximum = match (projection, key) {
            (EvidenceProjection::Ledger, "key_evidence") => 2_000,
            _ => 350,
        };
        compact.insert(
            key.to_string(),
            serde_json::Value::Array(deep_research_compact_string_array(
                evidence.get(key),
                DEEP_RESEARCH_MAX_DIGEST_STRINGS,
                maximum,
            )),
        );
    }
    if matches!(projection, EvidenceProjection::Ledger) {
        if let Some(source_coverage) = evidence
            .get("source_coverage")
            .and_then(compact_source_coverage)
        {
            compact.insert("source_coverage".to_string(), source_coverage);
        }
        if evidence.get("relevant_obligation_ids").is_some() {
            compact.insert(
                "relevant_obligation_ids".to_string(),
                serde_json::Value::Array(deep_research_compact_string_array(
                    evidence.get("relevant_obligation_ids"),
                    16,
                    160,
                )),
            );
        }
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

fn compact_source(
    source: &serde_json::Value,
    projection: EvidenceProjection,
) -> Option<serde_json::Value> {
    let safe_anchor = deep_research_traceable_source_anchor(source)?;
    let mut compact = serde_json::Map::new();
    if matches!(projection, EvidenceProjection::Ledger) {
        if let Some(source_id) = source.get("source_id").and_then(serde_json::Value::as_str) {
            compact.insert(
                "source_id".to_string(),
                serde_json::Value::String(source_id.chars().take(200).collect()),
            );
        }
    }
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
            match projection {
                EvidenceProjection::Display => 450,
                EvidenceProjection::Ledger => 2_000,
            },
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
    let excerpts = compact_source_evidence_excerpts(source, projection);
    if !excerpts.is_empty() {
        compact.insert(
            "evidence_excerpts".to_string(),
            serde_json::Value::Array(excerpts),
        );
    }
    compact.insert(
        "url_or_path".to_string(),
        serde_json::Value::String(deep_research_digest_text(&safe_anchor, 500)),
    );
    Some(serde_json::Value::Object(compact))
}

fn compact_source_coverage(value: &serde_json::Value) -> Option<serde_json::Value> {
    let bindings = value.as_array()?;
    let compact = bindings
        .iter()
        .take(65)
        .map(|binding| {
            let Some(binding) = binding.as_object() else {
                return serde_json::Value::Null;
            };
            let mut item = serde_json::Map::new();
            for key in ["source_id", "obligation_id"] {
                let value = binding
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(|value| {
                        serde_json::Value::String(value.chars().take(200).collect::<String>())
                    })
                    .unwrap_or(serde_json::Value::Null);
                item.insert(key.to_string(), value);
            }
            for key in ["completion_criterion_indexes", "roles"] {
                item.insert(
                    key.to_string(),
                    binding
                        .get(key)
                        .and_then(serde_json::Value::as_array)
                        .map(|values| {
                            serde_json::Value::Array(values.iter().take(9).cloned().collect())
                        })
                        .unwrap_or(serde_json::Value::Null),
                );
            }
            serde_json::Value::Object(item)
        })
        .collect::<Vec<_>>();
    Some(serde_json::Value::Array(compact))
}

fn compact_source_evidence_excerpts(
    source: &serde_json::Value,
    projection: EvidenceProjection,
) -> Vec<serde_json::Value> {
    let (item_limit, total_char_budget, per_item_char_limit) = match projection {
        EvidenceProjection::Display => (2usize, 600usize, 300usize),
        EvidenceProjection::Ledger => (4usize, 2_400usize, 700usize),
    };
    let mut seen = HashSet::new();
    let candidates = source
        .get("evidence_excerpts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|excerpt| {
            let focus = first_string_field(excerpt, &["focus"])?.trim();
            let quote = first_string_field(
                excerpt,
                &["quote_or_fact", "excerpt", "evidence", "quote", "fact"],
            )?
            .trim();
            if focus.is_empty() || quote.is_empty() {
                return None;
            }
            let key = format!(
                "{}|{}",
                focus.to_ascii_lowercase(),
                quote.to_ascii_lowercase()
            );
            seen.insert(key).then_some((focus, quote))
        })
        .take(item_limit)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Vec::new();
    }
    let per_item_budget = (total_char_budget / candidates.len()).min(per_item_char_limit);
    candidates
        .into_iter()
        .map(|(focus, quote)| {
            serde_json::json!({
                "focus": deep_research_digest_text(focus, 180),
                // deep_research_digest_text appends an ellipsis when it
                // truncates, so reserve one character to keep the aggregate
                // excerpt budget exact.
                "quote_or_fact": deep_research_digest_text(
                    quote,
                    per_item_budget.saturating_sub(1)
                )
            })
        })
        .collect()
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
    }) || source
        .get("evidence_excerpts")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|excerpts| {
            excerpts.iter().any(|excerpt| {
                first_string_field(
                    excerpt,
                    &["quote_or_fact", "excerpt", "evidence", "quote", "fact"],
                )
                .is_some_and(|value| !value.trim().is_empty())
            })
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
