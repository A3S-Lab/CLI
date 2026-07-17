//! Host-side DeepResearch workflow source, budgets, and safety envelope.

use super::*;

/// PTC source used by the `?` DeepResearch workflow. The workflow function is
/// deterministic and only schedules work; side effects live in Flow steps.
pub(super) fn deep_research_workflow_source() -> &'static str {
    static SOURCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SOURCE.get_or_init(|| {
        compact_workflow_source(concat!(
            include_str!("workflow/collection.js"),
            include_str!("workflow/direct_collection.js"),
            include_str!("workflow/policy.js"),
            include_str!("workflow/loop_prelude.js"),
            include_str!("workflow/loop.js"),
            include_str!("workflow/runtime.js")
        ))
    })
}

fn compact_workflow_source(source: &str) -> String {
    let mut compact = String::with_capacity(source.len());
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        compact.push_str(line);
        // These punctuators unambiguously separate JavaScript tokens, so a
        // newline after them carries no ASI meaning. Preserve every other
        // newline to keep `return`, postfix operators, regex literals, and
        // future workflow edits safe without depending on an opaque minifier.
        if !matches!(line.as_bytes().last(), Some(b';' | b',' | b'{')) {
            compact.push('\n');
        }
    }
    compact
}

pub(super) fn deep_research_report_target_note(query: &str) -> String {
    let slug = deep_research_report_slug(query);
    deep_research_prompts::report_target_note(&slug)
}

/// The directive sent to the agent for a `?` deep-research turn: decompose the
/// question, run the evidence fan-out through DynamicWorkflowRuntime, then
/// cross-check and synthesize a cited report. OS Runtime tool-call fan-out is
/// intentionally disabled; future OS Runtime integration should use its
/// Function-as-a-Service path instead.
#[cfg(test)]
pub(super) fn deep_research_prompt(query: &str, _os_runtime: bool) -> String {
    deep_research_prompts::initial_prompt(deep_research_prompts::InitialPrompt {
        query,
        workflow_source: deep_research_workflow_source(),
    })
}

pub(crate) fn deep_research_default_budget() -> BudgetPlan {
    budget_plan_for_effort_index(DEFAULT_TUI_EFFORT_INDEX, None, BudgetWorkload::DeepResearch)
}

