use super::{
    complete_html_document, deep_research_output_has_internal_leak, has_research_report_substance,
    is_deep_research_model_failure_text, looks_like_deep_research_fallback_draft,
    read_small_utf8_file, ResearchReportArtifacts,
};

pub(super) fn recovery_research_report_artifacts(artifacts: &ResearchReportArtifacts) -> bool {
    let markdown = read_small_utf8_file(&artifacts.markdown);
    let html = read_small_utf8_file(&artifacts.html);
    let (Some(markdown), Some(html)) = (markdown, html) else {
        return false;
    };
    looks_like_deep_research_recovery_report(&markdown)
        && looks_like_deep_research_recovery_report(&html)
        && !looks_like_deep_research_fallback_draft(&markdown)
        && !looks_like_deep_research_fallback_draft(&html)
        && !is_deep_research_model_failure_text(&markdown)
        && !is_deep_research_model_failure_text(&html)
        && !deep_research_output_has_internal_leak(&markdown)
        && !deep_research_output_has_internal_leak(&html)
        && complete_html_document(&html)
        && has_research_report_substance(&markdown, &html)
}

pub(super) fn looks_like_deep_research_recovery_report(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("deepresearch recovery report")
        || lower.contains("# deepresearch recovery report")
        || lower.contains("<title>deepresearch recovery report")
        || lower.contains("<h1>deepresearch recovery report")
}
