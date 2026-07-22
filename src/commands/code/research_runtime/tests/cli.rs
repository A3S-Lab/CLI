use super::*;
use a3s::research::{
    replay, CompletionCriterionAssessment, ContractAssessmentStatus, EvidenceRef, InquiryEvent,
    InquiryLimits, InquiryPhase, Question, ResearchContractAssessment, ResearchMethod,
    ResearchObligation, ResearchObligationAssessment, StopConditionAssessment,
};

struct CompletedCliReportFixture {
    workflow_output: String,
    evidence_id: String,
    claim_id: String,
}

struct CompletedReportLlm {
    section: serde_json::Value,
    editorial: serde_json::Value,
    guidance: serde_json::Value,
    presentation: serde_json::Value,
}

impl CompletedReportLlm {
    fn response_for_messages(&self, messages: &[Message]) -> anyhow::Result<LlmResponse> {
        let prompt = messages
            .iter()
            .flat_map(|message| &message.content)
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let value = if prompt.contains("CLOSED_SECTION_PACKET=") {
            self.section.clone()
        } else if prompt.contains("CLOSED_REPORT_EDITORIAL_PACKET=") {
            self.editorial.clone()
        } else if prompt.contains("CLOSED_REPORT_GUIDANCE_PACKET=") {
            self.guidance.clone()
        } else if prompt.contains("CLOSED_REPORT_PRESENTATION_PACKET=") {
            self.presentation.clone()
        } else if prompt.contains("CLOSED_SEMANTIC_AUDIT_PACKET=") {
            let target_id = if prompt.contains("\"target_id\":\"frame\"") {
                "frame"
            } else if prompt.contains("\"target_id\":\"section:1\"") {
                "section:1"
            } else {
                anyhow::bail!("semantic audit prompt omitted the expected target")
            };
            serde_json::json!({
                "reviews": [{
                    "target_id": target_id,
                    "checks": {
                        "claim_granularity": "clear",
                        "derived_quantities": "clear",
                        "temporal_labels": "clear",
                        "compatibility_scope": "clear",
                        "maintenance_scope": "clear",
                        "replacement_properties": "clear",
                        "promotional_attribution": "clear",
                        "sample_scope": "clear",
                        "unknown_item_quantifiers": "clear",
                        "evidence_gap_scope": "clear",
                        "recommendation_support": "clear",
                        "reader_language_and_internal_jargon": "clear"
                    },
                    "issues": []
                }]
            })
        } else {
            anyhow::bail!("unexpected completed-report structured generation prompt")
        };
        Ok(text_response(value.to_string()))
    }
}

#[async_trait::async_trait]
impl LlmClient for CompletedReportLlm {
    async fn complete(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.response_for_messages(messages)
    }

    async fn complete_streaming(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let response = self.response_for_messages(messages)?;
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = tx.send(StreamEvent::Done(response)).await;
        });
        Ok(rx)
    }

    fn native_structured_support(&self) -> a3s_code_core::llm::structured::NativeStructuredSupport {
        a3s_code_core::llm::structured::NativeStructuredSupport::ForcedTool
    }
}

