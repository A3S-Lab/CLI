#[tokio::test]
async fn llm_plan_and_independent_checker_can_finish_a_narrow_query_without_maker_fanout() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-planned-research-loop-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: false,
            targeted_direct: false,
            repeated_direct: false,
            digest_regression: false,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: false,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: false,
        },
    );
    executor.register_dynamic_tool(Arc::new(PlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(PlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let query = "adaptive loop current status";
    let mut args = super::deep_research_workflow_args_with_scope(
        query,
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "planned_web_search",
        "planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(45_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(12);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("the LLM-planned engineered loop should execute");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["plan"]["answer_shape"], "lookup", "{output:#}");
    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(output["mode"], "direct_web", "{output:#}");
    let first_source = &output["research"]["results"][0]["structured"]["sources"][0];
    assert_eq!(first_source["title"], "Adaptive loop current status");
    assert!(
        first_source["quote_or_fact"]
            .as_str()
            .is_some_and(|quote| quote.contains("service is operational")
                && !quote.contains("Search code")
                && !quote.contains("focus-visible")),
        "{output:#}"
    );
    assert!(
        first_source["reliability"]
            .as_str()
            .is_some_and(|reliability| reliability.contains("fixture")),
        "batch sections must preserve each search result's structured metadata: {output:#}"
    );
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 0);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn direct_evidence_uses_clean_search_facts_instead_of_json_ld_page_metadata() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-direct-json-ld-filter-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::new(AtomicUsize::new(0)),
            checker_calls: Arc::new(AtomicUsize::new(0)),
            maker_calls: Arc::new(AtomicUsize::new(0)),
            investigation: false,
            targeted_direct: false,
            repeated_direct: false,
            digest_regression: false,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: false,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: false,
        },
    );
    executor.register_dynamic_tool(Arc::new(PlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(MetadataOnlyPlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "planned_web_search",
        "metadata_planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(45_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(12);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("JSON-LD filtering should keep direct evidence reportable");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    let sources = output["research"]["results"][0]["structured"]["sources"]
        .as_array()
        .expect("direct evidence should retain sources");

    assert!(!sources.is_empty(), "{output:#}");
    assert!(sources.iter().all(|source| {
        source["quote_or_fact"]
            .as_str()
            .is_some_and(|quote| quote.contains("operational") && !quote.contains("@context"))
    }), "{output:#}");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn checker_failure_preserves_traceable_direct_evidence_as_a_degraded_verification() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-checker-failure-convergence-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: false,
            targeted_direct: false,
            repeated_direct: false,
            digest_regression: false,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: false,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: true,
        },
    );
    executor.register_dynamic_tool(Arc::new(PlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(PlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "planned_web_search",
        "planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(45_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(12);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("checker failure should converge around retained evidence");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["mode"], "direct_web", "{output:#}");
    assert_eq!(output["verification"]["status"], "degraded", "{output:#}");
    assert_eq!(output["verification"]["checker_completed"], false);
    assert!(output.get("checker").is_none(), "{output:#}");
    assert!(output["research"]["results"]
        .as_array()
        .is_some_and(|results| !results.is_empty()));
    assert_eq!(
        super::deep_research_collection_status(&output),
        "completed",
        "{output:#}"
    );
    assert!(!super::deep_research_workflow_needs_recovery_report(
        &result.output
    ));
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 0);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn checker_failure_preserves_traceable_maker_evidence_as_a_degraded_verification() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-maker-checker-failure-convergence-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: true,
            targeted_direct: false,
            repeated_direct: false,
            digest_regression: false,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: false,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: true,
        },
    );
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "Assess competing explanations",
        false,
        super::DeepResearchEvidenceScope::LocalOnly,
    );
    args["limits"]["timeoutMs"] = serde_json::json!(45_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(12);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("checker failure should retain completed maker evidence");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["mode"], "local_parallel_task", "{output:#}");
    assert_eq!(output["verification"]["status"], "degraded", "{output:#}");
    assert_eq!(output["verification"]["checker_completed"], false);
    assert!(output.get("checker").is_none(), "{output:#}");
    assert_eq!(
        super::deep_research_collection_status(&output),
        "completed",
        "{output:#}"
    );
    assert!(!super::deep_research_workflow_needs_recovery_report(
        &result.output
    ));
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 1);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn checker_routes_one_external_fact_gap_to_bounded_direct_retrieval() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-targeted-direct-follow-up-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: false,
            targeted_direct: true,
            repeated_direct: false,
            digest_regression: false,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: false,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: false,
        },
    );
    executor.register_dynamic_tool(Arc::new(PlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(PlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    args["input"]["current_date"] = serde_json::json!("2026-07-13");
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "planned_web_search",
        "planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(20);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("the targeted direct follow-up should execute");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["mode"], "direct_web", "{output:#}");
    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(output["research"]["completed_iterations"], 1, "{output:#}");
    assert_eq!(
        output["research"]["algorithm"], "llm_targeted_direct_retrieval",
        "{output:#}"
    );
    assert_eq!(output["research"]["metadata"]["engineered_loop"], true);
    assert_eq!(
        output["research"]["results"][0]["structured"]["sources"][0]["url_or_path"],
        "https://official.example/status",
        "{output:#}"
    );
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 2);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 0);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn partial_direct_gap_routes_to_maker_instead_of_another_search_loop() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-repeated-direct-convergence-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: false,
            targeted_direct: false,
            repeated_direct: true,
            digest_regression: false,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: false,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: false,
        },
    );
    executor.register_dynamic_tool(Arc::new(PlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(PlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "planned_web_search",
        "planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(24);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("the repeated direct gap should converge through a maker");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(output["plan"]["execution_route"], "direct_only");
    assert_eq!(checker_calls.load(Ordering::SeqCst), 3);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 1);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn llm_checker_can_request_one_targeted_follow_up_then_finalize() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-planned-investigation-loop-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: true,
            targeted_direct: false,
            repeated_direct: false,
            digest_regression: false,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: false,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: false,
        },
    );
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "Assess competing explanations and recommend an action",
        false,
        super::DeepResearchEvidenceScope::LocalOnly,
    );
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(12);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("the two-iteration LLM-planned loop should execute");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["plan"]["answer_shape"], "investigation");
    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(output["research"]["completed_rounds"], 2, "{output:#}");
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 2);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 2);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn checker_digest_keeps_maker_evidence_after_oversized_direct_evidence() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-checker-evidence-digest-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: false,
            targeted_direct: false,
            repeated_direct: false,
            digest_regression: true,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: false,
            // The checker deliberately outlives the retrieval clock. Its wait must
            // not consume the independent evidence-retrieval budget.
            first_checker_delay_ms: 600,
            retrieval_timeout_override_ms: 500,
            checker_failure: false,
        },
    );
    executor.register_dynamic_tool(Arc::new(OversizedPlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(PlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status evidence",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "oversized_planned_web_search",
        "planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["input"]["direct_web_max_results"] = serde_json::json!(12);
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(20);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("the oversized-evidence workflow should execute");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(output["plan"]["execution_route"], "direct_then_maker");
    assert_eq!(checker_calls.load(Ordering::SeqCst), 2);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 1);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn failed_maker_first_pass_recovers_through_direct_evidence_and_checker() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-maker-direct-recovery-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: false,
            targeted_direct: false,
            repeated_direct: false,
            digest_regression: false,
            linked_url_priority: false,
            maker_failure: true,
            maker_then_direct: false,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: false,
        },
    );
    executor.register_dynamic_tool(Arc::new(PlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(PlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status evidence",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "planned_web_search",
        "planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(16);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("maker failure should recover through direct evidence");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["mode"], "direct_web", "{output:#}");
    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 1);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn maker_first_direct_follow_up_is_visible_to_the_next_checker() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-maker-direct-checker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: false,
            targeted_direct: false,
            repeated_direct: false,
            digest_regression: false,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: true,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: false,
        },
    );
    executor.register_dynamic_tool(Arc::new(PlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(PlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status evidence",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "planned_web_search",
        "planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(20);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("maker-first direct follow-up should converge");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(output["mode"], "hybrid_direct_web_parallel", "{output:#}");
    assert_eq!(output["research"]["completed_rounds"], 1, "{output:#}");
    assert_eq!(
        output["seed_research"]["results"][0]["structured"]["sources"][0]["url_or_path"],
        "https://official.example/status",
        "{output:#}"
    );
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 2);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn source_observed_link_precedes_checker_generated_seed_url() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-observed-link-priority-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    let fetched_urls = Arc::new(Mutex::new(Vec::new()));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: false,
            targeted_direct: false,
            repeated_direct: false,
            digest_regression: false,
            linked_url_priority: true,
            maker_failure: false,
            maker_then_direct: false,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: false,
        },
    );
    executor.register_dynamic_tool(Arc::new(PlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(ObservedLinkFetchTool {
        fetched_urls: Arc::clone(&fetched_urls),
    }));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "planned_web_search",
        "observed_link_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(20);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("the linked-source follow-up should execute");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(checker_calls.load(Ordering::SeqCst), 2);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 0);
    let fetched_urls = fetched_urls.lock().unwrap();
    assert!(
        fetched_urls
            .iter()
            .any(|url| url == "https://observed.example/detail"),
        "source-observed link was not fetched first: {fetched_urls:?}"
    );
    assert!(
        !fetched_urls
            .iter()
            .any(|url| url == "https://invented.example/missing"),
        "checker-generated URL displaced a source-observed link: {fetched_urls:?}"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn insufficient_remaining_budget_finalizes_instead_of_starting_maker() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-budget-limited-convergence-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: false,
            targeted_direct: false,
            repeated_direct: true,
            digest_regression: false,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: false,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: false,
        },
    );
    executor.register_dynamic_tool(Arc::new(PlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(PlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "planned_web_search",
        "planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    let started_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 40_000;
    args["input"]["run_started_at_ms"] = serde_json::json!(started_at_ms);
    args["input"]["workflow_timeout_ms"] = serde_json::json!(60_000);
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(20);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("the budget-limited workflow should converge");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["budget_limited"], true, "{output:#}");
    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(output["checker"]["next_action"], "none", "{output:#}");
    assert!(
        output["checker"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no further maker pass")),
        "{output:#}"
    );
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 2);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 0);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn prompt_fallback_uses_observed_turn_budget_to_preserve_the_planned_maker() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-initial-maker-budget-reserve-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: false,
            targeted_direct: false,
            // This fixture selects direct_then_maker. Prompt structured output
            // packs its tracks into one bounded source-grounded maker turn.
            digest_regression: true,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: false,
            repeated_direct: true,
            first_checker_delay_ms: 0,
            retrieval_timeout_override_ms: 0,
            checker_failure: false,
        },
    );
    executor.register_dynamic_tool(Arc::new(OversizedPlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(PlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status evidence",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "oversized_planned_web_search",
        "planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["input"]["direct_web_max_results"] = serde_json::json!(12);
    let started_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 5_000;
    args["input"]["run_started_at_ms"] = serde_json::json!(started_at_ms);
    args["input"]["workflow_timeout_ms"] = serde_json::json!(60_000);
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(20);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("the event-timed prompt maker workflow should converge");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["plan"]["execution_route"], "direct_then_maker");
    assert_eq!(output["mode"], "hybrid_direct_web_parallel", "{output:#}");
    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        maker_calls.load(Ordering::SeqCst),
        1,
        "the runtime must not override the LLM-planned maker with two worst-case timeout ceilings"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn targeted_follow_up_closes_with_prior_check_when_recheck_cannot_fit() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-follow-up-checker-budget-reserve-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let planner_calls = Arc::new(AtomicUsize::new(0));
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::clone(&planner_calls),
            checker_calls: Arc::clone(&checker_calls),
            maker_calls: Arc::clone(&maker_calls),
            investigation: false,
            targeted_direct: true,
            repeated_direct: false,
            digest_regression: false,
            linked_url_priority: false,
            maker_failure: false,
            maker_then_direct: false,
            first_checker_delay_ms: 2_500,
            retrieval_timeout_override_ms: 0,
            checker_failure: false,
        },
    );
    executor.register_dynamic_tool(Arc::new(PlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(PlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "planned_web_search",
        "planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    let started_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 47_000;
    args["input"]["run_started_at_ms"] = serde_json::json!(started_at_ms);
    args["input"]["workflow_timeout_ms"] = serde_json::json!(60_000);
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(20);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("the prior checked findings should close before the hard fuse");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["mode"], "direct_web", "{output:#}");
    assert_eq!(output["budget_limited"], true, "{output:#}");
    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert!(
        output["checker"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("another independent checker pass")),
        "{output:#}"
    );
    assert_eq!(output["research"]["completed_iterations"], 1, "{output:#}");
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 0);

    let _ = std::fs::remove_dir_all(&workspace);
}
