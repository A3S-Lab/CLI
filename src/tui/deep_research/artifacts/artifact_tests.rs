#[cfg(test)]
mod source_anchor_tests {
    use super::*;

    #[test]
    fn recovery_does_not_replace_an_existing_completed_report() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-recovery-quality-monotonic-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let query = "quality monotonic report";
        let completed_output = serde_json::json!({
            "mode": "direct_web",
            "research": {
                "status": "success",
                "results": [{
                    "success": true,
                    "structured": {
                        "summary": "The completed report preserves a source-backed finding across later degraded retries.",
                        "sources": [{
                            "title": "Completed source",
                            "url_or_path": "https://example.com/completed",
                            "date": "2026-07-12",
                            "quote_or_fact": "This completed evidence must not be replaced by a recovery artifact.",
                            "reliability": "Deterministic fixture."
                        }],
                        "key_evidence": ["The completed report passed its evidence gate."],
                        "contradictions": [],
                        "confidence": "high",
                        "gaps": []
                    }
                }]
            }
        })
        .to_string();
        let completed = materialize_deep_research_completed_report_from_workflow_evidence(
            &workspace,
            query,
            &completed_output,
            None,
        )
        .expect("completed fixture should materialize");
        let previous_markdown = std::fs::read_to_string(&completed.markdown).unwrap();
        let previous_html = std::fs::read_to_string(&completed.html).unwrap();

        let recovery = materialize_deep_research_recovery_report(
            &workspace,
            query,
            "A later collection attempt degraded.",
            r#"{"mode":"direct_web","research":{"status":"failed","results":[]}}"#,
            None,
        )
        .expect("the degraded attempt should still write a diagnostic artifact");