fn completed_cli_report_fixture() -> CompletedCliReportFixture {
    let mut workflow = serde_json::json!({
        "query": "Explain the accepted CLI finding.",
        "research": {
            "status": "completed",
            "results": [{
                "summary": "The accepted source establishes the CLI finding.",
                "sources": [{
                    "title": "CLI fixture source",
                    "url_or_path": "https://example.test/cli",
                    "quote_or_fact": "The CLI report path reaches a terminal audited Inquiry.",
                    "reliability": "authoritative test fixture"
                }],
                "key_evidence": [
                    "The CLI report path reaches a terminal audited Inquiry."
                ],
                "contradictions": [],
                "gaps": [],
                "confidence": "high"
            }]
        }
    });
    let evidence =
        crate::tui::deep_research_test_accepted_evidence_ledger(&workflow.to_string(), None);
    assert_eq!(evidence.len(), 1, "fixture must retain one evidence item");
    let evidence_id = evidence[0].id.clone();
    let claim_id = evidence[0].claims[0].id.clone();
    let source_id = evidence[0].sources[0].id.clone();
    let obligation_id = "obligation:cli-report";
    let mut question = Question::queued(
        "question:cli-report",
        None,
        "What does the accepted CLI evidence establish?",
    );
    question.obligation_ids = vec![obligation_id.to_string()];
    let events = vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        InquiryEvent::ResearchObligationsCommitted {
            obligations: vec![ResearchObligation::new(
                obligation_id,
                "Completed CLI report",
                "Produce a source-backed report through the shared report pipeline",
                true,
                vec!["The report reaches the terminal audited Inquiry state".to_string()],
            )],
            stop_conditions: vec!["The accepted finding appears in a completed report".to_string()],
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![question],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                evidence_id.clone(),
                vec![claim_id.clone()],
                vec![source_id.clone()],
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:cli-report".to_string(),
            answer: "The accepted evidence establishes the terminal CLI report path.".to_string(),
            evidence_ids: vec![evidence_id.clone()],
        },
        InquiryEvent::ResearchContractAssessed {
            assessment: ResearchContractAssessment {
                obligations: vec![ResearchObligationAssessment {
                    obligation_id: obligation_id.to_string(),
                    criteria: vec![CompletionCriterionAssessment {
                        criterion_index: 0,
                        status: ContractAssessmentStatus::Satisfied,
                        rationale: "The accepted source supports the completed CLI report."
                            .to_string(),
                        evidence_ids: vec![evidence_id.clone()],
                    }],
                    primary_source: None,
                    independent_corroboration: None,
                }],
                stop_conditions: vec![StopConditionAssessment {
                    condition_index: 0,
                    status: ContractAssessmentStatus::Satisfied,
                    rationale: "The accepted finding can be rendered and audited.".to_string(),
                    evidence_ids: vec![evidence_id.clone()],
                }],
                diagnostics: Vec::new(),
            },
        },
    ];
    let state = replay(&events, &InquiryLimits::default()).expect("replay CLI report fixture");
    assert_eq!(state.phase, InquiryPhase::Outlining);
    workflow["inquiry"] = serde_json::json!({
        "events": events,
        "state": state,
    });
    CompletedCliReportFixture {
        workflow_output: workflow.to_string(),
        evidence_id,
        claim_id,
    }
}

#[test]
fn parses_deepresearch_cli_options() {
    let opts =
        parse_deepresearch_args(&["compare".into(), "runtimes".into()]).expect("deepresearch args");
    assert_eq!(opts.query, "compare runtimes");
    assert_eq!(opts.evidence_scope, None);

    let local_only = parse_deepresearch_args(&[
        "--local-only".into(),
        "compare".into(),
        "public".into(),
        "sources".into(),
    ])
    .expect("explicit local-only evidence scope");
    assert_eq!(
        local_only.evidence_scope,
        Some(crate::tui::DeepResearchEvidenceScope::LocalOnly)
    );

    let web = parse_deepresearch_args(&[
        "--web".into(),
        "do".into(),
        "not".into(),
        "use".into(),
        "web".into(),
    ])
    .expect("explicit web evidence scope");
    assert_eq!(
        web.evidence_scope,
        Some(crate::tui::DeepResearchEvidenceScope::WebAndWorkspace)
    );

    let conflict =
        parse_deepresearch_args(&["--local-only".into(), "--web".into(), "conflict".into()])
            .expect_err("conflicting evidence scopes");
    assert!(conflict.to_string().contains("conflicts"), "{conflict}");
}

#[tokio::test]
async fn deepresearch_cli_rejects_removed_runtime_selection() {
    let err = execute_deepresearch_in(
        &["--os".into(), "market".into()],
        Path::new("."),
        CodeConfig::default(),
        PathBuf::from(".a3s/memory"),
    )
    .await
    .expect_err("--os should be rejected before building a DeepResearch session");
    let message = err.to_string();
    assert!(
        message.contains("runtime selection has been removed"),
        "{message}"
    );
    assert!(message.contains("--web or --local-only"), "{message}");

    let local = parse_deepresearch_args(&["--local".into(), "market".into()])
        .expect_err("--local must not create a second runtime route");
    assert!(
        local
            .to_string()
            .contains("runtime selection has been removed"),
        "{local}"
    );
}

