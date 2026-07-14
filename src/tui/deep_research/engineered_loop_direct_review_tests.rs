#[derive(Clone)]
struct DirectReviewRoleTool {
    tool_name: &'static str,
    planner_calls: Arc<AtomicUsize>,
    checker_calls: Arc<AtomicUsize>,
    maker_calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for DirectReviewRoleTool {
    fn name(&self) -> &str {
        self.tool_name
    }

    fn description(&self) -> &str {
        "Returns a deterministic direct-then-review plan and coverage decision."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let schema_name = args
            .get("schema_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if schema_name == "deep_research_plan" {
            self.planner_calls.fetch_add(1, Ordering::SeqCst);
            return Ok(generated_object_output(serde_json::json!({
                "answer_shape": "investigation",
                "freshness_required": true,
                "workspace_evidence_required": false,
                "execution_route": "direct_then_review",
                "report_title": "Adaptive Loop Evidence Review",
                "phases": ["Retrieve independent evidence", "Synthesize and verify coverage"],
                "tracks": ["Primary evidence", "Independent corroboration"],
                "search_queries": [
                    "adaptive loop current status official",
                    "adaptive loop current status independent"
                ],
                "seed_urls": [],
                "budget": {
                    "retrieval_timeout_secs": 30,
                    "synthesis_timeout_secs": 45,
                    "max_iterations": 2,
                    "max_parallel_tasks": 2,
                    "max_steps_per_task": 2,
                    "direct_searches": 2,
                    "direct_fetches": 2
                },
                "stop_conditions": ["The current status is independently corroborated"]
            })));
        }
        if schema_name == "deep_research_check" {
            self.checker_calls.fetch_add(1, Ordering::SeqCst);
            let prompt = args
                .get("prompt")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            anyhow::ensure!(
                prompt.contains("https://official.example/status")
                    && prompt.contains("https://independent.example/status"),
                "the combined review lost direct evidence"
            );
            return Ok(generated_object_output(serde_json::json!({
                "decision": "finalize",
                "coverage_summary": "Two independently hosted sources cover the planned evidence tracks.",
                "report_summary": "The current status is independently corroborated by the retained sources.",
                "verified_findings": ["The requested current status is independently corroborated."],
                "unresolved_gaps": [],
                "contradictions": [],
                "next_action": "none",
                "search_queries": [],
                "seed_urls": [],
                "next_tracks": [],
                "reason": "The observable completion criterion is satisfied."
            })));
        }
        self.maker_calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolOutput::error(
            "direct_then_review must not schedule a redundant maker",
        ))
    }
}

#[tokio::test]
async fn direct_then_review_combines_synthesis_and_coverage_without_a_maker_turn() {
    let workspace = std::env::temp_dir().join(format!(
        "a3s-direct-review-{}-{}",
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
    let role = DirectReviewRoleTool {
        tool_name: "generate_object",
        planner_calls: Arc::clone(&planner_calls),
        checker_calls: Arc::clone(&checker_calls),
        maker_calls: Arc::clone(&maker_calls),
    };
    executor.register_dynamic_tool(Arc::new(role.clone()));
    executor.register_dynamic_tool(Arc::new(DirectReviewRoleTool {
        tool_name: "parallel_task",
        ..role
    }));
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
        .expect("the direct-then-review workflow should converge");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["plan"]["execution_route"], "direct_then_review");
    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(planner_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(maker_calls.load(Ordering::SeqCst), 0);

    let _ = std::fs::remove_dir_all(&workspace);
}
