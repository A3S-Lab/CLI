use super::*;

#[test]
fn workflow_evidence_uses_concise_topic_title() {
    assert_eq!(
        evidence_report_title("请用中文撰写一份关于2020年第8号台风“巴威”（Bavi）的精美、全面、可核验研究报告。明确口径。"),
        "2020年第8号台风“巴威”（Bavi）研究报告"
    );
    assert_eq!(
        evidence_report_title("请研究2020年第8号台风“巴威”（Bavi）的生命史、路径、强度、登陆时间、灾害影响和预警响应，截至2026-07-12复核来源有效性。"),
        "2020年第8号台风“巴威”（Bavi）研究报告"
    );
}

#[test]
fn workflow_evidence_derives_headings_without_domain_templates() {
    assert_eq!(
        finding_heading("任意领域的可核验结论。后续说明不进入标题。"),
        "任意领域的可核验结论"
    );
    assert_eq!(
        finding_heading("A source-backed conclusion. Supporting detail follows."),
        "A source-backed conclusion"
    );
    assert_eq!(
        finding_heading("Tokio 官方仓库显示项目仍持续发布，通常按月发布次版本，并公布 MSRV 1.71 与 LTS 回补政策：https://github.com/tokio-rs/tokio"),
        "Tokio 官方仓库显示项目仍持续发布"
    );
}

#[test]
fn chinese_inline_urls_select_only_the_explicit_source() {
    let sources = vec![
        (
            1,
            StructuredEvidenceSource {
                title: Some("Tokio".to_string()),
                url_or_path: "https://github.com/tokio-rs/tokio".to_string(),
                date: None,
                quote_or_fact: None,
                reliability: None,
            },
        ),
        (
            2,
            StructuredEvidenceSource {
                title: Some("Unrelated".to_string()),
                url_or_path: "https://example.com/unrelated".to_string(),
                date: None,
                quote_or_fact: None,
                reliability: None,
            },
        ),
    ];

    let selected = explicitly_cited_sources(
        "Tokio 持续发布：https://github.com/tokio-rs/tokio。",
        &sources,
    );

    assert_eq!(selected.len(), 1);
    assert_eq!(
        selected[0].1.url_or_path,
        "https://github.com/tokio-rs/tokio"
    );
    assert_eq!(
        verified_finding_statement("Tokio 持续发布：https://github.com/tokio-rs/tokio。"),
        "Tokio 持续发布"
    );
}

#[test]
fn workflow_evidence_uses_concise_chinese_comparison_title() {
    assert_eq!(
        evidence_report_title("请对比 Tokio 与 async-std 的架构和生态，给出明确选型建议。"),
        "Tokio 与 async-std 的架构和生态研究报告"
    );
    assert_eq!(
        evidence_report_title("请比较 Alpha 和 Beta；并给出结论。"),
        "Alpha 和 Beta研究报告"
    );
}

#[test]
fn workflow_evidence_uses_concise_english_comparison_title() {
    assert_eq!(
        evidence_report_title(
            "Compare Tokio and async-std for a production Rust TCP service. Cover architecture, maintenance, ecosystem, and migration."
        ),
        "Tokio and async-std for a production Rust TCP service — Research Report"
    );
}

#[test]
fn workflow_evidence_title_removes_internal_validation_annotations() {
    assert_eq!(
        evidence_report_title(
            "截至2026年7月12日（run-id e2e-20260712-0754-b81e cache-bust validation 91）请对比 Tokio 与 async-std，并给出建议。"
        ),
        "Tokio 与 async-std研究报告"
    );
    assert_eq!(
        evidence_report_title(
            "As of 2026-07-12 (run-id convergence-e2e cache-bust validation), compare Tokio and async-std. Give a recommendation."
        ),
        "Tokio and async-std — Research Report"
    );
}