#[tokio::test]
async fn deepresearch_cli_resolves_account_model_before_building_the_session() {
    let workspace = tempfile::tempdir().expect("DeepResearch workspace");
    let config = CodeConfig::from_acl(
        r#"
            default_model = "codex/account-model"
            memory {
              llmExtraction = false
            }
        "#,
    )
    .expect("account-model config");
    let scripted: Arc<dyn LlmClient> = Arc::new(ScriptedLlmClient::new(Vec::new()));
    let resolved_route = Arc::new(Mutex::new(None));
    let captured_route = Arc::clone(&resolved_route);

    let (session, _) = build_deepresearch_session_with_resolver(
        workspace.path().to_string_lossy().as_ref(),
        config,
        workspace.path().join("memory"),
        move |config, options, session_id| {
            *captured_route.lock().unwrap() = Some((
                config.default_model.clone(),
                options.session_id.clone(),
                session_id.to_string(),
                options.continuation_enabled,
                options.max_continuation_turns,
                options.max_tool_rounds,
                options.max_parallel_tasks,
                options.auto_delegation.as_ref().map(|delegation| {
                    (
                        delegation.enabled,
                        delegation.auto_parallel,
                        delegation.allow_manual_delegation,
                    )
                }),
                options.manual_delegation_enabled,
                options.auto_parallel_delegation,
            ));
            Ok(scripted)
        },
    )
    .await
    .expect("account-backed DeepResearch session");

    let (
        model,
        option_session_id,
        resolver_session_id,
        continuation_enabled,
        max_continuation_turns,
        max_tool_rounds,
        max_parallel_tasks,
        auto_delegation,
        manual_delegation_enabled,
        auto_parallel_delegation,
    ) = resolved_route
        .lock()
        .unwrap()
        .clone()
        .expect("DeepResearch model resolver call");
    assert_eq!(model.as_deref(), Some("codex/account-model"));
    assert_eq!(
        option_session_id.as_deref(),
        Some(resolver_session_id.as_str())
    );
    assert_eq!(session.id(), resolver_session_id);
    assert_eq!(continuation_enabled, Some(false));
    assert_eq!(max_continuation_turns, None);
    assert_eq!(max_tool_rounds, None);
    assert_eq!(max_parallel_tasks, Some(1));
    assert_eq!(auto_delegation, Some((false, false, true)));
    assert_eq!(manual_delegation_enabled, Some(true));
    assert_eq!(auto_parallel_delegation, Some(false));
}

#[test]
fn deepresearch_cli_policy_denies_model_writes_including_report_artifacts() {
    use a3s_code_core::permissions::PermissionDecision;

    let policy = deepresearch_cli_permission_policy();

    assert_eq!(
        policy.check(
            "write",
            &serde_json::json!({
                "file_path": ".a3s/research/local-test/report.md",
                "content": "# Report"
            })
        ),
        PermissionDecision::Deny
    );
    assert_eq!(
        policy.check(
            "Write",
            &serde_json::json!({
                "file_path": ".a3s/research/local-test/index.html",
                "content": "<!doctype html><html><body></body></html>"
            })
        ),
        PermissionDecision::Deny
    );
    assert_eq!(
        policy.check("read", &serde_json::json!({"file_path": "src/lib.rs"})),
        PermissionDecision::Allow
    );
    assert_eq!(
        policy.check("web_search", &serde_json::json!({"query": "a3s"})),
        PermissionDecision::Allow
    );
    assert_eq!(
        policy.check("bash", &serde_json::json!({"command": "ls -la"})),
        PermissionDecision::Deny
    );
    assert_eq!(
        policy.check(
            "write",
            &serde_json::json!({"file_path": "README.md", "content": "oops"})
        ),
        PermissionDecision::Deny
    );
    assert_eq!(
        policy.check(
            "write",
            &serde_json::json!({
                "file_path": ".a3s/research/local-test/../../README.md",
                "content": "path traversal"
            })
        ),
        PermissionDecision::Deny
    );
}

#[test]
fn deepresearch_cli_report_phase_allows_only_host_owned_structured_generation() {
    use a3s_code_core::permissions::PermissionDecision;

    assert_eq!(
        deep_research_report_phase_tool_permission(
            "generate_object",
            &serde_json::json!({"schema_name": "deep_research_report"}),
        ),
        PermissionDecision::Allow
    );
    assert_eq!(
        deep_research_report_phase_tool_permission(
            "write",
            &serde_json::json!({"file_path": ".a3s/research/topic/report.md"}),
        ),
        PermissionDecision::Deny
    );
    assert_eq!(
        deep_research_report_phase_tool_permission(
            "write",
            &serde_json::json!({"file_path": "README.md"}),
        ),
        PermissionDecision::Deny
    );
    assert_eq!(
        deep_research_report_phase_tool_permission(
            "web_search",
            &serde_json::json!({"query": "restart research"}),
        ),
        PermissionDecision::Deny
    );
}

