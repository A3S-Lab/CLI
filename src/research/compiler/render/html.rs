use super::shared::RenderContext;
use crate::research::compiler::{
    ClaimKind, ReportClaim, ReportDimension, ReportDocumentKind, ReportRelation,
};

pub(super) fn render(context: &RenderContext<'_>) -> String {
    let document = context.document;
    let labels = context.labels;
    let mut output = String::new();
    output.push_str("<!doctype html>\n");
    output.push_str(&format!("<html lang=\"{}\">\n<head>\n", labels.lang));
    output.push_str("<meta charset=\"utf-8\">\n");
    output.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    output.push_str(&format!(
        "<title>{}</title>\n<style>{}</style>\n</head>\n<body>\n",
        escape_html(&document.title),
        REPORT_CSS
    ));
    output.push_str("<a class=\"skip-link\" href=\"#report-main\">Skip to report</a>\n");
    output.push_str("<div class=\"report-shell\">\n<header class=\"report-hero\">\n");
    if document.kind == ReportDocumentKind::SourceBacked {
        output.push_str(&format!(
            "<p class=\"eyebrow\">{}</p>\n",
            escape_html(labels.source_backed)
        ));
    } else if document.kind == ReportDocumentKind::NoEvidence {
        output.push_str(&format!(
            "<p class=\"eyebrow\">{}</p>\n",
            escape_html(labels.no_evidence)
        ));
    }
    output.push_str(&format!("<h1>{}</h1>\n", escape_html(&document.title)));
    output.push_str("</header>\n");
    output.push_str(&format!(
        "<nav aria-label=\"{}\" class=\"report-nav\">\n",
        escape_attribute(labels.report_sections)
    ));
    if !document.direct_answer_claims.is_empty() {
        output.push_str(&format!(
            "<a href=\"#direct-answer\">{}</a>\n",
            escape_html(labels.direct_answer)
        ));
    }
    for (index, dimension) in document.dimensions.iter().enumerate() {
        output.push_str(&format!(
            "<a href=\"#dimension-{}\">{}</a>\n",
            index + 1,
            escape_html(&dimension.heading)
        ));
    }
    output.push_str(&format!(
        "<a href=\"#sources\">{}</a>\n</nav>\n",
        escape_html(labels.sources)
    ));
    output.push_str("<main id=\"report-main\">\n");
    if !document.direct_answer_claims.is_empty() {
        output.push_str("<section id=\"direct-answer\" class=\"report-section direct-answer\">\n");
        output.push_str(&format!("<h2>{}</h2>\n", escape_html(labels.direct_answer)));
        for claim in &document.direct_answer_claims {
            render_claim(&mut output, context, claim);
        }
        output.push_str("</section>\n");
    }
    for (index, dimension) in document.dimensions.iter().enumerate() {
        render_dimension(&mut output, context, dimension, index + 1);
    }
    output.push_str("<section id=\"sources\" class=\"report-section sources\">\n");
    output.push_str(&format!("<h2>{}</h2>\n<ol>\n", escape_html(labels.sources)));
    for source in &document.source_ledger {
        output.push_str(&format!(
            "<li id=\"source-{}\"><strong>[{}]</strong> ",
            source.number, source.number
        ));
        if safe_https_anchor(&source.canonical_anchor) {
            output.push_str(&format!(
                "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                escape_attribute(&source.canonical_anchor),
                escape_html(&source.title)
            ));
        } else {
            output.push_str(&format!(
                "<strong>{}</strong> <code>{}</code>",
                escape_html(&source.title),
                escape_html(&source.canonical_anchor)
            ));
        }
        output.push_str(&format!(
            "<span class=\"source-meta\">{}: {}</span>",
            escape_html(labels.captured),
            escape_html(&source.captured_at)
        ));
        if source.requested_anchor != source.canonical_anchor {
            output.push_str(&format!(
                "<span class=\"source-meta\">{}: ",
                escape_html(labels.requested_as)
            ));
            if safe_https_anchor(&source.requested_anchor) {
                output.push_str(&format!(
                    "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                    escape_attribute(&source.requested_anchor),
                    escape_html(&source.requested_anchor)
                ));
            } else {
                output.push_str(&format!(
                    "<code>{}</code>",
                    escape_html(&source.requested_anchor)
                ));
            }
            output.push_str("</span>");
        }
        output.push_str("</li>\n");
    }
    output.push_str("</ol>\n</section>\n</main>\n</div>\n</body>\n</html>\n");
    output
}

