#[derive(Clone, Copy)]
enum CheckerGateScenario {
    Supported,
    BoundedTrack,
    UnacceptedSource,
}

#[derive(Clone)]
struct DirectReviewRoleTool {
    tool_name: &'static str,
    planner_calls: Arc<AtomicUsize>,
    checker_calls: Arc<AtomicUsize>,
    maker_calls: Arc<AtomicUsize>,
    checker_scenario: CheckerGateScenario,
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
                    "synthesis_timeout_secs": 120,
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
            let first_track_status = match self.checker_scenario {
                CheckerGateScenario::BoundedTrack => "bounded",
                CheckerGateScenario::Supported | CheckerGateScenario::UnacceptedSource => {
                    "supported"
                }
            };
            let first_track_url = match self.checker_scenario {
                CheckerGateScenario::UnacceptedSource => "https://invented.example/status",
                CheckerGateScenario::Supported | CheckerGateScenario::BoundedTrack => {
                    "https://official.example/status"
                }
            };
            return Ok(generated_object_output(serde_json::json!({
                "decision": "finalize",
                "coverage_summary": "Two independently hosted sources cover the planned evidence tracks.",
                "report_summary": "The current status is independently corroborated by the retained sources.",
                "verified_findings": ["The requested current status is independently corroborated."],
                "track_assessments": [{
                    "plan_index": 0,
                    "status": first_track_status,
                    "finding": "The official source establishes the current status.",
                    "source_urls": [first_track_url]
                }, {
                    "plan_index": 1,
                    "status": "supported",
                    "finding": "An independent source corroborates the current status.",
                    "source_urls": ["https://independent.example/status"]
                }],
                "stop_condition_assessments": [{
                    "plan_index": 0,
                    "status": "supported",
                    "finding": "The current status is independently corroborated.",
                    "source_urls": [
                        "https://official.example/status",
                        "https://independent.example/status"
                    ]
                }],
                "unresolved_gaps": [],
                "limitations": [],
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

async fn run_direct_review_scenario(
    checker_scenario: CheckerGateScenario,
) -> (serde_json::Value, usize, usize, usize) {
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
        checker_scenario,
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
    let _ = std::fs::remove_dir_all(&workspace);

    (
        output,
        planner_calls.load(Ordering::SeqCst),
        checker_calls.load(Ordering::SeqCst),
        maker_calls.load(Ordering::SeqCst),
    )
}

#[tokio::test]
async fn direct_then_review_combines_synthesis_and_coverage_without_a_maker_turn() {
    let (output, planner_calls, checker_calls, maker_calls) =
        run_direct_review_scenario(CheckerGateScenario::Supported).await;

    assert_eq!(output["plan"]["execution_route"], "direct_then_review");
    assert_eq!(output["checker"]["decision"], "finalize", "{output:#}");
    assert_eq!(
        output["checker"]["contract_validation"]["finalize_gate_passed"],
        true
    );
    assert_eq!(planner_calls, 1);
    assert_eq!(checker_calls, 1);
    assert_eq!(maker_calls, 0);
}

#[tokio::test]
async fn checker_finalize_with_a_bounded_track_is_forced_to_degrade() {
    let (output, _, _, _) = run_direct_review_scenario(CheckerGateScenario::BoundedTrack).await;

    assert_eq!(output["checker"]["decision"], "degrade", "{output:#}");
    assert_eq!(output["mode"], "direct_web_degraded", "{output:#}");
    assert_eq!(
        output["checker"]["contract_validation"]["finalize_gate_passed"],
        false
    );
    assert_eq!(
        output["checker"]["track_assessments"][0]["status"],
        "bounded"
    );
}

#[tokio::test]
async fn checker_assessment_citing_an_unaccepted_url_is_rejected() {
    let (output, _, _, _) = run_direct_review_scenario(CheckerGateScenario::UnacceptedSource).await;

    assert_eq!(output["checker"]["decision"], "degrade", "{output:#}");
    assert_eq!(
        output["checker"]["contract_validation"]["invalid_source_reference_count"],
        1
    );
    assert_eq!(
        output["checker"]["track_assessments"][0]["status"],
        "bounded"
    );
    assert_eq!(
        output["checker"]["track_assessments"][0]["source_urls"],
        serde_json::json!([])
    );
}
