use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use a3s_code_core::tools::{Tool, ToolContext, ToolExecutor, ToolOutput};

#[derive(Clone)]
struct PlannedLoopTaskTool {
    tool_name: &'static str,
    planner_calls: Arc<AtomicUsize>,
    checker_calls: Arc<AtomicUsize>,
    maker_calls: Arc<AtomicUsize>,
    investigation: bool,
    targeted_direct: bool,
    repeated_direct: bool,
    digest_regression: bool,
    linked_url_priority: bool,
    maker_failure: bool,
    maker_then_direct: bool,
    first_checker_delay_ms: u64,
    retrieval_timeout_override_ms: u64,
    checker_failure_at: Option<usize>,
}

struct PlannedLoopSearchTool;
struct NoisyPlannedLoopSearchTool;
struct OversizedPlannedLoopSearchTool;
struct UnicodeFocusPlannerTool;
struct UnicodeFocusSearchTool;
struct UnicodeFocusFetchTool;
struct PlannedLoopFetchTool;
struct TransientPlannedLoopFetchTool {
    calls: Arc<AtomicUsize>,
}
struct MetadataOnlyPlannedLoopFetchTool;
struct LateEntityPlannedSeedTool;
struct LateEntityPlannedSeedFetchTool;
struct ObservedLinkFetchTool {
    fetched_urls: Arc<Mutex<Vec<String>>>,
}

fn parallel_output(structured: serde_json::Value) -> ToolOutput {
    ToolOutput::success("structured loop role completed").with_metadata(serde_json::json!({
        "task_count": 1,
        "result_count": 1,
        "success_count": 1,
        "failed_count": 0,
        "all_success": true,
        "partial_failure": false,
        "allow_partial_failure": false,
        "results": [{
            "task_id": "fixture-task",
            "agent": "deep-research",
            "success": true,
            "structured": structured
        }]
    }))
}

fn generated_object_output(object: serde_json::Value) -> ToolOutput {
    ToolOutput::success(
        serde_json::json!({
            "object": object,
            "repair_rounds": 0,
            "mode_used": "prompt"
        })
        .to_string(),
    )
    .with_metadata(serde_json::json!({
        "schema_name": "fixture",
        "mode_used": "prompt",
        "repair_rounds": 0
    }))
}

fn register_planned_loop_tools(executor: &ToolExecutor, tool: PlannedLoopTaskTool) {
    let mut object_tool = tool.clone();
    object_tool.tool_name = "generate_object";
    executor.register_dynamic_tool(Arc::new(object_tool));

    let mut task_tool = tool;
    task_tool.tool_name = "parallel_task";
    executor.register_dynamic_tool(Arc::new(task_tool));
}

fn use_planned_web_tools(source: &str, search_tool: &str, fetch_tool: &str) -> String {
    source
        .replace(
            "ctx.tool(\"web_search\"",
            &format!("ctx.tool(\"{search_tool}\""),
        )
        .replace(
            "ctx.tool(\"web_fetch\"",
            &format!("ctx.tool(\"{fetch_tool}\""),
        )
        .replace("tool: \"web_search\"", &format!("tool: \"{search_tool}\""))
        .replace("tool: \"web_fetch\"", &format!("tool: \"{fetch_tool}\""))
}

#[async_trait::async_trait]
impl Tool for PlannedLoopTaskTool {
    fn name(&self) -> &str {
        self.tool_name
    }