#[tokio::test]
async fn deepresearch_cli_completed_path_uses_terminal_shared_report_pipeline() {
    let workspace = tempfile::tempdir().expect("completed CLI report workspace");
    let fixture = completed_cli_report_fixture();
    let section = serde_json::json!({
        "section_id": "section:1",
        "markdown": "The accepted evidence establishes that the CLI report path reaches a terminal audited Inquiry. This means non-interactive publication now follows the same durable report boundary as the TUI, as shown by the [CLI fixture source](https://example.test/cli).",
    });
    let editorial_frame = serde_json::json!({
        "report_title": "Completed CLI DeepResearch report",
        "reader_labels": {
            "qualification_heading": "Evidence boundaries",
            "qualification_intro": "The following consequential points remain bounded.",
            "sources_heading": "Sources",
            "decision_heading": "Decision guidance",
            "evidence_limitation": "Evidence limitation",
            "primary_source_support": "Primary-source support",
            "independent_corroboration": "Independent corroboration",
            "established_boundary": "The evidence establishes this point.",
            "qualified_boundary": "The evidence supports a qualified conclusion.",
            "unresolved_boundary": "The evidence does not establish this point."
        },
        "editorial": {
            "thesis": "The CLI now completes and publishes through the shared audited report pipeline.",
            "track_coverage": [{
                "obligation_id": "obligation:cli-report",
                "status": "answered",
                "finding": "The accepted source establishes the terminal CLI report path.",
                "interpretation": "CLI and TUI report completion now share one publication authority.",
                "implication": "Successful non-interactive reports can be published without a second synthesis route.",
                "uncertainty": ""
            }]
        }
    });
    let guidance_frame = serde_json::json!({
        "decision_guidance": []
    });
    let presentation_frame = serde_json::json!({
        "presentation": {
            "narrative_mode": "briefing",
            "archetype": "analytical",
            "palette": "ocean",
            "density": "balanced",
            "hero": "split",
            "visual_stance": "shifted",
            "rationale": "A single source-backed engineering finding calls for a compact analytical briefing.",
            "section_plan": [{
                "heading": "Completed CLI report",
                "rhythm": "anchor",
                "composition": "evidence"
            }]
        }
    });
    let llm: Arc<dyn LlmClient> = Arc::new(CompletedReportLlm {
        section,
        editorial: editorial_frame,
        guidance: guidance_frame,
        presentation: presentation_frame,
    });
    let config = CodeConfig::from_acl(
        r#"
            default_model = "openai/x"
            memory {
              llmExtraction = false
            }
        "#,
    )
    .expect("completed CLI report config");
    let (session, report_tool_gate) = build_deepresearch_session_with_resolver(
        workspace.path().to_string_lossy().as_ref(),
        config,
        workspace.path().join("memory"),
        move |_, _, _| Ok(Arc::clone(&llm)),
    )
    .await
    .expect("completed CLI report session");
    crate::tui::deep_research_test_record_workflow_started(
        workspace.path(),
        "cli-completed-report",
        crate::tui::DeepResearchTestResearchSpec {
            query: "Explain the accepted CLI finding.".to_string(),
            current_date: "2026-07-19".to_string(),
            evidence_scope: "web_and_workspace".to_string(),
            required_claims: vec![
                "The report reaches the terminal audited Inquiry state".to_string()
            ],
            total_budget_ms: 60_000,
            retrieval_stage_budget_ms: 30_000,
            question_review_stage_budget_ms: 20_000,
            finalization_reserve_ms: 5_000,
            host_pid: std::process::id(),
        },
    )
    .await
    .expect("initialize completed CLI report journal");

    let synthesis = synthesize_deepresearch_report(
        &session,
        workspace.path(),
        "Explain the accepted CLI finding.",
        &fixture.workflow_output,
        0,
        None,
        "cli-completed-report",
        &report_tool_gate,
    )
    .await
    .expect("completed CLI report synthesis");

    assert_eq!(synthesis.status, DeepResearchReportStatus::Completed);
    assert!(synthesis.artifacts.markdown.is_file());
    assert!(synthesis.artifacts.html.is_file());
    let markdown = std::fs::read_to_string(&synthesis.artifacts.markdown)
        .expect("read completed CLI Markdown");
    let html = std::fs::read_to_string(&synthesis.artifacts.html).expect("read completed CLI HTML");
    assert!(markdown.contains("# Completed CLI DeepResearch report"));
    assert!(markdown.contains("## Completed CLI report"));
    assert!(markdown.contains("https://example.test/cli"));
    assert!(html.contains("Completed CLI DeepResearch report"));
    assert!(!crate::tui::deep_research_output_has_internal_leak(
        &markdown
    ));
    assert!(!report_tool_gate.report_only());

    let journal_artifacts = crate::tui::ResearchReportArtifacts {
        markdown: synthesis.artifacts.markdown.clone(),
        html: synthesis.artifacts.html.clone(),
    };
    let journal_outcome =
        crate::tui::settle_deep_research_cli_run(crate::tui::DeepResearchCliSettlement {
            workspace: workspace.path(),
            run_id: "cli-completed-report",
            query: "Explain the accepted CLI finding.",
            workflow_succeeded: true,
            workflow_output: &fixture.workflow_output,
            workflow_metadata: None,
            requested_outcome: crate::tui::ResearchOutcome::Completed,
            artifacts: &journal_artifacts,
        })
        .await
        .expect("settle completed CLI journal");
    assert_eq!(journal_outcome, crate::tui::ResearchOutcome::Completed);
    let run_status =
        crate::tui::deep_research_test_run_status(workspace.path(), "cli-completed-report")
            .await
            .expect("read completed CLI run status");
    assert!(run_status.contains("outcome: completed"), "{run_status}");
    assert!(
        run_status.contains("evidence: 1 accepted · 1 sources"),
        "{run_status}"
    );
    assert!(
        run_status.contains("active: 0 steps · 0 children"),
        "{run_status}"
    );
    assert!(run_status.contains("cited sources: 1"), "{run_status}");

    let (_, terminal_state) =
        crate::tui::deep_research_test_load_inquiry_state(workspace.path(), "cli-completed-report")
            .await
            .expect("load completed CLI Inquiry")
            .expect("completed CLI Inquiry journal");
    assert_eq!(terminal_state.phase, InquiryPhase::Completed);
    assert_eq!(
        terminal_state
            .evidence_catalog
            .get(&fixture.evidence_id)
            .map(|evidence| evidence.claim_ids.as_slice()),
        Some([fixture.claim_id].as_slice())
    );
    session.close().await;
}