#[test]
fn workflow_evidence_report_has_final_report_shape() {
    let evidence = vec![StructuredEvidenceItem {
        summary: "Official sources establish the tournament dates, host countries, and match count for the event.".to_string(),
        sources: vec![StructuredEvidenceSource {
            title: Some("Official Schedule".to_string()),
            url_or_path: "https://example.com/official-schedule".to_string(),
            date: Some("2026-07-09".to_string()),
            quote_or_fact: Some(
                "The official schedule lists the dates, venues, and match count."
                    .to_string(),
            ),
            reliability: Some("Official source".to_string()),
        }],
        key_evidence: vec![
            "The source provides a concrete date range and venue list.".to_string(),
        ],
        contradictions: vec!["Independent commentary does not change the official dates.".to_string()],
        gaps: vec!["The fixture-level draw should be rechecked after updates.".to_string()],
        confidence: Some("High for official schedule facts.".to_string()),
    }];

    let markdown = completed_report_markdown_from_workflow_evidence("schedule research", &evidence)
        .expect("structured evidence should produce a report");

    assert!(markdown.contains("## Executive Summary"), "{markdown}");
    assert!(markdown.contains("## Key Findings"), "{markdown}");
    assert!(markdown.contains("## Evidence Matrix"), "{markdown}");
    assert!(
        markdown.contains("## Source Quality And Confidence"),
        "{markdown}"
    );
    assert!(markdown.contains("## Gaps And Caveats"), "{markdown}");
    assert!(
        markdown.contains("https://example.com/official-schedule"),
        "{markdown}"
    );
    assert!(
        !markdown.contains("DeepResearch Recovery Report"),
        "{markdown}"
    );
    assert!(!markdown.contains("facts.."), "{markdown}");
}

#[test]
fn finalized_checker_summary_becomes_the_report_lead_and_filters_noise() {
    let evidence = vec![StructuredEvidenceItem {
        summary: "Direct collection found three traceable sources.".to_string(),
        sources: vec![
            StructuredEvidenceSource {
                title: Some("Rust release archive".to_string()),
                url_or_path: "https://blog.rust-lang.org/releases/".to_string(),
                date: None,
                quote_or_fact: Some("Rust 1.87.0 was announced on May 15, 2025.".to_string()),
                reliability: Some("Official source".to_string()),
            },
            StructuredEvidenceSource {
                title: Some("Rust lifecycle history".to_string()),
                url_or_path: "https://endoflife.date/rust".to_string(),
                date: None,
                quote_or_fact: Some("Rust 1.87.0 has a release date of May 15, 2025.".to_string()),
                reliability: Some("Independent chronology".to_string()),
            },
            StructuredEvidenceSource {
                title: Some("Unrelated May facts".to_string()),
                url_or_path: "https://example.com/may".to_string(),
                date: None,
                quote_or_fact: Some("May is the fifth month.".to_string()),
                reliability: Some("Unrelated".to_string()),
            },
        ],
        key_evidence: vec![],
        contradictions: vec![],
        gaps: vec![],
        confidence: Some("High".to_string()),
    }];
    let verified = "Rust 1.87.0 is the stable release dated May 15, 2025, confirmed by the official archive and an independent chronology";

    let markdown = completed_report_markdown_with_verified_summary(
        "Rust release on May 15, 2025",
        &evidence,
        Some(verified),
    )
    .expect("finalized evidence should produce a semantic report");

    assert!(markdown.contains(verified), "{markdown}");
    assert!(markdown.contains("### Verified finding"), "{markdown}");
    assert!(
        !markdown.contains("Direct collection found three traceable sources"),
        "{markdown}"
    );
    assert!(!markdown.contains("https://example.com/may"), "{markdown}");
}