    fn description(&self) -> &str {
        "Returns deterministic planner and checker decisions for the engineered-loop test."
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
        let description = args
            .pointer("/tasks/0/description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let role_output = |value| {
            if self.tool_name == "generate_object" {
                generated_object_output(value)
            } else {
                parallel_output(value)
            }
        };
        if schema_name == "deep_research_plan" || description.starts_with("Plan research") {
            self.planner_calls.fetch_add(1, Ordering::SeqCst);
            if self.investigation
                || self.digest_regression
                || self.maker_failure
                || self.maker_then_direct
            {
                return Ok(role_output(serde_json::json!({
                    "answer_shape": "investigation",
                    "freshness_required": false,
                    "workspace_evidence_required": false,
                    "execution_route": if self.digest_regression { "direct_then_maker" } else { "maker_first" },
                    "report_title": "Adaptive Loop Evidence Assessment",
                    "phases": [{
                        "name": "primary investigation",
                        "success_criterion": "Primary evidence and counterevidence are both covered."
                    }],
                    "tracks": if self.digest_regression {
                        vec![
                            serde_json::json!({
                                "title": "Primary explanation",
                                "focus": "Establish the primary evidence and causal account."
                            }),
                            serde_json::json!({
                                "title": "Independent challenge",
                                "focus": "Test the explanation against independent evidence."
                            }),
                        ]
                    } else {
                        vec![serde_json::json!({
                            "title": "Primary explanation",
                            "focus": "Establish the primary evidence and causal account."
                        })]
                    },
                    "search_queries": ["adaptive loop current status evidence"],
                    "seed_urls": if self.digest_regression {
                        vec![
                            "https://seed-1.example/status",
                            "https://seed-2.example/status",
                            "https://seed-3.example/status",
                            "https://seed-4.example/status",
                        ]
                    } else {
                        Vec::new()
                    },
                    "budget": {
                        "retrieval_timeout_ms": if self.retrieval_timeout_override_ms > 0 {
                            self.retrieval_timeout_override_ms
                        } else {
                            60000
                        },
                        "synthesis_timeout_ms": 30000,
                        "max_iterations": 2,
                        "max_parallel_tasks": 2,
                        "max_steps_per_task": 2,
                        "per_task_timeout_ms": 15000,
                        "direct_searches": 1,
                        "direct_fetches": if self.digest_regression { 4 } else { 2 }
                    },
                    "stop_conditions": ["checker confirms the recommendation survives counterevidence"]
                })));
            }
            return Ok(role_output(serde_json::json!({
                "answer_shape": "lookup",
                "freshness_required": true,
                "workspace_evidence_required": false,
                "execution_route": "direct_only",
                "report_title": "Adaptive Loop Current Status",
                "phases": [{
                    "name": "retrieve and verify",
                    "success_criterion": "The current fact is traceable and corroborated."
                }],
                "tracks": [{
                    "title": "Current fact",
                    "focus": "Retrieve and corroborate the requested status without broadening."
                }],
                "search_queries": ["adaptive loop current status official", "adaptive loop current status independent"],
                "seed_urls": [],
                "budget": {
                    "retrieval_timeout_ms": 30000,
                    "synthesis_timeout_ms": 15000,
                    "max_iterations": if self.targeted_direct || self.repeated_direct || self.linked_url_priority { 2 } else { 1 },
                    "max_parallel_tasks": 1,
                    "max_steps_per_task": 2,
                    "per_task_timeout_ms": 10000,
                    "direct_searches": if self.linked_url_priority { 1 } else { 2 },
                    "direct_fetches": if self.linked_url_priority { 1 } else { 2 }
                },
                "stop_conditions": ["the checker confirms the requested status is traceable"]
            })));
        }
        if schema_name == "deep_research_check" || description.starts_with("Check evidence") {
            anyhow::ensure!(
                args.get("max_repair_attempts")
                    .or_else(|| args.pointer("/tasks/0/max_repair_attempts"))
                    .and_then(serde_json::Value::as_u64)
                    == Some(1),
                "checker must receive one bounded structured-output repair attempt"
            );
            let checker_index = self.checker_calls.fetch_add(1, Ordering::SeqCst);
            let prompt = args
                .get("prompt")
                .or_else(|| args.pointer("/tasks/0/prompt"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            anyhow::ensure!(
                prompt.contains("Workflow budget:"),
                "checker prompt must expose the remaining workflow budget"
            );
            anyhow::ensure!(
                prompt.contains("A URL, title, or search snippet alone is a discovery lead")
                    && prompt.contains("exact supporting source URL"),
                "checker prompt must enforce source quality and claim traceability"
            );
            let checked_role_output = |mut value: serde_json::Value| {
                let source_url = [
                    "https://oversized-1.example/status",
                    "https://official.example/status",
                    "https://evidence2.example/research",
                    "https://evidence1.example/research",
                ]
                .into_iter()
                .find(|candidate| prompt.contains(candidate));
                let supported =
                    value.get("decision").and_then(serde_json::Value::as_str) == Some("finalize");
                let finding = value
                    .get("verified_findings")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|findings| findings.first())
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("The planned evidence obligation remains bounded.")
                    .to_string();
                let source_urls = source_url
                    .filter(|_| supported)
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                let track_count = if self.digest_regression { 2 } else { 1 };
                value["track_assessments"] = serde_json::Value::Array(
                    (0..track_count)
                        .map(|plan_index| {
                            serde_json::json!({
                                "plan_index": plan_index,
                                "status": if supported { "supported" } else { "bounded" },
                                "finding": finding.clone(),
                                "source_urls": source_urls.clone(),
                            })
                        })
                        .collect(),
                );
                value["stop_condition_assessments"] = serde_json::json!([{
                    "plan_index": 0,
                    "status": if supported { "supported" } else { "bounded" },
                    "finding": finding,
                    "source_urls": source_urls,
                }]);
                value["limitations"] = serde_json::json!([]);
                role_output(value)
            };
            if self.checker_failure_at == Some(checker_index) {
                return Ok(ToolOutput::error("simulated checker timeout"));
            }
            if checker_index == 0 && self.first_checker_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(
                    self.first_checker_delay_ms,
                ))
                .await;
            }
            if self.linked_url_priority && checker_index == 0 {
                return Ok(checked_role_output(serde_json::json!({
                    "decision": "continue",
                    "coverage_summary": "One linked primary detail should be fetched before finalizing.",
                    "report_summary": "The current status is supported, pending one linked primary detail.",
                    "verified_findings": ["The retained sources support the current status."],
                    "unresolved_gaps": ["linked primary detail"],
                    "contradictions": [],
                    "next_action": "direct_retrieval",
                    "search_queries": [],
                    "seed_urls": ["https://invented.example/missing"],
                    "next_tracks": [],
                    "reason": "A bounded linked-source fetch can close the gap."
                })));
            }
            if self.maker_failure {
                anyhow::ensure!(
                    prompt.chars().count() <= 30_000,
                    "checker prompt exceeded the bounded convergence envelope"
                );
                anyhow::ensure!(
                    prompt.contains("no usable evidence"),
                    "checker did not receive the failed maker attempt in cumulative evidence"
                );
                return Ok(checked_role_output(serde_json::json!({
                    "decision": "finalize",
                    "coverage_summary": "Direct recovery evidence is sufficient after the maker timed out.",
                    "report_summary": "Direct sources establish the requested current status.",
                    "verified_findings": ["The current status is confirmed by retained direct evidence."],
                    "unresolved_gaps": [],
                    "contradictions": [],
                    "next_action": "none",
                    "search_queries": [],
                    "seed_urls": [],
                    "next_tracks": [],
                    "reason": "The direct recovery sources satisfy the observable completion criterion."
                })));
            }
            if self.maker_then_direct {
                if checker_index == 0 {
                    return Ok(checked_role_output(serde_json::json!({
                        "decision": "continue",
                        "coverage_summary": "Maker evidence needs one current primary source.",
                        "report_summary": "The analysis is supported but still needs one current primary source.",
                        "verified_findings": ["The maker evidence supports the current analysis."],
                        "unresolved_gaps": ["one current primary source"],
                        "contradictions": [],
                        "next_action": "direct_retrieval",
                        "search_queries": ["adaptive loop current status official"],
                        "seed_urls": [],
                        "next_tracks": [],
                        "reason": "One bounded direct lookup closes the checked gap."
                    })));
                }
                anyhow::ensure!(
                    prompt.contains("https://official.example/status"),
                    "checker lost direct evidence gathered after a maker-first round"
                );
                return Ok(checked_role_output(serde_json::json!({
                    "decision": "finalize",
                    "coverage_summary": "Maker and direct evidence now cover the question.",
                    "report_summary": "Maker analysis and a current primary source jointly answer the question.",
                    "verified_findings": ["The current primary source corroborates the maker analysis."],
                    "unresolved_gaps": [],
                    "contradictions": [],
                    "next_action": "none",
                    "search_queries": [],
                    "seed_urls": [],
                    "next_tracks": [],
                    "reason": "The cumulative evidence satisfies the stop condition."
                })));
            }
            if (self.targeted_direct || self.repeated_direct) && checker_index == 0 {
                return Ok(checked_role_output(serde_json::json!({
                    "decision": "continue",
                    "coverage_summary": "The baseline is supported, but one current externally retrievable fact is missing.",
                    "report_summary": "The retained sources support the baseline finding.",
                    "verified_findings": ["The baseline finding is source-backed."],
                    "unresolved_gaps": ["one current fact"],
                    "contradictions": [],
                    "next_action": "direct_retrieval",
                    "search_queries": ["adaptive loop missing current fact official"],
                    "seed_urls": [],
                    "next_tracks": [],
                    "reason": "One bounded direct lookup can close the remaining gap."
                })));
            }
            if self.repeated_direct
                && checker_index == 1
                && self.maker_calls.load(Ordering::SeqCst) == 0
            {
                return Ok(checked_role_output(serde_json::json!({
                    "decision": "continue",
                    "coverage_summary": "The bounded direct retry did not close the checked gap.",
                    "report_summary": "The baseline remains supported, while one consequential gap is unresolved.",
                    "verified_findings": ["The baseline finding remains source-backed."],
                    "unresolved_gaps": ["one consequential gap still needs evidence production"],
                    "contradictions": [],
                    "next_action": "maker",
                    "search_queries": ["adaptive loop repeated lookup"],
                    "seed_urls": [],
                    "next_tracks": [],
                    "reason": "The direct attempt did not close the gap."
                })));
            }
            if self.investigation && checker_index == 0 {
                return Ok(checked_role_output(serde_json::json!({
                    "decision": "continue",
                    "coverage_summary": "The primary explanation is covered but counterevidence is still missing.",
                    "report_summary": "The current evidence supports the primary explanation.",
                    "verified_findings": ["The primary explanation is supported by traceable evidence."],
                    "unresolved_gaps": ["credible counterevidence"],
                    "contradictions": [],
                    "next_action": "maker",
                    "search_queries": [],
                    "seed_urls": [],
                    "next_tracks": [{
                        "title": "Counterevidence",
                        "focus": "Test the primary explanation against credible counterevidence."
                    }],
                    "reason": "One consequential gap justifies one targeted follow-up iteration."
                })));
            }
            if self.digest_regression && checker_index == 0 {
                anyhow::ensure!(
                    self.maker_calls.load(Ordering::SeqCst) == 1,
                    "direct_then_maker must run the planned maker before its first checker"
                );
                anyhow::ensure!(
                    prompt.contains("MAKER_DIGEST_MARKER")
                        && prompt.contains("https://oversized-1.example/status")
                        && prompt.contains("SOURCE_FACT_TAIL"),
                    "the first post-maker checker lost cumulative direct and maker evidence"
                );
                return Ok(checked_role_output(serde_json::json!({
                    "decision": "continue",
                    "coverage_summary": "Maker evidence is preserved; one direct fact remains.",
                    "report_summary": "The maker analysis is preserved and supported by direct evidence.",
                    "verified_findings": ["The maker analysis survives comparison with direct evidence."],
                    "unresolved_gaps": ["one bounded fact after maker analysis"],
                    "contradictions": [],
                    "next_action": "direct_retrieval",
                    "search_queries": ["adaptive loop post-maker fact"],
                    "seed_urls": [],
                    "next_tracks": [],
                    "reason": "One direct lookup can close the remaining gap."
                })));
            }
            if self.digest_regression && checker_index == 1 {
                anyhow::ensure!(
                    prompt.chars().count() <= 30_000,
                    "combined checker prompt exceeded the bounded convergence envelope"
                );
                anyhow::ensure!(
                    prompt.contains("MAKER_DIGEST_MARKER"),
                    "checker prompt dropped schema-validated maker evidence"
                );
                anyhow::ensure!(
                    prompt.contains("SOURCE_FACT_TAIL"),
                    "checker prompt truncated the retained source fact"
                );
                anyhow::ensure!(
                    prompt.contains("https://oversized-1.example/status"),
                    "checker prompt dropped the direct evidence class"
                );
                anyhow::ensure!(
                    prompt.contains("MAKER_DIGEST_MARKER")
                        && prompt.contains("https://oversized-1.example/status"),
                    "post-maker direct follow-up checker lost cumulative evidence"
                );
            }
            return Ok(checked_role_output(serde_json::json!({
                "decision": "finalize",
                "coverage_summary": "Two independently hosted fetched sources support the current fact.",
                "report_summary": "Two independent sources confirm the current fact.",
                "verified_findings": ["The requested current fact is independently corroborated."],
                "unresolved_gaps": [],
                "contradictions": [],
                "next_action": "none",
                "search_queries": [],
                "seed_urls": [],
                "next_tracks": [],
                "reason": "The observable completion criterion is satisfied."
            })));
        }
        if self.digest_regression {
            let prompt = args
                .get("prompt")
                .or_else(|| args.pointer("/tasks/0/prompt"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            anyhow::ensure!(
                prompt.contains("Observed candidate URLs")
                    && prompt.contains("https://oversized-1.example/status"),
                "maker prompt did not surface observed direct-evidence URLs"
            );
            anyhow::ensure!(
                prompt.contains("return the existing source-backed evidence")
                    && prompt.contains("without a tool call")
                    && prompt.contains("do not refetch"),
                "prompt fallback maker did not reuse the existing evidence package"
            );
            if self.tool_name == "generate_object" {
                anyhow::ensure!(
                    args.get("schema_name").and_then(serde_json::Value::as_str)
                        == Some("deep_research_evidence")
                        && args
                            .get("max_repair_attempts")
                            .and_then(serde_json::Value::as_u64)
                            == Some(1),
                    "source-grounded prompt maker must use one v5.2.2 structured call"
                );
            } else {
                anyhow::ensure!(
                    args.pointer("/tasks/0/max_steps")
                        .and_then(serde_json::Value::as_u64)
                        == Some(3),
                    "maker must reserve a structured-finalization turn after bounded evidence collection"
                );
                anyhow::ensure!(
                    args.get("tasks")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|tasks| tasks.len() == 1),
                    "prompt-based structured output must pack planned tracks into one maker request"
                );
            }
            anyhow::ensure!(
                prompt.contains("Primary explanation") && prompt.contains("Independent challenge"),
                "packed maker request lost a planned evidence track"
            );
        }
        let maker_index = self.maker_calls.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(
            args.get("min_success_count").is_none(),
            "DeepResearch must not cancel unverified maker outputs after a raw success count"
        );
        if self.maker_failure {
            return Ok(ToolOutput::error("simulated maker timeout"));
        }
        let url = if self.tool_name == "generate_object" {
            if self.digest_regression {
                "https://oversized-1.example/status".to_string()
            } else {
                "https://official.example/status".to_string()
            }
        } else {
            format!("https://evidence{}.example/research", maker_index + 1)
        };
        let gaps = if maker_index == 0 {
            vec!["credible counterevidence"]
        } else {
            Vec::new()
        };
        let structured = serde_json::json!({
            "summary": if self.digest_regression {
                "MAKER_DIGEST_MARKER: schema-validated maker evidence survived compaction."
                    .to_string()
            } else {
                format!("Iteration {} produced decision-relevant evidence.", maker_index + 1)
            },
            "sources": [{
                "title": format!("Evidence {}", maker_index + 1),
                "url_or_path": url.clone(),
                "date": "2026-07-12",
                "quote_or_fact": if self.digest_regression {
                    format!("Retained source fact: {} SOURCE_FACT_TAIL", "x".repeat(500))
                } else {
                    "Traceable evidence tests the requested decision.".to_string()
                },
                "reliability": "Deterministic integration fixture."
            }],
            "key_evidence": ["Traceable evidence tests the requested decision."],
            "contradictions": [],
            "confidence": "medium",
            "gaps": gaps
        });
        if self.tool_name == "generate_object" {
            return Ok(generated_object_output(structured));
        }
        Ok(
            ToolOutput::success("maker evidence completed").with_metadata(serde_json::json!({
                // A maker owns an independent child-task clock. Simulate a maker
                // that used more wall time than the direct retrieval budget so the
                // checker can still route one bounded direct follow-up afterward.
                "duration_ms": if self.maker_then_direct { 60_001 } else { 0 },
                "task_count": 1,
                "result_count": 1,
                "success_count": 1,
                "failed_count": 0,
                "all_success": true,
                "partial_failure": false,
                "allow_partial_failure": true,
                "results": [{
                    "task_id": format!("maker-{}", maker_index + 1),
                    "agent": "deep-research",
                    "success": true,
                    "source_anchors": [{ "tool": "web_fetch", "url_or_path": url }],
                    "structured": structured
                }]
            })),
        )
    }
}