#[tokio::test]
async fn deepresearch_cli_unreportable_evidence_finishes_as_a_degraded_host_report() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-deepresearch-cli-degraded-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let cfg = workspace.join("config.acl");
    test_config(&cfg);
    let agent = Agent::new(cfg.to_string_lossy().to_string()).await.unwrap();
    let llm = Arc::new(ScriptedLlmClient::new(vec![tool_call_response(
        "toolu_write_readme",
        "write",
        serde_json::json!({
            "file_path": "README.md",
            "content": "this model response must never be consumed"
        }),
    )]));
    let opts = SessionOptions::new()
        .with_llm_client(llm)
        .with_planning_mode(a3s_code_core::PlanningMode::Disabled);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(opts))
        .await
        .unwrap();
    let report_tool_gate = DeepResearchReportToolGate::default();

    let synthesis = synthesize_deepresearch_report(
        &session,
        &workspace,
        "no accepted evidence",
        r#"{"mode":"direct_web_degraded","research":{"status":"failed"}}"#,
        1,
        None,
        "cli-degraded-test",
        &report_tool_gate,
    )
    .await
    .expect("the host should materialize a bounded recovery report");

    assert_eq!(synthesis.status, DeepResearchReportStatus::Degraded);
    assert!(synthesis.artifacts.markdown.is_file());
    assert!(synthesis.artifacts.html.is_file());
    assert!(!workspace.join("README.md").exists());
    assert!(!crate::tui::deep_research_output_has_internal_leak(
        &std::fs::read_to_string(&synthesis.artifacts.markdown).unwrap()
    ));
    assert!(!report_tool_gate.report_only());

    let _ = std::fs::remove_dir_all(&workspace);
}
