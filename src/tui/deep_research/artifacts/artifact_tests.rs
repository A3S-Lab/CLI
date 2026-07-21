#[cfg(test)]
mod source_anchor_tests {
    use super::*;

    #[test]
    fn recovery_report_preserves_an_observed_seed_from_legacy_failed_output() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-seed-recovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let output = serde_json::json!({
            "mode": "local_parallel_task_failed",
            "research": {
                "status": "failed",
                "metadata": {
                    "task_count": 3,
                    "success_count": 0,
                    "failed_count": 3
                },
                "results": [],
                "warnings": {
                    "failed_tasks": [{"task_id": "official", "error_summary": "timed out"},
                        {"task_id": "independent", "error_summary": "interrupted"},
                        {"task_id": "cross-check", "error_summary": "timed out"}]
                }
            },
            "seed_research": {
                "status": "success",
                "algorithm": "direct_web_search_fetch",
                "metadata": {
                    "task_count": 1,
                    "success_count": 1,
                    "failed_count": 0,
                    "source_count": 1
                },
                "results": [{
                    "success": true,
                    "structured": {
                        "summary": "Direct collection preserved a traceable source before delegated fan-out failed.",
                        "sources": [{
                            "title": "Preserved direct source",
                            "url_or_path": "https://example.com/direct-seed",
                            "date": "2026-07-12",
                            "quote_or_fact": "The direct source was fetched before the parallel deadline.",
                            "reliability": "deterministic fixture"
                        }],
                        "confidence": "medium",
                        "key_evidence": ["The seed source is traceable."],
                        "contradictions": [],
                        "gaps": ["Delegated corroboration did not finish."]
                    }
                }]
            }
        })
        .to_string();