#[async_trait::async_trait]
impl Tool for PlannedLoopSearchTool {
    fn name(&self) -> &str {
        "planned_web_search"
    }

    fn description(&self) -> &str {
        "Returns two independently hosted current-status sources."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            serde_json::json!([
                {
                    "title": "Official current status",
                    "url": "https://official.example/status",
                    "content": "Adaptive loop current status is operational.",
                    "published_date": "2026-07-12",
                    "engines": ["fixture"]
                },
                {
                    "title": "Independent current status",
                    "url": "https://independent.example/status",
                    "content": "Adaptive loop current status is operational.",
                    "published_date": "2026-07-12",
                    "engines": ["fixture"]
                }
            ])
            .to_string(),
        ))
    }
}

#[async_trait::async_trait]
impl Tool for UnicodeFocusPlannerTool {
    fn name(&self) -> &str {
        "generate_object"
    }

    fn description(&self) -> &str {
        "Returns a plan and checker decision with Unicode retrieval queries."
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
            return Ok(generated_object_output(serde_json::json!({
                "answer_shape": "lookup",
                "freshness_required": false,
                "workspace_evidence_required": false,
                "execution_route": "direct_only",
                "report_title": "Requested condition assessment",
                "phases": [{
                    "name": "retrieve and verify",
                    "success_criterion": "The requested condition is independently corroborated."
                }],
                "tracks": [{
                    "title": "Requested condition",
                    "focus": "Retrieve and corroborate the requested condition."
                }],
                "search_queries": ["系统运行状态 官方记录", "系统运行状态 独立验证"],
                "seed_urls": [],
                "budget": {
                    "retrieval_timeout_ms": 30000,
                    "synthesis_timeout_ms": 15000,
                    "max_iterations": 1,
                    "max_parallel_tasks": 1,
                    "max_steps_per_task": 2,
                    "per_task_timeout_ms": 10000,
                    "direct_searches": 2,
                    "direct_fetches": 2
                },
                "stop_conditions": ["the requested condition is traceable to two sources"]
            })));
        }

