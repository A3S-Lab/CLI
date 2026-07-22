use super::shared::RenderContext;
use crate::research::compiler::{
    ClaimKind, ReportClaim, ReportDimension, ReportDocumentKind, ReportRelation,
};

pub(super) fn render(context: &RenderContext<'_>) -> String {
    let document = context.document;
    let labels = context.labels;
    let mut output = String::new();
    push_line(
        &mut output,
        &format!("# {}", escape_inline(&document.title)),
    );
    push_line(&mut output, "");
    if document.kind == ReportDocumentKind::SourceBacked {
        push_line(
            &mut output,
            &format!("*{}*", escape_inline(labels.source_backed)),
        );
        push_line(&mut output, "");
    } else if document.kind == ReportDocumentKind::NoEvidence {
        push_line(
            &mut output,
            &format!("*{}*", escape_inline(labels.no_evidence)),
        );
        push_line(&mut output, "");
    }

    if !document.direct_answer_claims.is_empty() {
        push_line(
            &mut output,
            &format!("## {}", escape_inline(labels.direct_answer)),
        );
        push_line(&mut output, "");
        for claim in &document.direct_answer_claims {
            render_claim(&mut output, context, claim);
        }
    }

    push_line(
        &mut output,
        &format!("## {}", escape_inline(labels.research_dimensions)),
    );
    push_line(&mut output, "");
    for dimension in &document.dimensions {
        render_dimension(&mut output, context, dimension);
    }

    push_line(
        &mut output,
        &format!("## {}", escape_inline(labels.sources)),
    );
    push_line(&mut output, "");
    for source in &document.source_ledger {
        push_line(
            &mut output,
            &format!("<a id=\"source-{}\"></a>", source.number),
        );
        let title = escape_inline(&source.title);
        let identity = if safe_https_anchor(&source.canonical_anchor) {
            format!("[{title}](<{}>)", source.canonical_anchor)
        } else {
            format!("{title} — `{}`", escape_code(&source.canonical_anchor))
        };
        push_line(
            &mut output,
            &format!(
                "- **[{}]** {} — {}: {}.",
                source.number,
                identity,
                labels.captured,
                escape_inline(&source.captured_at)
            ),
        );
        if source.requested_anchor != source.canonical_anchor {
            let requested = if safe_https_anchor(&source.requested_anchor) {
                format!("<{}>", source.requested_anchor)
            } else {
                format!("`{}`", escape_code(&source.requested_anchor))
            };
            push_line(
                &mut output,
                &format!("   - {}: {}", labels.requested_as, requested),
            );
        }
    }
    output
}

fn render_dimension(output: &mut String, context: &RenderContext<'_>, dimension: &ReportDimension) {
    let labels = context.labels;
    push_line(
        output,
        &format!("### {}", escape_inline(&dimension.heading)),
    );
    push_line(output, "");
    push_line(
        output,
        &format!(
            "**{}:** {}",
            labels.status,
            escape_inline(context.coverage_label(dimension.coverage))
        ),
    );
    push_line(output, "");
    if !dimension.claims.is_empty() {
        push_line(output, &format!("#### {}", escape_inline(labels.findings)));
        push_line(output, "");
        for claim in &dimension.claims {
            render_claim(output, context, claim);
        }
    }
    if !dimension.relations.is_empty() {
        push_line(
            output,
            &format!("#### {}", escape_inline(labels.contradiction)),
        );
        push_line(output, "");
        for relation in &dimension.relations {
            render_relation(output, context, relation);
        }
        push_line(output, "");
    }
    if !dimension.gaps.is_empty() {
        push_line(
            output,
            &format!("#### {}", escape_inline(labels.limitations)),
        );
        push_line(output, "");
        for gap in &dimension.gaps {
            push_line(output, &format!("- {}", escape_inline(&gap.text)));
        }
        push_line(output, "");
    }
    if context.document.kind == ReportDocumentKind::SourceBacked && !dimension.source_ids.is_empty()
    {
        push_line(
            output,
            &format!("#### {}", escape_inline(labels.retained_excerpts)),
        );
        push_line(output, "");
        for source_id in &dimension.source_ids {
            let Some(source) = context.source(source_id) else {
                continue;
            };
            push_line(
                output,
                &format!("**[{}] {}**", source.number, escape_inline(&source.title)),
            );
            push_line(output, "");
            for chunk in &source.chunks {
                for line in chunk.text.lines() {
                    push_line(output, &format!("    {line}"));
                }
                push_line(output, "");
            }
        }
    }
}

fn render_claim(output: &mut String, context: &RenderContext<'_>, claim: &ReportClaim) {
    let number = context
        .claim_number(&claim.id)
        .expect("every report claim has a presentation number");
    push_line(output, &format!("<a id=\"claim-{number}\"></a>"));
    let prefix = match claim.kind {
        ClaimKind::Fact => String::new(),
        ClaimKind::Inference => format!("**{}:** ", context.labels.inference),
        ClaimKind::Recommendation => format!("**{}:** ", context.labels.recommendation),
    };
    let citations = claim
        .citation_numbers
        .iter()
        .map(|number| format!("[{number}](#source-{number})"))
        .collect::<Vec<_>>()
        .join(" ");
    push_line(
        output,
        &format!("- {prefix}{} {citations}", escape_inline(&claim.text)),
    );
    render_basis(output, context, claim);
    if let Some(derivation) = &claim.derivation {
        push_line(
            output,
            &format!(
                "  - *{}: {}*",
                context.labels.derivation,
                escape_inline(&derivation.method)
            ),
        );
    }
    push_line(output, "");
}

fn render_basis(output: &mut String, context: &RenderContext<'_>, claim: &ReportClaim) {
    let basis = claim
        .basis_claim_ids
        .iter()
        .filter_map(|claim_id| context.claim_number(claim_id))
        .map(|number| format!("{} [{number}](#claim-{number})", context.labels.finding))
        .collect::<Vec<_>>();
    if !basis.is_empty() {
        push_line(
            output,
            &format!("  - *{}: {}*", context.labels.basis, basis.join(", ")),
        );
    }
}

fn render_relation(output: &mut String, context: &RenderContext<'_>, relation: &ReportRelation) {
    let references = relation
        .claim_ids
        .iter()
        .filter_map(|claim_id| context.claim_number(claim_id))
        .map(|number| format!("[{}](#claim-{number})", number))
        .collect::<Vec<_>>();
    if references.len() == 2 {
        push_line(
            output,
            &format!(
                "- **{}:** {} {} / {} {}.",
                context.labels.contradiction,
                context.labels.finding,
                references[0],
                context.labels.finding,
                references[1]
            ),
        );
    }
}

fn push_line(output: &mut String, line: &str) {
    output.push_str(line);
    output.push('\n');
}

fn escape_inline(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\\' | '`' | '*' | '_' | '[' | ']' => {
                escaped.push('\\');
                escaped.push(character);
            }
            '\n' | '\r' => escaped.push(' '),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn escape_code(value: &str) -> String {
    value.replace('`', "\\`")
}

fn safe_https_anchor(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}
