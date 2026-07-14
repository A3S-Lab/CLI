#[cfg(test)]
mod html_quality_gate_tests {
    use super::complete_html_document;

    #[test]
    fn rejects_bare_or_visually_hidden_html() {
        assert!(!complete_html_document(
            "<!doctype html><html><body><h1>Report</h1><p>Sources and confidence.</p></body></html>"
        ));
        let hidden = "<html><head><meta name=\"viewport\"><title>x</title><style>body{display:none}@media(max-width:600px){}@media print{}a:focus{}table{overflow-x:auto}h1{font-size:clamp(2rem,3vw,4rem)}</style></head><body><h1>x</h1></body></html>";
        assert!(!complete_html_document(hidden));
    }
}
fn has_research_report_substance(markdown: &str, html: &str) -> bool {
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
    if deep_research_contains_workflow_store_reference(&lower) {
        return true;
    }
    let markers = [
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

pub(crate) fn deep_research_contains_workflow_store_reference(text: &str) -> bool {
    [".a3s/workflow", ".a3s\\workflow"]
        .into_iter()
        .any(|marker| {
            text.match_indices(marker).any(|(index, _)| {
                text[index + marker.len()..]
                    .chars()
                    .next()
                    .is_none_or(|next| {
                        next == '/'
                            || next == '\\'
                            || next.is_whitespace()
                            || matches!(next, '`' | '"' | '\'' | ')' | ']' | '}' | ',' | ';' | ':')
                    })
            })
        })
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn has_report_source_anchor(text: &str) -> bool {
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

pub(super) fn normalize_research_source_anchor(value: &str) -> Option<String> {
    let raw = value.trim();
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return None;
    }
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
    let is_search_navigation = reqwest::Url::parse(&normalized).is_ok_and(|url| {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let path = url.path().to_ascii_lowercase();
        matches!(
            host.as_str(),
            "search.brave.com" | "duckduckgo.com" | "www.sogou.com" | "www.so.com"
        ) || ((host == "google.com" || host.ends_with(".google.com"))
            && path.starts_with("/search"))
            || ((host == "bing.com" || host.ends_with(".bing.com") || host == "cn.bing.com")
                && path.starts_with("/search"))
    });
    if normalized.len() < 4
        || normalized.starts_with("a3s://")
        || is_search_navigation
        || deep_research_contains_workflow_store_reference(&normalized)
        || deep_research_output_has_internal_leak(&normalized)
        || !looks_like_traceable_source(&normalized)
    {
        None
    } else {
        Some(normalized)
    }
}

fn canonical_research_source_anchor(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return None;
    }
    let canonical = if let Ok(mut url) = reqwest::Url::parse(trimmed) {
        if !matches!(url.scheme(), "http" | "https") || url.host_str()?.is_empty() {
            return None;
        }
        url.set_username("").ok()?;
        url.set_password(None).ok()?;
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
        url.set_query(None);
        if !safe_query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(safe_query.iter().map(|(key, value)| (key, value)));
        }
        url.set_fragment(None);
        url.to_string()
    } else {
        let path = trimmed.replace('\\', "/");
        path.strip_prefix("./").unwrap_or(&path).to_string()
    };
    normalize_research_source_anchor(&canonical)?;
    Some(canonical)
}

fn reported_research_source_candidates(value: &str) -> Vec<String> {
    let Some(exact) = canonical_research_source_anchor(value) else {
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
    let without_fragment = exact.split('#').next().unwrap_or(&exact);
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
        let Some(candidate) = canonical_research_source_anchor(candidate) else {
            continue;
        };
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn looks_like_traceable_source(value: &str) -> bool {
    if value.starts_with("http://") || value.starts_with("https://") {
        return reqwest::Url::parse(value)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .is_some_and(|host| !host.is_empty());
    }
    if value.contains("://")
        || value.starts_with('/')
        || value.starts_with('~')
        || value.split(['/', '\\']).any(|part| part == "..")
    {
        return false;
    }

    let without_fragment = value.split('#').next().unwrap_or(value);
    let without_line = without_fragment
        .rsplit_once(':')
        .filter(|(_, suffix)| suffix.chars().all(|ch| ch.is_ascii_digit()))
        .map(|(path, _)| path)
        .unwrap_or(without_fragment);
    let path = Path::new(without_line);
    let has_file_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            !extension.is_empty()
                && extension.len() <= 16
                && extension
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        });
    let has_relative_path_shape = without_line.contains('/') || without_line.contains('\\');
    has_file_extension || has_relative_path_shape
}

fn normalize_research_source_text(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn visible_char_count(text: &str) -> usize {
    text.chars()
        .filter(|ch| !ch.is_whitespace() && !ch.is_control())
        .count()
}

fn strip_html_tags(html: &str) -> String {
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
        || lower.contains("deepresearch fallback")
        || lower.contains("deep research fallback draft")
        || lower.contains("<title>deepresearch fallback draft")
        || lower.contains("<h1>deepresearch fallback draft")
        || lower.contains("# deepresearch fallback draft")
        || lower.contains("not a completed deepresearch report")
        || lower.contains("not a final report")
}

fn is_html_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("html" | "htm")
    )
}

fn is_nonempty_file(path: &Path) -> bool {
    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}
