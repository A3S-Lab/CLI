struct DefaultEmptyAdaptiveSearchTool {
    default_calls: Arc<AtomicUsize>,
    fallback_calls: Arc<AtomicUsize>,
    configuration_bound: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for DefaultEmptyAdaptiveSearchTool {
    fn name(&self) -> &str {
        "fallback_planned_web_search"
    }

    fn description(&self) -> &str {
        "Returns no default results and deterministic results for the bounded automatic fallback."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let uses_automatic_fallback = args
            .get("engines")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|engines| {
                engines.len() == 2
                    && engines[0].as_str() == Some("bing_cn")
                    && engines[1].as_str() == Some("brave")
            });
        if !uses_automatic_fallback {
            self.default_calls.fetch_add(1, Ordering::SeqCst);
            let selection_source = if self.configuration_bound.load(Ordering::SeqCst) == 0 {
                "builtin_default"
            } else {
                "config"
            };
            return Ok(ToolOutput::success("[]").with_metadata(serde_json::json!({
                "status": "complete",
                "engine_selection_source": selection_source,
                "selected_engines": ["ddg", "wiki"]
            })));
        }
        self.fallback_calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolOutput::success(
            serde_json::json!([
                {
                    "title": "Official current status",
                    "url": "https://official.example/status",
                    "content": "Adaptive loop current status is operational.",
                    "published_date": "2026-07-12",
                    "engines": ["Bing"]
                },
                {
                    "title": "Independent current status",
                    "url": "https://independent.example/status",
                    "content": "Adaptive loop current status is operational.",
                    "published_date": "2026-07-12",
                    "engines": ["Bing"]
                }
            ])
            .to_string(),
        ))
    }
}

#[tokio::test]
async fn empty_builtin_search_uses_one_bounded_multi_engine_fallback_per_planned_query() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-search-fallback-{}-{}",
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
            maker_calls,
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
    let default_calls = Arc::new(AtomicUsize::new(0));
    let fallback_calls = Arc::new(AtomicUsize::new(0));
    let configuration_bound = Arc::new(AtomicUsize::new(0));
    executor.register_dynamic_tool(Arc::new(DefaultEmptyAdaptiveSearchTool {
        default_calls: Arc::clone(&default_calls),
        fallback_calls: Arc::clone(&fallback_calls),
        configuration_bound: Arc::clone(&configuration_bound),
    }));
    executor.register_dynamic_tool(Arc::new(PlannedLoopFetchTool));
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let mut args = super::deep_research_workflow_args_with_scope(
        "adaptive loop current status evidence",
        false,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = use_planned_web_tools(
        args["source"].as_str().unwrap(),
        "fallback_planned_web_search",
        "planned_web_fetch",
    );
    args["source"] = serde_json::Value::String(source);
    args["limits"]["timeoutMs"] = serde_json::json!(60_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(20);

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("the bounded search fallback should converge");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(default_calls.load(Ordering::SeqCst), 2);
    assert_eq!(fallback_calls.load(Ordering::SeqCst), 2);
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 1);

    configuration_bound.store(1, Ordering::SeqCst);
    let _configured_result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("configured search selection should remain authoritative");
    assert_eq!(default_calls.load(Ordering::SeqCst), 4);
    assert_eq!(fallback_calls.load(Ordering::SeqCst), 2);

    let _ = std::fs::remove_dir_all(&workspace);
}
