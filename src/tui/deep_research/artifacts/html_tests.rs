use super::*;
use crate::tui::deep_research_report_generation::{
    ReportSectionComposition, ReportSectionRhythm, ReportSectionTreatment,
};

#[test]
fn markdown_report_html_renders_safe_clickable_sources() {
    let html = deep_research_completed_report_html(
        "Clickable sources",
        "# Clickable sources\n\n- [Official <source>](https://example.com/docs?a=1&b=2)\n- Bare: https://example.org/evidence.\n- [unsafe](javascript:alert(1))",
    );

    assert!(
        html.contains(
            "<a href=\"https://example.com/docs?a=1&amp;b=2\">Official &lt;source&gt;</a>"
        ),
        "{html}"
    );
    assert!(
        html.contains("<a href=\"https://example.org/evidence\">https://example.org/evidence</a>."),
        "{html}"
    );
    assert!(html.contains("<a href=\"\">unsafe</a>"), "{html}");
}

#[test]
fn markdown_table_keeps_escaped_pipes_inside_cells() {
    let fragment = deep_research_markdown_to_html_fragment(
        "| Finding | Source |\n| --- | --- |\n| Alpha \\| Beta | [`a\\|b`](https://example.com/a%7Cb) |",
    );

    assert!(fragment.contains("<table>"), "{fragment}");
    assert_eq!(fragment.matches("<td>").count(), 2, "{fragment}");
    assert!(fragment.contains("<td>Alpha | Beta</td>"), "{fragment}");
    assert!(
        fragment.contains("<td><a href=\"https://example.com/a%7Cb\"><code>a|b</code></a></td>"),
        "{fragment}"
    );
}

#[test]
fn markdown_report_html_escapes_raw_html_and_blocks_dangerous_links() {
    let fragment = deep_research_markdown_to_html_fragment(
        "<script>alert('xss')</script>\n\n<img src=x onerror=alert(1)>\n\n[unsafe](javascript:alert(1)) [encoded](jav&#x61;script:alert(2)) [safe](https://example.com)",
    );

    assert!(!fragment.contains("<script"), "{fragment}");
    assert!(!fragment.contains("<img"), "{fragment}");
    assert!(fragment.contains("&lt;script&gt;"), "{fragment}");
    assert!(
        fragment.contains("&lt;img src=x onerror=alert(1)&gt;"),
        "{fragment}"
    );
    assert_eq!(fragment.matches("href=\"\"").count(), 2, "{fragment}");
    assert!(
        fragment.contains("<a href=\"https://example.com\">safe</a>"),
        "{fragment}"
    );
}

#[test]
fn markdown_report_html_keeps_fetched_relative_links_as_plain_text() {
    let fragment = deep_research_markdown_to_html_fragment(
        "[Joint Typhoon Warning Center](/wiki/JTWC) and [local](../private/report.md)",
    );

    assert!(
        fragment.contains("Joint Typhoon Warning Center"),
        "{fragment}"
    );
    assert!(fragment.contains("and local"), "{fragment}");
    assert!(!fragment.contains("href=\"/wiki/"), "{fragment}");
    assert!(!fragment.contains("href=\"../"), "{fragment}");
}

#[test]
fn markdown_report_html_sanitizes_query_fallback_title() {
    let html = deep_research_completed_report_html(
        "Analyze https://user:password@example.com/private?token=secret#fragment",
        "Findings without a level-one heading.",
    );

    assert!(
        html.contains("<title>Analyze https://example.com/private</title>"),
        "{html}"
    );
    for secret in ["user", "password", "token=secret", "#fragment"] {
        assert!(!html.contains(secret), "{html}");
    }
}

#[test]
fn editorial_report_html_has_responsive_print_and_accessibility_contract() {
    let html = deep_research_completed_report_html(
        "一份非常长的中文研究请求，需要生成专业、清晰、可验证且适合移动设备阅读的报告，并且不要把完整用户提示直接当作页面标题反复展示",
        "# 简洁研究标题\n\n## 核心发现\n\n结论正文。\n\n## Sources\n\n- [来源](https://example.com)",
    );

    assert!(html.contains("<html lang=\"zh-CN\">"), "{html}");
    assert!(html.contains("class=\"hero\""), "{html}");
    assert!(html.contains("class=\"report-shell\""), "{html}");
    assert!(html.contains("@media(max-width:820px)"), "{html}");
    assert!(html.contains("横向滑动查看全部列"), "{html}");
    assert!(html.contains("@media print"), "{html}");
    assert!(html.contains("prefers-reduced-motion"), "{html}");
    assert!(html.contains(":focus-visible"), "{html}");
    assert!(
        html.contains("<strong>01</strong><span>引用来源</span>"),
        "{html}"
    );
    assert!(html.contains("class=\"toc\""), "{html}");
    assert!(html.contains("section--findings"), "{html}");
    assert!(html.contains("section--sources"), "{html}");
    assert!(html.contains("aria-label=\"报告元数据\""), "{html}");
    assert_eq!(html.matches("<h1>").count(), 1, "{html}");
    assert!(!html.contains("<script"), "{html}");
}

