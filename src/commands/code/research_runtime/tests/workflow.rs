use super::*;

#[tokio::test]
async fn deepresearch_cli_local_workflow_to_report_artifacts_e2e() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-deepresearch-cli-e2e-{}-{}",
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
                "file_path": ".a3s/research/local-workflow-e2e/report.md",
                "content": "# Local Workflow E2E\n\n## Findings\n\nThe workflow produced deterministic evidence and completed fan-out before synthesis, giving the report enough source-backed material to explain the result.\n\n## Sources\n\n- https://example.com/research\n\n## Confidence\n\nConfidence is high for this test because the evidence path is deterministic and verified by workflow metadata.\n",
            }),
        ),
        tool_call_response(
            "toolu_write_html",
            "write",
            serde_json::json!({
                "file_path": ".a3s/research/local-workflow-e2e/index.html",
                "content": "<!doctype html><html><body><h1>Local Workflow E2E</h1><section><h2>Findings</h2><p>The workflow produced deterministic evidence and completed fan-out before synthesis.</p></section><section><h2>Sources</h2><p>Evidence source: https://example.com/research. Confidence is high for this deterministic test.</p></section></body></html>",
            }),
        ),
        text_response(
            "Report complete.\nA3S_RESEARCH_VIEW: .a3s/research/local-workflow-e2e/index.html",
        ),
    ]));
    let opts = SessionOptions::new()
        .with_llm_client(llm)
        .with_permission_policy(deepresearch_cli_permission_policy())
        .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
        .with_max_tool_rounds(6);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(opts))
        .await
        .unwrap();
    session.register_dynamic_workflow_runtime().unwrap();

    let mut workflow_args = deep_research_workflow_args("local workflow e2e", false);
    workflow_args["input"]["tracks"] = serde_json::json!([
        {
            "title": "Local evidence",
            "focus": "Inspect local workflow evidence for the report."
        },
        {
            "title": "Source confidence",
            "focus": "Check source confidence and caveats independently."
        },
        {
            "title": "Sequential synthesis",
            "focus": "This should not run as a parallel child.",
            "parallelizable": false
        }
    ]);
    let workflow = run_deepresearch_workflow(&session, workflow_args)
        .await
        .expect("local DeepResearch workflow should complete");
    assert_eq!(workflow.exit_code, 0, "{}", workflow.output);
    assert!(
        workflow.output.contains("local_parallel_task"),
        "{}",
        workflow.output
    );
    let workflow_json: serde_json::Value =
        serde_json::from_str(&workflow.output).expect("workflow output should be JSON");
    assert_eq!(workflow_json["research"]["status"], "success");
    assert!(
        workflow_json["research"].get("output").is_none(),
        "DeepResearch workflow output should not expose raw parallel_task text"
    );
    assert_eq!(
        workflow_json["research"]["results"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let metadata = workflow.metadata.as_ref().expect("workflow metadata");
    assert_eq!(metadata["dynamic_workflow"]["status"], "Completed");
    assert_eq!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["status"],
        "completed"
    );
    assert_eq!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]["tool"],
        "parallel_task"
    );
    assert_eq!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]["metadata"]
            ["task_count"],
        serde_json::json!(2)
    );
    assert_eq!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]["metadata"]
            ["result_count"],
        serde_json::json!(2)
    );
    assert_eq!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]["metadata"]
            ["results"][0]["structured"]["summary"],
        "Structured DeepResearch track evidence confirms local fan-out completed before synthesis."
    );
    let report_tool_gate = DeepResearchReportToolGate::default();

    let synthesis = synthesize_deepresearch_report(
        &session,
        &workspace,
        "local workflow e2e",
        false,
        &workflow.output,
        workflow.exit_code,
        workflow.metadata.as_ref(),
        &report_tool_gate,
    )
    .await
    .unwrap();
    let DeepResearchReportSynthesis {
        text: final_text,
        artifacts,
        status,
    } = synthesis;

    assert_eq!(status, DeepResearchReportStatus::Completed);
    assert!(
        final_text.contains("A3S_RESEARCH_VIEW: .a3s/research/local-workflow-e2e/index.html"),
        "{final_text}"
    );
    assert_eq!(
        artifacts.markdown,
        workspace
            .join(".a3s/research/local-workflow-e2e/report.md")
            .canonicalize()
            .unwrap()
    );
    assert_eq!(
        artifacts.html,
        workspace
            .join(".a3s/research/local-workflow-e2e/index.html")
            .canonicalize()
            .unwrap()
    );
    assert!(std::fs::read_to_string(&artifacts.markdown)
        .unwrap()
        .contains("workflow produced deterministic evidence"));
    assert!(std::fs::read_to_string(&artifacts.html)
        .unwrap()
        .contains("Local Workflow E2E"));
    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn deepresearch_workflow_runs_bounded_recursive_rounds() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-deepresearch-recursive-rounds-{}-{}",
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
    let llm = Arc::new(ScriptedLlmClient::new(vec![]));
    let opts = SessionOptions::new()
        .with_llm_client(llm)
        .with_permission_policy(deepresearch_cli_permission_policy())
        .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
        .with_max_tool_rounds(8);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(opts))
        .await
        .unwrap();
    session.register_dynamic_workflow_runtime().unwrap();

    let mut workflow_args = deep_research_workflow_args("recursive rounds e2e", false);
    workflow_args["input"]["local_research_rounds"] = serde_json::json!(3);
    workflow_args["input"]["local_max_parallel_tasks"] = serde_json::json!(3);
    workflow_args["input"]["tracks"] = serde_json::json!([
        {
            "title": "Facts",
            "focus": "Gather the strongest factual evidence."
        },
        {
            "title": "Caveats",
            "focus": "Gather caveats and uncertainty."
        }
    ]);

    let workflow = run_deepresearch_workflow(&session, workflow_args)
        .await
        .expect("recursive DeepResearch workflow should complete");
    assert_eq!(workflow.exit_code, 0, "{}", workflow.output);
    let output: serde_json::Value =
        serde_json::from_str(&workflow.output).expect("workflow output should be JSON");
    assert_eq!(output["mode"], "local_parallel_task");
    assert_eq!(
        output["research"]["algorithm"],
        "bounded_recursive_parallel_retrieval_summary"
    );
    assert_eq!(output["research"]["max_rounds"], serde_json::json!(3));
    assert_eq!(output["research"]["completed_rounds"], serde_json::json!(2));
    assert_eq!(output["research"]["stop_reason"], "bounded_rounds_complete");
    assert_eq!(
        output["research"]["rounds"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(
        output["research"]["metadata"]["task_count"],
        serde_json::json!(4)
    );
    assert_eq!(
        output["research"]["metadata"]["success_count"],
        serde_json::json!(4)
    );

    let metadata = workflow.metadata.as_ref().expect("workflow metadata");
    assert_eq!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["status"],
        "completed"
    );
    assert_eq!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research_round_2"]["status"],
        "completed"
    );
    assert!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]
            .get("local_research_round_3")
            .is_none(),
        "workflow should early-stop instead of exhausting every allowed round"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn deepresearch_workflow_sanitizes_partial_parallel_failures() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-deepresearch-partial-failure-{}-{}",
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
    let llm = Arc::new(ScriptedLlmClient::new(vec![]));
    let opts = SessionOptions::new()
        .with_llm_client(llm)
        .with_permission_policy(deepresearch_cli_permission_policy())
        .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
        .with_max_tool_rounds(4);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(opts))
        .await
        .unwrap();
    session.register_dynamic_workflow_runtime().unwrap();

    let mut workflow_args = deep_research_workflow_args("partial failure e2e", false);
    let source = workflow_args["source"].as_str().unwrap().replacen(
        "agent: \"explore\",",
        "agent: index === 0 ? \"explore\" : \"missing-agent\",",
        1,
    );
    workflow_args["source"] = serde_json::json!(source);
    workflow_args["input"]["tracks"] = serde_json::json!([
        {
            "title": "Successful branch",
            "focus": "Return structured evidence."
        },
        {
            "title": "Failed branch",
            "focus": "This branch is routed to a missing test agent."
        }
    ]);

    let workflow = run_deepresearch_workflow(&session, workflow_args)
        .await
        .expect("partial DeepResearch workflow should complete with usable evidence");
    assert_eq!(workflow.exit_code, 0, "{}", workflow.output);
    let output: serde_json::Value =
        serde_json::from_str(&workflow.output).expect("workflow output should be JSON");
    assert_eq!(output["mode"], "local_parallel_task");
    assert_eq!(output["research"]["status"], "partial_success");
    assert_eq!(
        output["research"]["metadata"]["success_count"],
        serde_json::json!(1)
    );
    assert_eq!(
        output["research"]["metadata"]["failed_count"],
        serde_json::json!(1)
    );
    assert_eq!(
        output["research"]["results"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        output["research"]["metadata"]["results"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert!(
        output["research"]["warnings"]["failed_tasks"][0]["error_summary"]
            .as_str()
            .is_some_and(|summary| summary
                .contains("Delegated task failed before returning usable evidence")),
        "{}",
        workflow.output
    );
    assert!(
        !workflow.output.contains("Unknown agent type"),
        "{}",
        workflow.output
    );
    assert!(
        output["research"].get("output").is_none(),
        "sanitized DeepResearch output must not contain raw parallel_task text"
    );
    assert!(
        !workflow.output.contains("Executed 2 tasks in parallel"),
        "{}",
        workflow.output
    );

    let prompt = deep_research_synthesis_prompt(
        "partial failure e2e",
        false,
        &workflow.output,
        workflow.metadata.as_ref(),
    );
    assert!(prompt.contains("failed_tasks"), "{prompt}");
    assert!(!prompt.contains("Executed 2 tasks in parallel"), "{prompt}");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn deepresearch_workflow_retains_source_evidence_when_metadata_is_incomplete() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-deepresearch-retained-evidence-{}-{}",
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
    let llm = Arc::new(StructuredCoercionFailsLlmClient);
    let opts = SessionOptions::new()
        .with_llm_client(llm)
        .with_permission_policy(deepresearch_cli_permission_policy())
        .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
        .with_max_tool_rounds(4);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(opts))
        .await
        .unwrap();
    session.register_dynamic_workflow_runtime().unwrap();

    let mut workflow_args =
        deep_research_workflow_args("latest Rust stable official version", false);
    workflow_args["input"]["local_research_rounds"] = serde_json::json!(1);
    workflow_args["input"]["local_max_parallel_tasks"] = serde_json::json!(1);
    workflow_args["input"]["tracks"] = serde_json::json!([
        {
            "title": "Official source",
            "focus": "Find the official latest Rust stable version."
        }
    ]);

    let workflow = run_deepresearch_workflow(&session, workflow_args)
        .await
        .expect("workflow should retain useful source-backed evidence");
    assert_eq!(workflow.exit_code, 0, "{}", workflow.output);
    let output: serde_json::Value =
        serde_json::from_str(&workflow.output).expect("workflow output should be JSON");
    assert_eq!(output["mode"], "local_parallel_task_partial_success");
    assert_eq!(output["research"]["status"], "partial_success");
    assert_eq!(output["research"]["stop_reason"], "source_notes_retained");
    assert_eq!(
        output["research"]["metadata"]["success_count"],
        serde_json::json!(1)
    );
    assert_eq!(
        output["research"]["results"][0]["structured"]["summary"],
        "The latest stable Rust version is 1.96.1, released on 2026-06-30."
    );
    assert_eq!(
        output["research"]["results"][0]["structured"]["sources"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert!(
        output["research"]["warnings"]["failed_rounds"][0]["error_summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("Delegated task failed")),
        "{}",
        workflow.output
    );
    assert!(
        !workflow.output.contains("[structured output failed"),
        "{}",
        workflow.output
    );
    assert!(
        !workflow.output.contains("schema coercion"),
        "{}",
        workflow.output
    );
    assert!(
        !workflow.output.contains("raw delegated"),
        "{}",
        workflow.output
    );
    assert!(!workflow.output.contains("salvage"), "{}", workflow.output);
    assert!(!workflow.output.contains("salvaged"), "{}", workflow.output);
    assert!(!workflow.output.contains("Task ID:"), "{}", workflow.output);

    let prompt = deep_research_synthesis_prompt(
        "latest Rust stable official version",
        false,
        &workflow.output,
        workflow.metadata.as_ref(),
    );
    assert!(prompt.contains("1.96.1"), "{prompt}");
    assert!(
        prompt.contains("https://blog.rust-lang.org/2026/06/30/Rust-1.96.1/"),
        "{prompt}"
    );
    assert!(!prompt.contains("[structured output failed"), "{prompt}");
    assert!(!prompt.contains("schema coercion"), "{prompt}");
    assert!(!prompt.contains("raw delegated"), "{prompt}");
    assert!(!prompt.contains("salvage"), "{prompt}");
    assert!(!prompt.contains("salvaged"), "{prompt}");
    assert!(!prompt.contains("Task ID:"), "{prompt}");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn deepresearch_workflow_forces_local_when_os_runtime_requested() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-deepresearch-cli-runtime-disabled-{}-{}",
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
    let llm = Arc::new(ScriptedLlmClient::new(vec![]));
    let opts = SessionOptions::new()
        .with_llm_client(llm)
        .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
        .with_max_tool_rounds(4);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(opts))
        .await
        .unwrap();
    session.register_dynamic_workflow_runtime().unwrap();
    let seen_args = std::sync::Arc::new(Mutex::new(Vec::new()));
    session
        .register_dynamic_tool(Arc::new(StructuredRuntimeTool {
            seen_args: std::sync::Arc::clone(&seen_args),
        }))
        .unwrap();

    let args = deep_research_workflow_args("runtime disabled", true);
    let budget = deep_research_default_budget();
    let workflow_budget =
        deep_research_workflow_budget_for_query("runtime disabled", false, budget);
    assert_eq!(args["input"]["os_runtime"], false);
    assert_eq!(args["allowed_tools"], serde_json::json!([]));
    assert_eq!(
        args["input"]["local_max_parallel_tasks"],
        serde_json::json!(workflow_budget.local_max_parallel_tasks)
    );
    assert_eq!(args["input"]["local_research_rounds"], serde_json::json!(1));
    let workflow = run_deepresearch_workflow(&session, args)
        .await
        .expect("DeepResearch workflow should stay local even if runtime was requested");

    assert_eq!(workflow.exit_code, 0, "{}", workflow.output);
    let output: serde_json::Value =
        serde_json::from_str(&workflow.output).expect("workflow output should be JSON");
    assert_eq!(output["mode"], "local_parallel_task");
    assert_eq!(
        output["research"]["metadata"]["results"][0]["structured"]["summary"],
        "Structured DeepResearch track evidence confirms local fan-out completed before synthesis."
    );
    assert_eq!(
        seen_args.lock().unwrap().len(),
        0,
        "DeepResearch must not call the OS Runtime tool-call fan-out path"
    );
    let metadata = workflow.metadata.as_ref().expect("workflow metadata");
    assert_eq!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["status"],
        "completed"
    );
    assert_eq!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]["metadata"]
            ["task_count"],
        serde_json::json!(1)
    );
    assert_eq!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]["local_research"]["output"]["metadata"]
            ["result_count"],
        serde_json::json!(1)
    );
    assert!(
        metadata["dynamic_workflow"]["snapshot"]["steps"]
            .get("runtime_preflight")
            .is_none()
            && metadata["dynamic_workflow"]["snapshot"]["steps"]
                .get("runtime_research")
                .is_none(),
        "runtime tool-call fan-out steps should not be scheduled"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}