pub(super) fn deep_research_budget_for_effort_index(
    effort: usize,
    context_limit: u32,
) -> BudgetPlan {
    budget_plan_for_effort_index(effort, Some(context_limit), BudgetWorkload::DeepResearch)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DeepResearchSafetyEnvelope {
    pub(super) max_iterations: usize,
    pub(super) max_parallel_tasks: usize,
    pub(super) max_steps_per_task: usize,
    pub(super) per_task_timeout_ms: u64,
    pub(super) workflow_timeout_ms: u64,
    pub(super) workflow_max_tool_calls: usize,
    pub(super) workflow_max_output_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DeepResearchEvidenceScope {
    LocalOnly,
    #[default]
    WebAndWorkspace,
}

impl DeepResearchEvidenceScope {
    pub(super) fn network_enabled(self) -> bool {
        matches!(self, Self::WebAndWorkspace)
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::LocalOnly => "offline/local-only evidence",
            Self::WebAndWorkspace => {
                "web available; workspace only when the query explicitly depends on local artifacts"
            }
        }
    }
}

pub(super) fn deep_research_evidence_scope_from_args(
    args: &serde_json::Value,
    query: &str,
) -> DeepResearchEvidenceScope {
    match args
        .pointer("/input/evidence_scope")
        .and_then(serde_json::Value::as_str)
    {
        Some("local_only") => DeepResearchEvidenceScope::LocalOnly,
        Some("web_and_workspace") => DeepResearchEvidenceScope::WebAndWorkspace,
        _ => deep_research_inferred_evidence_scope(query),
    }
}

#[cfg(test)]
pub(super) fn deep_research_workflow_args(query: &str, os_runtime: bool) -> serde_json::Value {
    let mut args = deep_research_workflow_args_with_scope(
        query,
        os_runtime,
        deep_research_inferred_evidence_scope(query),
    );
    let tracks = serde_json::json!([{
        "title": "Fixture facts",
        "focus": "Collect the primary facts required by this deterministic test."
    }, {
        "title": "Fixture corroboration",
        "focus": "Collect one independent corroborating source for this deterministic test."
    }]);
    args["input"]["research_plan"] = serde_json::json!({
        "answer_shape": "briefing",
        "report_title": "Fixture Research Report",
        "freshness_required": false,
        "workspace_evidence_required": false,
        "execution_route": "direct_only",
        "phases": [{
            "name": "evidence",
            "success_criterion": "fixture source is traceable"
        }],
        "tracks": tracks,
        "search_queries": [],
        "seed_urls": [],
        "budget": {
            "retrieval_timeout_ms": 90000,
            "synthesis_timeout_ms": 30000,
            "max_iterations": args["input"]["local_research_rounds"].clone(),
            "max_parallel_tasks": args["input"]["local_max_parallel_tasks"].clone(),
            "max_steps_per_task": args["input"]["local_max_steps"].clone(),
            "per_task_timeout_ms": args["input"]["local_parallel_task_timeout_ms"].clone(),
            "direct_searches": 2,
            "direct_fetches": 2
        },
        "stop_conditions": ["fixture evidence satisfies the existing test gate"]
    });
    args["input"]["research_plan_fixture"] = serde_json::Value::Bool(true);
    args["input"]["engineered_loop_fixture"] = serde_json::Value::Bool(true);
    args
}

pub(super) fn deep_research_workflow_args_with_scope(
    query: &str,
    os_runtime: bool,
    evidence_scope: DeepResearchEvidenceScope,
) -> serde_json::Value {
    deep_research_workflow_args_for_budget(
        query,
        os_runtime,
        evidence_scope,
        deep_research_default_budget(),
    )
}

pub(super) fn deep_research_safety_envelope(
    evidence_scope: DeepResearchEvidenceScope,
    budget: BudgetPlan,
) -> DeepResearchSafetyEnvelope {
    // These values are safety ceilings only. The semantic planner chooses the
    // actual stages, iteration count, parallelism, and clocks for the query.
    // Keeping this envelope query-agnostic prevents a second rules engine from
    // silently overriding the LLM-authored plan.
    DeepResearchSafetyEnvelope {
        max_iterations: 4,
        max_parallel_tasks: budget.max_parallel_tasks.clamp(1, 4),
        max_steps_per_task: budget.deep_research_child_steps.clamp(1, 2),
        per_task_timeout_ms: 120_000,
        workflow_timeout_ms: if evidence_scope.network_enabled() {
            300_000
        } else {
            210_000
        },
        workflow_max_tool_calls: budget.workflow_max_tool_calls.clamp(4, 240),
        workflow_max_output_bytes: budget
            .workflow_max_output_bytes
            .clamp(256 * 1024, 2 * 1024 * 1024),
    }
}

pub(crate) fn deep_research_workflow_timeout_ms(args: &serde_json::Value) -> u64 {
    args.pointer("/limits/timeoutMs")
        .and_then(serde_json::Value::as_u64)
        .filter(|timeout_ms| *timeout_ms >= 1_000)
        .unwrap_or(DEEP_RESEARCH_SCRIPT_TIMEOUT_MS)
}

pub(crate) fn deep_research_workflow_host_timeout_ms(args: &serde_json::Value) -> u64 {
    deep_research_workflow_timeout_ms(args).saturating_add(DEEP_RESEARCH_WORKFLOW_HOST_GRACE_MS)
}

pub(crate) fn deep_research_workflow_timeout_tool_result(
    workspace: &Path,
    args: &serde_json::Value,
    message: String,
) -> Result<ToolCallResult, String> {
    let Some(recovered) = recover_deep_research_workflow_run_from_store(workspace, args) else {
        return Err(message);
    };
    Ok(ToolCallResult {
        name: "dynamic_workflow".to_string(),
        output: recovered.output.unwrap_or(message),
        exit_code: recovered.exit_code,
        metadata: Some(recovered.metadata),
        error_kind: None,
    })
}

pub(super) fn deep_research_workflow_args_for_budget(
    query: &str,
    _os_runtime: bool,
    evidence_scope: DeepResearchEvidenceScope,
    budget: BudgetPlan,
) -> serde_json::Value {
    let os_runtime = false;
    let current_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let run_started_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let safety = deep_research_safety_envelope(evidence_scope, budget);
    let loop_contract = loop_engineering::deep_research_loop_contract(
        query,
        &current_date,
        evidence_scope.label(),
        safety.max_parallel_tasks,
        safety.max_steps_per_task,
    );
    let tracks = Vec::<serde_json::Value>::new();
    serde_json::json!({
        "source": deep_research_workflow_source(),
        "input": {
            "query": query,
            "current_date": current_date,
            "run_started_at_ms": run_started_at_ms,
            "loop_contract": loop_contract,
            "tracks": tracks,
            "os_runtime": os_runtime,
            "evidence_scope": match evidence_scope {
                DeepResearchEvidenceScope::LocalOnly => "local_only",
                DeepResearchEvidenceScope::WebAndWorkspace => "web_and_workspace",
            },
            "local_max_parallel_tasks": safety.max_parallel_tasks,
            "local_research_rounds": safety.max_iterations,
            "local_max_steps": safety.max_steps_per_task,
            "local_parallel_task_timeout_ms": safety.per_task_timeout_ms,
            "workflow_timeout_ms": safety.workflow_timeout_ms,
        },
        "limits": {
            "timeoutMs": safety.workflow_timeout_ms,
            "maxToolCalls": safety.workflow_max_tool_calls,
            "maxOutputBytes": safety.workflow_max_output_bytes
        }
    })
}

pub(super) fn should_use_os_runtime_for_deep_research(_query: &str, _os_available: bool) -> bool {
    false
}

pub(super) const DEEP_RESEARCH_PROMPT_SUCCESS_OUTPUT_LIMIT: usize = 1200;
pub(super) const DEEP_RESEARCH_PROMPT_TEXT_LIMIT: usize = 12_000;
pub(super) const DEEP_RESEARCH_MAX_DIGEST_EVIDENCE: usize = 18;
pub(super) const DEEP_RESEARCH_MAX_DIGEST_SOURCES: usize = 12;
pub(super) const DEEP_RESEARCH_MAX_DIGEST_STRINGS: usize = 12;
