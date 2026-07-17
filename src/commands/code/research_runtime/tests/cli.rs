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
                "file_path": ".a3s/research/local-test/../../README.md",
                "content": "path traversal"
            })
        ),
        PermissionDecision::Deny
    );
}

#[test]
fn deepresearch_cli_report_phase_allows_only_structured_generation_and_report_files() {
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
        PermissionDecision::Allow
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
        false,
        r#"{"mode":"direct_web_degraded","research":{"status":"failed"}}"#,
        1,
        None,
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