        anyhow::ensure!(
            schema_name == "deep_research_check",
            "unexpected structured call: {schema_name}"
        );
        let prompt = args
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        anyhow::ensure!(
            prompt.contains("系统运行状态")
                && prompt.contains("https://primary.example/record")
                && prompt.contains("https://independent.example/review"),
            "the checker did not receive both Unicode-focused sources"
        );
        Ok(generated_object_output(serde_json::json!({
            "decision": "finalize",
            "coverage_summary": "Two fetched sources corroborate the requested condition.",
            "report_summary": "The requested operating condition is independently corroborated.",
            "verified_findings": ["Two fetched records corroborate the requested condition."],
            "track_assessments": [{
                "plan_index": 0,
                "status": "supported",
                "finding": "Two fetched records corroborate the requested condition.",
                "source_urls": [
                    "https://primary.example/record",
                    "https://independent.example/review"
                ]
            }],
            "stop_condition_assessments": [{
                "plan_index": 0,
                "status": "supported",
                "finding": "The requested condition is traceable to two sources.",
                "source_urls": [
                    "https://primary.example/record",
                    "https://independent.example/review"
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
        })))
    }
}

#[async_trait::async_trait]
impl Tool for UnicodeFocusSearchTool {
    fn name(&self) -> &str {
        "unicode_focus_web_search"
    }

    fn description(&self) -> &str {
        "Returns candidates whose relevance signal exists only in Unicode text."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let query = args
            .get("query")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        anyhow::ensure!(
            query.contains("状态")
                && !query
                    .chars()
                    .any(|character| character.is_ascii_alphabetic()),
            "the regression fixture requires a pure CJK retrieval query"
        );
        Ok(ToolOutput::success(
            serde_json::json!([
                {
                    "title": "状态条目甲",
                    "url": "https://primary.example/record",
                    "content": "可供后续抓取核验的候选页面。",
                    "engines": ["fixture"]
                },
                {
                    "title": "状态条目乙",
                    "url": "https://independent.example/review",
                    "content": "可供后续抓取核验的候选页面。",
                    "engines": ["fixture"]
                }
            ])
            .to_string(),
        ))
    }
}

#[async_trait::async_trait]
impl Tool for UnicodeFocusFetchTool {
    fn name(&self) -> &str {
        "unicode_focus_web_fetch"
    }

    fn description(&self) -> &str {
        "Returns substantive fetched text matching a Unicode retrieval query."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let url = args
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        Ok(ToolOutput::success(format!(
            "# 系统运行状态核验记录\n\n官方记录与独立验证通过可追溯日志确认系统运行状态符合核验条件，并说明核验范围、观察时间与证据边界。来源：{url}"
        )))
    }
}

#[async_trait::async_trait]
impl Tool for NoisyPlannedLoopSearchTool {
    fn name(&self) -> &str {
        "noisy_planned_web_search"
    }

    fn description(&self) -> &str {
        "Returns useful pages mixed with non-document web assets."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let result = |title: &str, url: &str| {
            serde_json::json!({
                "title": title,
                "url": url,
                "content": "Adaptive loop current status is operational.",
                "published_date": "2026-07-12",
                "engines": ["fixture"]
            })
        };
        Ok(ToolOutput::success(
            serde_json::json!([
                result(
                    "Adaptive loop status avatar",
                    "https://avatars.githubusercontent.com/u/42"
                ),
                result(
                    "Adaptive loop status avatar",
                    "https://secure.gravatar.com/avatar/abc"
                ),
                result(
                    "Adaptive loop status profile",
                    "https://api.github.com/users/example"
                ),
                result(
                    "Adaptive loop status followers",
                    "https://api.github.com/users/example/followers"
                ),
                result(
                    "Adaptive loop status following template",
                    "https://api.github.com/users/example/following{/other_user}"
                ),
                result(
                    "Adaptive loop status gist template",
                    "https://api.github.com/users/example/gists{/gist_id}"
                ),
                result(
                    "Adaptive loop status archive",
                    "https://downloads.example/adaptive-loop-current-status.zip"
                ),
                result(
                    "Truncated adaptive loop result",
                    "https://truncated.example/adaptive-loop-current-status…"
                ),
                result(
                    "Adaptive loop status font",
                    "https://cdn.example/adaptive-loop-current-status.woff2"
                ),
                result("Official current status", "https://official.example/status"),
                result(
                    "Independent current status",
                    "https://independent.example/status"
                )
            ])
            .to_string(),
        ))
    }
}

#[async_trait::async_trait]
impl Tool for OversizedPlannedLoopSearchTool {
    fn name(&self) -> &str {
        "oversized_planned_web_search"
    }

    fn description(&self) -> &str {
        "Returns enough direct evidence to exceed the former blind prefix limit."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let results = (1..=12)
            .map(|index| {
                serde_json::json!({
                    "title": format!("Oversized direct source {index}"),
                    "url": format!("https://oversized-{index}.example/status"),
                    "content": format!(
                        "Adaptive loop current status evidence from source {index}. {}",
                        "Detailed direct evidence. ".repeat(80)
                    ),
                    "published_date": "2026-07-12",
                    "engines": ["fixture"]
                })
            })
            .collect::<Vec<_>>();
        Ok(ToolOutput::success(
            serde_json::to_string(&results).expect("fixture search results should serialize"),
        ))
    }
}

#[async_trait::async_trait]
impl Tool for PlannedLoopFetchTool {
    fn name(&self) -> &str {
        "planned_web_fetch"
    }

    fn description(&self) -> &str {
        "Returns fetched evidence for the planned-loop test."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let url = args
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        Ok(ToolOutput::success(format!(
            "# Provide feedback\n\n# Search code, repositories, users, issues, pull requests\n\n* [Code](/adaptive-loop-current-status) * [Issues](/adaptive-loop-current-status/issues)\n\n[data-color-mode=\"light\"] .HeaderMenu-link:focus-visible {{ outline-color: var(--color-accent-fg); }}\n\n# Adaptive loop current status\n\nAs of 2026-07-12 the service is operational. Source: {url}"
        )))
    }
}

#[async_trait::async_trait]
impl Tool for TransientPlannedLoopFetchTool {
    fn name(&self) -> &str {
        "transient_planned_web_fetch"
    }

    fn description(&self) -> &str {
        "Fails the initial fetch batch with transport errors, then succeeds on retry."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call < 2 {
            return Ok(ToolOutput::error("TLS handshake EOF"));
        }
        let url = args
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        Ok(ToolOutput::success(format!(
            "# Adaptive loop current status\n\nAs of 2026-07-12 the service is operational. Source: {url}"
        )))
    }
}

#[async_trait::async_trait]
impl Tool for MetadataOnlyPlannedLoopFetchTool {
    fn name(&self) -> &str {
        "metadata_planned_web_fetch"
    }

    fn description(&self) -> &str {
        "Returns a JSON-LD-only page body so direct evidence falls back to the clean search fact."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            r#"\[{"@context":"https://schema.org/","@type":"BlogPosting","headline":"Adaptive loop status","description":"Machine metadata must not become the reader-facing evidence quote."}\]"#,
        ))
    }
}

