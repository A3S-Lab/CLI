use super::frozen_fixture::{load_frozen_replays, FrozenReplay};
use super::*;

fn replay(case_id: &str) -> FrozenReplay {
    load_frozen_replays()
        .into_iter()
        .find(|replay| replay.id == case_id)
        .unwrap_or_else(|| panic!("missing frozen replay `{case_id}`"))
}

fn claim_document(replay: &FrozenReplay) -> ReportDocument {
    let ledger = admit_claim_ledger(&replay.contract, &replay.catalog, replay.proposal.clone())
        .unwrap_or_else(|error| panic!("{}: admit frozen ledger: {error}", replay.id));
    build_report_document(&replay.contract, &replay.catalog, &ledger)
        .unwrap_or_else(|error| panic!("{}: build frozen document: {error}", replay.id))
}

fn document_claims(document: &ReportDocument) -> impl Iterator<Item = &ReportClaim> {
    document.direct_answer_claims.iter().chain(
        document
            .dimensions
            .iter()
            .flat_map(|dimension| dimension.claims.iter()),
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[test]
fn markdown_and_html_project_the_same_frozen_document_content() {
    for replay in load_frozen_replays() {
        let document = if replay.fault_stage.as_deref() == Some("report_generation") {
            build_source_backed_document(&replay.contract, &replay.catalog)
                .expect("source-backed frozen document")
        } else {
            claim_document(&replay)
        };
        let rendered = render_report_document(&document);

        assert!(rendered.markdown.contains(&document.title), "{}", replay.id);
        assert!(rendered.html.contains(&document.title), "{}", replay.id);
        for claim in document_claims(&document) {
            assert_eq!(
                rendered.markdown.matches(&claim.text).count(),
                1,
                "{}: Markdown claim `{}`",
                replay.id,
                claim.id
            );
            assert_eq!(
                rendered.html.matches(&claim.text).count(),
                1,
                "{}: HTML claim `{}`",
                replay.id,
                claim.id
            );
        }
        for dimension in &document.dimensions {
            assert!(
                rendered.markdown.contains(&dimension.heading),
                "{}",
                replay.id
            );
            assert!(
                rendered.html.contains(&html_escape(&dimension.heading)),
                "{}",
                replay.id
            );
            for gap in &dimension.gaps {
                assert!(rendered.markdown.contains(&gap.text), "{}", replay.id);
                assert!(
                    rendered.html.contains(&html_escape(&gap.text)),
                    "{}",
                    replay.id
                );
            }
        }
        for source in &document.source_ledger {
            assert!(rendered.markdown.contains(&source.title), "{}", replay.id);
            assert!(rendered.html.contains(&source.title), "{}", replay.id);
            let citation = format!("[{}]", source.number);
            assert!(rendered.markdown.contains(&citation), "{}", replay.id);
            assert!(rendered.html.contains(&citation), "{}", replay.id);
        }
    }
}

#[test]
fn f01_renders_typed_contradiction_without_internal_claim_ids() {
    let replay = replay("F01");
    let document = claim_document(&replay);

    let rendered = render_report_document(&document);

    assert!(rendered.markdown.contains("Contradiction"));
    assert!(rendered.html.contains("Contradiction"));
    assert!(rendered.markdown.contains("[1](#claim-1)"));
    assert!(rendered.markdown.contains("[2](#claim-2)"));
    assert!(!rendered.markdown.contains("announcement-date"));
    assert!(!rendered.html.contains("deployment-record-date"));
}

#[test]
fn f04_uses_chinese_host_labels_in_both_artifacts() {
    let replay = replay("F04");
    let document = claim_document(&replay);

    let rendered = render_report_document(&document);

    assert!(rendered.markdown.contains("## 直接结论"));
    assert!(rendered.markdown.contains("## 研究维度"));
    assert!(rendered.markdown.contains("## 来源"));
    assert!(rendered.html.contains("<html lang=\"zh\">"));
    assert!(rendered.html.contains("直接结论"));
    assert!(!rendered.html.contains("Direct Answer"));
}

#[test]
fn f06_source_backed_fallback_renders_retained_excerpts_and_reader_safe_limits() {
    let replay = replay("F06");
    let document =
        build_source_backed_document(&replay.contract, &replay.catalog).expect("F06 fallback");

    let rendered = render_report_document(&document);

    assert!(rendered.markdown.contains("30 September 2027"));
    assert!(rendered.html.contains("30 September 2027"));
    for forbidden_internal_term in ["workflow", "model", "claim synthesis", "packet"] {
        assert!(
            !rendered
                .markdown
                .to_ascii_lowercase()
                .contains(forbidden_internal_term),
            "{forbidden_internal_term}"
        );
        assert!(
            !rendered
                .html
                .to_ascii_lowercase()
                .contains(forbidden_internal_term),
            "{forbidden_internal_term}"
        );
    }
}

#[test]
fn html_is_responsive_printable_and_does_not_depend_on_markdown_parsing() {
    let replay = replay("F08");
    let document = claim_document(&replay);

    let rendered = render_report_document(&document);

    assert!(rendered.html.starts_with("<!doctype html>"));
    assert!(rendered
        .html
        .contains("name=\"viewport\" content=\"width=device-width, initial-scale=1\""));
    assert!(rendered.html.contains("@media (max-width: 640px)"));
    assert!(rendered.html.contains("@media print"));
    assert!(rendered.html.contains("overflow-wrap: anywhere"));
    assert!(rendered.html.contains("<main"));
    assert!(rendered
        .html
        .contains("<nav aria-label=\"Report sections\""));
    assert!(!rendered.html.contains("<article><h1>"));
}

#[test]
fn renderers_escape_untrusted_prose_and_only_link_https_sources() {
    let replay = replay("F08");
    let mut document = claim_document(&replay);
    document.direct_answer_claims[0].text =
        "<script>alert('x')</script> [unsafe](javascript:alert(1))".to_string();

    let rendered = render_report_document(&document);

    assert!(!rendered.markdown.contains("<script>"));
    assert!(rendered.markdown.contains("&lt;script&gt;"));
    assert!(!rendered.html.contains("<script>"));
    assert!(rendered.html.contains("&lt;script&gt;"));
    assert!(!rendered.html.contains("href=\"local://"));
    assert!(!rendered.markdown.contains("](local://"));
    assert!(rendered
        .html
        .contains("href=\"https://docs.example.test/cedar/6/requirements\""));
}