#[test]
fn editorial_report_uses_distinct_information_shapes_instead_of_one_markdown_shell() {
    let html = deep_research_completed_report_html(
        "Compare Tokio and async-std",
        "# Tokio and async-std\n\n## Executive Summary\n\n- Tokio is active.\n- async-std is deprecated.\n\n## Key Findings\n\n### Maintenance\n\nLifecycle evidence.\n\n### Adoption\n\nAdoption evidence.\n\n## Evidence Matrix\n\n| Finding | Source |\n| --- | --- |\n| Maintenance | [Docs](https://example.com/docs) |\n\n## Gaps And Caveats\n\n- No workload benchmark.\n\n## Source Quality And Confidence\n\nConfidence is medium-high.\n\n## Sources\n\n- [Docs](https://example.com/docs)",
    );

    for class in [
        "section--summary",
        "section--findings",
        "class=\"key-point\"",
        "section--matrix",
        "class=\"table-wrap\"",
        "section--caveats",
        "section--confidence",
        "section--sources",
    ] {
        assert!(html.contains(class), "missing {class}: {html}");
    }
    assert!(html.contains("<strong>02</strong><span>Key findings</span>"));
    assert!(html.contains("href=\"#section-6\""));
}

#[test]
fn report_master_presentation_changes_the_site_composition_for_the_same_content() {
    let markdown = "# Shared evidence\n\n## Executive Summary\n\nA source-backed answer.\n\n## Key Findings\n\n### Finding\n\nInterpretation and implication.\n\n## Sources\n\n- [Source](https://example.com/source)";
    let analytical = ReportPresentation {
        narrative_mode: crate::tui::deep_research_report_generation::ReportNarrativeMode::Pyramid,
        archetype: crate::tui::deep_research_report_generation::ReportArchetype::Analytical,
        palette: crate::tui::deep_research_report_generation::ReportPalette::Graphite,
        density: crate::tui::deep_research_report_generation::ReportDensity::Compact,
        hero: crate::tui::deep_research_report_generation::ReportHero::Metrics,
        visual_stance: crate::tui::deep_research_report_generation::ReportVisualStance::Safe,
        rationale: "Dense comparison for a decision reader.".to_string(),
        section_plan: vec![
            ReportSectionTreatment {
                heading: "Executive Summary".to_string(),
                rhythm: ReportSectionRhythm::Anchor,
                composition: ReportSectionComposition::Prose,
            },
            ReportSectionTreatment {
                heading: "Key Findings".to_string(),
                rhythm: ReportSectionRhythm::Dense,
                composition: ReportSectionComposition::KeyPoints,
            },
            ReportSectionTreatment {
                heading: "Sources".to_string(),
                rhythm: ReportSectionRhythm::Breathing,
                composition: ReportSectionComposition::SourceLedger,
            },
        ],
    };
    let chronicle = ReportPresentation {
        narrative_mode: crate::tui::deep_research_report_generation::ReportNarrativeMode::Narrative,
        archetype: crate::tui::deep_research_report_generation::ReportArchetype::Chronicle,
        palette: crate::tui::deep_research_report_generation::ReportPalette::Amber,
        density: crate::tui::deep_research_report_generation::ReportDensity::Spacious,
        hero: crate::tui::deep_research_report_generation::ReportHero::Statement,
        visual_stance: crate::tui::deep_research_report_generation::ReportVisualStance::Bold,
        rationale: "Ordered change needs a chronological reading rhythm.".to_string(),
        section_plan: vec![
            ReportSectionTreatment {
                heading: "Executive Summary".to_string(),
                rhythm: ReportSectionRhythm::Breathing,
                composition: ReportSectionComposition::Prose,
            },
            ReportSectionTreatment {
                heading: "Key Findings".to_string(),
                rhythm: ReportSectionRhythm::Anchor,
                composition: ReportSectionComposition::Timeline,
            },
            ReportSectionTreatment {
                heading: "Sources".to_string(),
                rhythm: ReportSectionRhythm::Dense,
                composition: ReportSectionComposition::SourceLedger,
            },
        ],
    };

    let analytical_html = deep_research_completed_report_html_with_presentation(
        "Shared evidence",
        markdown,
        Some(&analytical),
        Some("The evidence supports an immediate bounded decision."),
    );
    let chronicle_html = deep_research_completed_report_html_with_presentation(
        "Shared evidence",
        markdown,
        Some(&chronicle),
        Some("The evidence explains how the situation changed over time."),
    );

    assert!(analytical_html.contains(
        "class=\"mode-pyramid archetype-analytical palette-graphite density-compact hero-metrics stance-safe\""
    ));
    assert!(chronicle_html.contains(
        "class=\"mode-narrative archetype-chronicle palette-amber density-spacious hero-statement stance-bold\""
    ));
    assert!(analytical_html.contains("The evidence supports an immediate bounded decision."));
    assert!(chronicle_html.contains("The evidence explains how the situation changed over time."));
    assert!(analytical_html.contains("class=\"evidence-profile\""));
    assert!(!chronicle_html.contains("class=\"evidence-profile\""));
    assert!(analytical_html.contains("rhythm-dense composition-key-points"));
    assert!(chronicle_html.contains("rhythm-anchor composition-timeline"));
    assert_ne!(analytical_html, chronicle_html);
}

