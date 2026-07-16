use super::*;

fn observed_workflow_output(sources: &[&str]) -> String {
    let sources = sources
        .iter()
        .enumerate()
        .map(|(index, url)| {
            serde_json::json!({
                "title": format!("Observed source {index}"),
                "url_or_path": url,
                "quote_or_fact": "Observed evidence for the report.",
                "reliability": "Traceable source"
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "mode": "direct_web",
        "checker": { "decision": "finalize" },
        "research": {
            "status": "success",
            "results": [{
                "success": true,
                "structured": {
                    "summary": "Observed evidence",
                    "sources": sources,
                    "key_evidence": ["Observed evidence for the report."],
                    "contradictions": [],
                    "confidence": "high",
                    "gaps": []
                }
            }]
        }
    })
    .to_string()
}

#[test]
fn source_display_domains_do_not_become_unobserved_local_citations() {
    let query = "Compare runtime maintenance";
    let crates = "https://crates.io/crates/runtime";
    let docs = "https://docs.rs/runtime";
    let workflow_output = observed_workflow_output(&[crates, docs]);
    let markdown = format!(
        "# Runtime report\n\n\
         ## Findings\n\nThe runtime remains maintained, with explicit limitations in the available evidence.\n\n\
         ## Sources\n\n\
         - Runtime on crates.io — {crates}\n\
         - Runtime documentation on docs.rs — {docs}\n\n\
         ## Limitations\n\nConfidence is bounded by the cited evidence.\n"
    );
    let html = deep_research_completed_report_html(query, &markdown);

    assert!(
        deep_research_report_content_sources_trace_workflow(
            &markdown,
            &html,
            query,
            &workflow_output,
            None,
        ),
        "display-only domain names must not be treated as local source paths"
    );
}

#[test]
fn legitimate_unavailable_finding_is_not_placeholder_content() {
    let markdown = "# Runtime report\n\n## Findings\n\nIndependent adoption data is not yet available, so the report does not claim a precise market share. The available release history still supports a bounded maintenance comparison.\n\n## Sources\n\n- https://example.com/releases\n\n## Limitations\n\nConfidence is limited by the unavailable adoption data.\n";
    let html = deep_research_completed_report_html("Runtime report", markdown);

    assert!(
        has_research_report_substance(markdown, &html),
        "a disclosed evidence gap is a legitimate finding, not placeholder text"
    );
}

#[test]
fn source_trace_diagnostic_names_every_unobserved_citation() {
    let observed = "https://example.com/observed";
    let unobserved = "https://example.com/unobserved";
    let workflow_output = observed_workflow_output(&[observed]);
    let markdown = format!(
        "# Runtime report\n\n## Findings\n\nA substantive comparison with bounded confidence and explicit limitations.\n\n## Sources\n\n- {observed}\n- {unobserved}\n\n## Limitations\n\nConfidence is limited to observed evidence.\n"
    );
    let html = deep_research_completed_report_html("Runtime report", &markdown);

    let diagnostic = deep_research_report_source_trace_diagnostic(
        &markdown,
        &html,
        "Runtime report",
        &workflow_output,
        None,
    )
    .expect_err("an unobserved citation must be rejected with a diagnostic");

    assert!(diagnostic.contains(unobserved), "{diagnostic}");
    assert!(
        diagnostic.contains("not observed in this run"),
        "{diagnostic}"
    );
}

#[test]
fn generated_report_sanitizes_one_unobserved_url_without_discarding_the_report() {
    let observed = "https://example.com/observed";
    let mistyped = "https://example.com/obesrved";
    let workflow_output = observed_workflow_output(&[observed]);
    let markdown = format!(
        "# Runtime report\n\n\
         ## Findings\n\nA supported finding cites [the observed source]({observed}), while the readable label from [a mistyped duplicate]({mistyped}) should remain without an unsafe link.\n\n\
         ## Sources\n\n\
         - [Observed source]({observed})\n\
         - [Mistyped duplicate]({mistyped})\n\n\
         ## Limitations\n\nConfidence is bounded by the one observed source and the unavailable corroboration.\n"
    );

    let cleaned = sanitize_unobserved_markdown_http_citations(
        &markdown,
        "Runtime report",
        &workflow_output,
        None,
    );
    assert!(cleaned.contains(observed), "{cleaned}");
    assert!(!cleaned.contains(mistyped), "{cleaned}");
    assert!(cleaned.contains("a mistyped duplicate"), "{cleaned}");
    assert!(!cleaned.contains("- Mistyped duplicate"), "{cleaned}");

    let html = deep_research_completed_report_html("Runtime report", &cleaned);
    deep_research_report_source_trace_diagnostic(
        &cleaned,
        &html,
        "Runtime report",
        &workflow_output,
        None,
    )
    .expect("the remaining report should still pass the complete source trace gate");
}

#[test]
fn internal_status_text_has_a_specific_rejection_diagnostic() {
    let workflow_output = observed_workflow_output(&["https://example.com/observed"]);
    let answer = "# Runtime report\n\n## Findings\n\nCreated the report directory and wrote the HTML report. This deliberately contains enough additional text to pass the length boundary.\n\n## Sources\n\n- https://example.com/observed\n\n## Limitations\n\nConfidence is bounded.\n";

    let diagnostic = deep_research_report_rejection_diagnostic_from_answer_text(
        "Runtime report",
        answer,
        &workflow_output,
        None,
    )
    .expect("internal status text must have a rejection diagnostic");

    assert!(
        diagnostic.contains("internal workflow or tool-status"),
        "{diagnostic}"
    );
}