        assert!(recovery
            .html
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-recovery"));
        assert_eq!(
            std::fs::read_to_string(&completed.markdown).unwrap(),
            previous_markdown
        );
        assert_eq!(
            std::fs::read_to_string(&completed.html).unwrap(),
            previous_html
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn completed_report_materializer_accepts_reportable_evidence_from_all_runtime_modes() {
        for mode in ["direct_web", "local_parallel_task"] {
            let workspace = std::env::temp_dir().join(format!(
                "a3s-deepresearch-materializer-{mode}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&workspace).unwrap();
            let output = serde_json::json!({
                "mode": mode,
                "research": {
                    "status": "success",
                    "results": [{
                        "success": true,
                        "structured": {
                            "summary": "The official source confirms the reportable finding.",
                            "sources": [{
                                "title": "Official evidence",
                                "url_or_path": "https://example.com/official-evidence",
                                "quote_or_fact": "The official evidence directly supports the material finding used in this report.",
                                "reliability": "Primary fixture source."
                            }],
                            "key_evidence": ["The material finding is directly supported."],
                            "contradictions": [],
                            "confidence": "High for the cited finding.",
                            "gaps": ["The fixture intentionally covers one bounded claim."]
                        }
                    }]
                }
            })
            .to_string();

            let parsed = serde_json::from_str::<serde_json::Value>(&output).unwrap();
            assert_eq!(
                deep_research_collection_status(&parsed),
                "completed",
                "unexpected collection status for {mode}: {output}"
            );
            let evidence = deep_research_structured_evidence_from_workflow(&output, None);
            assert!(
                !evidence.is_empty(),
                "no compact evidence for {mode}: {output}"
            );
            let markdown = evidence::completed_report_markdown_from_workflow_evidence(
                &format!("{mode} deterministic report"),
                &evidence,
            )
            .expect("structured evidence should produce report markdown");
            assert!(
                !deep_research_output_has_internal_leak(&markdown),
                "materialized markdown leaked internals for {mode}: {markdown}"
            );

            let artifacts = materialize_deep_research_completed_report_from_workflow_evidence(
                &workspace,
                &format!("{mode} deterministic report"),
                &output,
                None,
            )
            .expect("valid structured evidence should materialize without model synthesis");
            let markdown = std::fs::read_to_string(&artifacts.markdown).unwrap();
            assert!(markdown.contains("Official evidence"), "{markdown}");
            assert!(artifacts.html.is_file());

            let _ = std::fs::remove_dir_all(&workspace);
        }
    }

    #[test]
    fn completed_report_materializer_preserves_checker_report_context() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-checker-context-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let output = serde_json::json!({
            "mode": "hybrid_direct_web_parallel",
            "plan": {
                "report_title": "Runtime Adoption Decision Guide",
                "execution_route": "direct_then_maker"
            },
            "checker": {
                "decision": "finalize",
                "coverage_summary": "The cumulative package is reportable with one explicit gap.",
                "report_summary": "The evidence supports a bounded production pilot.",
                "verified_findings": [
                    "The runtime documents production-ready component support."
                ],
                "unresolved_gaps": [
                    "Independent interoperability benchmarks remain unavailable."
                ],
                "contradictions": [
                    "Source terminology differs on the maturity label."
                ],
                "next_action": "none"
            },
            "research": {
                "status": "success",
                "results": [{
                    "success": true,
                    "structured": {
                        "summary": "Direct collection found one traceable source.",
                        "sources": [{
                            "title": "Official runtime documentation",
                            "url_or_path": "https://example.com/runtime",
                            "quote_or_fact": "The runtime documents component support.",
                            "reliability": "Official documentation"
                        }],
                        "key_evidence": ["The runtime documents component support."],
                        "contradictions": [],
                        "confidence": "Medium-high",
                        "gaps": []
                    }
                }]
            }
        })
        .to_string();

        let artifacts = materialize_deep_research_completed_report_from_workflow_evidence(
            &workspace,
            "A deliberately verbose raw query that should not become the report heading",
            &output,
            None,
        )
        .expect("checker context should materialize as a completed report");
        let markdown = std::fs::read_to_string(&artifacts.markdown).unwrap();

        assert!(
            markdown.starts_with("# Runtime Adoption Decision Guide\n"),
            "{markdown}"
        );
        assert!(
            markdown.contains("The evidence supports a bounded production pilot."),
            "{markdown}"
        );
        assert!(
            markdown.contains("Independent interoperability benchmarks remain unavailable."),
            "{markdown}"
        );
        assert!(
            markdown.contains("Source terminology differs on the maturity label."),
            "{markdown}"
        );
        assert!(
            !markdown.contains("No material contradictions or gaps were captured"),
            "{markdown}"
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn completed_report_materializer_uses_evidence_when_verification_is_degraded() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-degraded-verification-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let output = serde_json::json!({
            "mode": "direct_web",
            "verification": {
                "status": "degraded",
                "checker_completed": false,
                "error": "simulated checker timeout"
            },
            "research": {
                "status": "partial_success",
                "results": [{
                    "success": true,
                    "structured": {
                        "summary": "Direct collection found one traceable source.",
                        "sources": [{
                            "title": "Runtime documentation",
                            "url_or_path": "https://example.com/runtime-docs",
                            "quote_or_fact": "The runtime provides an event-driven scheduler.",
                            "reliability": "Primary documentation."
                        }],
                        "key_evidence": ["The runtime provides an event-driven scheduler."],
                        "contradictions": [],
                        "confidence": "Medium pending independent verification.",
                        "gaps": []
                    }
                }, {
                    "success": false,
                    "error": "one unrelated source fetch failed"
                }]
            }
        })
        .to_string();

        let parsed = serde_json::from_str::<serde_json::Value>(&output).unwrap();
        assert_eq!(deep_research_collection_status(&parsed), "completed");
        assert!(!deep_research_workflow_needs_recovery_report(&output));

        let artifacts = materialize_deep_research_completed_report_from_workflow_evidence(
            &workspace,
            "Assess the runtime",
            &output,
            None,
        )
        .expect("traceable evidence should survive an unavailable checker");
        let markdown = std::fs::read_to_string(&artifacts.markdown).unwrap();

        assert!(
            markdown.contains("The runtime provides an event-driven scheduler"),
            "{markdown}"
        );
        assert!(
            markdown.contains("Independent verification did not complete in this run"),
            "{markdown}"
        );
        assert!(!markdown.contains("Verified finding"), "{markdown}");
        assert!(!markdown.contains("simulated checker timeout"), "{markdown}");
        assert!(!markdown.contains("DeepResearch Recovery Report"), "{markdown}");

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn completed_report_materializer_rejects_success_without_reportable_evidence() {
        let workspace = std::env::temp_dir().join(format!(
            "a3s-deepresearch-empty-success-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        let output = serde_json::json!({
            "mode": "local_parallel_task",
            "research": { "status": "success", "results": [] }
        })
        .to_string();

        assert!(
            materialize_deep_research_completed_report_from_workflow_evidence(
                &workspace,
                "empty success",
                &output,
                None,
            )
            .is_none()
        );
        assert!(!workspace.join(".a3s/research").exists());

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn recovery_report_preserves_direct_web_seed_when_parallel_fanout_fails() {
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
            markdown.contains("captured 0/3 delegated research tasks"),
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
                    "query_term_count": 3,
                    "matched_query_term_count": 3,
                    "fetched_query_term_count": 2,
                    "freshness_required": true,
                    "dated_source_count": 0,
                    "query_terms_truncated": true
                }
            }
        })
        .to_string();

        let summary = workflow_evidence_summary(&direct).expect("direct summary");
        assert!(
            summary.contains("2 source(s) across 2 host(s), 1 fetched across 1 host(s)"),
            "{summary}"
        );
        assert!(summary.contains("topic 3/3, fetched text 2/3"), "{summary}");
        assert!(summary.contains("0/2 source(s) are dated"), "{summary}");
        assert!(
            summary.contains("direct completion was disabled"),
            "{summary}"
        );

        let hybrid = serde_json::json!({
            "mode": "hybrid_direct_web_parallel",
            "research": { "metadata": { "success_count": 2, "task_count": 3 } },
            "seed_research": {
                "metadata": {
                    "source_count": 4,
                    "host_count": 3,
                    "fetched_count": 2,
                    "fetched_host_count": 2,
                    "query_term_count": 2,
                    "matched_query_term_count": 2,
                    "fetched_query_term_count": 2
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
            summary.contains("4 source(s) across 3 host(s), 2 fetched across 2 host(s)"),
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

        let markdown = format!("# Report\n\n## Sources\n\n- {kbs}\n");
        let (anchors, explicit) = markdown_report_source_anchors(&markdown, "report query");
        assert!(explicit);
        assert!(anchors.contains(
            &"https://world.kbs.co.kr/service/news_view.htm?lang=e&seq_code=155851".to_string()
        ));
        assert!(!anchors.contains(&"untrusted-report-url".to_string()));
    }

    #[test]
    fn report_source_candidates_do_not_use_substring_or_case_folded_matches() {
        let observed = HashSet::from([
            "https://example.com/report".to_string(),
            "https://example.com/report/".to_string(),
            "docs/Secrets.md".to_string(),
            "docs/archive/".to_string(),
        ]);
        for reported in [
            "https://example.com/report-fabricated",
            "https://example.com/Report",
            "docs/secrets.md",
            "docs/archive",
        ] {
            assert!(
                reported_research_source_candidates(reported)
                    .iter()
                    .all(|candidate| !observed.contains(candidate)),
                "{reported:?} must not match a distinct observed resource"
            );
        }
    }

    #[test]
    fn report_citation_collectors_cover_plain_local_table_cells() {
        let markdown =
            "# Report\n\n## Sources\n\n| Source | Note |\n| --- | --- |\n| docs/unobserved.md | fixture |\n";
        let (anchors, explicit) = markdown_report_source_anchors(markdown, "report query");
        assert!(explicit);
        assert!(anchors.contains(&"docs/unobserved.md".to_string()));

        let html = "<html><body><h2>Sources</h2><table><tr><td>docs/html-source.md</td></tr></table></body></html>";
        let mut anchors = Vec::new();
        let mut seen = HashSet::new();
        collect_html_source_section_local_anchors(html, &mut anchors, &mut seen);
        assert_eq!(anchors, vec!["docs/html-source.md"]);
    }

    #[test]
    fn report_citation_collector_marks_unsanitized_urls_untrusted() {
        let markdown = "# Report\n\n## Sources\n\n- https://user:password@example.com/source?token=secret#fragment\n";
        let (anchors, explicit) = markdown_report_source_anchors(markdown, "report query");
        assert!(explicit);
        assert!(
            anchors.contains(&"untrusted-report-url".to_string()),
            "{anchors:?}"
        );
        assert!(!anchors.contains(&"https://example.com/source".to_string()));
    }

    #[test]
    fn html_link_targets_require_exact_attribute_names() {
        let html = "<a data-href=\"https://example.com/metadata\" xhref=\"https://example.com/lookalike\" href=\"https://example.com/source\">source</a>";
        assert_eq!(html_link_targets(html), vec!["https://example.com/source"]);
    }

    #[test]
    fn source_target_scanners_preserve_balanced_url_parentheses() {
        let target = "https://example.com/spec_(v2)";
        assert_eq!(http_source_targets(&format!("See {target}.")), vec![target]);
        assert_eq!(
            http_source_targets("See https://example.com/plain)."),
            vec!["https://example.com/plain"]
        );

        let mut anchors = Vec::new();
        let mut seen = HashSet::new();
        collect_markdown_link_anchors(
            &format!("[specification]({target})"),
            &mut anchors,
            &mut seen,
        );
        assert_eq!(anchors, vec![target]);

        anchors.clear();
        seen.clear();
        collect_markdown_link_anchors(
            "[plain](https://example.com/plain))",
            &mut anchors,
            &mut seen,
        );
        assert_eq!(anchors, vec!["https://example.com/plain"]);

        let query = format!("Analyze {target}");
        let markdown = format!("# {}\n", markdown_plain_text(&query));
        let html = deep_research_completed_report_html(&query, &markdown);
        assert!(
            html_report_source_anchors(&html, &query).is_empty(),
            "the exact balanced-parenthesis query URL should remain title-only: {html}"
        );
        let derived_markdown = format!("# {target} — Research Report\n");
        let derived_html = deep_research_completed_report_html(&query, &derived_markdown);
        assert!(
            markdown_report_source_anchors(&derived_markdown, &query)
                .0
                .is_empty(),
            "a query-derived report title must not become a citation"
        );
        assert!(
            html_report_source_anchors(&derived_html, &query).is_empty(),
            "a query-derived HTML title must not become a citation: {derived_html}"
        );
        let markdown_with_body = format!("{derived_markdown}\nBody citation: {target}\n");
        assert!(
            !markdown_report_source_anchors(&markdown_with_body, &query)
                .0
                .is_empty(),
            "the same URL must still be validated when cited in the report body"
        );
        let html_with_body = derived_html.replace(
            "</article>",
            &format!("<p>Body citation: {target}</p></article>"),
        );
        assert!(
            !html_report_source_anchors(&html_with_body, &query).is_empty(),
            "the same URL must still be validated in the HTML body"
        );

        let unicode_target = "https://example.com/研究_(v2)";
        let unicode_query = format!("Analyze {unicode_target}");
        let unicode_markdown = format!("# {}\n", markdown_plain_text(&unicode_query));
        let unicode_html = deep_research_completed_report_html(&unicode_query, &unicode_markdown);
        assert!(
            html_report_source_anchors(&unicode_html, &unicode_query).is_empty(),
            "percent-encoded HTML href must match its Unicode query URL: {unicode_html}"
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
        assert!(
            html_report_source_anchors(&qualified_html, &qualified_query).is_empty(),
            "the sanitized query URL should remain title-only: {qualified_html}"
        );
        let title_end = "Analyze https://example.com/resource</h1>";
        assert!(qualified_html.contains(title_end), "{qualified_html}");
        let changed_query = qualified_html.replacen(
            title_end,
            "Analyze <a href=\"https://example.com/other\">https://example.com/resource</a></h1>",
            1,
        );
        assert!(
            !html_report_source_anchors(&changed_query, &qualified_query).is_empty(),
            "sanitizing the query must not authorize a different title target"
        );
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

        let markdown = completed_report_markdown_from_answer_text(query, &answer)
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
