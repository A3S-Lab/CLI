use super::{html_escape, markdown_backslash_unescape, markdown_plain_text};
use comrak::{markdown_to_html, Options};
use std::collections::HashSet;

#[path = "html_composition.rs"]
mod composition;
#[path = "html_style.rs"]
mod style;

use composition::compose_report_fragment;
use style::REPORT_CSS;

pub(super) fn deep_research_completed_report_html(query: &str, markdown: &str) -> String {
    let lower_markdown = markdown.to_ascii_lowercase();
    let recovery = lower_markdown.contains("# deepresearch recovery report");
    let qualified = lower_markdown.contains("evidence grade: official-source snapshot")
        || markdown.contains("证据等级：官方来源快照");
    let title = concise_report_title(&deep_research_markdown_report_title(markdown, query));
    let language = if contains_cjk(markdown) || contains_cjk(query) {
        "zh-CN"
    } else {
        "en"
    };
    let raw_body = strip_first_h1(&deep_research_markdown_to_html_fragment(markdown));
    let declared_source_count = markdown_cited_source_count(markdown);
    let source_count = if declared_source_count == 0 {
        unique_external_source_count(&raw_body)
    } else {
        declared_source_count
    };
    let composition = compose_report_fragment(&raw_body, language);
    let finding_count = composition.finding_count.max(
        markdown
            .lines()
            .filter(|line| line.starts_with("### "))
            .count(),
    );
    let reading_minutes = estimated_reading_minutes(markdown, language);
    let theme = report_theme(query, markdown);
    let body_class = if recovery {
        format!("{theme} report-degraded")
    } else if qualified {
        format!("{theme} report-qualified")
    } else {
        theme.to_string()
    };
    let evidence_label = if recovery && language == "zh-CN" {
        "证据不足 · 已降级"
    } else if recovery {
        "Insufficient evidence · Degraded"
    } else if qualified && language == "zh-CN" {
        "官方快照 · 待独立核验"
    } else if qualified {
        "Official snapshot · Cross-check pending"
    } else if language == "zh-CN" {
        "来源可追溯"
    } else {
        "Traceable evidence"
    };
    let sources_label = if language == "zh-CN" {
        "引用来源"
    } else {
        "Cited sources"
    };
    let sources_separator = if language == "zh-CN" { "：" } else { ": " };
    let confidence_label = if recovery && language == "zh-CN" {
        "不可作为最终定论"
    } else if recovery {
        "Not a final domain conclusion"
    } else if qualified && language == "zh-CN" {
        "可交付，但不是双源定论"
    } else if qualified {
        "Usable, not a two-source conclusion"
    } else if language == "zh-CN" {
        "明确置信度与局限"
    } else {
        "Confidence & limits stated"
    };
    let brief_label = if recovery && language == "zh-CN" {
        "A3S 深度研究 · 降级报告"
    } else if recovery {
        "A3S Deep Research · Degraded"
    } else if qualified && language == "zh-CN" {
        "A3S 深度研究 · 权威快照"
    } else if qualified {
        "A3S Deep Research · Authoritative snapshot"
    } else if language == "zh-CN" {
        "A3S 深度研究"
    } else {
        "A3S Deep Research"
    };
    let reading_label = if language == "zh-CN" {
        "研究报告"
    } else {
        "Research report"
    };
    let metadata_label = if language == "zh-CN" {
        "报告元数据"
    } else {
        "Report metadata"
    };
    let profile_label = if language == "zh-CN" {
        "证据画像"
    } else {
        "Evidence profile"
    };
    let findings_label = if language == "zh-CN" {
        "核心发现"
    } else {
        "Key findings"
    };
    let sections_label = if language == "zh-CN" {
        "叙事章节"
    } else {
        "Story sections"
    };
    let reading_time_label = if language == "zh-CN" {
        "分钟阅读"
    } else {
        "Min read"
    };
    let thesis = if recovery && language == "zh-CN" {
        "本次证据链未达到完成门槛；页面仅保留可核验来源、失败边界与下一步。"
    } else if recovery {
        "This run did not meet the evidence gate; the page preserves only traceable sources, failure limits, and next actions."
    } else if qualified && language == "zh-CN" {
        "官方数据已经形成可用快照；独立来源本次未返回，因此证据等级保持为待交叉核验。"
    } else if qualified {
        "Official data produced a usable snapshot; the independent source did not return, so cross-verification remains pending."
    } else if language == "zh-CN" {
        "以可追溯证据为轴，分开展示核心结论、证据强度与尚未解决的边界。"
    } else {
        "A source-backed reading experience separating conclusions, evidence strength, and unresolved limits."
    };
    format!(
        r#"<!doctype html>
<html lang="{language}">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title}</title><style>{css}</style></head>
<body class="{theme}">
<header class="hero"><div class="hero-inner"><div class="hero-grid"><div><p class="eyebrow">{brief_label}</p><h1>{title}</h1><p class="hero-thesis">{thesis}</p><div class="signal-row"><span class="signal">● <b>{evidence_label}</b></span><span class="signal">{confidence_label}</span></div></div><aside class="evidence-profile" aria-label="{profile_label}"><p class="profile-label">{profile_label}</p><div class="profile-grid"><div><strong>{source_count:02}</strong><span>{sources_label}</span></div><div><strong>{finding_count:02}</strong><span>{findings_label}</span></div><div><strong>{reading_minutes:02}</strong><span>{reading_time_label}</span></div></div></aside></div></div></header>
<main><div class="report-shell"><aside class="rail" aria-label="{metadata_label}"><p class="rail-label">{reading_label}</p>{toc}<dl class="rail-stat"><dt>{sections_label}</dt><dd>{section_count:02}</dd></dl></aside><article id="report">{body}</article></div></main>
<p class="footer-note">{brief_label} · {evidence_label} · {sources_label}{sources_separator}{source_count}</p>
</body></html>
"#,
        language = language,
        title = html_escape(&title),
        css = REPORT_CSS,
        theme = body_class,
        brief_label = brief_label,
        thesis = thesis,
        evidence_label = evidence_label,
        confidence_label = confidence_label,
        profile_label = profile_label,
        source_count = source_count,
        sources_label = sources_label,
        finding_count = finding_count,
        findings_label = findings_label,
        reading_minutes = reading_minutes,
        reading_time_label = reading_time_label,
        metadata_label = metadata_label,
        reading_label = reading_label,
        toc = composition.toc,
        sections_label = sections_label,
        section_count = composition.section_count,
        body = composition.body,
        sources_separator = sources_separator,
    )
}

