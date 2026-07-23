use super::ReportDocument;

mod html;
mod markdown;
mod shared;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RenderedReport {
    pub(super) markdown: String,
    pub(super) html: String,
}

pub(super) fn render_report_document(document: &ReportDocument) -> RenderedReport {
    let context = shared::RenderContext::new(document);
    RenderedReport {
        markdown: markdown::render(&context),
        html: html::render(&context),
    }
}
