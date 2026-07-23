use super::{html_escape, markdown_backslash_unescape, markdown_plain_text};
use crate::tui::deep_research_report_generation::{ReportHero, ReportPresentation};
use comrak::{markdown_to_html, Options};
use std::collections::HashSet;

#[path = "html_composition.rs"]
mod composition;
#[path = "html_style.rs"]
mod style;

use composition::compose_report_fragment;
use style::REPORT_CSS;

pub(super) fn deep_research_completed_report_html(query: &str, markdown: &str) -> String {
    deep_research_completed_report_html_with_presentation(query, markdown, None, None)
}

pub(super) fn deep_research_completed_report_html_with_presentation(
    query: &str,
    markdown: &str,
    presentation: Option<&ReportPresentation>,
    authored_thesis: Option<&str>,
) -> String {
    let lower_markdown = markdown.to_ascii_lowercase();
    let recovery = lower_markdown.contains("# deepresearch recovery report");
    let source_backed = markdown.contains("这是可核查的来源证据视图")
        || markdown.contains("This is a verifiable source-evidence view");
    let no_evidence = markdown.contains("本次检索没有获得可安全发布的来源文字")
        || markdown.contains("This retrieval obtained no source text that can be published safely");
    let degraded = recovery || source_backed || no_evidence;
    let title = concise_report_title(&deep_research_markdown_report_title(markdown, query));
    let labels = report_labels(query, degraded);
    let language = labels.language;
    let raw_body = mark_ineligible_source_evidence(&strip_first_h1(
        &deep_research_markdown_to_html_fragment(markdown),
    ));
    let source_count = unique_external_source_count(&raw_body);
    let section_plan = presentation
        .map(|presentation| presentation.section_plan.as_slice())
        .unwrap_or_default();
    let composition = compose_report_fragment(&raw_body, section_plan);
    let finding_count = composition
        .finding_count
        .max(markdown_declared_finding_count(markdown));
    let (content_count, content_label) = if finding_count > 0 {
        (finding_count, labels.findings)
    } else {
        (report_content_section_count(markdown), labels.sections)
    };
    let reading_minutes = estimated_reading_minutes(&raw_body);
    let theme = presentation
        .map(ReportPresentation::body_classes)
        .unwrap_or_else(|| ReportPresentation::default().body_classes());
    let body_class = if degraded {
        format!("{theme} report-degraded")
    } else {
        theme.to_string()
    };
    let thesis = authored_thesis
        .map(str::trim)
        .filter(|thesis| (12..=1_200).contains(&thesis.chars().count()))
        .unwrap_or(labels.fallback_thesis);
    let hero_support = match presentation.map(|value| value.hero) {
        Some(ReportHero::Statement) => String::new(),
        Some(ReportHero::Split) => composition.hero_guide.clone(),
        Some(ReportHero::Metrics) | None => format!(
            r#"<aside class="evidence-profile" aria-label="{profile_label}"><p class="profile-label">{profile_label}</p><div class="profile-grid"><div><strong>{source_count:02}</strong><span>{sources_label}</span></div><div><strong>{content_count:02}</strong><span>{content_label}</span></div><div><strong>{reading_minutes:02}</strong><span>{reading_time_label}</span></div></div></aside>"#,
            profile_label = labels.profile,
            sources_label = labels.sources,
            reading_time_label = labels.reading_time,
        ),
    };
    format!(
        r#"<!doctype html>
<html lang="{language}">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title}</title><style>:root{{--table-scroll-hint:'{table_scroll_hint}';}}{css}</style></head>
<body class="{theme}">
<header class="hero"><div class="hero-inner"><div class="hero-grid"><div><p class="eyebrow">{brief_label}</p><h1>{title}</h1><p class="hero-thesis">{thesis}</p><div class="signal-row"><span class="signal">● <b>{evidence_label}</b></span><span class="signal">{confidence_label}</span></div></div>{hero_support}</div></div></header>
<main><div class="report-shell"><aside class="rail" aria-label="{metadata_label}"><p class="rail-label">{reading_label}</p>{toc}</aside><article id="report">{body}</article></div></main>
<p class="footer-note">{brief_label} · {evidence_label}</p>
</body></html>
"#,
        language = language,
        title = html_escape(&title),
        css = REPORT_CSS,
        table_scroll_hint = labels.table_scroll_hint,
        theme = body_class,
        brief_label = labels.brief,
        thesis = html_escape(thesis),
        evidence_label = labels.evidence,
        confidence_label = labels.confidence,
        hero_support = hero_support,
        metadata_label = labels.metadata,
        reading_label = labels.reading,
        toc = composition.toc,
        body = composition.body,
    )
}