#[test]
fn finalized_checker_summary_keeps_source_backed_supporting_findings() {
    let evidence = vec![StructuredEvidenceItem {
        summary: "Direct collection found two traceable sources.".to_string(),
        sources: vec![StructuredEvidenceSource {
            title: Some("Tokio runtime documentation".to_string()),
            url_or_path: "https://docs.rs/tokio/latest/tokio/".to_string(),
            date: None,
            quote_or_fact: Some(
                "Tokio provides an event-driven runtime and task scheduler.".to_string(),
            ),
            reliability: Some("Official documentation".to_string()),
        }],
        key_evidence: vec!["Tokio provides an event-driven runtime and task scheduler.".to_string()],
        contradictions: vec![],
        gaps: vec!["No independent benchmark was retained.".to_string()],
        confidence: Some("Medium".to_string()),
    }];

    let markdown = completed_report_markdown_with_verified_summary(
        "Compare Rust runtimes",
        &evidence,
        Some("The retained documentation supports a bounded runtime comparison"),
    )
    .expect("finalized evidence should retain its supporting finding");

    assert!(
        markdown.contains("### 1. Tokio runtime documentation"),
        "{markdown}"
    );
    assert!(
        markdown.contains("Tokio provides an event-driven runtime and task scheduler"),
        "{markdown}"
    );
    assert!(
        !markdown.contains("Direct collection found two traceable sources"),
        "{markdown}"
    );
}

#[test]
fn verified_report_context_keeps_reader_title_findings_and_checker_gaps_consistent() {
    let evidence = vec![StructuredEvidenceItem {
        summary: "Direct collection found one traceable source.".to_string(),
        sources: vec![StructuredEvidenceSource {
            title: Some("Primary runtime documentation".to_string()),
            url_or_path: "https://example.com/runtime-docs".to_string(),
            date: Some("2026-07-13".to_string()),
            quote_or_fact: Some("The runtime documents component support.".to_string()),
            reliability: Some("Official documentation".to_string()),
        }],
        key_evidence: vec!["The runtime documents component support.".to_string()],
        contradictions: vec![],
        gaps: vec![],
        confidence: Some("Medium".to_string()),
    }];
    let findings = vec!["The runtime has documented component support.".to_string()];
    let checker_caveats =
        vec!["Independent interoperability evidence remains unavailable.".to_string()];

    let markdown = completed_report_markdown_with_verified_context(
        "Research a deliberately long subject whose raw request should not become the displayed report title",
        &evidence,
        Some("Component Adoption Decision Guide"),
        Some("The retained evidence supports a bounded pilot."),
        &findings,
        &checker_caveats,
    )
    .expect("verified report context should produce a report");

    assert!(
        markdown.starts_with("# Component Adoption Decision Guide\n"),
        "{markdown}"
    );
    assert!(
        markdown.contains("### 1. The runtime has documented component support"),
        "{markdown}"
    );
    assert_eq!(
        markdown
            .matches("The runtime has documented component support")
            .count(),
        2,
        "the verified conclusion must have a readable body, not only a heading: {markdown}"
    );
    assert!(
        markdown.contains("Independent interoperability evidence remains unavailable."),
        "{markdown}"
    );
    assert!(
        !markdown.contains("No material contradictions or gaps were captured"),
        "{markdown}"
    );
    assert!(!markdown.contains("pilot.."), "{markdown}");
    assert!(
        !markdown.contains("### 1. Primary runtime documentation"),
        "mechanical direct evidence must not become a duplicate finding: {markdown}"
    );
}

