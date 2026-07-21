// Materialization and semantic depth gates for structured model reports.

pub(crate) fn materialize_deep_research_completed_report_from_generation(
    workspace: &Path,
    query: &str,
    generated: &GeneratedDeepResearchReport,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Result<ResearchReportArtifacts, String> {
    validate_generated_report_depth(generated, workflow_output)?;
    let markdown = normalize_report_markdown_candidate(query, &generated.markdown)
        .ok_or_else(|| {
            "content rejected: structured generation did not contain a completed Markdown report"
                .to_string()
        })?;
    let markdown = sanitize_unobserved_markdown_http_citations(
        &markdown,
        query,
        workflow_output,
        workflow_metadata,
    );
    let html = deep_research_completed_report_html_with_presentation(
        query,
        &markdown,
        Some(&generated.presentation),
        Some(&generated.editorial.thesis),
    );
    validate_deep_research_completed_report_content(
        &markdown,
        &html,
        query,
        workflow_output,
        workflow_metadata,
    )?;

    let slug = deep_research_report_slug(query);
    let (root, report_dir) = prepare_research_report_directory(workspace, &slug)?;
    write_research_report_pair(
        &report_dir.join("report.md"),
        markdown,
        &report_dir.join("index.html"),
        html,
    )?;

    let rel_html = format!(".a3s/research/{slug}/index.html");
    let artifacts = trusted_research_report_artifact_paths(&rel_html, &root)
        .ok_or_else(|| "completed report artifacts failed path validation".to_string())?;
    completed_research_report_artifacts(&artifacts)
        .then_some(artifacts)
        .ok_or_else(|| "completed report artifacts failed content validation".to_string())
}

fn validate_generated_report_depth(
    generated: &GeneratedDeepResearchReport,
    workflow_output: &str,
) -> Result<(), String> {
    let thesis = generated.editorial.thesis.trim();
    if thesis.chars().count() < 12 {
        return Err(
            "content rejected: the report has no substantive answer-first thesis".to_string(),
        );
    }
    if generated.presentation.rationale.trim().chars().count() < 12 {
        return Err(
            "content rejected: report-master presentation lacks a content-specific rationale"
                .to_string(),
        );
    }

    let planned_obligations = planned_report_obligations(workflow_output)?;
    validate_report_obligation_coverage(
        &generated.editorial,
        planned_obligations.as_deref(),
    )
}

fn planned_report_obligations(
    workflow_output: &str,
) -> Result<Option<Vec<a3s::research::ResearchObligation>>, String> {
    let workflow = serde_json::from_str::<serde_json::Value>(workflow_output.trim())
        .map_err(|error| format!("decode DeepResearch workflow for report coverage: {error}"))?;
    match validated_inquiry_projection(&workflow)? {
        ValidatedInquiryProjection::Inquiry { state, .. } => {
            if state.obligations.is_empty() {
                return Err(
                    "content rejected: Inquiry report context contains no research obligations"
                        .to_string(),
                );
            }
            Ok(Some(
                state
                    .obligations
                    .into_iter()
                    .collect(),
            ))
        }
        // Historical checked-loop artifacts predate stable research
        // obligation IDs. They retain their legacy publication validators but
        // never receive fuzzy title matching here.
        ValidatedInquiryProjection::LegacyCheckedLoop => Ok(None),
    }
}
