#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ReportComposition {
    pub(super) body: String,
    pub(super) toc: String,
    pub(super) section_count: usize,
    pub(super) finding_count: usize,
}

pub(super) fn compose_report_fragment(fragment: &str, language: &str) -> ReportComposition {
    let Some(first_heading) = fragment.find("<h2>") else {
        return ReportComposition {
            body: format!(
                "<section class=\"report-section section--narrative\"><div class=\"section-body prose\">{fragment}</div></section>"
            ),
            ..ReportComposition::default()
        };
    };

    let mut body = String::new();
    let preamble = fragment[..first_heading].trim();
    if !preamble.is_empty() {
        body.push_str(&format!(
            "<section class=\"report-section section--lead\"><div class=\"section-body prose\">{preamble}</div></section>"
        ));
    }

    let mut toc = String::from("<nav class=\"toc\" aria-label=\"");
    toc.push_str(if language == "zh-CN" {
        "报告目录"
    } else {
        "Report contents"
    });
    toc.push_str("\">");

    let mut remaining = &fragment[first_heading..];
    let mut section_count = 0usize;
    let mut finding_count = 0usize;
    while let Some(heading_start) = remaining.find("<h2>") {
        let after_heading = &remaining[heading_start + "<h2>".len()..];
        let Some(heading_end) = after_heading.find("</h2>") else {
            body.push_str(remaining);
            break;
        };
        let heading_html = &after_heading[..heading_end];
        let content_start = heading_start + "<h2>".len() + heading_end + "</h2>".len();
        let after_current = &remaining[content_start..];
        let next_heading = after_current.find("<h2>").unwrap_or(after_current.len());
        let raw_content = after_current[..next_heading].trim();

        section_count += 1;
        let section_id = format!("section-{section_count}");
        let heading_text = decode_basic_entities(&strip_html_tags(heading_html));
        let kind = section_kind(&heading_text);
        let (content, section_findings) = if kind == "findings" {
            compose_findings(raw_content)
        } else {
            (wrap_tables(raw_content, &heading_text), 0)
        };
        finding_count = finding_count.saturating_add(section_findings);

        toc.push_str(&format!(
            "<a href=\"#{section_id}\"><span>{section_count:02}</span>{}</a>",
            escape_attribute(&heading_text)
        ));
        body.push_str(&format!(
            "<section id=\"{section_id}\" class=\"report-section section--{kind}\"><div class=\"section-index\">{section_count:02}</div><h2>{heading_html}</h2><div class=\"section-body\">{content}</div></section>"
        ));

        remaining = &after_current[next_heading..];
        if remaining.is_empty() {
            break;
        }
    }
    toc.push_str("</nav>");

    ReportComposition {
        body,
        toc,
        section_count,
        finding_count,
    }
}

fn compose_findings(content: &str) -> (String, usize) {
    let Some(first_heading) = content.find("<h3>") else {
        return (wrap_tables(content, "Key findings"), 0);
    };
    let lead = content[..first_heading].trim();
    let mut output = String::new();
    if !lead.is_empty() {
        output.push_str(&format!("<div class=\"findings-lead prose\">{lead}</div>"));
    }
    output.push_str("<div class=\"findings-list\">");

    let mut remaining = &content[first_heading..];
    let mut count = 0usize;
    while let Some(heading_start) = remaining.find("<h3>") {
        let after_heading = &remaining[heading_start + "<h3>".len()..];
        let Some(heading_end) = after_heading.find("</h3>") else {
            output.push_str(remaining);
            break;
        };
        let heading = &after_heading[..heading_end];
        let content_start = heading_start + "<h3>".len() + heading_end + "</h3>".len();
        let after_current = &remaining[content_start..];
        let next_heading = after_current.find("<h3>").unwrap_or(after_current.len());
        let detail = wrap_tables(
            after_current[..next_heading].trim(),
            &strip_html_tags(heading),
        );
        count += 1;
        output.push_str(&format!(
            "<article class=\"finding\"><div class=\"finding-number\">{count:02}</div><div class=\"finding-content\"><h3>{heading}</h3>{detail}</div></article>"
        ));
        remaining = &after_current[next_heading..];
        if remaining.is_empty() {
            break;
        }
    }
    output.push_str("</div>");
    (output, count)
}

fn wrap_tables(content: &str, label: &str) -> String {
    if !content.contains("<table>") {
        return content.to_string();
    }
    content
        .replace(
            "<table>",
            &format!(
                "<div class=\"table-wrap\" role=\"region\" aria-label=\"{}\" tabindex=\"0\"><table>",
                escape_attribute(label)
            ),
        )
        .replace("</table>", "</table></div>")
}

fn section_kind(heading: &str) -> &'static str {
    let lower = heading.to_ascii_lowercase();
    if lower.contains("summary") || heading.contains("摘要") {
        "summary"
    } else if lower.contains("finding") || heading.contains("发现") {
        "findings"
    } else if lower.contains("matrix") || heading.contains("矩阵") {
        "matrix"
    } else if lower.contains("caveat")
        || lower.contains("gap")
        || lower.contains("limit")
        || heading.contains("局限")
        || heading.contains("注意")
    {
        "caveats"
    } else if lower.contains("confidence")
        || lower.contains("quality")
        || heading.contains("置信")
        || heading.contains("质量")
    {
        "confidence"
    } else if lower.contains("source")
        || lower.contains("reference")
        || heading.contains("来源")
        || heading.contains("参考")
    {
        "sources"
    } else {
        "narrative"
    }
}

fn strip_html_tags(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut in_tag = false;
    for ch in value.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    output
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn decode_basic_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_sections_gain_distinct_compositions_and_navigation() {
        let fragment = "<h2>Executive Summary</h2><ul><li>One</li><li>Two</li></ul><h2>Key Findings</h2><h3>First</h3><p>Detail.</p><h3>Second</h3><p>Detail.</p><h2>Evidence Matrix</h2><table><tr><td>A</td></tr></table><h2>Sources</h2><ul><li>Source</li></ul>";
        let composition = compose_report_fragment(fragment, "en");

        assert_eq!(composition.section_count, 4);
        assert_eq!(composition.finding_count, 2);
        assert!(composition.body.contains("section--summary"));
        assert!(composition.body.contains("class=\"finding\""));
        assert!(composition.body.contains("class=\"table-wrap\""));
        assert!(composition.body.contains("section--sources"));
        assert!(composition.toc.contains("href=\"#section-4\""));
    }
}
