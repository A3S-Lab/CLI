fn research_report_artifacts_from_output_with_slug(
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
fn research_report_artifact_slug_matches(
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
fn clean_research_report_marker_value(raw: &str) -> Option<String> {
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

fn trusted_research_report_artifacts(
    candidate: &str,
    workspace: &Path,
) -> Option<ResearchReportArtifacts> {
    let artifacts = trusted_research_report_artifact_paths(candidate, workspace)?;
    completed_research_report_artifacts(&artifacts).then_some(artifacts)
}

fn trusted_research_report_artifact_paths(
    candidate: &str,
    workspace: &Path,
) -> Option<ResearchReportArtifacts> {
    let root = workspace.canonicalize().ok()?;
    let candidate = Path::new(candidate);
    let unresolved_path = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace.join(candidate)
    };
    let unresolved_metadata = std::fs::symlink_metadata(&unresolved_path).ok()?;
    if unresolved_metadata.file_type().is_symlink() || !unresolved_metadata.is_file() {
        return None;
    }
    let path = unresolved_path.canonicalize().ok()?;
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
    let markdown_path = path.parent()?.join("report.md");
    let markdown_metadata = std::fs::symlink_metadata(&markdown_path).ok()?;
    if markdown_metadata.file_type().is_symlink() || !markdown_metadata.is_file() {
        return None;
    }
    let markdown = markdown_path.canonicalize().ok()?;
    if !is_nonempty_file(&markdown)
        || markdown.parent() != path.parent()
        || markdown.file_name() != Some(std::ffi::OsStr::new("report.md"))
    {
        return None;
    }
    Some(ResearchReportArtifacts {
        markdown,
        html: path,
    })
}

fn completed_research_report_artifacts(artifacts: &ResearchReportArtifacts) -> bool {
    let markdown = read_small_utf8_file(&artifacts.markdown);
    let html = read_small_utf8_file(&artifacts.html);
    let (Some(markdown), Some(html)) = (markdown, html) else {
        return false;
    };
    !looks_like_deep_research_fallback_draft(&markdown)
        && !looks_like_deep_research_fallback_draft(&html)
        && !looks_like_deep_research_recovery_report(&markdown)
        && !looks_like_deep_research_recovery_report(&html)
        && !is_deep_research_model_failure_text(&markdown)
        && !is_deep_research_model_failure_text(&html)
        && !deep_research_output_has_internal_leak(&markdown)
        && !deep_research_output_has_internal_leak(&html)
        && complete_html_document(&html)
        && has_research_report_substance(&markdown, &html)
}

#[cfg(test)]
fn deep_research_report_sources_trace_workflow(
    artifacts: &ResearchReportArtifacts,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> bool {
    let markdown = read_small_utf8_file(&artifacts.markdown);
    let html = read_small_utf8_file(&artifacts.html);
    let (Some(markdown), Some(html)) = (markdown, html) else {
        return false;
    };

    deep_research_report_content_sources_trace_workflow(
        &markdown,
        &html,
        query,
        workflow_output,
        workflow_metadata,
    )
}

#[cfg(test)]
fn deep_research_report_content_sources_trace_workflow(
    markdown: &str,
    html: &str,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> bool {
    deep_research_report_source_trace_diagnostic(
        markdown,
        html,
        query,
        workflow_output,
        workflow_metadata,
    )
    .is_ok()
}

fn deep_research_report_source_trace_diagnostic(
    markdown: &str,
    html: &str,
    _query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Result<(), String> {
    let anchors = deep_research_workflow_source_anchors(workflow_output, workflow_metadata);
    if anchors.is_empty() {
        // A DeepResearch report is only "completed" when it can be traced to
        // evidence gathered by this run. Failing open here lets a polished
        // model answer—or an old deterministic-slug report—mask a collection
        // failure that captured no source at all. Callers will materialize an
        // explicit recovery report instead.
        return Err(
            "source trace rejected: the workflow captured no traceable research sources"
                .to_string(),
        );
    }

    let sources = anchors
        .iter()
        .enumerate()
        .map(
            |(index, anchor)| super::deep_research_report_audit::ReportSourceReference {
                source_id: format!("source:{index}"),
                anchor: anchor.clone(),
            },
        )
        .collect::<Vec<_>>();
    let audit = super::deep_research_report_audit::audit_report(
        markdown,
        html,
        &sources,
        super::deep_research_report_audit::CitationRequirement::AtLeastOne,
    );
    if !audit.passed {
        return Err(format!("source trace rejected: {}", audit.reason));
    }

    let observed = anchors
        .iter()
        .filter_map(|anchor| super::deep_research_report_audit::canonical_citation_target(anchor))
        .collect::<HashSet<_>>();
    let mut unmatched = super::deep_research_report_audit::report_citation_targets(markdown, html)
        .into_iter()
        .filter(|citation| {
            !citation.starts_with('#')
                && !citation.starts_with("mailto:")
                && !observed.contains(citation)
        })
        .collect::<Vec<_>>();
    unmatched.sort();
    unmatched.dedup();
    if unmatched.is_empty() {
        return Ok(());
    }

    let displayed = unmatched.iter().take(8).cloned().collect::<Vec<_>>();
    let omitted = unmatched.len().saturating_sub(displayed.len());
    let mut message = format!(
        "source trace rejected: {} citation{} were not observed in this run: {}",
        unmatched.len(),
        if unmatched.len() == 1 { "" } else { "s" },
        displayed.join(", ")
    );
    if omitted > 0 {
        message.push_str(&format!(", plus {omitted} more"));
    }
    Err(message)
}

fn sanitize_unobserved_markdown_http_citations(
    markdown: &str,
    _query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> String {
    let observed = deep_research_workflow_source_anchors(workflow_output, workflow_metadata)
        .into_iter()
        .filter_map(|anchor| super::deep_research_report_audit::canonical_citation_target(&anchor))
        .collect::<HashSet<_>>();
    if observed.is_empty() {
        return markdown.to_string();
    }

    let unobserved = super::deep_research_report_audit::report_citation_targets(markdown, "")
        .into_iter()
        .filter(|citation| citation.starts_with("http://") || citation.starts_with("https://"))
        .filter(|citation| !observed.contains(citation))
        .collect::<HashSet<_>>();
    if unobserved.is_empty() {
        return markdown.to_string();
    }

    let invalid_targets = http_source_targets(markdown)
        .into_iter()
        .filter(|target| {
            super::deep_research_report_audit::canonical_citation_target(target)
                .is_some_and(|target| unobserved.contains(&target))
        })
        .collect::<HashSet<_>>();
    if invalid_targets.is_empty() {
        return markdown.to_string();
    }

    let mut cleaned = Vec::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with("# ") {
            cleaned.push(line.to_string());
        } else {
            cleaned.push(strip_unobserved_http_targets(line, &invalid_targets));
        }
    }

    let mut output = cleaned.join("\n");
    if markdown.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn strip_unobserved_http_targets(line: &str, invalid_targets: &HashSet<String>) -> String {
    let mut output = line.to_string();
    let mut targets = invalid_targets.iter().collect::<Vec<_>>();
    targets.sort_by_key(|target| std::cmp::Reverse(target.len()));
    for target in targets {
        let mut cursor = 0;
        while let Some(offset) = output[cursor..].find(target.as_str()) {
            let start = cursor + offset;
            let end = start + target.len();
            if let Some((link_start, link_end, label)) =
                markdown_link_replacement(&output, start, end)
            {
                output.replace_range(link_start..link_end, &label);
                cursor = link_start + label.len();
            } else {
                output.replace_range(start..end, "");
                cursor = start;
            }
        }
    }
    output
}

fn markdown_link_replacement(
    text: &str,
    target_start: usize,
    target_end: usize,
) -> Option<(usize, usize, String)> {
    if target_start < 2 || text.get(target_start - 2..target_start)? != "](" {
        return None;
    }
    let label_start = text[..target_start - 2].rfind('[')?;
    if text[label_start + 1..target_start - 2]
        .chars()
        .any(|ch| matches!(ch, '\n' | '\r'))
    {
        return None;
    }
    let suffix = &text[target_end..];
    let close_offset = suffix.find(')')?;
    if suffix[..close_offset]
        .chars()
        .any(|ch| matches!(ch, '\n' | '\r'))
    {
        return None;
    }
    let link_end = target_end + close_offset + 1;
    let label = text[label_start + 1..target_start - 2].to_string();
    Some((label_start, link_end, label))
}

fn http_source_targets(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut cursor = 0;
    let mut targets = Vec::new();
    while cursor < text.len() {
        let next = ["http://", "https://"]
            .into_iter()
            .filter_map(|prefix| lower[cursor..].find(prefix).map(|index| cursor + index))
            .min();
        let Some(start) = next else {
            break;
        };
        let mut nested_parentheses = 0usize;
        let end = text[start..]
            .char_indices()
            .find_map(|(offset, ch)| {
                if ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | '\'' | '`' | ']' | '}') {
                    return Some(start + offset);
                }
                match ch {
                    '(' => nested_parentheses += 1,
                    ')' if nested_parentheses == 0 => return Some(start + offset),
                    ')' => nested_parentheses -= 1,
                    _ => {}
                }
                None
            })
            .unwrap_or(text.len());
        let candidate =
            text[start..end].trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '_', '~']);
        targets.push(candidate.to_string());
        cursor = end;
    }
    targets
}

fn read_small_utf8_file(path: &Path) -> Option<String> {
    const MAX_REPORT_VALIDATION_BYTES: u64 = 2 * 1024 * 1024;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_REPORT_VALIDATION_BYTES
    {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn complete_html_document(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("<html")
        && lower.contains("</html>")
        && lower.contains("<body")
        && lower.contains("</body>")
        && lower.contains("name=\"viewport\"")
        && lower.contains("<title>")
        && lower.contains("</title>")
        && lower.contains("<style")
        && lower.matches("<h1").count() == 1
        && lower.contains("@media")
        && lower.contains("max-width")
        && lower.contains("@media print")
        && lower.contains(":focus")
        && lower.contains("overflow-x")
        && lower.contains("clamp(")
        && !lower.contains("<script")
        && !lower.contains("body{display:none")
        && !lower.contains("body {display:none")
        && !lower.contains("font-size:0")
        && !lower.contains("font-size:1px")
        && !lower.contains("width:200vw")
}
