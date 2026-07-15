use super::*;

#[test]
fn parses_deepresearch_cli_options() {
    let opts = parse_deepresearch_args(&["--local".into(), "rust".into(), "async".into()])
        .expect("local deepresearch args");
    assert_eq!(opts.query, "rust async");
    assert_eq!(opts.runtime_mode, DeepResearchRuntimeMode::Local);

    let opts =
        parse_deepresearch_args(&["--os".into(), "market".into()]).expect("os deepresearch args");
    assert_eq!(opts.query, "market");
    assert_eq!(opts.runtime_mode, DeepResearchRuntimeMode::Os);

    let opts = parse_deepresearch_args(&["compare".into(), "runtimes".into()])
        .expect("auto deepresearch args");
    assert_eq!(opts.query, "compare runtimes");
    assert_eq!(opts.runtime_mode, DeepResearchRuntimeMode::Auto);
}

#[tokio::test]
async fn deepresearch_cli_os_mode_is_temporarily_disabled() {
    let err = execute_deepresearch_in(
        &["--os".into(), "market".into()],
        Path::new("."),
        CodeConfig::default(),
        PathBuf::from(".a3s/memory"),
    )
    .await
    .expect_err("--os should be disabled before touching OS Runtime");
    let message = err.to_string();
    assert!(message.contains("temporarily disabled"), "{message}");
    assert!(message.contains("Function-as-a-Service"), "{message}");
}

#[test]
fn deepresearch_cli_policy_only_allows_report_artifact_writes() {
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
        PermissionDecision::Allow
    );
    assert_eq!(
        policy.check(
            "Write",
            &serde_json::json!({
                "file_path": ".a3s/research/local-test/index.html",
                "content": "<!doctype html><html><body></body></html>"
            })
        ),
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
                "file_path": "/tmp/workspace/.a3s/research/local-test/index.html",
                "content": "ambiguous absolute path"
            })
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
    assert_eq!(
        policy.check(
            "edit",
            &serde_json::json!({
                "file_path": ".a3s/research/local-test/..\\..\\README.md",
                "old_string": "before",
                "new_string": "after"
            })
        ),
        PermissionDecision::Deny
    );
    assert_eq!(
        policy.check("bash", &serde_json::json!({"command": "rm -rf target"})),
        PermissionDecision::Deny
    );
}