fn estimated_reading_minutes(markdown: &str, language: &str) -> usize {
    let units = if language == "zh-CN" {
        markdown
            .chars()
            .filter(|ch| !ch.is_whitespace() && !ch.is_ascii_punctuation())
            .count()
            .div_ceil(450)
    } else {
        markdown.split_whitespace().count().div_ceil(220)
    };
    units.max(1)
}

fn report_theme(_query: &str, _markdown: &str) -> &'static str {
    "theme-editorial"
}

fn unique_external_source_count(fragment: &str) -> usize {
    let mut sources = HashSet::new();
    let mut remaining = fragment;

    while let Some(start) = remaining.find("href=\"") {
        remaining = &remaining[start + "href=\"".len()..];
        let Some(end) = remaining.find('"') else {
            break;
        };
        let href = &remaining[..end];
        if href.starts_with("https://") || href.starts_with("http://") {
            sources.insert(href);
        }
        remaining = &remaining[end + 1..];
    }

    sources.len()
}

fn markdown_cited_source_count(markdown: &str) -> usize {
    let mut in_sources = false;
    let mut sources = HashSet::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("## ") {
            let heading = heading.trim();
            in_sources = heading.eq_ignore_ascii_case("sources")
                || heading.eq_ignore_ascii_case("references")
                || matches!(heading, "来源" | "资料来源" | "参考文献");
            continue;
        }
        if !in_sources || !trimmed.starts_with("- ") {
            continue;
        }
        let item = trimmed.trim_start_matches("- ").trim();
        let url = if let Some(start) = item.find("](") {
            let rest = &item[start + 2..];
            rest.find(')').map(|end| &rest[..end])
        } else {
            [item.find("https://"), item.find("http://")]
                .into_iter()
                .flatten()
                .min()
                .map(|start| {
                    item[start..]
                        .split(|ch: char| ch.is_whitespace() || matches!(ch, ')' | ']' | '>'))
                        .next()
                        .unwrap_or_default()
                        .trim_end_matches(['.', ',', ';', '。', '，', '；'])
                })
        };
        if let Some(url) =
            url.filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        {
            sources.insert(url.to_string());
        }
    }
    sources.len()
}