#[test]
fn verified_findings_use_explicit_source_anchors_and_separate_detailed_analysis() {
    let evidence = vec![
        StructuredEvidenceItem {
            summary: "Primary documentation establishes the supported production behavior."
                .to_string(),
            sources: vec![StructuredEvidenceSource {
                title: Some("Primary documentation".to_string()),
                url_or_path: "https://example.com/primary".to_string(),
                date: None,
                quote_or_fact: Some("The production behavior is documented.".to_string()),
                reliability: Some("Official".to_string()),
            }],
            key_evidence: vec!["The behavior is supported in production.".to_string()],
            contradictions: vec![],
            gaps: vec![],
            confidence: Some("High".to_string()),
        },
        StructuredEvidenceItem {
            summary: "Independent analysis confirms the migration constraint.".to_string(),
            sources: vec![StructuredEvidenceSource {
                title: Some("Independent analysis".to_string()),
                url_or_path: "https://example.net/independent".to_string(),
                date: None,
                quote_or_fact: Some("The migration constraint was reproduced.".to_string()),
                reliability: Some("Independent".to_string()),
            }],
            key_evidence: vec!["The migration constraint was reproduced.".to_string()],
            contradictions: vec![],
            gaps: vec![],
            confidence: Some("Medium-high".to_string()),
        },
    ];
    let findings = vec![
        "The migration constraint is independently confirmed. Source: https://example.net/independent"
            .to_string(),
    ];

    let markdown = completed_report_markdown_with_verified_context(
        "Compare two production options",
        &evidence,
        Some("Production Decision"),
        Some("The evidence supports a conditional recommendation."),
        &findings,
        &[],
    )
    .expect("verified report should render");
    let conclusion = markdown
        .split("## Evidence Analysis")
        .next()
        .expect("conclusion section");

    assert!(
        conclusion.contains("[Independent analysis](https://example.net/independent)"),
        "{markdown}"
    );
    assert!(
        !conclusion.contains("[Primary documentation](https://example.com/primary)"),
        "an explicit checker citation must not fan out to unrelated sources: {markdown}"
    );
    assert!(markdown.contains("## Evidence Analysis"), "{markdown}");
}

#[test]
fn report_caveats_prioritize_checker_gaps_and_drop_transport_noise() {
    let evidence = vec![StructuredEvidenceItem {
        summary: "The retained source supports a bounded recommendation.".to_string(),
        sources: vec![StructuredEvidenceSource {
            title: Some("Retained source".to_string()),
            url_or_path: "https://example.com/source".to_string(),
            date: None,
            quote_or_fact: Some("The recommendation is bounded.".to_string()),
            reliability: Some("Official".to_string()),
        }],
        key_evidence: vec!["The recommendation is bounded.".to_string()],
        contradictions: vec![],
        gaps: std::iter::once(
            "Collection errors: web_fetch returned no usable page text.".to_string(),
        )
        .chain((1..=12).map(|index| format!("Secondary evidence gap {index}.")))
        .collect(),
        confidence: Some("Medium".to_string()),
    }];
    let checker_gaps = vec!["The decisive benchmark is still unavailable.".to_string()];
    let markdown = completed_report_markdown_with_verified_context(
        "A bounded comparison",
        &evidence,
        None,
        Some("A conditional answer is supportable."),
        &[],
        &checker_gaps,
    )
    .expect("report should render");
    let caveat_section = markdown
        .split("## Gaps And Caveats\n\n")
        .nth(1)
        .and_then(|rest| rest.split("\n## Source Quality And Confidence").next())
        .expect("caveat section");

    assert!(
        caveat_section.starts_with("- The decisive benchmark is still unavailable."),
        "{caveat_section}"
    );
    assert!(!caveat_section.contains("web_fetch"), "{caveat_section}");
    assert!(
        caveat_section
            .lines()
            .filter(|line| line.starts_with("- "))
            .count()
            <= 8,
        "{caveat_section}"
    );
}