fn render_dimension(
    output: &mut String,
    context: &RenderContext<'_>,
    dimension: &ReportDimension,
    ordinal: usize,
) {
    let labels = context.labels;
    output.push_str(&format!(
        "<section id=\"dimension-{ordinal}\" class=\"report-section dimension\">\n<h2>{}</h2>\n",
        escape_html(&dimension.heading)
    ));
    output.push_str(&format!(
        "<p class=\"coverage\"><span>{}</span>{}</p>\n",
        escape_html(labels.status),
        escape_html(context.coverage_label(dimension.coverage))
    ));
    if !dimension.claims.is_empty() {
        output.push_str(&format!("<h3>{}</h3>\n", escape_html(labels.findings)));
        for claim in &dimension.claims {
            render_claim(output, context, claim);
        }
    }
    if !dimension.relations.is_empty() {
        output.push_str(&format!(
            "<div class=\"relations\"><h3>{}</h3>\n",
            escape_html(labels.contradiction)
        ));
        for relation in &dimension.relations {
            render_relation(output, context, relation);
        }
        output.push_str("</div>\n");
    }
    if !dimension.gaps.is_empty() {
        output.push_str(&format!(
            "<aside class=\"limitations\"><h3>{}</h3><ul>\n",
            escape_html(labels.limitations)
        ));
        for gap in &dimension.gaps {
            output.push_str(&format!("<li>{}</li>\n", escape_html(&gap.text)));
        }
        output.push_str("</ul></aside>\n");
    }
    if context.document.kind == ReportDocumentKind::SourceBacked && !dimension.source_ids.is_empty()
    {
        output.push_str(&format!(
            "<div class=\"retained-excerpts\"><h3>{}</h3>\n",
            escape_html(labels.retained_excerpts)
        ));
        for source_id in &dimension.source_ids {
            let Some(source) = context.source(source_id) else {
                continue;
            };
            output.push_str(&format!(
                "<article class=\"source-excerpt\"><h4><a href=\"#source-{}\">[{}]</a> {}</h4>\n",
                source.number,
                source.number,
                escape_html(&source.title)
            ));
            for chunk in &source.chunks {
                output.push_str(&format!(
                    "<pre><code>{}</code></pre>\n",
                    escape_html(&chunk.text)
                ));
            }
            output.push_str("</article>\n");
        }
        output.push_str("</div>\n");
    }
    output.push_str("</section>\n");
}

fn render_claim(output: &mut String, context: &RenderContext<'_>, claim: &ReportClaim) {
    let number = context
        .claim_number(&claim.id)
        .expect("every report claim has a presentation number");
    let kind_class = match claim.kind {
        ClaimKind::Fact => "fact",
        ClaimKind::Inference => "inference",
        ClaimKind::Recommendation => "recommendation",
    };
    output.push_str(&format!(
        "<article id=\"claim-{number}\" class=\"claim {kind_class}\"><p>"
    ));
    match claim.kind {
        ClaimKind::Fact => {}
        ClaimKind::Inference => output.push_str(&format!(
            "<strong>{}:</strong> ",
            escape_html(context.labels.inference)
        )),
        ClaimKind::Recommendation => output.push_str(&format!(
            "<strong>{}:</strong> ",
            escape_html(context.labels.recommendation)
        )),
    }
    output.push_str(&escape_html(&claim.text));
    for citation in &claim.citation_numbers {
        output.push_str(&format!(
            " <a class=\"citation\" href=\"#source-{citation}\">[{citation}]</a>"
        ));
    }
    output.push_str("</p>\n");
    let basis = claim
        .basis_claim_ids
        .iter()
        .filter_map(|claim_id| context.claim_number(claim_id))
        .collect::<Vec<_>>();
    if !basis.is_empty() {
        output.push_str(&format!(
            "<p class=\"basis\"><strong>{}:</strong> ",
            escape_html(context.labels.basis)
        ));
        for (index, basis_number) in basis.iter().enumerate() {
            if index > 0 {
                output.push_str(", ");
            }
            output.push_str(&format!(
                "<a href=\"#claim-{basis_number}\">{} {basis_number}</a>",
                escape_html(context.labels.finding)
            ));
        }
        output.push_str("</p>\n");
    }
    if let Some(derivation) = &claim.derivation {
        output.push_str(&format!(
            "<p class=\"derivation\"><strong>{}:</strong> {}</p>\n",
            escape_html(context.labels.derivation),
            escape_html(&derivation.method)
        ));
    }
    output.push_str("</article>\n");
}

