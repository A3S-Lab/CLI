use crate::tui::deep_research_report_generation::{
    ReportSectionComposition, ReportSectionRhythm, ReportSectionTreatment,
};

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ReportComposition {
    pub(super) body: String,
    pub(super) toc: String,
    pub(super) hero_guide: String,
    pub(super) finding_count: usize,
}

pub(super) fn compose_report_fragment(
    fragment: &str,
    section_plan: &[ReportSectionTreatment],
) -> ReportComposition {
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

    let mut toc = String::from("<nav class=\"toc\" aria-label=\"Report contents\">");

    let mut remaining = &fragment[first_heading..];
    let mut section_count = 0usize;
    let mut finding_count = 0usize;
    let mut hero_links = String::new();
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
        let treatment = matching_section_treatment(&heading_text, section_plan);
        let composition = treatment
            .map(|treatment| treatment.composition)
            .unwrap_or(ReportSectionComposition::Prose);
        let rhythm = treatment
            .map(|treatment| treatment.rhythm)
            .unwrap_or_else(|| default_section_rhythm(composition));
        let kind = section_kind(composition);
        let (content, section_findings) =
            compose_section_content(raw_content, &heading_text, composition);
        finding_count = finding_count.saturating_add(section_findings);

        toc.push_str(&format!(
            "<a href=\"#{section_id}\"><span>{section_count:02}</span>{}</a>",
            escape_attribute(&heading_text)
        ));
        if section_count <= 4 {
            hero_links.push_str(&format!(
                "<li><a href=\"#{section_id}\"><span>{section_count:02}</span><b>{}</b></a></li>",
                escape_attribute(&heading_text)
            ));
        }
        body.push_str(&format!(
            "<section id=\"{section_id}\" class=\"report-section section--{kind} {} {}\"><div class=\"section-index\">{section_count:02}</div><h2>{heading_html}</h2><div class=\"section-body\">{content}</div></section>",
            rhythm.class_name(),
            composition.class_name(),
        ));

        remaining = &after_current[next_heading..];
        if remaining.is_empty() {
            break;
        }
    }
    toc.push_str("</nav>");
    let hero_guide = if hero_links.is_empty() {
        String::new()
    } else {
        let label = "Reading path";
        format!(
            "<aside class=\"hero-map\" aria-label=\"{label}\"><p class=\"profile-label\">{label}</p><ol>{hero_links}</ol></aside>"
        )
    };

    ReportComposition {
        body,
        toc,
        hero_guide,
        finding_count,
    }
}

fn compose_section_content(
    content: &str,
    heading: &str,
    composition: ReportSectionComposition,
) -> (String, usize) {
    match composition {
        ReportSectionComposition::KeyPoints => compose_subheaded_blocks(
            content,
            heading,
            "key-points-list",
            "key-point",
            "key-point-number",
        ),
        ReportSectionComposition::Timeline => {
            let (content, _) = compose_subheaded_blocks(
                content,
                heading,
                "timeline-list",
                "timeline-entry",
                "timeline-marker",
            );
            (content, 0)
        }
        ReportSectionComposition::Process => {
            let (content, _) = compose_subheaded_blocks(
                content,
                heading,
                "process-list",
                "process-step",
                "process-number",
            );
            (content, 0)
        }
        _ => (wrap_tables(content, heading), 0),
    }
}

fn compose_subheaded_blocks(
    content: &str,
    label: &str,
    list_class: &str,
    item_class: &str,
    number_class: &str,
) -> (String, usize) {
    let Some(first_heading) = content.find("<h3>") else {
        return (wrap_tables(content, label), 0);
    };
    let lead = content[..first_heading].trim();
    let mut output = String::new();
    if !lead.is_empty() {
        output.push_str(&format!(
            "<div class=\"composition-lead prose\">{lead}</div>"
        ));
    }
    output.push_str(&format!("<div class=\"{list_class}\">"));

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
            "<article class=\"{item_class}\"><div class=\"{number_class}\">{count:02}</div><div class=\"composition-content\"><h3>{heading}</h3>{detail}</div></article>"
        ));
        remaining = &after_current[next_heading..];
        if remaining.is_empty() {
            break;
        }
    }
    output.push_str("</div>");
    (output, count)
}

fn matching_section_treatment<'a>(
    heading: &str,
    section_plan: &'a [ReportSectionTreatment],
) -> Option<&'a ReportSectionTreatment> {
    section_plan
        .iter()
        .find(|treatment| treatment.heading.trim() == heading.trim())
}

fn default_section_rhythm(composition: ReportSectionComposition) -> ReportSectionRhythm {
    match composition {
        ReportSectionComposition::Comparison
        | ReportSectionComposition::KeyPoints
        | ReportSectionComposition::Evidence
        | ReportSectionComposition::SourceLedger => ReportSectionRhythm::Dense,
        _ => ReportSectionRhythm::Breathing,
    }
}

fn section_kind(composition: ReportSectionComposition) -> &'static str {
    match composition {
        ReportSectionComposition::KeyPoints => "findings",
        ReportSectionComposition::Comparison => "matrix",
        ReportSectionComposition::Evidence => "evidence",
        ReportSectionComposition::SourceLedger => "sources",
        ReportSectionComposition::Prose
        | ReportSectionComposition::Timeline
        | ReportSectionComposition::Process => "narrative",
    }
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
    fn report_sections_default_to_neutral_composition_without_heading_classification() {
        let fragment = "<h2>Executive Summary</h2><ul><li>One</li><li>Two</li></ul><h2>Key Findings</h2><h3>First</h3><p>Detail.</p><h3>Second</h3><p>Detail.</p><h2>Evidence Matrix</h2><table><tr><td>A</td></tr></table><h2>Sources</h2><ul><li>Source</li></ul>";
        let composition = compose_report_fragment(fragment, &[]);

        assert_eq!(composition.finding_count, 0);
        assert_eq!(composition.body.matches("section--narrative").count(), 4);
        assert!(!composition.body.contains("class=\"key-point\""));
        assert!(composition.body.contains("class=\"table-wrap\""));
        assert!(composition.toc.contains("href=\"#section-4\""));
        assert!(composition.hero_guide.contains("Reading path"));
    }

    #[test]
    fn section_plan_controls_rhythm_and_semantic_composition_without_heading_keywords() {
        let fragment = "<h2>What changed</h2><h3>Before</h3><p>Old state.</p><h3>After</h3><p>New state.</p><h2>What to do next</h2><ol><li>Act.</li></ol>";
        let plan = [
            ReportSectionTreatment {
                heading: "What changed".to_string(),
                rhythm: ReportSectionRhythm::Anchor,
                composition: ReportSectionComposition::Timeline,
            },
            ReportSectionTreatment {
                heading: "What to do next".to_string(),
                rhythm: ReportSectionRhythm::Dense,
                composition: ReportSectionComposition::Process,
            },
        ];

        let composition = compose_report_fragment(fragment, &plan);

        assert!(composition
            .body
            .contains("rhythm-anchor composition-timeline"));
        assert!(composition.body.contains("class=\"timeline-entry\""));
        assert!(composition
            .body
            .contains("rhythm-dense composition-process"));
    }
}