#[async_trait::async_trait]
impl Tool for LateEntityPlannedSeedTool {
    fn name(&self) -> &str {
        "generate_object"
    }

    fn description(&self) -> &str {
        "Returns a plan whose relevant seed identity occurs late in a long query."
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
            return Ok(generated_object_output(serde_json::json!({
                "answer_shape": "lookup",
                "freshness_required": false,
                "workspace_evidence_required": false,
                "execution_route": "direct_only",
                "report_title": "Project Quasar isolation",
                "phases": ["retrieve the planned source"],
                "tracks": ["Project Quasar isolation boundary"],
                "search_queries": [],
                "seed_urls": ["https://github.com/example/project-quasar"],
                "budget": {
                    "retrieval_timeout_ms": 30000,
                    "synthesis_timeout_ms": 15000,
                    "max_iterations": 1,
                    "max_parallel_tasks": 1,
                    "max_steps_per_task": 1,
                    "per_task_timeout_ms": 10000,
                    "direct_searches": 0,
                    "direct_fetches": 1
                },
                "stop_conditions": ["the planned source establishes the isolation boundary"]
            })));
        }
        anyhow::ensure!(
            schema_name == "deep_research_check",
            "unexpected structured call: {schema_name}"
        );
        let prompt = args
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        anyhow::ensure!(
            prompt.contains("Project Quasar uses a separate guest kernel"),
            "the checker did not receive the fetched planned-seed evidence"
        );
        Ok(generated_object_output(serde_json::json!({
            "decision": "finalize",
            "coverage_summary": "The fetched project source establishes the requested boundary.",
            "report_summary": "Project Quasar uses a separate guest kernel for isolation.",
            "verified_findings": ["Project Quasar uses a separate guest kernel."],
            "track_assessments": [{
                "plan_index": 0,
                "status": "supported",
                "finding": "Project Quasar uses a separate guest kernel.",
                "source_urls": ["https://github.com/example/project-quasar"]
            }],
            "stop_condition_assessments": [{
                "plan_index": 0,
                "status": "supported",
                "finding": "The planned project source establishes the isolation boundary.",
                "source_urls": ["https://github.com/example/project-quasar"]
            }],
            "unresolved_gaps": [],
            "limitations": [],
            "contradictions": [],
            "next_action": "none",
            "search_queries": [],
            "seed_urls": [],
            "next_tracks": [],
            "reason": "The planned source directly answers the bounded question."
        })))
    }
}

#[async_trait::async_trait]
impl Tool for LateEntityPlannedSeedFetchTool {
    fn name(&self) -> &str {
        "late_entity_planned_seed_fetch"
    }

    fn description(&self) -> &str {
        "Returns substantive text for a planned seed with a late query identity."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "# Project Quasar\n\nProject Quasar uses a separate guest kernel to isolate untrusted workloads from the host kernel. Its project documentation also describes explicit filesystem and network boundaries.",
        ))
    }
}

#[async_trait::async_trait]
impl Tool for ObservedLinkFetchTool {
    fn name(&self) -> &str {
        "observed_link_web_fetch"
    }

    fn description(&self) -> &str {
        "Records fetch order and exposes a source-observed absolute link."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let url = args
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.fetched_urls.lock().unwrap().push(url.clone());
        Ok(ToolOutput::success(format!(
            "# Adaptive loop current status\n\nAdaptive loop current status is operational. Read the [primary status detail](/detail) for the source-backed isolation boundary. The unrelated [project license](/LICENSE) is not evidence for that gap. Source: {url}"
        )))
    }
}

include!("engineered_loop_integration_tests.rs");
include!("engineered_loop_direct_review_tests.rs");
include!("engineered_loop_search_fallback_tests.rs");