fn render_relation(output: &mut String, context: &RenderContext<'_>, relation: &ReportRelation) {
    let references = relation
        .claim_ids
        .iter()
        .filter_map(|claim_id| context.claim_number(claim_id))
        .collect::<Vec<_>>();
    if references.len() == 2 {
        output.push_str(&format!(
            "<p class=\"contradiction\"><strong>{}:</strong> <a href=\"#claim-{}\">{} {}</a> / <a href=\"#claim-{}\">{} {}</a>.</p>\n",
            escape_html(context.labels.contradiction),
            references[0],
            escape_html(context.labels.finding),
            references[0],
            references[1],
            escape_html(context.labels.finding),
            references[1]
        ));
    }
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn escape_attribute(value: &str) -> String {
    escape_html(value)
}

fn safe_https_anchor(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

const REPORT_CSS: &str = r#"
:root {
  color-scheme: light;
  --paper: #f7f5ef;
  --surface: #ffffff;
  --ink: #17211d;
  --muted: #5d6a63;
  --line: #d9ded9;
  --accent: #176b55;
  --accent-soft: #e6f2ed;
  --warning: #8a4b08;
  font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body {
  margin: 0;
  color: var(--ink);
  background: var(--paper);
  line-height: 1.65;
  overflow-wrap: anywhere;
}
a { color: var(--accent); text-underline-offset: 0.18em; }
a:focus-visible { outline: 3px solid #f3b34c; outline-offset: 3px; }
.skip-link { position: fixed; left: 1rem; top: -5rem; z-index: 10; padding: .7rem 1rem; background: var(--ink); color: white; }
.skip-link:focus { top: 1rem; }
.report-shell { width: min(1120px, calc(100% - 3rem)); margin: 0 auto; padding: 3rem 0 5rem; }
.report-hero { padding: clamp(2rem, 5vw, 4.5rem); color: white; background: linear-gradient(135deg, #173e34, #176b55); border-radius: 1.4rem; box-shadow: 0 24px 60px rgba(20, 48, 40, .16); }
.report-hero h1 { max-width: 18ch; margin: .2rem 0 0; font-family: ui-serif, Georgia, serif; font-size: clamp(2.2rem, 6vw, 4.8rem); line-height: 1.02; letter-spacing: -.035em; }
.eyebrow { margin: 0; font-size: .78rem; font-weight: 750; letter-spacing: .12em; text-transform: uppercase; opacity: .8; }
.report-nav { position: sticky; top: 0; z-index: 4; display: flex; gap: .7rem; margin: 1.2rem 0 2rem; padding: .8rem; overflow-x: auto; background: rgba(247, 245, 239, .94); border-bottom: 1px solid var(--line); backdrop-filter: blur(12px); }
.report-nav a { flex: 0 0 auto; padding: .45rem .7rem; color: var(--ink); text-decoration: none; border-radius: .5rem; }
.report-nav a:hover { background: var(--accent-soft); }
main { display: grid; gap: 1.2rem; }
.report-section { padding: clamp(1.35rem, 3vw, 2.4rem); background: var(--surface); border: 1px solid var(--line); border-radius: 1rem; box-shadow: 0 10px 30px rgba(28, 45, 38, .05); }
.report-section h2 { margin: 0 0 1rem; font-family: ui-serif, Georgia, serif; font-size: clamp(1.55rem, 3vw, 2.25rem); line-height: 1.2; }
.report-section h3 { margin-top: 1.7rem; font-size: 1rem; letter-spacing: .02em; }
.coverage { display: flex; gap: .7rem; align-items: baseline; color: var(--muted); }
.coverage span { padding: .18rem .45rem; color: var(--accent); background: var(--accent-soft); border-radius: .35rem; font-size: .75rem; font-weight: 750; text-transform: uppercase; letter-spacing: .04em; }
.claim { margin: .8rem 0; padding: .9rem 1rem; border-left: 4px solid var(--accent); background: #fbfcfa; border-radius: 0 .65rem .65rem 0; }
.claim p { margin: 0; }
.claim .basis, .claim .derivation { margin-top: .45rem; color: var(--muted); font-size: .92rem; }
.claim.recommendation { border-left-color: #6e53a5; background: #faf8ff; }
.citation { display: inline-flex; min-width: 1.7rem; justify-content: center; font-size: .82em; font-weight: 750; text-decoration: none; }
.limitations, .relations { margin-top: 1.2rem; padding: .9rem 1rem; border-radius: .7rem; background: #fff8ed; color: #59350d; }
.limitations h3, .relations h3 { margin-top: 0; color: var(--warning); }
.source-excerpt { margin: 1rem 0; }
.source-excerpt h4 { margin-bottom: .5rem; }
pre { max-width: 100%; margin: 0; padding: 1rem; overflow: auto; white-space: pre-wrap; background: #14211d; color: #eef6f2; border-radius: .65rem; }
code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
.sources ol { padding-left: 1.4rem; }
.sources li { margin: .85rem 0; }
.source-meta { display: block; color: var(--muted); font-size: .88rem; }
@media (max-width: 640px) {
  .report-shell { width: min(100% - 1rem, 1120px); padding-top: .5rem; }
  .report-hero { padding: 1.5rem; border-radius: .9rem; }
  .report-nav { margin-top: .5rem; }
  .report-section { padding: 1.1rem; border-radius: .8rem; }
  .coverage { align-items: flex-start; flex-direction: column; gap: .35rem; }
}
@media print {
  body { background: white; color: black; }
  .report-shell { width: 100%; padding: 0; }
  .report-hero { padding: 0 0 1rem; color: black; background: none; box-shadow: none; }
  .report-nav, .skip-link { display: none; }
  .report-section { break-inside: avoid; border: 0; box-shadow: none; padding: 1rem 0; }
  a { color: black; text-decoration: none; }
}
"#;