#[test]
fn split_hero_uses_the_report_reading_path_instead_of_decorative_metrics() {
    let presentation = ReportPresentation::default();
    let html = deep_research_completed_report_html_with_presentation(
        "A bounded decision",
        "# Decision\n\n## Answer\n\nAct now.\n\n## Trade-offs\n\nThe boundary is explicit.\n\n## Sources\n\n- [Source](https://example.com)",
        Some(&presentation),
        Some("The evidence changes the decision boundary."),
    );

    assert!(html.contains("class=\"hero-map\""), "{html}");
    assert!(html.contains("Reading path"), "{html}");
    assert!(!html.contains("class=\"evidence-profile\""), "{html}");
}

#[test]
fn recovery_report_is_visually_and_semantically_degraded() {
    let html = deep_research_completed_report_html(
        "Current market state",
        "# DeepResearch Recovery Report\n\n## Findings\n\nEvidence collection did not complete.\n\n## Sources And Evidence\n\n- https://example.com/partial\n\n## Confidence And Limits\n\nConfidence is low.",
    );

    assert!(
        html.contains("class=\"theme-editorial report-degraded\""),
        "{html}"
    );
    assert!(html.contains("Insufficient evidence · Degraded"), "{html}");
    assert!(html.contains("Not a final domain conclusion"), "{html}");
    assert!(html.contains("<title>DeepResearch Recovery Report</title>"));
}

#[test]
fn editorial_report_wraps_unparsed_relative_markdown_without_page_overflow() {
    let html = deep_research_completed_report_html(
        "台风巴威",
        "# 台风巴威研究\n\n## 证据\n\n- [菲律宾大气地球物理和天文管理局](/wiki/%E8%8F%B2%E5%BE%8B%E8%B3%93%E5%A4%A7%E6%B0%A3%E5%9C%B0%E7%90%83%E7%89%A9%E7%90%86%E5%92%8C%E5%A4%A9%E6%96%87%E7%AE%A1%E7%90%86%E5%B1%80",
    );

    assert!(
        html.contains("article{min-width:0;max-width:100%"),
        "{html}"
    );
    assert!(html.contains("overflow-wrap:anywhere"), "{html}");
    assert!(
        html.contains(".table-wrap { width: 100%; overflow-x: auto;"),
        "{html}"
    );
    assert!(
        html.contains("table { width: 100%; min-width: 720px;"),
        "{html}"
    );
}

#[test]
fn editorial_report_derives_a_semantic_title_without_double_ellipsis() {
    let title = concise_report_title(
        "请研究2020年第8号台风“巴威”（Bavi）的生命史、路径、强度、登陆时间、灾害影响和预警…研究报告",
    );

    assert_eq!(title, "2020年第8号台风“巴威”（Bavi）研究");
    assert!(!title.contains('…'));
}

#[test]
fn editorial_report_preserves_an_ordinary_descriptive_cjk_title() {
    let title = "Tokio 与 async-std：维护状态、生态采用与迁移建议（截至 2026 年 7 月）";
    let html = deep_research_completed_report_html(
        "Compare two Rust runtimes",
        &format!("# {title}\n\n## 结论\n\n证据支持该结论。"),
    );

    assert!(html.contains(&format!("<title>{title}</title>")), "{html}");
    assert!(html.contains(&format!("<h1>{title}</h1>")), "{html}");
}

