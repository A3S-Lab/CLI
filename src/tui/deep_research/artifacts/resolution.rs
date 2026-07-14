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

fn deep_research_report_sources_trace_workflow(
    artifacts: &ResearchReportArtifacts,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> bool {
    let anchors = deep_research_workflow_source_anchors(workflow_output, workflow_metadata);
    if anchors.is_empty() {
        // A DeepResearch report is only "completed" when it can be traced to
        // evidence gathered by this run. Failing open here lets a polished
        // model answer—or an old deterministic-slug report—mask a collection
        // failure that captured no source at all. Callers will materialize an
        // explicit recovery report instead.
        return false;
    }

    let markdown = read_small_utf8_file(&artifacts.markdown);
    let html = read_small_utf8_file(&artifacts.html);
    let (Some(markdown), Some(html)) = (markdown, html) else {
        return false;
    };
    let observed = anchors.into_iter().collect::<HashSet<_>>();
    let (cited, has_explicit_source_citation) = markdown_report_source_anchors(&markdown, query);
    let html_cited = html_report_source_anchors(&html, query);
    has_explicit_source_citation
        && !html_cited.is_empty()
        && cited.iter().chain(html_cited.iter()).all(|citation| {
            reported_research_source_candidates(citation)
                .iter()
                .any(|candidate| observed.contains(candidate))
        })
}

fn markdown_report_source_anchors(markdown: &str, query: &str) -> (Vec<String>, bool) {
    let mut anchors = Vec::new();
    let mut seen = HashSet::new();
    let mut in_source_section = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let heading = trimmed.trim_start_matches('#').trim();
            in_source_section = is_source_section_heading(heading);
            continue;
        }
        if in_source_section && is_explicit_source_citation_line(trimmed) {
            collect_source_anchors_from_citation_line(trimmed, &mut anchors, &mut seen);
        }
    }
    let explicit_count = anchors.len();
    let query_heading = format!("# {}", markdown_plain_text(query));
    let query_targets = http_source_targets(&markdown_plain_text(query))
        .into_iter()
        .filter_map(|target| canonical_query_http_target(&target))
        .collect::<HashSet<_>>();
    let mut checked_report_heading = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if !checked_report_heading && trimmed.starts_with("# ") {
            checked_report_heading = true;
            let heading_targets = http_source_targets(trimmed);
            let is_query_derived_heading = !heading_targets.is_empty()
                && heading_targets.iter().all(|target| {
                    canonical_query_http_target(target)
                        .is_some_and(|canonical| query_targets.contains(&canonical))
                });
            if trimmed == query_heading || is_query_derived_heading {
                continue;
            }
        }
        collect_http_source_anchors(line, &mut anchors, &mut seen);
        collect_markdown_link_anchors(line, &mut anchors, &mut seen);
    }
    (anchors, explicit_count > 0)
}

fn html_report_source_anchors(html: &str, query: &str) -> Vec<String> {
    let mut anchors = Vec::new();
    let mut seen = HashSet::new();
    let decoded = html.replace("&amp;", "&");
    let without_query_title = remove_matching_html_element(&decoded, "title", query);
    let without_query_heading = remove_matching_html_element(&without_query_title, "h1", query);
    collect_http_source_anchors(&without_query_heading, &mut anchors, &mut seen);
    collect_html_link_anchors(&without_query_heading, &mut anchors, &mut seen);
    collect_html_code_anchors(&without_query_heading, &mut anchors, &mut seen);
    collect_html_source_section_local_anchors(&without_query_heading, &mut anchors, &mut seen);
    anchors
}

fn remove_matching_html_element(html: &str, tag: &str, query: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let opening = format!("<{tag}");
    let closing = format!("</{tag}>");
    let expected =
        normalize_html_display_text(&markdown_backslash_unescape(&markdown_plain_text(query)));
    let mut cursor = 0;

    while let Some(offset) = lower[cursor..].find(&opening) {
        let start = cursor + offset;
        let name_end = start + opening.len();
        if !lower[name_end..]
            .chars()
            .next()
            .is_some_and(|ch| ch == '>' || ch.is_whitespace())
        {
            cursor = name_end;
            continue;
        }
        let Some(open_end_offset) = lower[name_end..].find('>') else {
            break;
        };
        let content_start = name_end + open_end_offset + 1;
        let Some(close_offset) = lower[content_start..].find(&closing) else {
            break;
        };
        let content_end = content_start + close_offset;
        let display =
            normalize_html_display_text(&strip_html_tags(&html[content_start..content_end]));
        let element_end = content_end + closing.len();
        let (has_http_target, targets_match_query) =
            html_element_targets_match_query(&html[content_start..content_end], &display, query);
        if targets_match_query && (display == expected || has_http_target) {
            let mut filtered = String::with_capacity(html.len());
            filtered.push_str(&html[..start]);
            filtered.push(' ');
            filtered.push_str(&html[element_end..]);
            return filtered;
        }
        cursor = element_end;
    }
    html.to_string()
}