fn contains_cjk(value: &str) -> bool {
    value
        .chars()
        .any(|ch| ('\u{3400}'..='\u{9fff}').contains(&ch))
}

fn concise_report_title(value: &str) -> String {
    let mut clean = value.split_whitespace().collect::<Vec<_>>().join(" ");
    for prefix in ["请研究", "请分析"] {
        if let Some(rest) = clean.strip_prefix(prefix) {
            clean = rest
                .trim_start_matches(['：', ':', '，', ',', ' '])
                .to_string();
            break;
        }
    }
    for suffix in [
        "…研究报告",
        "研究报告",
        "Deep Research Report",
        "Research Report",
    ] {
        if let Some(rest) = clean.strip_suffix(suffix) {
            clean = rest
                .trim_end_matches(['：', ':', '，', ',', ' ', '…', '—', '–', '-'])
                .to_string();
            break;
        }
    }
    if contains_cjk(&clean) {
        if let Some((subject, _)) = clean.split_once('的') {
            let subject = subject.trim_end_matches(['：', ':', '，', ',', ' ', '…']);
            if (4..=36).contains(&subject.chars().count()) {
                return format!("{subject}研究");
            }
        }
    }
    let limit = if contains_cjk(&clean) { 40 } else { 92 };
    if clean.chars().count() <= limit {
        return clean;
    }
    let prefix = clean.chars().take(limit).collect::<String>();
    let shortened = prefix
        .rsplit_once(' ')
        .map(|(head, _)| head)
        .unwrap_or(&prefix);
    format!(
        "{}…",
        shortened.trim_end_matches([':', ';', ',', '，', '；', '：'])
    )
}

fn strip_first_h1(fragment: &str) -> String {
    let Some(start) = fragment.find("<h1>") else {
        return fragment.to_string();
    };
    let Some(relative_end) = fragment[start..].find("</h1>") else {
        return fragment.to_string();
    };
    let end = start + relative_end + "</h1>".len();
    format!("{}{}", &fragment[..start], &fragment[end..])
}

fn deep_research_markdown_report_title(markdown: &str, query: &str) -> String {
    markdown
        .lines()
        .find_map(|line| line.trim().strip_prefix("# "))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(markdown_backslash_unescape)
        .unwrap_or_else(|| markdown_backslash_unescape(&markdown_plain_text(query)))
}

fn deep_research_markdown_to_html_fragment(markdown: &str) -> String {
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.autolink = true;
    options.render.unsafe_ = false;
    options.render.escape = true;
    strip_relative_report_links(&markdown_to_html(markdown, &options))
}

fn strip_relative_report_links(fragment: &str) -> String {
    let mut output = String::with_capacity(fragment.len());
    let mut remaining = fragment;

    while let Some(anchor_start) = remaining.find("<a href=\"") {
        output.push_str(&remaining[..anchor_start]);
        let anchor = &remaining[anchor_start..];
        let href = &anchor["<a href=\"".len()..];
        let Some(href_end) = href.find('"') else {
            output.push_str(anchor);
            return output;
        };
        let href_value = &href[..href_end];
        let Some(open_end) = anchor.find('>') else {
            output.push_str(anchor);
            return output;
        };
        let allowed = href_value.is_empty()
            || href_value.starts_with('#')
            || href_value.starts_with("https://")
            || href_value.starts_with("http://")
            || href_value.starts_with("mailto:");
        if allowed {
            output.push_str(&anchor[..=open_end]);
            remaining = &anchor[open_end + 1..];
            continue;
        }

        let inner = &anchor[open_end + 1..];
        let Some(close_start) = inner.find("</a>") else {
            output.push_str(inner);
            return output;
        };
        output.push_str(&inner[..close_start]);
        remaining = &inner[close_start + "</a>".len()..];
    }

    output.push_str(remaining);
    output
}

#[cfg(test)]
#[path = "html_tests.rs"]
mod tests;
