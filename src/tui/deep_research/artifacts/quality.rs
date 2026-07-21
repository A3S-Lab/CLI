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
    visible_char_count(markdown_text) >= MIN_MARKDOWN_TEXT_CHARS
        && visible_char_count(&html_text) >= MIN_HTML_TEXT_CHARS
        && markdown_text
            .lines()
            .find(|line| !line.trim().is_empty())
            .is_some_and(|line| line.trim().starts_with("# "))
        && markdown_text
            .lines()
            .any(|line| line.trim_start().starts_with("## "))
        && html.contains("<main")
        && html.contains("<article")
        && html.contains("<h1")
}

pub(crate) fn deep_research_output_has_internal_leak(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if deep_research_contains_workflow_store_reference(&lower) {
        return true;
    }
    [
        "a3s://tool-output",
        "[tool output truncated",
        "full output artifact:",
        "dynamicworkflowruntime output:",
        "dynamicworkflowruntime metadata:",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
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
