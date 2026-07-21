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
    let title = concise_report_title(&deep_research_markdown_report_title(markdown, query));
    let language = "und";
    let raw_body = strip_first_h1(&deep_research_markdown_to_html_fragment(markdown));
    let source_count = unique_external_source_count(&raw_body);
    let section_plan = presentation
        .map(|presentation| presentation.section_plan.as_slice())
        .unwrap_or_default();
    let composition = compose_report_fragment(&raw_body, section_plan);
    let finding_count = composition.finding_count.max(
        markdown
            .lines()
            .filter(|line| line.starts_with("### "))
            .count(),
    );
    let reading_minutes = estimated_reading_minutes(markdown);
    let theme = presentation
        .map(ReportPresentation::body_classes)
        .unwrap_or_else(|| ReportPresentation::default().body_classes());
    let body_class = if recovery {
        format!("{theme} report-degraded")
    } else {
        theme.to_string()
    };
    let evidence_label = if recovery {
        "Insufficient evidence · Degraded"
    } else {
        "Traceable evidence"
    };
    let sources_label = "Cited sources";
    let confidence_label = if recovery {
        "Not a final domain conclusion"
    } else {
        "Confidence & limits stated"
    };
    let brief_label = if recovery {
        "A3S Deep Research · Degraded"
    } else {
        "A3S Deep Research"
    };
    let reading_label = "Research report";
    let metadata_label = "Report metadata";
    let profile_label = "Evidence profile";
    let findings_label = "Key findings";
    let reading_time_label = "Min read";
    let fallback_thesis = if recovery {
        "This run did not meet the evidence gate; the page preserves only traceable sources, failure limits, and next actions."
    } else {
        "A source-backed reading experience separating conclusions, evidence strength, and unresolved limits."
    };
    let thesis = authored_thesis
        .map(str::trim)
        .filter(|thesis| (12..=1_200).contains(&thesis.chars().count()))
        .unwrap_or(fallback_thesis);
    let hero_support = match presentation.map(|value| value.hero) {
        Some(ReportHero::Statement) => String::new(),
        Some(ReportHero::Split) => composition.hero_guide.clone(),
        Some(ReportHero::Metrics) | None => format!(
            r#"<aside class="evidence-profile" aria-label="{profile_label}"><p class="profile-label">{profile_label}</p><div class="profile-grid"><div><strong>{source_count:02}</strong><span>{sources_label}</span></div><div><strong>{finding_count:02}</strong><span>{findings_label}</span></div><div><strong>{reading_minutes:02}</strong><span>{reading_time_label}</span></div></div></aside>"#
        ),
    };
    format!(
        r#"<!doctype html>
<html lang="{language}">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title}</title><style>{css}</style></head>
<body class="{theme}">
<header class="hero"><div class="hero-inner"><div class="hero-grid"><div><p class="eyebrow">{brief_label}</p><h1>{title}</h1><p class="hero-thesis">{thesis}</p><div class="signal-row"><span class="signal">● <b>{evidence_label}</b></span><span class="signal">{confidence_label}</span></div></div>{hero_support}</div></div></header>
<main><div class="report-shell"><aside class="rail" aria-label="{metadata_label}"><p class="rail-label">{reading_label}</p>{toc}</aside><article id="report">{body}</article></div></main>
<p class="footer-note">{brief_label} · {evidence_label}</p>
</body></html>
"#,
        language = language,
        title = html_escape(&title),
        css = REPORT_CSS,
        theme = body_class,
        brief_label = brief_label,
        thesis = html_escape(thesis),
        evidence_label = evidence_label,
        confidence_label = confidence_label,
        hero_support = hero_support,
        metadata_label = metadata_label,
        reading_label = reading_label,
        toc = composition.toc,
        body = composition.body,
    )
}

fn estimated_reading_minutes(markdown: &str) -> usize {
    markdown
        .chars()
        .filter(|ch| !ch.is_whitespace() && !ch.is_control())
        .count()
        .div_ceil(900)
        .max(1)
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