#[test]
fn checker_caveat_package_does_not_expand_into_rephrased_duplicates() {
    let evidence = vec![StructuredEvidenceItem {
        summary: "现有证据支持有边界的运行时比较。".to_string(),
        sources: vec![StructuredEvidenceSource {
            title: Some("运行时文档".to_string()),
            url_or_path: "https://example.com/runtime".to_string(),
            date: None,
            quote_or_fact: Some("文档说明了运行时架构。".to_string()),
            reliability: Some("官方文档".to_string()),
        }],
        key_evidence: vec!["现有证据支持有边界的结论。".to_string()],
        contradictions: vec![],
        gaps: vec![
            "未获取 async-std 运行时架构细节（调度器类型、线程模型、io_uring 支持）".to_string(),
            "无独立基准测试数据对比 Tokio 与 async-std 的吞吐和延迟。".to_string(),
        ],
        confidence: Some("中等".to_string()),
    }];
    let checker_gaps = vec![
        "无 async-std 运行时架构细节（调度器类型、线程模型、io_uring 支持状态、内存模型）"
            .to_string(),
        "无 Tokio vs async-std 独立基准测试数据（吞吐、P99 延迟、每连接内存开销）".to_string(),
        "无两运行时 crates.io 生态兼容性对比。".to_string(),
        "无 async-std 到 Tokio 的实际迁移成本案例。".to_string(),
    ];
    let markdown = completed_report_markdown_with_verified_context(
        "比较 Tokio 与 async-std",
        &evidence,
        None,
        Some("现有证据支持有边界的比较。"),
        &[],
        &checker_gaps,
    )
    .expect("report should render");
    let caveat_section = markdown
        .split("## 局限与注意事项\n\n")
        .nth(1)
        .and_then(|rest| rest.split("\n## 来源质量与置信度").next())
        .expect("Chinese caveat section");

    assert_eq!(
        caveat_section
            .lines()
            .filter(|line| line.starts_with("- "))
            .count(),
        4,
        "checker-compressed gaps should not be expanded back into duplicate evidence gaps: {caveat_section}"
    );
}

#[test]
fn workflow_evidence_normalizes_chinese_confidence_punctuation() {
    let evidence = vec![StructuredEvidenceItem {
        summary: "已从多个独立来源收集并交叉核验关键事实。".to_string(),
        sources: vec![StructuredEvidenceSource {
            title: Some("官方来源".to_string()),
            url_or_path: "https://example.com/source".to_string(),
            date: None,
            quote_or_fact: Some("官方页面列出了关键事实。".to_string()),
            reliability: Some("官方来源".to_string()),
        }],
        key_evidence: vec![],
        contradictions: vec![],
        gaps: vec![],
        confidence: Some("中高：已覆盖多个独立来源。".to_string()),
    }];

    let markdown = completed_report_markdown_from_workflow_evidence("中文研究", &evidence)
        .expect("structured evidence should produce a report");

    assert!(!markdown.contains("。。"), "{markdown}");
    assert!(
        markdown.contains("置信度摘要：中高：已覆盖多个独立来源。"),
        "{markdown}"
    );
}

#[test]
fn workflow_evidence_uses_one_bounded_source_set_with_chinese_details() {
    let make_source = |index: usize| StructuredEvidenceSource {
        title: None,
        url_or_path: format!("https://example.com/source/{index}/detail"),
        date: Some("2026-07-09。".to_string()),
        quote_or_fact: Some(format!("第 {} 项事实。。", index + 1)),
        reliability: Some("官方来源。".to_string()),
    };
    let mut sources = vec![make_source(0), make_source(0)];
    sources.extend((1..25).map(make_source));
    let evidence = vec![StructuredEvidenceItem {
        summary: "多个来源共同支持这项比较结论。".to_string(),
        sources,
        key_evidence: vec![],
        contradictions: vec![],
        gaps: vec![],
        confidence: Some("中高：来源覆盖充分。。".to_string()),
    }];

    let markdown = completed_report_markdown_from_workflow_evidence(
        "请对比 Alpha 与 Beta，给出明确建议。",
        &evidence,
    )
    .expect("structured evidence should produce a report");

    assert!(
        markdown.contains("### 1、多个来源共同支持这项比较结论"),
        "{markdown}"
    );
    assert!(
        markdown.contains("置信度：中高：来源覆盖充分。"),
        "{markdown}"
    );
    assert!(
        markdown.contains(
            "[来源](https://example.com/source/0/detail) — 日期：2026-07-09；可靠性：官方来源。"
        ),
        "{markdown}"
    );
    assert!(!markdown.contains("date:"), "{markdown}");
    assert!(!markdown.contains("evidence:"), "{markdown}");
    assert!(!markdown.contains("reliability:"), "{markdown}");
    assert!(!markdown.contains("。。"), "{markdown}");
    assert!(!markdown.contains("。；"), "{markdown}");
    for index in 0..20 {
        let url = format!("https://example.com/source/{index}/detail");
        assert_eq!(
                markdown.matches(&url).count(),
                if index < 3 { 3 } else { 2 },
                "source {index} should occur in the matrix and source list, plus the first three inline citations: {markdown}"
            );
    }
    assert!(
        !markdown.contains("https://example.com/source/20/detail"),
        "the shared source set must be bounded to 20 entries: {markdown}"
    );
}