#[derive(Clone, Copy)]
struct ReportLabels {
    language: &'static str,
    brief: &'static str,
    evidence: &'static str,
    sources: &'static str,
    confidence: &'static str,
    reading: &'static str,
    metadata: &'static str,
    profile: &'static str,
    findings: &'static str,
    sections: &'static str,
    reading_time: &'static str,
    fallback_thesis: &'static str,
    table_scroll_hint: &'static str,
}

fn report_labels(query: &str, degraded: bool) -> ReportLabels {
    if query.chars().any(is_han_character) {
        ReportLabels {
            language: "zh-CN",
            brief: if degraded {
                "A3S 深度研究 · 已降级"
            } else {
                "A3S 深度研究"
            },
            evidence: if degraded {
                "证据不足 · 已降级"
            } else {
                "证据可追溯"
            },
            sources: "引用来源",
            confidence: if degraded {
                "非最终领域结论"
            } else {
                "已说明置信度与限制"
            },
            reading: "研究报告",
            metadata: "报告元数据",
            profile: "证据概况",
            findings: "关键发现",
            sections: "报告章节",
            reading_time: "分钟阅读",
            fallback_thesis: if degraded {
                "本次运行未达到证据门槛；页面仅保留可追溯来源、失败边界与后续行动。"
            } else {
                "一份区分结论、证据强度与未决限制的可追溯研究报告。"
            },
            table_scroll_hint: "← 横向滑动查看全部列 →",
        }
    } else {
        ReportLabels {
            language: "en",
            brief: if degraded {
                "A3S Deep Research · Degraded"
            } else {
                "A3S Deep Research"
            },
            evidence: if degraded {
                "Insufficient evidence · Degraded"
            } else {
                "Traceable evidence"
            },
            sources: "Cited sources",
            confidence: if degraded {
                "Not a final domain conclusion"
            } else {
                "Confidence & limits stated"
            },
            reading: "Research report",
            metadata: "Report metadata",
            profile: "Evidence profile",
            findings: "Key findings",
            sections: "Report sections",
            reading_time: "Min read",
            fallback_thesis: if degraded {
                "This run did not meet the evidence gate; the page preserves only traceable sources, failure limits, and next actions."
            } else {
                "A source-backed reading experience separating conclusions, evidence strength, and unresolved limits."
            },
            table_scroll_hint: "← swipe to inspect all columns →",
        }
    }
}

fn is_han_character(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F
    )
}

fn report_content_section_count(markdown: &str) -> usize {
    markdown
        .lines()
        .filter_map(|line| line.strip_prefix("## ").map(str::trim))
        .filter(|heading| {
            !heading.eq_ignore_ascii_case("sources")
                && !heading.eq_ignore_ascii_case("source ledger")
                && !matches!(*heading, "来源" | "参考来源")
        })
        .count()
        .max(1)
}

fn markdown_declared_finding_count(markdown: &str) -> usize {
    let mut in_findings = false;
    let mut count = 0usize;
    for line in markdown.lines().map(str::trim) {
        if let Some(heading) = line.strip_prefix("## ").map(str::trim) {
            in_findings = matches!(
                heading.to_ascii_lowercase().as_str(),
                "findings" | "key findings"
            ) || matches!(heading, "研究发现" | "核心发现" | "关键发现");
            continue;
        }
        if !in_findings {
            continue;
        }
        if line.starts_with("### ") || line.starts_with("- ") || line.starts_with("* ") {
            count = count.saturating_add(1);
        }
    }
    count
}

fn estimated_reading_minutes(fragment: &str) -> usize {
    let mut visible = String::with_capacity(fragment.len());
    let mut inside_tag = false;
    for character in fragment.chars() {
        match character {
            '<' => inside_tag = true,
            '>' => {
                inside_tag = false;
                visible.push(' ');
            }
            _ if !inside_tag => visible.push(character),
            _ => {}
        }
    }
    let han_count = visible
        .chars()
        .filter(|character| is_han_character(*character))
        .count();
    let word_count = visible
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count();
    han_count.div_ceil(500).max(word_count.div_ceil(220)).max(1)
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

fn concise_report_title(value: &str) -> String {
    let clean = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let limit = 96;
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

fn mark_ineligible_source_evidence(fragment: &str) -> String {
    [
        "<blockquote>\n<p><strong>证据资格：不可用于结论</strong>",
        "<blockquote>\n<p><strong>Claim eligibility: not eligible for conclusions</strong>",
    ]
    .into_iter()
    .fold(fragment.to_string(), |html, marker| {
        html.replace(
            marker,
            &marker.replacen(
                "<blockquote>",
                "<blockquote class=\"report-evidence-ineligible\">",
                1,
            ),
        )
    })
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