fn html_element_targets_match_query(html: &str, display: &str, query: &str) -> (bool, bool) {
    let query_targets = http_source_targets(&markdown_plain_text(query))
        .into_iter()
        .filter_map(|target| canonical_query_http_target(&target))
        .collect::<HashSet<_>>();
    let targets = http_source_targets(display)
        .into_iter()
        .chain(html_link_targets(html))
        .collect::<Vec<_>>();
    let has_http_target = !targets.is_empty();
    let targets_match_query = targets.iter().all(|target| {
        canonical_query_http_target(target)
            .is_some_and(|canonical| query_targets.contains(&canonical))
    });
    (has_http_target, targets_match_query)
}

fn canonical_query_http_target(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str()?.is_empty() {
        return None;
    }
    Some(url.to_string())
}

fn normalize_html_display_text(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_source_section_heading(heading: &str) -> bool {
    let lower = heading.to_ascii_lowercase();
    lower.contains("source")
        || lower.contains("reference")
        || heading.contains("来源")
        || heading.contains("参考文献")
        || heading.contains("引用")
}

fn is_explicit_source_citation_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.contains("http://")
        || lower.contains("https://")
        || line.contains("](")
        || line.contains('`')
        || line.starts_with('|')
    {
        return true;
    }
    if ["- ", "* ", "+ "]
        .iter()
        .any(|prefix| line.starts_with(prefix))
    {
        return true;
    }
    line.split_once(". ").is_some_and(|(number, _)| {
        !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit())
    })
}

fn collect_source_anchors_from_citation_line(
    line: &str,
    anchors: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    collect_http_source_anchors(line, anchors, seen);
    collect_markdown_link_anchors(line, anchors, seen);

    let mut code = false;
    for segment in line.split('`') {
        if code {
            push_canonical_source_anchor(segment, anchors, seen);
        }
        code = !code;
    }

    for token in line.split_whitespace() {
        push_local_citation_anchor(token, anchors, seen);
    }
}

fn collect_http_source_anchors(text: &str, anchors: &mut Vec<String>, seen: &mut HashSet<String>) {
    for target in http_source_targets(text) {
        push_canonical_source_anchor(&target, anchors, seen);
    }
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
        let candidate = text[start..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
        targets.push(candidate.to_string());
        cursor = end;
    }
    targets
}

fn collect_markdown_link_anchors(
    line: &str,
    anchors: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let mut cursor = 0;
    while let Some(start) = line[cursor..].find("](") {
        let target_start = cursor + start + 2;
        let mut nested_parentheses = 0usize;
        let mut escaped = false;
        let end = line[target_start..]
            .char_indices()
            .find_map(|(offset, ch)| {
                if escaped {
                    escaped = false;
                    return None;
                }
                if ch == '\\' {
                    escaped = true;
                    return None;
                }
                match ch {
                    '(' => nested_parentheses += 1,
                    ')' if nested_parentheses == 0 => return Some(offset),
                    ')' => nested_parentheses -= 1,
                    _ => {}
                }
                None
            });
        let Some(end) = end else {
            break;
        };
        push_canonical_source_anchor(&line[target_start..target_start + end], anchors, seen);
        cursor = target_start + end + 1;
    }
}

fn collect_html_link_anchors(html: &str, anchors: &mut Vec<String>, seen: &mut HashSet<String>) {
    for target in html_link_targets(html) {
        push_canonical_source_anchor(&target, anchors, seen);
    }
}

