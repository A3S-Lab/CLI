#[cfg(test)]
pub(crate) fn materialize_deep_research_fallback_draft(
    workspace: &Path,
    query: &str,
    answer_text: &str,
    workflow_output: &str,
) -> Result<ResearchReportArtifacts, String> {
    let slug = deep_research_report_slug(query);
    let rel_html = format!(".a3s/research/{slug}/index.html");
    let (root, report_dir) = prepare_research_report_directory(workspace, &slug)?;

    let answer = deep_research_fallback_answer(answer_text, workflow_output);
    let evidence = deep_research_fallback_evidence(workflow_output);
    let artifact_note = deep_research_fallback_artifact_note(answer_text);
    let query_markdown = markdown_plain_text(query);
    let markdown = format!(
        "# DeepResearch Fallback Draft\n\n\
         > This is an incomplete fallback draft. It is not a completed DeepResearch report and \
         should not be opened automatically as a final RemoteUI view.\n\n\
         ## Query\n\n{query_markdown}\n\n\
         ## Draft Answer\n\n{answer}\n\n\
         ## Workflow Evidence Digest\n\n```json\n{evidence}\n```\n\n\
         ## Artifact Note\n\n\
         {artifact_note}\n"
    );
    let html = format!(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>DeepResearch Fallback Draft</title>\
         <style>body{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;line-height:1.6;margin:0;background:#f7f7f7;color:#111}}main{{max-width:920px;margin:0 auto;padding:32px 20px}}.banner{{border-left:4px solid #b45309;background:#fff7ed;padding:14px 16px;margin:16px 0}}section{{background:#fff;border:1px solid #ddd;border-radius:8px;padding:20px;margin:16px 0}}pre{{white-space:pre-wrap;word-break:break-word;background:#111;color:#f5f5f5;border-radius:6px;padding:16px;overflow:auto}}</style></head>\n\
         <body><main><h1>DeepResearch Fallback Draft</h1>\
         <div class=\"banner\">This draft was generated after DeepResearch failed to complete. It is not a final report and RemoteUI should not open it automatically.</div>\
         <section><h2>Query</h2><p>{query_html}</p></section>\
         <section><h2>Draft Answer</h2><pre>{answer_html}</pre></section>\
         <section><h2>Workflow Evidence Digest</h2><pre>{evidence_html}</pre></section>\
         <section><h2>Artifact Note</h2><p>{artifact_note_html}</p></section>\
         </main></body></html>\n",
        query_html = html_escape(query),
        answer_html = html_escape(&answer),
        evidence_html = html_escape(&evidence),
        artifact_note_html = html_escape(&artifact_note),
    );

    write_research_report_pair(
        &report_dir.join("report.md"),
        markdown,
        &report_dir.join("index.html"),
        html,
    )?;

    let artifacts = trusted_research_report_artifact_paths(&rel_html, &root)
        .ok_or_else(|| "fallback draft artifacts failed validation".to_string())?;
    Ok(artifacts)
}
pub(crate) fn deep_research_report_slug(query: &str) -> String {
    const MAX_READABLE_SLUG_BYTES: usize = 80;
    let base = super::asset_naming::asset_slug(query);
    let canonical_ascii = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase();
    if base != "asset"
        && base.len() <= MAX_READABLE_SLUG_BYTES
        && query.is_ascii()
        && base == canonical_ascii
    {
        return base;
    }
    if query.trim().is_empty() {
        return base;
    }

    let hash = deep_research_query_hash(query);
    let hash_text = format!("{hash:016x}");
    let hash_text = &hash_text[..12];
    if base == "asset" {
        return format!("research-{hash_text}");
    }

    let readable = base
        .chars()
        .take(MAX_READABLE_SLUG_BYTES)
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if readable.is_empty() {
        format!("research-{hash_text}")
    } else {
        format!("{readable}-{hash_text}")
    }
}
fn deep_research_query_hash(query: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in query.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
fn deep_research_fallback_evidence(workflow_output: &str) -> String {
    let evidence = deep_research_prompt_workflow_output(workflow_output);
    if evidence.trim().is_empty() {
        "{}".to_string()
    } else if deep_research_output_has_internal_leak(&evidence) {
        serde_json::json!({
            "status": "internal_logs_withheld",
            "note": "A3S Code captured diagnostics, but raw tool logs are not written into DeepResearch fallback artifacts."
        })
        .to_string()
    } else {
        evidence
    }
}

#[cfg(test)]
fn deep_research_fallback_answer(answer_text: &str, workflow_output: &str) -> String {
    let answer = answer_text.trim();
    if !answer.is_empty()
        && !is_deep_research_model_failure_text(answer)
        && !deep_research_output_has_internal_leak(answer)
    {
        return answer.to_string();
    }
    workflow_evidence_summary(workflow_output).unwrap_or_else(|| {
        "The model did not return a final synthesis, but A3S Code preserved a sanitized workflow evidence digest below.".to_string()
    })
}

fn is_deep_research_model_failure_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    // Keep historical pre-sectioned diagnostic spellings from being promoted
    // when an older interrupted run is recovered. Active report content uses
    // targeted revision and durable-resume terminology.
    lower.contains("deepresearch synthesis model call timed out")
        || lower.contains("deepresearch synthesis model call failed")
        || lower.contains("deepresearch repair model call timed out")
        || lower.contains("deepresearch repair model call failed")
}

#[cfg(test)]
fn deep_research_fallback_artifact_note(answer_text: &str) -> String {
    let answer = answer_text.trim();
    let mut note = "This fallback draft was materialized by A3S Code because the model response did not create the required completed report artifacts.".to_string();
    if !answer.is_empty() && is_deep_research_model_failure_text(answer) {
        note.push_str("\n\nModel synthesis status: ");
        note.push_str(answer);
    }
    note
}

pub(crate) fn workflow_evidence_summary(workflow_output: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(workflow_output).ok()?;
    let status = deep_research_collection_status(&value);
    let metadata = value
        .pointer("/research/metadata")
        .or_else(|| value.pointer("/metadata"));
    let success_count = metadata
        .and_then(|v| v.get("success_count"))
        .and_then(serde_json::Value::as_u64);
    let task_count = metadata
        .and_then(|v| v.get("task_count"))
        .and_then(serde_json::Value::as_u64);
    let result_count = metadata
        .and_then(|v| v.get("result_count"))
        .and_then(serde_json::Value::as_u64);
    let mode = value
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let evidence_noun = if mode == "direct_web" {
        "direct web evidence item"
    } else {
        "delegated research task"
    };
    let plural = |count: u64| {
        if mode == "direct_web" {
            if count == 1 {
                "direct web evidence item"
            } else {
                "direct web evidence items"
            }
        } else if count == 1 {
            "delegated research task"
        } else {
            "delegated research tasks"
        }
    };
    let count_text = match (success_count, task_count.or(result_count)) {
        (Some(success), Some(total)) => format!("{success}/{total} {}", plural(total)),
        (Some(success), None) => format!("{success} successful {}", plural(success)),
        (None, Some(total)) => format!("{total} {}", plural(total)),
        (None, None) => evidence_noun.to_string(),
    };
    let mut summary = format!(
        "The evidence collection phase ended with {status} status and captured {count_text}. A sanitized evidence digest is preserved below."
    );
    let direct_metadata = if mode == "direct_web" {
        value.pointer("/research/metadata")
    } else {
        value.pointer("/seed_research/metadata")
    };
    if let Some(coverage) = direct_metadata.and_then(direct_web_coverage_summary) {
        summary.push(' ');
        summary.push_str(&coverage);
    }
    if workflow_output.contains("README.md") {
        summary.push_str(" The evidence includes `README.md` as a cited local source.");
    }
    Some(summary)
}

fn direct_web_coverage_summary(metadata: &serde_json::Value) -> Option<String> {
    let source_count = metadata.get("source_count")?.as_u64()?;
    let host_count = metadata
        .get("host_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let fetched_count = metadata
        .get("fetched_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let fetched_host_count = metadata
        .get("fetched_host_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let mut summary = format!(
        "Web coverage: {source_count} semantically selected source(s) across {host_count} host(s), {fetched_count} fetched across {fetched_host_count} host(s)."
    );
    if metadata
        .get("freshness_required")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        let dated_source_count = metadata
            .get("dated_source_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        summary.push_str(&format!(
            " Freshness requires dates; {dated_source_count}/{source_count} source(s) are dated."
        ));
    }
    Some(summary)
}

fn html_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn markdown_plain_text(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() || ch.is_control() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if pending_space {
            normalized.push(' ');
            pending_space = false;
        }
        normalized.push(ch);
    }
    if normalized.is_empty() {
        return "DeepResearch query".to_string();
    }

    let targets = http_source_targets(&normalized);
    let mut escaped = String::with_capacity(normalized.len());
    let mut cursor = 0;
    let mut only_leading_digits = true;
    for target in targets {
        let Some(offset) = normalized[cursor..].find(&target) else {
            continue;
        };
        let target_start = cursor + offset;
        push_markdown_plain_segment(
            &normalized[cursor..target_start],
            &mut escaped,
            &mut only_leading_digits,
        );
        if let Some(safe_target) = canonical_research_source_anchor(&target) {
            escaped.push_str(&safe_target);
        } else {
            push_markdown_plain_segment(&target, &mut escaped, &mut only_leading_digits);
        }
        only_leading_digits = false;
        cursor = target_start + target.len();
    }
    push_markdown_plain_segment(
        &normalized[cursor..],
        &mut escaped,
        &mut only_leading_digits,
    );
    escaped
}

pub(super) fn markdown_backslash_unescape(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek().is_some_and(|next| next.is_ascii_punctuation()) {
            plain.push(chars.next().expect("peeked punctuation"));
        } else {
            plain.push(ch);
        }
    }
    plain
}

fn push_markdown_plain_segment(
    segment: &str,
    escaped: &mut String,
    only_leading_digits: &mut bool,
) {
    for ch in segment.chars() {
        let block_prefix = escaped.is_empty() && matches!(ch, '#' | '+' | '-');
        let ordered_list_prefix = ch == '.' && *only_leading_digits && !escaped.is_empty();
        if block_prefix
            || ordered_list_prefix
            || matches!(
                ch,
                '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '<' | '>' | '(' | ')' | '!' | '|'
            )
        {
            escaped.push('\\');
        }
        escaped.push(ch);
        *only_leading_digits &= ch.is_ascii_digit();
    }
}