#[test]
fn print_layout_does_not_expand_every_inline_link_or_pin_large_tables() {
    let html = deep_research_completed_report_html(
        "Printable report",
        "# Printable report\n\nA [source](https://example.com/very/long/path).",
    );

    assert!(!html.contains("attr(href)"), "{html}");
    assert!(
        html.contains(".key-point, .timeline-entry, .process-step, tr { break-inside: avoid; }"),
        "{html}"
    );
    assert!(html.contains(".table-wrap { overflow: visible;"), "{html}");
}

#[test]
fn editorial_report_counts_unique_external_source_urls() {
    let html = deep_research_completed_report_html(
        "Source count",
        "# Source count\n\n## Summary\n\nLead with [supporting context](https://context.example.net).\n\n## Sources\n\n- [Primary](https://example.com/evidence) — evidence: see [nested detail](https://nested.example.net)\n- [Primary again](https://example.com/evidence)\n- [Secondary](http://example.org/report)\n- [Internal](/local/report)",
    );

    assert!(
        html.contains("<strong>02</strong><span>Cited sources</span>"),
        "{html}"
    );
    assert!(!html.contains("Cited sources: 2"), "{html}");
}

#[test]
fn editorial_hero_background_tracks_intrinsic_content_height() {
    let html = deep_research_completed_report_html(
        "Responsive hero",
        "# Responsive hero\n\n## Summary\n\nLead.",
    );

    assert!(
        html.contains("body{margin:0;background:var(--paper)"),
        "{html}"
    );
    assert!(
        html.contains(".hero{background:var(--navy);color:#fff;overflow:hidden"),
        "{html}"
    );
    assert!(html.contains("class=\"hero-inner\""), "{html}");
    assert!(
        !html.contains("linear-gradient(180deg,var(--navy)"),
        "{html}"
    );
    assert!(!html.contains("440px"), "{html}");
    assert!(!html.contains("360px"), "{html}");
}

#[test]
fn editorial_lead_style_targets_paragraph_after_first_section_heading() {
    let html = deep_research_completed_report_html(
        "Lead paragraph",
        "# Lead paragraph\n\n## Executive summary\n\nThis paragraph is the report lead.\n\nMore detail.",
    );

    assert!(
        html.contains(".section--summary .section-body > p:first-child"),
        "{html}"
    );
    assert!(
        html.contains("class=\"report-section section--summary rhythm-anchor composition-prose\"")
            && html.contains("<h2>Executive summary</h2>")
            && html.contains("<p>This paragraph is the report lead.</p>"),
        "{html}"
    );
}

#[test]
fn mobile_rail_keeps_navigation_without_decorative_section_counts() {
    let html = deep_research_completed_report_html(
        "Mobile metadata",
        "# Mobile metadata\n\n## Summary\n\nLead.\n\n## Sources\n\n- [Source](https://example.com)",
    );

    assert!(html.contains("class=\"toc\""), "{html}");
    assert!(html.contains("href=\"#section-2\""), "{html}");
    assert!(!html.contains("rail-stat"), "{html}");
}

#[test]
fn analytical_mobile_layout_overrides_the_archetype_sidebar_grid() {
    let html = deep_research_completed_report_html_with_presentation(
        "Responsive analysis",
        "# Responsive analysis\n\n## Findings\n\nThe bounded conclusion.\n\n## Sources\n\n- [Source](https://example.com)",
        Some(&ReportPresentation {
            archetype: crate::tui::deep_research_report_generation::ReportArchetype::Analytical,
            ..ReportPresentation::default()
        }),
        Some("The layout must preserve a readable article width on narrow screens."),
    );

    assert!(
        html.contains("body.archetype-analytical .report-shell { grid-template-columns:1fr; }"),
        "{html}"
    );
}

#[test]
fn print_source_ledger_is_single_column_and_does_not_create_a_footer_only_page() {
    let html = deep_research_completed_report_html(
        "Printable sources",
        "# Printable sources\n\n## Sources\n\n- [One](https://example.com/one)\n- [Two](https://example.com/two)",
    );

    assert!(
        html.contains(".composition-source-ledger { break-inside:avoid; }"),
        "{html}"
    );
    assert!(
        html.contains(
            ".composition-source-ledger .section-body > ul { grid-template-columns:1fr; }"
        ),
        "{html}"
    );
    assert!(
        html.contains(".composition-source-ledger .section-body > ul > li { padding:10px 12px 10px 44px; break-inside:avoid;"),
        "{html}"
    );
    assert!(
        html.contains("body.density-compact main, body.density-spacious main, main { max-width: none; padding: 20px 0 0; }"),
        "{html}"
    );
    assert!(html.contains(".footer-note { display:none; }"), "{html}");
}