fn html_link_targets(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut targets = Vec::new();
    for attribute in ["href", "src"] {
        let mut cursor = 0;
        while let Some(found) = lower[cursor..].find(attribute) {
            let attribute_start = cursor + found;
            let has_name_boundary = attribute_start == 0
                || lower[..attribute_start]
                    .chars()
                    .next_back()
                    .is_some_and(|ch| ch == '<' || ch.is_whitespace());
            if !has_name_boundary {
                cursor = attribute_start + attribute.len();
                continue;
            }
            let mut value_start = attribute_start + attribute.len();
            while html[value_start..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
            {
                value_start += html[value_start..].chars().next().unwrap().len_utf8();
            }
            if !html[value_start..].starts_with('=') {
                cursor = value_start;
                continue;
            }
            value_start += 1;
            while html[value_start..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
            {
                value_start += html[value_start..].chars().next().unwrap().len_utf8();
            }
            let Some(quote) = html[value_start..].chars().next() else {
                break;
            };
            if !matches!(quote, '"' | '\'') {
                cursor = value_start + quote.len_utf8();
                continue;
            }
            let target_start = value_start + quote.len_utf8();
            let Some(end) = html[target_start..].find(quote) else {
                break;
            };
            targets.push(html[target_start..target_start + end].to_string());
            cursor = target_start + end + quote.len_utf8();
        }
    }
    targets
}

fn collect_html_code_anchors(html: &str, anchors: &mut Vec<String>, seen: &mut HashSet<String>) {
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(start) = lower[cursor..].find("<code>") {
        let value_start = cursor + start + "<code>".len();
        let Some(end) = lower[value_start..].find("</code>") else {
            break;
        };
        push_canonical_source_anchor(&html[value_start..value_start + end], anchors, seen);
        cursor = value_start + end + "</code>".len();
    }
}

fn collect_html_source_section_local_anchors(
    html: &str,
    anchors: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    let mut in_source_section = false;

    while cursor < html.len() {
        let Some(open_offset) = lower[cursor..].find('<') else {
            if in_source_section {
                for token in html[cursor..].split_whitespace() {
                    push_local_citation_anchor(token, anchors, seen);
                }
            }
            break;
        };
        let open = cursor + open_offset;
        if in_source_section {
            for token in html[cursor..open].split_whitespace() {
                push_local_citation_anchor(token, anchors, seen);
            }
        }
        let Some(close_offset) = lower[open..].find('>') else {
            break;
        };
        let close = open + close_offset;
        let tag = lower[open + 1..close].trim();
        let name = tag
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_end_matches('/');
        if matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
            let closing = format!("</{name}>");
            let heading_start = close + 1;
            let Some(heading_offset) = lower[heading_start..].find(&closing) else {
                in_source_section = false;
                cursor = heading_start;
                continue;
            };
            let heading_end = heading_start + heading_offset;
            let heading = strip_html_tags(&html[heading_start..heading_end]);
            in_source_section = is_source_section_heading(heading.trim());
            cursor = heading_end + closing.len();
        } else {
            cursor = close + 1;
        }
    }
}

fn push_local_citation_anchor(
    candidate: &str,
    anchors: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let token = candidate.trim_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '`' | '"'
                    | '\''
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | ','
                    | ';'
                    | '.'
                    | ':'
                    | '!'
                    | '?'
            )
    });
    let has_source_extension = Path::new(token)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.chars().any(|ch| ch.is_ascii_alphabetic()));
    if token.contains('/') || token.contains('\\') || has_source_extension {
        push_canonical_source_anchor(token, anchors, seen);
    }
}

fn push_canonical_source_anchor(
    candidate: &str,
    anchors: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    let candidate = candidate.trim();
    let lower = candidate.to_ascii_lowercase();
    let mut mark_untrusted = || {
        let marker = "untrusted-report-url".to_string();
        if seen.insert(marker.clone()) {
            anchors.push(marker);
        }
    };
    if candidate.starts_with("//") || lower.contains("&#") || lower.contains("&colon;") {
        mark_untrusted();
        return;
    }
    if let Ok(url) = reqwest::Url::parse(candidate) {
        if !matches!(url.scheme(), "http" | "https") {
            mark_untrusted();
            return;
        }
        if !url.username().is_empty() || url.password().is_some() {
            mark_untrusted();
            return;
        }
    }
    if let Some(anchor) = canonical_research_source_anchor(candidate) {
        if seen.insert(anchor.clone()) {
            anchors.push(anchor);
        }
    }
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