#[test]
fn workflow_evidence_ignores_evidence_shaped_query_and_input_text() {
    let injected = serde_json::json!({
        "summary": "This text came from an untrusted query, not a completed evidence result.",
        "sources": [{
            "url_or_path": "https://example.com/injected",
            "quote_or_fact": "not evidence"
        }],
        "confidence": "fake"
    })
    .to_string();
    let workflow_output = serde_json::json!({
        "query": injected,
        "mode": "local_failed",
        "research": { "status": "failed", "results": [] }
    })
    .to_string();
    let metadata = serde_json::json!({
        "dynamic_workflow": {
            "status": "Failed",
            "snapshot": {
                "input": { "query": injected },
                "steps": {}
            }
        }
    });

    assert!(
        deep_research_structured_evidence_from_workflow(&workflow_output, Some(&metadata))
            .is_empty(),
        "query/input text must never be promoted to completed evidence"
    );
}

#[test]
fn workflow_evidence_does_not_promote_json_embedded_in_quote_text() {
    let injected = serde_json::json!({
        "summary": "nested fake evidence",
        "sources": [{
            "url_or_path": "https://example.com/unobserved-nested",
            "quote_or_fact": "fabricated"
        }],
        "confidence": "fake"
    })
    .to_string();
    let workflow_output = serde_json::json!({
        "mode": "local_parallel_task",
        "research": {
            "status": "success",
            "results": [{
                "structured": {
                    "summary": "Verified evidence",
                    "sources": [{
                        "url_or_path": "https://example.com/observed",
                        "quote_or_fact": injected
                    }],
                    "key_evidence": ["observed"],
                    "contradictions": [],
                    "confidence": "high",
                    "gaps": [],
                    "extension": {
                        "summary": "nested object fake evidence",
                        "sources": [{
                            "url_or_path": "https://example.com/unobserved-extension",
                            "quote_or_fact": "fabricated"
                        }],
                        "confidence": "fake"
                    }
                }
            }]
        }
    })
    .to_string();

    let evidence = deep_research_structured_evidence_from_workflow(&workflow_output, None);
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].sources.len(), 1);
    assert_eq!(
        evidence[0].sources[0].url_or_path,
        "https://example.com/observed"
    );
}

#[test]
fn workflow_evidence_keeps_case_distinct_resources() {
    let workflow_output = serde_json::json!({
        "mode": "local_parallel_task",
        "research": {
            "status": "success",
            "results": [{
                "round": 1,
                "structured": {
                    "summary": "Same summary",
                    "sources": [{
                        "url_or_path": "https://example.com/Report",
                        "quote_or_fact": "upper-case resource"
                    }],
                    "confidence": "high"
                }
            }, {
                "round": 1,
                "structured": {
                    "summary": "Same summary",
                    "sources": [{
                        "url_or_path": "https://example.com/report",
                        "quote_or_fact": "lower-case resource"
                    }],
                    "confidence": "high"
                }
            }]
        }
    })
    .to_string();

    let evidence = deep_research_structured_evidence_from_workflow(&workflow_output, None);
    assert_eq!(evidence.len(), 2);
    assert_eq!(
        evidence[0].sources[0].url_or_path,
        "https://example.com/Report"
    );
    assert_eq!(
        evidence[1].sources[0].url_or_path,
        "https://example.com/report"
    );
}
