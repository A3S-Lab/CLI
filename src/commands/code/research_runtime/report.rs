//! DeepResearch report artifact materialization and validation.

use std::collections::HashSet;
use std::path::Path;

use crate::commands::code::naming::asset_slug;

use super::evidence::*;
use super::ResearchReportArtifacts;

pub(crate) const RESEARCH_VIEW_MARKER: &str = "A3S_RESEARCH_VIEW:";

pub(crate) fn research_report_artifacts_from_output_for_query(
    output: &str,
    workspace: &Path,
    query: &str,
) -> Option<ResearchReportArtifacts> {
    let expected_slug = deep_research_report_slug(query);
    research_report_artifacts_from_output_with_slug(output, workspace, Some(&expected_slug))
}

pub(crate) fn deep_research_report_artifacts_from_output_for_query(
    output: &str,
    workspace: &Path,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Option<ResearchReportArtifacts> {
    let artifacts = research_report_artifacts_from_output_for_query(output, workspace, query)?;
    deep_research_report_sources_trace_workflow(&artifacts, workflow_output, workflow_metadata)
        .then_some(artifacts)
}

pub(crate) fn clean_deep_research_final_text_from_artifacts(
    artifacts: &ResearchReportArtifacts,
    workspace: &Path,
) -> Option<String> {
    let markdown = read_small_utf8_file(&artifacts.markdown)?;
    if deep_research_output_has_internal_leak(&markdown) {
        return None;
    }
    let root = workspace.canonicalize().ok()?;
    let rel_html = artifacts.html.strip_prefix(&root).ok()?.to_string_lossy();
    let rel_html = rel_html.replace('\\', "/");
    let body = markdown.trim();
    if body.is_empty() {
        return None;
    }
    Some(format!("{body}\n\n{RESEARCH_VIEW_MARKER} {rel_html}"))
}

pub(crate) fn materialize_deep_research_completed_report_from_markdown(
    workspace: &Path,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Option<ResearchReportArtifacts> {
    let root = workspace.canonicalize().ok()?;
    let slug = deep_research_report_slug(query);
    let report_dir = root.join(".a3s").join("research").join(&slug);
    let markdown_path = report_dir.join("report.md");
    let markdown = read_small_utf8_file(&markdown_path)?;
    if looks_like_deep_research_fallback_draft(&markdown)
        || is_deep_research_model_failure_text(&markdown)
        || deep_research_output_has_internal_leak(&markdown)
        || visible_char_count(markdown.trim()) < 120
    {
        return None;
    }

    std::fs::create_dir_all(&report_dir).ok()?;
    let html = deep_research_completed_report_html(query, &markdown);
    std::fs::write(report_dir.join("index.html"), html).ok()?;

    let rel_html = format!(".a3s/research/{slug}/index.html");
    let artifacts = trusted_research_report_artifact_paths(&rel_html, &root)?;
    deep_research_report_sources_trace_workflow(&artifacts, workflow_output, workflow_metadata)
        .then_some(artifacts)
}

pub(crate) fn deep_research_completed_report_html(query: &str, markdown: &str) -> String {
    let title = deep_research_markdown_report_title(markdown, query);
    let body = deep_research_markdown_to_html_fragment(markdown);
    format!(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title>\
         <style>\
         :root{{color-scheme:light dark}}\
         body{{margin:0;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;line-height:1.62;background:#f8fafc;color:#111827}}\
         main{{max-width:960px;margin:0 auto;padding:36px 22px 56px}}\
         article{{background:#fff;border:1px solid #d1d5db;border-radius:8px;padding:28px}}\
         h1{{font-size:2rem;line-height:1.15;margin:0 0 18px}}\
         h2{{font-size:1.25rem;margin:28px 0 10px;border-top:1px solid #e5e7eb;padding-top:18px}}\
         h3{{font-size:1.05rem;margin:22px 0 8px}}\
         p,li{{font-size:1rem}}\
         a{{color:#0f766e}}\
         code{{background:#f3f4f6;border-radius:4px;padding:0 4px}}\
         pre{{white-space:pre-wrap;word-break:break-word;background:#111827;color:#f9fafb;border-radius:6px;padding:14px;overflow:auto}}\
         ul{{padding-left:22px}}\
         @media (prefers-color-scheme:dark){{body{{background:#0b0f14;color:#e5e7eb}}article{{background:#111827;border-color:#374151}}h2{{border-color:#374151}}code{{background:#1f2937}}a{{color:#5eead4}}}}\
         </style></head>\n\
         <body><main><article>{body}</article></main></body></html>\n",
        title = html_escape(&title),
        body = body,
    )
}

pub(crate) fn deep_research_markdown_report_title(markdown: &str, query: &str) -> String {
    markdown
        .lines()
        .find_map(|line| line.trim().strip_prefix("# "))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(query)
        .to_string()
}

pub(crate) fn deep_research_markdown_to_html_fragment(markdown: &str) -> String {
    let mut html = String::new();
    let mut in_code = false;
    let mut in_list = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            if in_code {
                html.push_str("</code></pre>");
            } else {
                html.push_str("<pre><code>");
            }
            in_code = !in_code;
            continue;
        }

        if in_code {
            html.push_str(&html_escape(line));
            html.push('\n');
            continue;
        }

        if trimmed.is_empty() {
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            continue;
        }

        if let Some((level, text)) = markdown_heading(trimmed) {
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            html.push_str(&format!("<h{level}>{}</h{level}>", html_escape(text)));
            continue;
        }

        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            if !in_list {
                html.push_str("<ul>");
                in_list = true;
            }
            html.push_str(&format!("<li>{}</li>", html_escape(item.trim())));
            continue;
        }

        if in_list {
            html.push_str("</ul>");
            in_list = false;
        }
        html.push_str(&format!("<p>{}</p>", html_escape(trimmed)));
    }

    if in_code {
        html.push_str("</code></pre>");
    }
    if in_list {
        html.push_str("</ul>");
    }
    html
}

pub(crate) fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if !(1..=3).contains(&hashes) {
        return None;
    }
    let text = line.get(hashes..)?.strip_prefix(' ')?.trim();
    (!text.is_empty()).then_some((hashes, text))
}

pub(crate) fn research_report_artifacts_from_output_with_slug(
    output: &str,
    workspace: &Path,
    expected_slug: Option<&str>,
) -> Option<ResearchReportArtifacts> {
    output.lines().rev().find_map(|line| {
        let marker_at = line.find(RESEARCH_VIEW_MARKER)?;
        let raw = &line[marker_at + RESEARCH_VIEW_MARKER.len()..];
        let candidate = clean_research_report_marker_value(raw)?;
        let artifacts = trusted_research_report_artifacts(&candidate, workspace)?;
        match expected_slug {
            Some(slug) if !research_report_artifact_slug_matches(&artifacts, slug) => None,
            _ => Some(artifacts),
        }
    })
}

pub(crate) fn research_report_artifact_slug_matches(
    artifacts: &ResearchReportArtifacts,
    expected_slug: &str,
) -> bool {
    artifacts
        .html
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        == Some(expected_slug)
}

pub(crate) fn clean_research_report_marker_value(raw: &str) -> Option<String> {
    let mut value = raw.trim();
    value = value
        .trim_start_matches(['`', '"', '\'', '<'])
        .trim_end_matches(['`', '"', '\'', '>', '.', ',', ';']);
    if value.is_empty() || value.starts_with("file://") {
        return None;
    }
    value
        .split_whitespace()
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn trusted_research_report_artifacts(
    candidate: &str,
    workspace: &Path,
) -> Option<ResearchReportArtifacts> {
    let artifacts = trusted_research_report_artifact_paths(candidate, workspace)?;
    completed_research_report_artifacts(&artifacts).then_some(artifacts)
}

pub(crate) fn trusted_research_report_artifact_paths(
    candidate: &str,
    workspace: &Path,
) -> Option<ResearchReportArtifacts> {
    let root = workspace.canonicalize().ok()?;
    let candidate = Path::new(candidate);
    let path = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace.join(candidate)
    }
    .canonicalize()
    .ok()?;
    if !is_nonempty_file(&path) || !path.starts_with(&root) || !is_html_path(&path) {
        return None;
    }
    let rel = path.strip_prefix(&root).ok()?;
    let mut components = rel.components();
    let first = components.next()?.as_os_str();
    let second = components.next()?.as_os_str();
    let slug = components.next()?.as_os_str();
    let file = components.next()?.as_os_str();
    if components.next().is_some() {
        return None;
    }
    if first != std::ffi::OsStr::new(".a3s") || second != std::ffi::OsStr::new("research") {
        return None;
    }
    if slug.is_empty() || file != std::ffi::OsStr::new("index.html") {
        return None;
    }
    let markdown = path.parent()?.join("report.md").canonicalize().ok()?;
    if !is_nonempty_file(&markdown) || !markdown.starts_with(&root) {
        return None;
    }
    Some(ResearchReportArtifacts {
        markdown,
        html: path,
    })
}

pub(crate) fn completed_research_report_artifacts(artifacts: &ResearchReportArtifacts) -> bool {
    let markdown = read_small_utf8_file(&artifacts.markdown);
    let html = read_small_utf8_file(&artifacts.html);
    let (Some(markdown), Some(html)) = (markdown, html) else {
        return false;
    };
    !looks_like_deep_research_fallback_draft(&markdown)
        && !looks_like_deep_research_fallback_draft(&html)
        && !is_deep_research_model_failure_text(&markdown)
        && !is_deep_research_model_failure_text(&html)
        && !deep_research_output_has_internal_leak(&markdown)
        && !deep_research_output_has_internal_leak(&html)
        && complete_html_document(&html)
        && has_research_report_substance(&markdown, &html)
}

pub(crate) fn deep_research_report_sources_trace_workflow(
    artifacts: &ResearchReportArtifacts,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> bool {
    let anchors = deep_research_workflow_source_anchors(workflow_output, workflow_metadata);
    if anchors.is_empty() {
        return true;
    }

    let markdown = read_small_utf8_file(&artifacts.markdown);
    let html = read_small_utf8_file(&artifacts.html);
    let (Some(markdown), Some(html)) = (markdown, html) else {
        return false;
    };
    let report_text =
        normalize_research_source_text(&format!("{}\n{}", markdown, strip_html_tags(&html)));
    anchors
        .iter()
        .any(|anchor| report_text_contains_source_anchor(&report_text, anchor))
}

pub(crate) fn deep_research_workflow_source_anchors(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Vec<String> {
    let mut anchors = Vec::new();
    let mut seen = HashSet::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(workflow_output) {
        let digest = deep_research_workflow_output_digest(&value);
        collect_deep_research_source_anchors(&digest, &mut anchors, &mut seen);
    }
    if let Some(metadata) = workflow_metadata {
        let digest = deep_research_workflow_metadata_digest(metadata);
        collect_deep_research_source_anchors(&digest, &mut anchors, &mut seen);
    }
    anchors
}

pub(crate) fn collect_deep_research_source_anchors(
    value: &serde_json::Value,
    anchors: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if key == "url_or_path" {
                    if let Some(anchor) = value
                        .as_str()
                        .and_then(normalize_research_source_anchor)
                        .filter(|anchor| seen.insert(anchor.clone()))
                    {
                        anchors.push(anchor);
                    }
                }
                collect_deep_research_source_anchors(value, anchors, seen);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_deep_research_source_anchors(item, anchors, seen);
            }
        }
        _ => {}
    }
}

pub(crate) fn read_small_utf8_file(path: &Path) -> Option<String> {
    const MAX_REPORT_VALIDATION_BYTES: u64 = 2 * 1024 * 1024;
    let metadata = path.metadata().ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_REPORT_VALIDATION_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

pub(crate) fn complete_html_document(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("<html")
        && lower.contains("</html>")
        && lower.contains("<body")
        && lower.contains("</body>")
}

pub(crate) fn has_research_report_substance(markdown: &str, html: &str) -> bool {
    const MIN_MARKDOWN_TEXT_CHARS: usize = 120;
    const MIN_HTML_TEXT_CHARS: usize = 120;

    let markdown_text = markdown.trim();
    let html_text = strip_html_tags(html);
    if visible_char_count(markdown_text) < MIN_MARKDOWN_TEXT_CHARS
        || visible_char_count(&html_text) < MIN_HTML_TEXT_CHARS
    {
        return false;
    }

    let combined = format!("{markdown_text}\n{html_text}").to_lowercase();
    let placeholder_markers = [
        "placeholder",
        "lorem ipsum",
        "todo",
        "tbd",
        "coming soon",
        "under construction",
        "not yet available",
        "待补充",
        "占位",
    ];
    if placeholder_markers
        .iter()
        .any(|marker| combined.contains(marker))
    {
        return false;
    }

    let has_findings = contains_any(
        &combined,
        &[
            "finding",
            "findings",
            "conclusion",
            "conclusions",
            "analysis",
            "recommendation",
            "recommendations",
            "结论",
            "分析",
            "发现",
            "建议",
        ],
    );
    let has_sources = contains_any(
        &combined,
        &[
            "source",
            "sources",
            "evidence",
            "citation",
            "citations",
            "来源",
            "证据",
            "引用",
        ],
    );
    let has_confidence = contains_any(
        &combined,
        &[
            "confidence",
            "caveat",
            "caveats",
            "limitation",
            "limitations",
            "risk",
            "risks",
            "uncertain",
            "uncertainty",
            "置信",
            "限制",
            "风险",
            "不确定",
        ],
    );

    has_findings && has_sources && has_confidence && has_report_source_anchor(&combined)
}

pub(crate) fn deep_research_output_has_internal_leak(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let markers = [
        ".a3s-flow/dynamic-workflows",
        "a3s://tool-output",
        "[tool output truncated",
        "full output artifact:",
        "permission denied: tool",
        "max tool rounds",
        "dynamicworkflowruntime output:",
        "dynamicworkflowruntime metadata:",
        "dynamicworkflowruntime evidence package:",
        "dynamicworkflowruntime diagnostic package:",
        "dynamicworkflowruntime structured evidence",
        "provided dynamicworkflowruntime",
        "evidence digest:",
        "run diagnostics:",
        "provided evidence digest",
        "provided run diagnostics",
        "workflow runtime/evidence-package",
        "workflow evidence\n\n```text",
        "created the target report directory",
        "created the report directory",
        "created `.a3s/research",
        "created .a3s/research",
        "created the markdown report",
        "created the standalone",
        "markdown report written",
        "markdown report written to",
        "wrote the markdown report",
        "wrote the standalone",
        "wrote the html report",
        "wrote the standalone responsive html artifact",
        "verifying the two required report artifacts",
        "targeted verification passed",
        "report.md exists",
        "index.html exists",
        "written and verified successfully",
        "batch verification was unavailable",
        "file-read access is blocked",
        "file-read tooling is currently blocked",
        "unable to verify the two required files",
        "targeted verification could not be performed",
        "verification could not be performed",
        "remaining unverified contract items",
        "step 2 complete",
        "step 3 complete",
        "● searched",
        "● ran",
        "● read ",
        "⎿",
    ];
    if markers.iter().any(|marker| lower.contains(marker)) {
        return true;
    }

    let json_field_hits = [
        "\"summary\"",
        "\"sources\"",
        "\"key_evidence\"",
        "\"contradictions\"",
        "\"confidence\"",
        "\"gaps\"",
        "\"url_or_path\"",
        "\"quote_or_fact\"",
    ]
    .iter()
    .filter(|field| lower.contains(**field))
    .count();
    json_field_hits >= 3
}

pub(crate) fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

pub(crate) fn has_report_source_anchor(text: &str) -> bool {
    contains_any(
        text,
        &[
            "http://",
            "https://",
            "readme.md",
            "design.md",
            "cargo.toml",
            "package.json",
            "pyproject.toml",
            "src/",
            "crates/",
            "apps/",
            "docs/",
            ".a3s/",
            ".rs",
            ".ts",
            ".tsx",
            ".js",
            ".jsx",
            ".py",
            ".go",
            ".java",
            ".md",
            ".mdx",
            ".pdf",
        ],
    )
}

pub(crate) fn normalize_research_source_anchor(value: &str) -> Option<String> {
    let normalized = normalize_research_source_text(value)
        .trim_matches(|ch: char| {
            matches!(
                ch,
                '`' | '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}'
            )
        })
        .trim_end_matches(['.', ',', ';', ':', ')', ']'])
        .trim()
        .trim_end_matches('/')
        .to_string();
    if normalized.len() < 4
        || normalized.starts_with("a3s://")
        || normalized.contains(".a3s-flow/dynamic-workflows")
        || deep_research_output_has_internal_leak(&normalized)
    {
        None
    } else {
        Some(normalized)
    }
}

pub(crate) fn normalize_research_source_text(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn report_text_contains_source_anchor(report_text: &str, anchor: &str) -> bool {
    if report_text.contains(anchor) {
        return true;
    }
    anchor
        .strip_suffix('/')
        .filter(|value| value.len() >= 4)
        .is_some_and(|value| report_text.contains(value))
}

pub(crate) fn visible_char_count(text: &str) -> usize {
    text.chars()
        .filter(|ch| !ch.is_whitespace() && !ch.is_control())
        .count()
}

pub(crate) fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

pub(crate) fn looks_like_deep_research_fallback_draft(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("deepresearch fallback draft")
        || lower.contains("fallback draft")
        || lower.contains("not a completed deepresearch report")
        || lower.contains("not a final report")
}

pub(crate) fn is_html_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("html" | "htm")
    )
}

pub(crate) fn is_nonempty_file(path: &Path) -> bool {
    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

pub(crate) fn materialize_deep_research_fallback_draft(
    workspace: &Path,
    query: &str,
    answer_text: &str,
    workflow_output: &str,
) -> Result<ResearchReportArtifacts, String> {
    let slug = deep_research_report_slug(query);
    let rel_html = format!(".a3s/research/{slug}/index.html");
    let report_dir = workspace.join(".a3s").join("research").join(&slug);
    std::fs::create_dir_all(&report_dir)
        .map_err(|e| format!("could not create {}: {e}", report_dir.display()))?;

    let answer = deep_research_fallback_answer(answer_text, workflow_output);
    let evidence = deep_research_fallback_evidence(workflow_output);
    let artifact_note = deep_research_fallback_artifact_note(answer_text);
    let markdown = format!(
        "# DeepResearch Fallback Draft\n\n\
         > This is an incomplete fallback draft. It is not a completed DeepResearch report and \
         should not be opened automatically as a final RemoteUI view.\n\n\
         ## Query\n\n{query}\n\n\
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

    std::fs::write(report_dir.join("report.md"), markdown)
        .map_err(|e| format!("could not write fallback report.md: {e}"))?;
    std::fs::write(report_dir.join("index.html"), html)
        .map_err(|e| format!("could not write fallback index.html: {e}"))?;

    let artifacts = trusted_research_report_artifact_paths(&rel_html, workspace)
        .ok_or_else(|| "fallback draft artifacts failed validation".to_string())?;
    Ok(artifacts)
}

pub(crate) fn deep_research_report_slug(query: &str) -> String {
    const MAX_READABLE_SLUG_BYTES: usize = 80;
    let base = asset_slug(query);
    if base != "asset" && base.len() <= MAX_READABLE_SLUG_BYTES && query.is_ascii() {
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

pub(crate) fn deep_research_query_hash(query: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in query.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub(crate) fn nonempty_report_section(text: &str, fallback: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn deep_research_fallback_evidence(workflow_output: &str) -> String {
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

pub(crate) fn deep_research_fallback_answer(answer_text: &str, workflow_output: &str) -> String {
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

pub(crate) fn is_deep_research_model_failure_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("deepresearch synthesis model call timed out")
        || lower.contains("deepresearch synthesis model call failed")
        || lower.contains("deepresearch repair model call timed out")
        || lower.contains("deepresearch repair model call failed")
}

pub(crate) fn deep_research_fallback_artifact_note(answer_text: &str) -> String {
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
    let count_text = match (success_count, task_count.or(result_count)) {
        (Some(success), Some(total)) => format!("{success}/{total} delegated research tasks"),
        (Some(success), None) => format!("{success} successful delegated research tasks"),
        (None, Some(total)) => format!("{total} delegated research results"),
        (None, None) => "delegated research evidence".to_string(),
    };
    let mut summary = format!(
        "The evidence collection phase ended with {status} status and captured {count_text}. A sanitized evidence digest is preserved below."
    );
    if workflow_output.contains("README.md") {
        summary.push_str(" The evidence includes `README.md` as a cited local source.");
    }
    Some(summary)
}

pub(crate) fn html_escape(text: &str) -> String {
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