#[tokio::test]
async fn deepresearch_cli_synthesis_denies_non_report_writes_before_fallback() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-deepresearch-cli-denied-write-{}-{}",
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
    let llm = Arc::new(ScriptedLlmClient::new(vec![
        tool_call_response(
            "toolu_write_readme",
            "write",
            serde_json::json!({
                "file_path": "README.md",
                "content": "DeepResearch should not write ordinary workspace files.",
            }),
        ),
        text_response(
            "Synthesis recovered after a denied workspace write but did not write report files.",
        ),
        text_response("Repair also did not write report files."),
    ]));
    let report_tool_gate = DeepResearchReportToolGate::default();
    let permission_policy = deepresearch_cli_permission_policy();
    let opts = SessionOptions::new()
        .with_llm_client(llm)
        .with_permission_policy(permission_policy.clone())
        .with_permission_checker(Arc::new(DeepResearchPermissionChecker {
            base: permission_policy,
            report_tool_gate: report_tool_gate.clone(),
        }))
        .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
        .with_max_tool_rounds(4);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(opts))
        .await
        .unwrap();
    let synthesis = synthesize_deepresearch_report(
        &session,
        &workspace,
        "denied write fallback",
        false,
        r#"{"mode":"local_parallel_task","research":"evidence after denied write"}"#,
        0,
        None,
        &report_tool_gate,
    )
    .await
    .expect("host fallback should materialize after denied non-report write");
    let DeepResearchReportSynthesis {
        text: final_text,
        artifacts,
        status,
    } = synthesis;

    assert_eq!(status, DeepResearchReportStatus::FallbackDraft);
    assert!(
        !workspace.join("README.md").exists(),
        "DeepResearch CLI policy must block non-report writes"
    );
    assert!(
        final_text.contains("DeepResearch fallback draft written at"),
        "{final_text}"
    );
    assert!(!final_text.contains("A3S_RESEARCH_VIEW"), "{final_text}");
    assert_eq!(
        artifacts.markdown,
        workspace
            .join(".a3s/research/denied-write-fallback/report.md")
            .canonicalize()
            .unwrap()
    );
    assert_eq!(
        artifacts.html,
        workspace
            .join(".a3s/research/denied-write-fallback/index.html")
            .canonicalize()
            .unwrap()
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn deepresearch_cli_repair_pass_writes_required_markdown_and_html_artifacts() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-deepresearch-cli-artifacts-{}-{}",
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
    let llm = Arc::new(ScriptedLlmClient::new(vec![
        text_response("Initial synthesis without a report marker."),
        tool_call_response(
            "toolu_write_markdown",
            "write",
            serde_json::json!({
                "file_path": ".a3s/research/local-test/report.md",
                "content": "# Local Test\n\n## Findings\n\nThis source-backed markdown report summarizes the gathered DeepResearch evidence, explains the main finding, and records caveats for review.\n\n## Sources\n\n- https://example.com/research\n\n## Confidence\n\nConfidence is medium because this deterministic test evidence is compact but traceable.\n",
            }),
        ),
        tool_call_response(
            "toolu_write_html",
            "write",
            serde_json::json!({
                "file_path": ".a3s/research/local-test/index.html",
                "content": "<!doctype html><html><body><h1>Local Test</h1><section><h2>Findings</h2><p>This source-backed report summarizes gathered DeepResearch evidence, caveats, and the main finding for review.</p></section><section><h2>Sources</h2><p>Evidence source: https://example.com/research. Confidence is medium.</p></section></body></html>",
            }),
        ),
        text_response(
            "Step 2 complete: Markdown report written.\nTargeted verification could not be performed because file-read tooling is currently blocked.\nA3S_RESEARCH_VIEW: .a3s/research/local-test/index.html",
        ),
    ]));
    let report_tool_gate = DeepResearchReportToolGate::default();
    let permission_policy = deepresearch_cli_permission_policy();
    let opts = SessionOptions::new()
        .with_llm_client(llm)
        .with_permission_policy(permission_policy.clone())
        .with_permission_checker(Arc::new(DeepResearchPermissionChecker {
            base: permission_policy,
            report_tool_gate: report_tool_gate.clone(),
        }))
        .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
        .with_max_tool_rounds(6);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(opts))
        .await
        .unwrap();

    let synthesis = synthesize_deepresearch_report(
        &session,
        &workspace,
        "local test",
        false,
        r#"{"mode":"local_parallel_task","research":"evidence"}"#,
        0,
        None,
        &report_tool_gate,
    )
    .await
    .unwrap_or_else(|error| {
        let markdown = workspace.join(".a3s/research/local-test/report.md");
        let html = workspace.join(".a3s/research/local-test/index.html");
        panic!(
            "{error}; markdown_exists={}; html_exists={}",
            markdown.exists(),
            html.exists()
        )
    });
    let DeepResearchReportSynthesis {
        text: final_text,
        artifacts,
        status,
    } = synthesis;

    assert_eq!(status, DeepResearchReportStatus::Completed);
    assert!(
        final_text.contains("A3S_RESEARCH_VIEW: .a3s/research/local-test/index.html"),
        "{final_text}"
    );
    assert!(
        final_text.contains("# Local Test"),
        "dirty repair text should be rebuilt from validated report.md: {final_text}"
    );
    assert!(
        !final_text.contains("Step 2 complete")
            && !final_text.contains("Targeted verification could not be performed"),
        "internal repair narration must not survive final synthesis text: {final_text}"
    );
    assert_eq!(
        artifacts.markdown,
        workspace
            .join(".a3s/research/local-test/report.md")
            .canonicalize()
            .unwrap()
    );
    assert_eq!(
        artifacts.html,
        workspace
            .join(".a3s/research/local-test/index.html")
            .canonicalize()
            .unwrap()
    );
    assert!(std::fs::metadata(&artifacts.markdown).unwrap().len() > 0);
    assert!(std::fs::metadata(&artifacts.html).unwrap().len() > 0);
    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn deepresearch_cli_materializes_fallback_artifacts_when_model_never_writes_report() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-deepresearch-cli-fallback-{}-{}",
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
    let llm = Arc::new(ScriptedLlmClient::new(vec![
        text_response("Initial synthesis without report files."),
        text_response("Repair also forgot to write the report files."),
    ]));
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
        "fallback only",
        false,
        r#"{"mode":"local_parallel_task","research":"fallback evidence"}"#,
        0,
        None,
        &report_tool_gate,
    )
    .await
    .expect("host fallback should materialize draft artifacts");
    let DeepResearchReportSynthesis {
        text: final_text,
        artifacts,
        status,
    } = synthesis;

    assert_eq!(status, DeepResearchReportStatus::FallbackDraft);
    assert!(
        final_text.contains("DeepResearch fallback draft written at"),
        "{final_text}"
    );
    assert!(!final_text.contains("A3S_RESEARCH_VIEW"), "{final_text}");
    assert_eq!(
        artifacts.markdown,
        workspace
            .join(".a3s/research/fallback-only/report.md")
            .canonicalize()
            .unwrap()
    );
    assert_eq!(
        artifacts.html,
        workspace
            .join(".a3s/research/fallback-only/index.html")
            .canonicalize()
            .unwrap()
    );
    let markdown = std::fs::read_to_string(&artifacts.markdown).unwrap();
    assert!(markdown.contains("Repair also forgot"));
    assert!(markdown.contains("fallback evidence"));
    assert!(markdown.contains("DeepResearch Fallback Draft"));
    assert!(!markdown.contains("A3S_RESEARCH_VIEW"));
    let html = std::fs::read_to_string(&artifacts.html).unwrap();
    assert!(html.contains("DeepResearch Fallback Draft"));
    assert!(html.contains("fallback evidence"));
    assert!(!html.contains("A3S_RESEARCH_VIEW"));

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn deepresearch_cli_failed_collection_without_sources_falls_back_without_model_recovery() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-deepresearch-cli-no-evidence-{}-{}",
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
    let llm = Arc::new(ScriptedLlmClient::new(vec![text_response(
        "Incorrect recovery should not be used.\nA3S_RESEARCH_VIEW: .a3s/research/no-evidence/index.html",
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
        "no evidence",
        false,
        "dynamic_workflow timed out before evidence was available",
        1,
        None,
        &report_tool_gate,
    )
    .await
    .expect("host fallback should materialize draft artifacts");

    assert_eq!(synthesis.status, DeepResearchReportStatus::FallbackDraft);
    assert!(
        synthesis
            .text
            .contains("DeepResearch fallback draft written at"),
        "{}",
        synthesis.text
    );
    assert!(!synthesis.text.contains("A3S_RESEARCH_VIEW"));
    let markdown = std::fs::read_to_string(&synthesis.artifacts.markdown).unwrap();
    assert!(markdown.contains("DeepResearch Fallback Draft"));
    assert!(markdown.contains("evidence collection failed"));
    assert!(!markdown.contains("Incorrect recovery should not be used"));

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn deepresearch_cli_dirty_synthesis_is_repaired_or_falls_back_cleanly() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-deepresearch-cli-dirty-fallback-{}-{}",
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
    let llm = Arc::new(ScriptedLlmClient::new(vec![
        text_response(
            "● Searched web fifa results\n⎿ [tool output truncated: showing first bytes]\nerror: Max tool rounds (30) exceeded",
        ),
        text_response(
            "DynamicWorkflowRuntime evidence package:\n```json\n{\"summary\":\"raw\",\"sources\":[],\"confidence\":\"low\"}\n```",
        ),
    ]));
    let opts = SessionOptions::new().with_planning_mode(a3s_code_core::PlanningMode::Disabled);
    let session = agent
        .session_async(
            workspace.to_string_lossy().to_string(),
            Some(opts.with_llm_client(llm)),
        )
        .await
        .unwrap();
    let report_tool_gate = DeepResearchReportToolGate::default();

    let synthesis = synthesize_deepresearch_report(
        &session,
        &workspace,
        "dirty fallback",
        false,
        r#"{"mode":"local_parallel_task","research":{"metadata":{"success_count":1,"task_count":1},"output":"● Searched web\n⎿ [tool output truncated]"}}"#,
        0,
        None,
        &report_tool_gate,
    )
    .await
    .expect("host fallback should materialize when synthesis remains dirty");
    let DeepResearchReportSynthesis {
        text: final_text,
        artifacts,
        status,
    } = synthesis;

    assert_eq!(status, DeepResearchReportStatus::FallbackDraft);
    assert!(
        final_text.contains("DeepResearch fallback draft written at"),
        "{final_text}"
    );
    assert!(!final_text.contains("A3S_RESEARCH_VIEW"), "{final_text}");
    assert!(
        !deep_research_output_has_internal_leak(&final_text),
        "{final_text}"
    );
    let markdown = std::fs::read_to_string(&artifacts.markdown).unwrap();
    let html = std::fs::read_to_string(&artifacts.html).unwrap();
    assert!(
        !deep_research_output_has_internal_leak(&markdown),
        "{markdown}"
    );
    assert!(!deep_research_output_has_internal_leak(&html), "{html}");

    let _ = std::fs::remove_dir_all(&workspace);
}