        let artifacts = materialize_deep_research_recovery_report(
            &workspace,
            "preserve direct seed",
            "Evidence fan-out stopped at its bounded deadline.",
            &output,
            None,
        )
        .expect("failed fan-out should retain direct-web evidence in recovery artifacts");
        let markdown = std::fs::read_to_string(&artifacts.markdown).unwrap();
        assert!(
            markdown.contains("https://example.com/direct-seed"),
            "{markdown}"
        );
        assert!(
            markdown.contains("only 0 of 3 planned research tasks produced validated evidence"),
            "{markdown}"
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn workflow_summary_surfaces_direct_web_coverage_and_freshness_gaps() {
        let direct = serde_json::json!({
            "mode": "direct_web",
            "research": {
                "metadata": {
                    "success_count": 1,
                    "task_count": 1,
                    "source_count": 2,
                    "host_count": 2,
                    "fetched_count": 1,
                    "fetched_host_count": 1,
                    "evidence_selection_mode": "semantic_chunk_ids",
                    "freshness_required": true,
                    "dated_source_count": 0
                }
            }
        })
        .to_string();

        let summary = workflow_evidence_summary(&direct).expect("direct summary");
        assert!(
            summary.contains("2 semantically selected source(s) across 2 host(s), 1 fetched across 1 host(s)"),
            "{summary}"
        );
        assert!(summary.contains("0/2 source(s) are dated"), "{summary}");

        let hybrid = serde_json::json!({
            "mode": "hybrid_direct_web_parallel",
            "research": { "metadata": { "success_count": 2, "task_count": 3 } },
            "seed_research": {
                "metadata": {
                    "source_count": 4,
                    "host_count": 3,
                    "fetched_count": 2,
                    "fetched_host_count": 2,
                    "evidence_selection_mode": "semantic_chunk_ids"
                }
            }
        })
        .to_string();
        let summary = workflow_evidence_summary(&hybrid).expect("hybrid summary");
        assert!(
            summary.contains("2/3 delegated research tasks"),
            "{summary}"
        );
        assert!(
            summary.contains("4 semantically selected source(s) across 3 host(s), 2 fetched across 2 host(s)"),
            "{summary}"
        );
    }

    #[test]
    fn source_anchors_require_a_url_or_path_shape() {
        assert_eq!(
            normalize_research_source_anchor("https://example.com/research"),
            Some("https://example.com/research".to_string())
        );
        assert_eq!(
            normalize_research_source_anchor("README.md"),
            Some("readme.md".to_string())
        );
        assert_eq!(
            normalize_research_source_anchor("src/tui/mod.rs:42"),
            Some("src/tui/mod.rs:42".to_string())
        );
        assert!(normalize_research_source_anchor("not checked").is_none());
        assert!(normalize_research_source_anchor("official source").is_none());
        assert!(
            normalize_research_source_anchor("https://search.brave.com/search?q=tokio").is_none()
        );
        assert!(
            normalize_research_source_anchor("https://www.google.com/search?q=tokio").is_none()
        );
        assert!(normalize_research_source_anchor("../outside.txt").is_none());
        assert!(normalize_research_source_anchor("/etc/passwd").is_none());
    }

    #[test]
    fn canonical_source_anchors_preserve_safe_identity_query_parameters() {
        let kbs = "https://world.kbs.co.kr/service/news_view.htm?lang=e&Seq_Code=155851";
        assert_eq!(
            canonical_research_source_anchor(kbs).as_deref(),
            Some("https://world.kbs.co.kr/service/news_view.htm?lang=e&seq_code=155851")
        );

        let sanitized = canonical_research_source_anchor(
            "https://example.com/article?utm_source=campaign&token=secret&id=article-1&secret=hidden&lang=zh#fragment",
        )
        .expect("the safe article identity should remain traceable");
        assert_eq!(
            sanitized,
            "https://example.com/article?id=article-1&lang=zh"
        );
        for removed in ["utm_", "token", "secret", "fragment"] {
            assert!(!sanitized.contains(removed), "{sanitized}");
        }
    }

    #[test]
    fn report_citations_are_structural_and_do_not_depend_on_heading_language() {
        let markdown = "# Report\n\n## 任意标题\n\n| Value |\n| --- |\n| docs/plain-text.md |\n\n[证据](docs/cited.md)\n";
        let targets =
            super::super::deep_research_report_audit::report_citation_targets(markdown, "");
        assert!(targets.contains("docs/cited.md"), "{targets:?}");
        assert!(!targets.contains("docs/plain-text.md"), "{targets:?}");
    }

    #[test]
    fn report_citation_targets_do_not_authorize_a_sanitized_variant() {
        let markdown = "# Report\n\n## Sources\n\n- https://user:password@example.com/source?token=secret#fragment\n";
        let targets =
            super::super::deep_research_report_audit::report_citation_targets(markdown, "");
        assert!(!targets.contains("https://example.com/source"), "{targets:?}");
    }

    #[test]
    fn structural_target_scanners_preserve_balanced_url_parentheses() {
        let target = "https://example.com/spec_(v2)";
        assert_eq!(http_source_targets(&format!("See {target}.")), vec![target]);
        assert_eq!(
            http_source_targets("See https://example.com/plain)."),
            vec!["https://example.com/plain"]
        );

        let targets = super::super::deep_research_report_audit::report_citation_targets(
            &format!("[specification]({target})"),
            "",
        );
        assert!(targets.contains(target), "{targets:?}");

        let plain_targets = super::super::deep_research_report_audit::report_citation_targets(
            "[plain](https://example.com/plain))",
            "",
        );
        assert!(
            plain_targets.contains("https://example.com/plain"),
            "{plain_targets:?}"
        );

        let qualified_query =
            "Analyze https://user:password@example.com/resource?token=secret#section".to_string();
        let qualified_markdown = format!("# {}\n", markdown_plain_text(&qualified_query));
        let qualified_html =
            deep_research_completed_report_html(&qualified_query, &qualified_markdown);
        for secret in ["user", "password", "token=secret", "#section"] {
            assert!(!qualified_markdown.contains(secret), "{qualified_markdown}");
            assert!(!qualified_html.contains(secret), "{qualified_html}");
        }
        assert!(
            qualified_markdown.contains("https://example.com/resource"),
            "{qualified_markdown}"
        );
        let title_end = "Analyze https://example.com/resource</h1>";
        assert!(qualified_html.contains(title_end), "{qualified_html}");
    }

    #[test]
    fn recovery_answer_text_sanitizes_url_credentials_query_and_fragment() {
        let answer = format!(
            "# Recovery\n\n## Findings\n\n{}\n\n## Sources\n\n- https://user:password@example.com/source?token=secret#fragment\n\n## Confidence\n\nLow confidence.",
            "Substantive recovery analysis with explicit caveats and limitations. ".repeat(4)
        );
        let recovered = deep_research_recovery_result_text(&answer, "");
        assert!(recovered.contains("https://example.com/source"));
        assert!(!recovered.contains("password"), "{recovered}");
        assert!(!recovered.contains("token=secret"), "{recovered}");
        assert!(!recovered.contains("#fragment"), "{recovered}");
    }

    #[test]
    fn recovery_sources_include_metadata_only_observed_anchors() {
        let metadata = serde_json::json!({
            "dynamic_workflow": {
                "snapshot": {
                    "steps": {
                        "local_research": {
                            "output": {
                                "metadata": {
                                    "results": [{
                                        "structured": {
                                            "summary": "Recovered source-backed evidence.",
                                            "sources": [{
                                                "title": "Recovered Source",
                                                "url_or_path": "https://example.com/metadata-only",
                                                "quote_or_fact": "The source was preserved in workflow metadata."
                                            }],
                                            "confidence": "medium"
                                        }
                                    }]
                                }
                            }
                        }
                    }
                }
            }
        });

        let sources = deep_research_recovery_sources("", Some(&metadata), "metadata-recovery");
        assert!(
            sources.contains("https://example.com/metadata-only"),
            "{sources}"
        );
        assert!(!sources.contains("No traceable"), "{sources}");
        let status = deep_research_recovery_evidence_status("", Some(&metadata));
        assert!(status.contains("recovery metadata preserved 1"), "{status}");
        assert!(status.contains("successful research tools"), "{status}");

        let root = std::env::temp_dir().join(format!(
            "a3s-recovery-metadata-only-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let artifacts = materialize_deep_research_recovery_report(
            &root,
            "metadata-only timeout",
            "##",
            "dynamic_workflow timed out while gathering evidence",
            Some(&metadata),
        )
        .expect("metadata-only timeout recovery should materialize");
        let markdown = std::fs::read_to_string(&artifacts.markdown).unwrap();
        assert!(
            markdown.contains("https://example.com/metadata-only"),
            "{markdown}"
        );
        assert!(
            markdown.contains("recovery metadata preserved 1"),
            "{markdown}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_sources_surface_bounded_omission_count() {
        let sources = (0..14)
            .map(|index| {
                serde_json::json!({
                    "title": format!("Source {index}"),
                    "url_or_path": format!("https://example.com/source-{index}"),
                    "quote_or_fact": format!("Observed fact {index}")
                })
            })
            .collect::<Vec<_>>();
        let output = serde_json::json!({
            "mode": "local_parallel_task",
            "research": {
                "status": "partial_success",
                "results": [{
                    "structured": {
                        "summary": "Many observed sources",
                        "sources": sources,
                        "confidence": "medium"
                    }
                }]
            }
        })
        .to_string();

        let rendered = deep_research_recovery_sources(&output, None, "bounded-sources");
        assert_eq!(rendered.matches("https://example.com/source-").count(), 12);
        assert!(
            rendered.contains("2 additional captured source entries were omitted"),
            "{rendered}"
        );
    }

    #[test]
    fn recovery_sources_add_omissions_across_bounded_evidence_items() {
        let evidence = (0..2)
            .map(|round| {
                let sources = (0..14)
                    .map(|index| {
                        serde_json::json!({
                            "title": format!("Source {round}-{index}"),
                            "url_or_path": format!("https://example.com/source-{round}-{index}"),
                            "quote_or_fact": format!("Observed fact {round}-{index}")
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "round": round + 1,
                    "structured": {
                        "summary": format!("Observed evidence round {round}"),
                        "sources": sources,
                        "confidence": "medium"
                    }
                })
            })
            .collect::<Vec<_>>();
        let output = serde_json::json!({
            "mode": "local_parallel_task",
            "research": {
                "status": "partial_success",
                "results": evidence.clone()
            }
        })
        .to_string();
        // Runtime metadata can repeat the same evidence projection. It must
        // not double the omitted count reported from workflow output.
        let metadata = serde_json::json!({ "results": evidence });

        let rendered = deep_research_recovery_sources(&output, Some(&metadata), "bounded-rounds");
        assert_eq!(rendered.matches("https://example.com/source-").count(), 12);
        assert!(
            rendered.contains("16 additional captured source entries were omitted"),
            "{rendered}"
        );
    }

    #[test]
    fn recovery_sources_surface_bounded_evidence_item_count() {
        let evidence = (0..20)
            .map(|index| {
                serde_json::json!({
                    "round": index + 1,
                    "structured": {
                        "summary": format!("Observed evidence {index}"),
                        "sources": [{
                            "title": format!("Source {index}"),
                            "url_or_path": format!("https://example.com/evidence-{index}"),
                            "quote_or_fact": format!("Observed fact {index}")
                        }],
                        "confidence": "medium"
                    }
                })
            })
            .collect::<Vec<_>>();
        let output = serde_json::json!({
            "mode": "local_parallel_task",
            "research": {
                "status": "partial_success",
                "results": evidence.clone()
            }
        })
        .to_string();
        let metadata = serde_json::json!({ "results": evidence });

        let rendered = deep_research_recovery_sources(&output, Some(&metadata), "bounded-evidence");
        assert_eq!(
            rendered.matches("https://example.com/evidence-").count(),
            12
        );
        assert!(
            rendered.contains("6 additional captured source entries were omitted"),
            "{rendered}"
        );
        assert!(
            rendered.contains("2 additional evidence items were omitted"),
            "{rendered}"
        );
    }

    #[test]
    fn completed_report_heading_treats_query_as_plain_text() {
        let query = "runtime comparison\n\n## Sources\n\n- https://example.com/injected";
        let answer = format!(
            "## Findings\n\n{}\n\n## Sources\n\n- https://example.com/observed\n\n## Confidence\n\nConfidence is medium.",
            "Source-backed analysis with explicit limitations and caveats. ".repeat(4)
        );

        let markdown = normalize_report_markdown_candidate(query, &answer)
            .expect("substantive answer should produce report Markdown");

        assert_eq!(markdown.matches("\n## Sources\n").count(), 1, "{markdown}");
        assert_eq!(
            markdown.lines().next(),
            Some("# runtime comparison ## Sources - https://example.com/injected")
        );
        let html = deep_research_completed_report_html(query, &markdown);
        assert!(
            html.contains(
                "<title>runtime comparison ## Sources - https://example.com/injected</title>"
            ),
            "{html}"
        );
    }

    #[test]
    fn workflow_log_filter_respects_directory_boundaries() {
        assert!(deep_research_output_has_internal_leak(
            "diagnostic: .a3s/workflow/run-123.jsonl"
        ));
        assert!(deep_research_output_has_internal_leak(
            r"diagnostic: .a3s\workflow\run-123.jsonl"
        ));
        assert!(deep_research_output_has_internal_leak(
            "history is stored under `.a3s/workflow`"
        ));
        assert!(!deep_research_output_has_internal_leak(
            "source: .a3s/workflows/operating-procedure.md"
        ));
        assert!(!deep_research_output_has_internal_leak(
            "source: .a3s/workflow.asset.json"
        ));
    }
}

#[cfg(all(test, unix))]
mod artifact_boundary_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn recovery_report_refuses_symlinked_slug_directory() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-symlink-boundary-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let research = workspace.join(".a3s/research");
        std::fs::create_dir_all(&research).unwrap();
        symlink(&workspace, research.join("escape")).unwrap();

        let error = materialize_deep_research_recovery_report(
            &workspace,
            "escape",
            "no completed synthesis",
            "",
            None,
        )
        .expect_err("a symlinked report slug must be rejected before any write");

        assert!(
            error.contains("symlinked DeepResearch directory"),
            "{error}"
        );
        assert!(!workspace.join("report.md").exists());
        assert!(!workspace.join("index.html").exists());
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn report_marker_refuses_symlinked_markdown_sibling() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-symlinked-markdown-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let report_dir = workspace.join(".a3s/research/symlinked-markdown");
        std::fs::create_dir_all(&report_dir).unwrap();
        std::fs::write(
            workspace.join("README.md"),
            "# Workspace file\n\n## Findings\n\nThis workspace document has enough report-like text to expose a symlink validation bug.\n\n## Sources\n\n- https://example.com/source\n\n## Confidence\n\nConfidence is high for this deterministic fixture.\n",
        )
        .unwrap();
        std::fs::write(
            report_dir.join("index.html"),
            "<!doctype html><html><body><h1>Report</h1><h2>Findings</h2><p>This HTML has enough source-backed analysis and confidence detail for validation.</p><h2>Sources</h2><p>https://example.com/source</p><h2>Confidence</h2><p>High confidence.</p></body></html>",
        )
        .unwrap();
        symlink(workspace.join("README.md"), report_dir.join("report.md")).unwrap();

        assert!(
            research_report_artifacts_from_output(
                "A3S_RESEARCH_VIEW: .a3s/research/symlinked-markdown/index.html",
                &workspace,
            )
            .is_none(),
            "a report marker must not smuggle an arbitrary workspace file through report.md"
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn inline_code_outside_sources_is_not_a_local_source_citation() {
        let query = "Compare two runtimes";
        let observed = "https://example.com/runtime-status";
        let markdown = format!(
            "# Runtime report\n\n## Findings\n\nThe unavailable candidate `crates.io/crates/missing` is a limitation, and `owner/repository` is a repository slug rather than a citation.\n\n## Sources\n\n- [Runtime status]({observed})\n\n## Limitations\n\nConfidence is limited by the unavailable candidate.\n"
        );
        let html = deep_research_completed_report_html(query, &markdown);
        let citations =
            super::super::deep_research_report_audit::report_citation_targets(&markdown, &html);

        assert!(citations.contains(observed), "{citations:?}");
        assert!(
            !citations.contains("crates.io/crates/missing"),
            "{citations:?}"
        );
        assert!(
            !citations.contains("owner/repository"),
            "{citations:?}"
        );
    }

    #[test]
    fn recovery_report_refuses_hard_linked_artifact_target() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-hard-linked-artifact-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let report_dir = workspace.join(".a3s/research/hard-linked-artifact");
        std::fs::create_dir_all(&report_dir).unwrap();
        let protected = workspace.join("README.md");
        std::fs::write(&protected, "protected workspace content").unwrap();
        std::fs::hard_link(&protected, report_dir.join("report.md")).unwrap();

        let error = materialize_deep_research_recovery_report(
            &workspace,
            "hard linked artifact",
            "recovery",
            "workflow failed",
            None,
        )
        .expect_err("a hard-linked report target must be rejected before writing");
        assert!(
            error.contains("hard-linked DeepResearch artifact"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(&protected).unwrap(),
            "protected workspace content"
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    fn assessed_report_depth_workflow() -> String {
        use a3s::research::{
            replay, CompletionCriterionAssessment, ContractAssessmentStatus, EvidenceRef,
            InquiryEvent, InquiryLimits, Question, ResearchContractAssessment, ResearchMethod,
            ResearchObligation, ResearchObligationAssessment, StopConditionAssessment,
        };

        let obligations = vec![
            ResearchObligation::new(
                "obligation:mechanism",
                "Mechanism and causes",
                "Establish the documented mechanism and causes",
                true,
                vec!["The mechanism is supported by traceable evidence".to_string()],
            ),
            ResearchObligation::new(
                "obligation:consequences",
                "Counterevidence and consequences",
                "Establish counterevidence and consequences",
                true,
                vec!["Consequences are supported by traceable evidence".to_string()],
            ),
        ];
        let mut mechanism = Question::queued(
            "question:mechanism",
            None,
            "What does the evidence establish about the mechanism?",
        );
        mechanism.obligation_ids = vec!["obligation:mechanism".to_string()];
        let mut consequences = Question::queued(
            "question:consequences",
            None,
            "What does the evidence establish about the consequences?",
        );
        consequences.obligation_ids = vec!["obligation:consequences".to_string()];
        let events = vec![
            InquiryEvent::StrategySelected {
                method: ResearchMethod::Focused,
            },
            InquiryEvent::ResearchObligationsCommitted {
                obligations,
                stop_conditions: vec![
                    "Both material obligations are closed by accepted evidence".to_string(),
                ],
            },
            InquiryEvent::QuestionsQueued {
                questions: vec![mechanism, consequences],
            },
            InquiryEvent::EvidenceAccepted {
                evidence: EvidenceRef::new(
                    "evidence:mechanism",
                    vec!["claim:mechanism".to_string()],
                    vec!["source:mechanism".to_string()],
                ),
            },
            InquiryEvent::EvidenceAccepted {
                evidence: EvidenceRef::new(
                    "evidence:consequences",
                    vec!["claim:consequences".to_string()],
                    vec!["source:consequences".to_string()],
                ),
            },
            InquiryEvent::QuestionAnswered {
                question_id: "question:mechanism".to_string(),
                answer: "The accepted evidence establishes the mechanism.".to_string(),
                evidence_ids: vec!["evidence:mechanism".to_string()],
            },
            InquiryEvent::QuestionAnswered {
                question_id: "question:consequences".to_string(),
                answer: "The accepted evidence establishes the consequences.".to_string(),
                evidence_ids: vec!["evidence:consequences".to_string()],
            },
            InquiryEvent::ResearchContractAssessed {
                assessment: ResearchContractAssessment {
                    obligations: vec![
                        ResearchObligationAssessment {
                            obligation_id: "obligation:mechanism".to_string(),
                            criteria: vec![CompletionCriterionAssessment {
                                criterion_index: 0,
                                status: ContractAssessmentStatus::Satisfied,
                                rationale: "The mechanism evidence satisfies the criterion."
                                    .to_string(),
                                evidence_ids: vec!["evidence:mechanism".to_string()],
                            }],
                            primary_source: None,
                            independent_corroboration: None,
                        },
                        ResearchObligationAssessment {
                            obligation_id: "obligation:consequences".to_string(),
                            criteria: vec![CompletionCriterionAssessment {
                                criterion_index: 0,
                                status: ContractAssessmentStatus::Satisfied,
                                rationale: "The consequences evidence satisfies the criterion."
                                    .to_string(),
                                evidence_ids: vec!["evidence:consequences".to_string()],
                            }],
                            primary_source: None,
                            independent_corroboration: None,
                        },
                    ],
                    stop_conditions: vec![StopConditionAssessment {
                        condition_index: 0,
                        status: ContractAssessmentStatus::Satisfied,
                        rationale: "Both material obligations are evidence-answered.".to_string(),
                        evidence_ids: vec![
                            "evidence:mechanism".to_string(),
                            "evidence:consequences".to_string(),
                        ],
                    }],
                    diagnostics: Vec::new(),
                },
            },
        ];
        let state =
            replay(&events, &InquiryLimits::default()).expect("valid report-depth inquiry fixture");
        serde_json::json!({
            "mode": "inquiry_collection_wave",
            "execution": {
                "mode": "collect_only",
                "terminal_authority": "host_inquiry_reducer"
            },
            "inquiry": {
                "events": events,
                "state": state
            }
        })
        .to_string()
    }

    fn benchmark_publication_workflow(qualified: bool) -> String {
        use a3s::research::{
            replay, CompletionCriterionAssessment, ContractAssessmentStatus, EvidenceRef,
            InquiryEvent, InquiryLimits, OutlineSection, Question, ResearchContractAssessment,
            ResearchMethod, ResearchObligation, ResearchObligationAssessment, ResearchOutline,
            StopConditionAssessment,
        };

        let mut workflow = serde_json::json!({
            "plan": {
                "report_title": "Benchmark boundary",
                "tracks": [{
                    "id": "obligation:benchmark-boundary",
                    "title": "Benchmark boundary",
                    "focus": "Establish only the documented benchmark boundary",
                    "material": true,
                    "completion_criteria": [
                        "The documented range is retained without inventing a product threshold"
                    ],
                    "evidence_requirements": {
                        "primary_source_required": false,
                        "independent_corroboration_required": false
                    }
                }],
                "stop_conditions": [
                    "The documented range is established and any decision threshold remains bounded"
                ]
            },
            "research": {
                "status": "success",
                "results": [{
                    "success": true,
                    "structured": {
                        "summary": "The benchmark has a documented range.",
                        "sources": [{
                            "title": "Published benchmark",
                            "url_or_path": "https://example.com/benchmark",
                            "quote_or_fact": "The benchmark covers 1M-10M vectors.",
                            "reliability": "Published source"
                        }],
                        "key_evidence": ["The benchmark covers 1M-10M vectors."],
                        "contradictions": [],
                        "gaps": [],
                        "confidence": "medium"
                    }
                }]
            }
        });
        let accepted = accepted_evidence_ledger(&workflow.to_string(), None)
            .into_iter()
            .next()
            .expect("benchmark evidence");
        let claim_ids = accepted
            .claims
            .iter()
            .map(|claim| claim.id.clone())
            .collect::<Vec<_>>();
        let source_ids = accepted
            .sources
            .iter()
            .map(|source| source.id.clone())
            .collect::<Vec<_>>();
        let evidence_id = accepted.id.clone();
        let obligation_id = "obligation:benchmark-boundary";
        let question_id = "question:benchmark-boundary";
        let section_id = "section:benchmark-boundary";
        let mut question = Question::queued(
            question_id,
            None,
            "What benchmark boundary does the accepted source establish?",
        );
        question.obligation_ids = vec![obligation_id.to_string()];
        let status = if qualified {
            ContractAssessmentStatus::Bounded
        } else {
            ContractAssessmentStatus::Satisfied
        };
        let events = vec![
            InquiryEvent::StrategySelected {
                method: ResearchMethod::Focused,
            },
            InquiryEvent::ResearchObligationsCommitted {
                obligations: vec![ResearchObligation::new(
                    obligation_id,
                    "Benchmark boundary",
                    "Establish only the documented benchmark boundary",
                    true,
                    vec![
                        "The documented range is retained without inventing a product threshold"
                            .to_string(),
                    ],
                )],
                stop_conditions: vec![
                    "The documented range is established and any decision threshold remains bounded"
                        .to_string(),
                ],
            },
            InquiryEvent::QuestionsQueued {
                questions: vec![question],
            },
            InquiryEvent::EvidenceAccepted {
                evidence: EvidenceRef::new(
                    evidence_id.clone(),
                    claim_ids.clone(),
                    source_ids.clone(),
                ),
            },
            InquiryEvent::QuestionAnswered {
                question_id: question_id.to_string(),
                answer: "The accepted evidence establishes a 1M-10M benchmark range.".to_string(),
                evidence_ids: vec![evidence_id.clone()],
            },
            InquiryEvent::ResearchContractAssessed {
                assessment: ResearchContractAssessment {
                    obligations: vec![ResearchObligationAssessment {
                        obligation_id: obligation_id.to_string(),
                        criteria: vec![CompletionCriterionAssessment {
                            criterion_index: 0,
                            status,
                            rationale: if qualified {
                                "The range is supported, but it does not establish a product decision threshold."
                            } else {
                                "The accepted evidence establishes the requested documented range."
                            }
                            .to_string(),
                            evidence_ids: vec![evidence_id.clone()],
                        }],
                        primary_source: None,
                        independent_corroboration: None,
                    }],
                    stop_conditions: vec![StopConditionAssessment {
                        condition_index: 0,
                        status,
                        rationale: if qualified {
                            "No accepted evidence establishes a below-range product threshold."
                        } else {
                            "The requested documented range is established."
                        }
                        .to_string(),
                        evidence_ids: vec![evidence_id.clone()],
                    }],
                    diagnostics: Vec::new(),
                },
            },
            InquiryEvent::OutlineCommitted {
                outline: ResearchOutline {
                    sections: vec![OutlineSection {
                        id: section_id.to_string(),
                        heading: "Evidence".to_string(),
                        purpose: "State the supported range and its decision boundary.".to_string(),
                        perspective_ids: Vec::new(),
                        question_ids: vec![question_id.to_string()],
                        claim_ids: claim_ids.clone(),
                        source_ids: source_ids.clone(),
                        composition_hint: "Lead with the supported range.".to_string(),
                    }],
                },
            },
            InquiryEvent::SectionDrafted {
                section_id: section_id.to_string(),
                content: "The published benchmark covers 1M-10M vectors.".to_string(),
                citation_ids: claim_ids
                    .iter()
                    .chain(source_ids.iter())
                    .cloned()
                    .collect(),
            },
            InquiryEvent::AuditCompleted {
                passed: true,
                issues: Vec::new(),
            },
        ];
        let state = replay(&events, &InquiryLimits::default())
            .expect("valid benchmark publication inquiry");
        workflow["mode"] = serde_json::json!("inquiry_collection_wave");
        workflow["execution"] = serde_json::json!({
            "mode": "collect_only",
            "terminal_authority": "host_inquiry_reducer"
        });
        workflow["inquiry"] = serde_json::json!({
            "events": events,
            "state": state
        });
        workflow.to_string()
    }

    #[test]
    fn generated_report_depth_gate_requires_every_exact_inquiry_obligation_id() {
        let workflow = assessed_report_depth_workflow();
        let coverage = |obligation_id: &str| ReportTrackCoverage {
            obligation_id: obligation_id.to_string(),
            status: ReportTrackStatus::Answered,
            finding: format!("A supported finding for {obligation_id}."),
            interpretation: format!("The evidence explains why {obligation_id} matters."),
            implication: "The finding changes the reader's decision boundary.".to_string(),
            uncertainty: "The conclusion remains bounded by source recency.".to_string(),
        };
        let mut generated = GeneratedDeepResearchReport {
            markdown: "# Report\n\nA substantive source-backed report body with analysis, implications, confidence, and limitations.\n\n## Sources\n\n- https://example.com/source"
                .to_string(),
            editorial: ReportEditorialPlan {
                thesis: "The evidence supports a bounded answer to the investigation.".to_string(),
                track_coverage: vec![coverage("obligation:mechanism")],
            },
            presentation: ReportPresentation {
                rationale: "An analytical composition fits the causal comparison and decision audience."
                    .to_string(),
                ..ReportPresentation::default()
            },
        };

        let error = validate_generated_report_depth(&generated, &workflow).unwrap_err();
        assert!(
            error.contains("obligation:consequences"),
            "{error}"
        );

        generated
            .editorial
            .track_coverage
            .push(coverage("obligation:consequences"));
        validate_generated_report_depth(&generated, &workflow).unwrap();

        generated.editorial.track_coverage[1].obligation_id =
            "counterevidence-and-consequences".to_string();
        let error = validate_generated_report_depth(&generated, &workflow).unwrap_err();
        assert!(error.contains("unknown obligation ID"), "{error}");
    }

    #[test]
    fn generated_report_publication_does_not_rewrite_closed_evidence_prose_lexically() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-quantity-gate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let query = "Compare the documented benchmark boundary";
        let workflow = benchmark_publication_workflow(false);
        let generated = GeneratedDeepResearchReport {
            markdown: "# Benchmark boundary\n\nUse the product below 1M vectors.\n\n## Evidence\n\nThe published benchmark covers 1M-10M vectors.\n\n## Sources\n\n- https://example.com/benchmark"
                .to_string(),
            editorial: ReportEditorialPlan {
                thesis: "The retained evidence supports a bounded benchmark comparison."
                    .to_string(),
                track_coverage: vec![ReportTrackCoverage {
                    obligation_id: "obligation:benchmark-boundary".to_string(),
                    status: ReportTrackStatus::Bounded,
                    finding: "The benchmark publishes a tested range.".to_string(),
                    interpretation: "The tested range does not establish a lower threshold."
                        .to_string(),
                    implication: "A product decision needs a workload-specific test.".to_string(),
                    uncertainty: "No evidence establishes a below-1M cutoff.".to_string(),
                }],
            },
            presentation: ReportPresentation {
                rationale: "A compact analytical briefing fits a bounded benchmark decision."
                    .to_string(),
                ..ReportPresentation::default()
            },
        };

        let artifacts = materialize_deep_research_completed_report_from_generation(
            &workspace, query, &generated, &workflow, None,
        )
        .expect("closed-evidence publication should not use language-specific text matching");
        let markdown = std::fs::read_to_string(&artifacts.markdown).unwrap();
        assert!(markdown.contains("below 1M"), "{markdown}");
        assert!(markdown.contains("covers 1M-10M"), "{markdown}");
        assert!(!markdown.contains("## Evidence boundary"), "{markdown}");

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn recovery_report_demotes_an_embedded_report_title() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-recovery-title-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let answer = "# A source-backed partial report\n\n## Findings\n\nThe collected evidence supports a bounded partial finding with enough explanation to preserve for the reader, but not enough coverage for a completed report.\n\n## Sources\n\n- https://example.com/partial\n\n## Limitations\n\nThe uncollected dimensions remain unknown and require a later retry.";

        let artifacts = materialize_deep_research_recovery_report(
            &workspace,
            "partial report with a title",
            answer,
            "workflow failed",
            None,
        )
        .expect("a useful partial synthesis should converge to a valid recovery artifact");
        let html = std::fs::read_to_string(&artifacts.html).unwrap();
        assert_eq!(html.to_ascii_lowercase().matches("<h1").count(), 1);
        assert!(html.contains("A source-backed partial report"));

        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn recovery_report_preserves_safe_validation_feedback_without_classifying_its_words() {
        let result = deep_research_recovery_result_text(
            "report does not cite every source declared by its closed evidence plan",
            r#"{"mode":"direct_web_degraded","research":{"status":"partial_success"}}"#,
        );
        assert!(result.contains("does not cite every source"), "{result}");
    }

    #[test]
    fn recovery_report_preflights_both_targets_before_replacing_either_file() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-pair-preflight-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let report_dir = workspace.join(".a3s/research/pair-preflight");
        std::fs::create_dir_all(&report_dir).unwrap();
        let old_markdown =
            "# Previous valid report\n\nThis content must survive a rejected HTML target.";
        std::fs::write(report_dir.join("report.md"), old_markdown).unwrap();
        let protected = workspace.join("protected.html");
        std::fs::write(&protected, "protected HTML").unwrap();
        std::fs::hard_link(&protected, report_dir.join("index.html")).unwrap();

        let error = materialize_deep_research_recovery_report(
            &workspace,
            "pair preflight",
            "recovery",
            "workflow failed",
            None,
        )
        .expect_err("an unsafe HTML target must reject the whole artifact pair");

        assert!(
            error.contains("hard-linked DeepResearch artifact"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(report_dir.join("report.md")).unwrap(),
            old_markdown,
            "preflight failure must preserve the previous Markdown generation"
        );
        assert_eq!(
            std::fs::read_to_string(&protected).unwrap(),
            "protected HTML"
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }
}
