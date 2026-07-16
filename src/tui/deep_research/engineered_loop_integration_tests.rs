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
            checker_failure_at: None,
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
async fn direct_collection_skips_non_document_url_candidates() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-low-value-source-filter-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let fetched_urls = Arc::new(Mutex::new(Vec::new()));
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
            checker_failure_at: None,
        },
    );
    executor.register_dynamic_tool(Arc::new(NoisyPlannedLoopSearchTool));
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
        "noisy_planned_web_search",
        "observed_link_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(45_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(12);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("low-value source candidates should be filtered");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let fetched_urls = fetched_urls.lock().unwrap();
    assert_eq!(fetched_urls.len(), 2, "{fetched_urls:?}");
    assert!(
        fetched_urls.iter().all(|url| {
            url == "https://official.example/status" || url == "https://independent.example/status"
        }),
        "{fetched_urls:?}"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn transient_fetch_failures_receive_one_bounded_transport_retry() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-direct-fetch-retry-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let calls = Arc::new(AtomicUsize::new(0));
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
            checker_failure_at: None,
        },
    );
    executor.register_dynamic_tool(Arc::new(PlannedLoopSearchTool));
    executor.register_dynamic_tool(Arc::new(TransientPlannedLoopFetchTool {
        calls: Arc::clone(&calls),
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
        "transient_planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(45_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(16);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("transient fetches should recover inside the direct collection step");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(
        output["research"]["metadata"]["transport_retry_count"],
        2,
        "{output:#}"
    );
    assert_eq!(
        output["research"]["metadata"]["transport_retry_success_count"],
        2,
        "{output:#}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 4);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn zero_evidence_after_a_checked_follow_up_skips_a_redundant_checker_call() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-zero-evidence-follow-up-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    let checker_calls = Arc::new(AtomicUsize::new(0));
    let maker_calls = Arc::new(AtomicUsize::new(0));
    register_planned_loop_tools(
        &executor,
        PlannedLoopTaskTool {
            tool_name: "parallel_task",
            planner_calls: Arc::new(AtomicUsize::new(0)),
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
            checker_failure_at: None,
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
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(20);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("zero accepted evidence should converge without a second checker");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(output["mode"], "direct_web_degraded", "{output:#}");
    assert_eq!(output["checker"]["decision"], "degrade", "{output:#}");
    assert_eq!(output["zero_evidence_after_follow_up"], true, "{output:#}");
    assert_eq!(checker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 0);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn metadata_only_pages_remain_discovery_leads_instead_of_evidence() {
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
            checker_failure_at: None,
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
        .expect("JSON-LD filtering should keep metadata-only pages out of evidence");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert_eq!(output["research"]["metadata"]["source_count"], 0);
    assert_eq!(output["research"]["metadata"]["fetched_count"], 0);
    assert!(
        output["research"]["metadata"]["candidate_leads"]
            .as_array()
            .is_some_and(|leads| !leads.is_empty()),
        "{output:#}"
    );
    assert!(
        output["research"]["results"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "search snippets must not become structured evidence: {output:#}"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn planned_seed_uses_page_identity_when_the_entity_occurs_late_in_a_long_query() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-late-entity-planned-seed-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let executor = ToolExecutor::new(workspace.to_string_lossy().to_string());
    executor.register_dynamic_tool(Arc::new(LateEntityPlannedSeedTool));
    executor.register_dynamic_tool(Arc::new(LateEntityPlannedSeedFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let query = format!(
        "{}最后核验 Project Quasar 的隔离边界。",
        "请基于可追溯的一手资料核验生产隔离方案，并明确区分已经证实的事实、独立验证和仍待验证的判断。".repeat(4)
    );
    let mut args = super::deep_research_workflow_args_with_scope(
        &query,
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "planned_web_search",
        "late_entity_planned_seed_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(45_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(8);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("the planned seed should remain relevant after query compaction");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(
        output["research"]["metadata"]["source_count"], 1,
        "{output:#}"
    );
    assert_eq!(
        output["research"]["results"][0]["structured"]["sources"][0]["url_or_path"],
        "https://github.com/example/project-quasar",
        "{output:#}"
    );
    assert!(
        output["research"]["results"][0]["structured"]["sources"][0]["quote_or_fact"]
            .as_str()
            .is_some_and(|quote| quote.contains("separate guest kernel")),
        "{output:#}"
    );

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
            checker_failure_at: Some(0),
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
            checker_failure_at: Some(0),
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
            checker_failure_at: None,
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
            checker_failure_at: None,
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
            checker_failure_at: None,
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
            checker_failure_at: None,
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
    assert!(
        output["seed_research"]["metadata"]["source_count"]
            .as_u64()
            .is_some_and(|count| count >= 2),
        "cumulative direct evidence lost its source count: {output:#}"
    );
    assert!(
        output["seed_research"]["metadata"]["fetched_count"]
            .as_u64()
            .is_some_and(|count| count >= 2),
        "cumulative direct evidence lost its fetched-page count: {output:#}"
    );
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
            checker_failure_at: None,
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
            checker_failure_at: None,
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
            checker_failure_at: None,
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
            .any(|url| url == "https://official.example/detail"),
        "source-observed relative link was not resolved and fetched first: {fetched_urls:?}; output: {output:#}"
    );
    assert!(
        !fetched_urls
            .iter()
            .any(|url| url == "https://invented.example/missing"),
        "checker-generated URL displaced a source-observed link: {fetched_urls:?}"
    );
    assert!(
        !fetched_urls
            .iter()
            .any(|url| url == "https://official.example/LICENSE"),
        "an unrelated sibling link entered the evidence follow-up: {fetched_urls:?}"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn insufficient_remaining_budget_degrades_instead_of_claiming_completion() {
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
            checker_failure_at: None,
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
    assert_eq!(output["mode"], "direct_web_degraded", "{output:#}");
    assert_eq!(output["checker"]["decision"], "degrade", "{output:#}");
    assert_eq!(output["checker"]["next_action"], "none", "{output:#}");
    assert!(
        output["checker"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("remain") && reason.contains("degraded")),
        "{output:#}"
    );
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 1);
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
            checker_failure_at: None,
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
            checker_failure_at: None,
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

    assert_eq!(output["mode"], "direct_web_degraded", "{output:#}");
    assert_eq!(output["budget_limited"], true, "{output:#}");
    assert_eq!(output["checker"]["decision"], "degrade", "{output:#}");
    assert!(
        output["checker"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("run remains degraded")),
        "{output:#}"
    );
    assert_eq!(output["research"]["completed_iterations"], 1, "{output:#}");
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 0);

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn targeted_follow_up_remains_degraded_when_recheck_fails() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-follow-up-checker-failure-convergence-{}-{}",
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
            checker_failure_at: Some(1),
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
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(20);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("a failed follow-up recheck should converge around the prior checked findings");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["mode"], "direct_web_degraded", "{output:#}");
    assert_eq!(output["checker"]["decision"], "degrade", "{output:#}");
    assert_eq!(output["verification"]["status"], "degraded", "{output:#}");
    assert_eq!(output["verification"]["checker_completed"], false);
    assert_eq!(output["verification"]["prior_checker_retained"], true);
    assert!(output["checker"]["unresolved_gaps"]
        .as_array()
        .is_some_and(|gaps| gaps.iter().any(|gap| gap
            .as_str()
            .is_some_and(|gap| gap.contains("not independently rechecked")))));
    assert_eq!(output["research"]["completed_iterations"], 1, "{output:#}");
    assert_eq!(
        super::deep_research_collection_status(&output),
        "completed",
        "{output:#}"
    );
    assert!(
        !super::deep_research_evidence_package_is_complete_for_query(
            query,
            super::DeepResearchEvidenceScope::WebAndWorkspace,
            &result.output,
            result.metadata.as_ref(),
        )
    );
    assert!(!super::deep_research_workflow_needs_recovery_report(
        &result.output
    ));
    assert_eq!(
        super::deep_research_report_outcome_for_workflow(
            query,
            super::DeepResearchEvidenceScope::WebAndWorkspace,
            &result.output,
            result.metadata.as_ref(),
        ),
        super::DeepResearchRunOutcome::Qualified,
    );
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 2);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 0);

    let _ = std::fs::remove_dir_all(&workspace);
}
