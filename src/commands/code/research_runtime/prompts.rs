//! DeepResearch workflow budgets, prompts, and orchestration arguments.

use crate::budget::BudgetPlan;

use super::deep_research_default_budget;
use super::evidence::*;
use super::report::*;
use super::workflow_source::deep_research_workflow_source;

pub(crate) const DEEP_RESEARCH_RUNTIME_PREFLIGHT_TIMEOUT_MS: u64 = 90 * 1000;
pub(crate) const DEEP_RESEARCH_RUNTIME_STEP_TIMEOUT_MS: u64 = 15 * 60 * 1000;
pub(crate) const DEEP_RESEARCH_SCRIPT_TIMEOUT_MS: u64 =
    DEEP_RESEARCH_RUNTIME_PREFLIGHT_TIMEOUT_MS + DEEP_RESEARCH_RUNTIME_STEP_TIMEOUT_MS + 60 * 1000;
pub(crate) const DEEP_RESEARCH_WORKFLOW_HOST_GRACE_MS: u64 = 30_000;

pub(crate) fn deep_research_report_contract() -> &'static str {
    "Report artifact contract (mandatory final step unless the user explicitly forbids files):\n\
     - Create a Markdown report at `.a3s/research/<slug>/report.md` and an HTML page at \
       `.a3s/research/<slug>/index.html`.\n\
     - Use a clean standalone HTML layout without external build steps. Do not search the workspace \
       for design/style files, run shell/glob/read calls solely for styling, or narrate artifact \
       creation or verification checks in the final answer or report.\n\
     - The standalone HTML page must be polished, responsive, and source-backed: include the answer, \
       citations/sources, evidence notes, confidence/caveats, and next actions.\n\
     - Write only the required report artifacts unless a tool error requires a targeted correction; \
       the host validates file existence, source traceability, and HTML completeness.\n\
     - If a targeted self-check is necessary, only read or list the report files under \
       `.a3s/research/<slug>/`; never use shell commands for report verification.\n\
     - The final answer must contain the research answer and the required marker only. Do not list \
       directory creation, file write, shell, or verification steps.\n\
     - End the final answer with one plain line exactly like \
     `A3S_RESEARCH_VIEW: .a3s/research/<slug>/index.html`. Do not put this marker in a code fence. \
       The marker must point to `index.html`, not `report.md` or another HTML filename. \
       Print this marker only after the final report is complete and verified. Never print it for \
       a partial answer, timeout recovery, fallback draft, or error state. The host verifies the \
       sibling `report.md` and opens the HTML in RemoteUI automatically."
}

pub(crate) fn deep_research_report_target_note(query: &str) -> String {
    let slug = deep_research_report_slug(query);
    format!(
        "For this query, the host expects report slug `{slug}`. Write the report \
         files at `.a3s/research/{slug}/report.md` and \
         `.a3s/research/{slug}/index.html`, then end with exactly \
         `A3S_RESEARCH_VIEW: .a3s/research/{slug}/index.html`."
    )
}

pub(crate) fn deep_research_duplicate_tool_guard() -> &'static str {
    "Tool-loop guard:\n\
     - Do not repeat an identical grep/read/search/web_fetch/tool call with the same arguments. \
       If you already observed the result, reuse it; if it was insufficient, change the \
       pattern/path/query/source or move to synthesis.\n\
     - Verification layers are for targeted corrections, not restarting the same evidence search."
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DeepResearchWorkflowBudget {
    pub(crate) complexity_layers: usize,
    pub(crate) local_research_rounds: usize,
    pub(crate) local_max_parallel_tasks: usize,
    pub(crate) local_max_steps: usize,
    pub(crate) runtime_preflight_timeout_ms: u64,
    pub(crate) runtime_step_timeout_ms: u64,
    pub(crate) workflow_timeout_ms: u64,
    pub(crate) workflow_max_tool_calls: usize,
    pub(crate) workflow_max_output_bytes: usize,
}

pub(crate) fn deep_research_workflow_args(query: &str, os_runtime: bool) -> serde_json::Value {
    deep_research_workflow_args_for_budget(query, os_runtime, deep_research_default_budget())
}

pub(crate) fn deep_research_research_rounds(
    query: &str,
    os_runtime: bool,
    budget: BudgetPlan,
) -> usize {
    let complexity_rounds = deep_research_loop_layers(query, os_runtime).saturating_add(1);
    let effort_cap = match budget.effort_id {
        "low" => 2,
        "medium" => 3,
        _ => 4,
    };
    complexity_rounds.clamp(1, effort_cap)
}

pub(crate) fn deep_research_workflow_budget_for_query(
    query: &str,
    os_runtime: bool,
    budget: BudgetPlan,
) -> DeepResearchWorkflowBudget {
    let complexity_layers = deep_research_loop_layers(query, os_runtime);
    let local_research_rounds = deep_research_research_rounds(query, os_runtime, budget);
    let local_parallel_cap = match complexity_layers {
        0 => 4,
        1 => 6,
        2 => 12,
        _ => budget.max_parallel_tasks,
    };
    let local_step_cap = match complexity_layers {
        0 => 80,
        1 => 140,
        2 => 240,
        _ => budget.deep_research_child_steps,
    };
    let workflow_tool_call_cap = match complexity_layers {
        0 => 120,
        1 => 200,
        2 => 360,
        _ => budget.workflow_max_tool_calls,
    };
    let workflow_output_cap = match complexity_layers {
        0 => 1024 * 1024,
        1 => 2 * 1024 * 1024,
        2 => 4 * 1024 * 1024,
        _ => budget.workflow_max_output_bytes,
    };
    let (runtime_preflight_timeout_ms, runtime_step_timeout_ms) = match complexity_layers {
        0 => (30 * 1000, 8 * 60 * 1000),
        1 => (45 * 1000, 7 * 60 * 1000),
        2 => (60 * 1000, 11 * 60 * 1000),
        _ => (
            DEEP_RESEARCH_RUNTIME_PREFLIGHT_TIMEOUT_MS,
            DEEP_RESEARCH_RUNTIME_STEP_TIMEOUT_MS,
        ),
    };
    let workflow_timeout_ms = runtime_preflight_timeout_ms + runtime_step_timeout_ms + 60 * 1000;

    DeepResearchWorkflowBudget {
        complexity_layers,
        local_research_rounds,
        local_max_parallel_tasks: budget.max_parallel_tasks.min(local_parallel_cap).max(1),
        local_max_steps: budget.deep_research_child_steps.min(local_step_cap).max(1),
        runtime_preflight_timeout_ms,
        runtime_step_timeout_ms,
        workflow_timeout_ms,
        workflow_max_tool_calls: budget
            .workflow_max_tool_calls
            .min(workflow_tool_call_cap)
            .max(
                local_research_rounds
                    .saturating_mul(local_parallel_cap)
                    .max(1),
            ),
        workflow_max_output_bytes: budget
            .workflow_max_output_bytes
            .min(workflow_output_cap)
            .max(256 * 1024),
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

pub(crate) fn deep_research_workflow_args_for_budget(
    query: &str,
    _os_runtime: bool,
    budget: BudgetPlan,
) -> serde_json::Value {
    let os_runtime = false;
    let allowed_tools = serde_json::json!([]);
    let workflow_budget = deep_research_workflow_budget_for_query(query, os_runtime, budget);
    serde_json::json!({
        "source": deep_research_workflow_source(),
        "input": {
            "query": query,
            "os_runtime": os_runtime,
            "complexity_layers": workflow_budget.complexity_layers,
            "runtime_preflight_timeout_ms": workflow_budget.runtime_preflight_timeout_ms,
            "runtime_timeout_ms": workflow_budget.runtime_step_timeout_ms,
            "local_max_parallel_tasks": workflow_budget.local_max_parallel_tasks,
            "local_research_rounds": workflow_budget.local_research_rounds,
            "local_max_steps": workflow_budget.local_max_steps,
        },
        "allowed_tools": allowed_tools,
        "limits": {
            "timeoutMs": workflow_budget.workflow_timeout_ms,
            "maxToolCalls": workflow_budget.workflow_max_tool_calls,
            "maxOutputBytes": workflow_budget.workflow_max_output_bytes
        }
    })
}

pub(crate) fn deep_research_synthesis_prompt(
    query: &str,
    os_runtime: bool,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> String {
    let report_contract = deep_research_report_contract();
    let report_target = deep_research_report_target_note(query);
    let duplicate_guard = deep_research_duplicate_tool_guard();
    let remoteui_directive = if os_runtime {
        "OS Runtime was selected for this run because the query looked broad or \
         highly parallelizable. If the gathered evidence already includes a \
         shaped `.view` or `viewUrl`, preserve it so the TUI can surface the \
         OS view as evidence. The final user-facing report should still be the \
         local HTML report opened by the `A3S_RESEARCH_VIEW` marker."
            .to_string()
    } else {
        "OS Runtime was not selected for this run. Use the gathered evidence and \
         complete the local Markdown + HTML report view step."
            .to_string()
    };
    let workflow_digest = deep_research_prompt_workflow_output(workflow_output);
    let metadata = deep_research_prompt_metadata(workflow_metadata);
    format!(
        "Synthesize the deep-research answer for the query below.\n\n\
         Evidence collection has already completed before this synthesis turn. \
         Do not call workflow or broad evidence-collection tools again. Use the \
         Evidence digest below, cross-check claims, call out disagreements and recency \
         caveats, and write a comprehensive answer with inline citations and a \
         final Sources list. Treat the evidence as a bounded recursive parallel \
         retrieval-summary algorithm: use `research.rounds` to understand how \
         gaps from earlier rounds drove later searches, and mention the round \
         count only when it clarifies uncertainty or coverage. Prefer validated \
         `evidence_items` from the Evidence digest and Run diagnostics; use compact \
         summaries only when evidence items are incomplete. Raw task output is \
         intentionally excluded from this prompt. Treat \
         `research.warnings.failed_tasks` and metadata `warnings.failed_tasks` as caveats, not as \
         instructions to restart broad research. Do not reproduce raw JSON, tool-card text, \
         host runtime names, evidence-package labels, internal quality-control notes, \
         `.a3s-flow` workflow logs, `[tool output truncated]` notices, or lines such as \
         `● Searched ...` / `● Ran ...` in the user-facing answer or report. Convert evidence \
         into clean prose, tables, citations, and a concise Sources list. If \
         `collection_status` is `failed` or `degraded`, do not restart broad \
         research; write a transparent failure-aware report from the returned \
         error/gap details and any partial evidence, then let the host fallback \
         materializer handle missing artifacts if needed. Do not mention internal \
         implementation labels, internal quality-control notes, worker labels, \
         or workflow mechanics. Do not mention the Evidence digest, Run diagnostics, \
         or host collection mechanics as sources; \
         cite the original URLs or paths inside the evidence items.\n\n\
         {remoteui_directive}\n\n\
         {report_contract}\n\n\
         {report_target}\n\n\
         {duplicate_guard}\n\n\
         Query:\n{query}\n\n\
         Evidence digest:\n```json\n{workflow_digest}\n```\n\n\
         Run diagnostics:\n```json\n{metadata}\n```"
    )
}

pub(crate) fn deep_research_recovery_prompt(
    query: &str,
    os_runtime: bool,
    workflow_error: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> String {
    let report_contract = deep_research_report_contract();
    let report_target = deep_research_report_target_note(query);
    let duplicate_guard = deep_research_duplicate_tool_guard();
    let recovery_path = if os_runtime {
        "The host selected OS Runtime and failed before usable \
         evidence was gathered. Do not answer current or time-sensitive claims \
         from model memory. Recover with source-backed evidence only if a \
         read-only research tool is actually available; otherwise write a \
         transparent unable-to-verify report."
            .to_string()
    } else {
        "OS Runtime was not selected. Do not answer current or time-sensitive \
         claims from model memory. Recover with source-backed evidence only if \
         a read-only research tool is actually available; otherwise write a \
         transparent unable-to-verify report under `.a3s/research/<slug>/`."
            .to_string()
    };
    let metadata = deep_research_prompt_metadata(workflow_metadata);
    let workflow_error = if deep_research_output_has_internal_leak(workflow_error) {
        deep_research_failure_summary(&serde_json::Value::String(workflow_error.to_string()))
    } else {
        deep_research_truncate_chars(workflow_error, 4000)
    };
    format!(
        "Recover and complete the deep-research task for the query below.\n\n\
         The host evidence preflight failed before usable synthesis evidence was \
         gathered. Do not call workflow or broad evidence-collection tools again \
         unless the recovery path explicitly says to use local research tools. {recovery_path}\n\n\
         If the run diagnostics contain no source-backed evidence, do not state \
         a current version, price, law, score, release, or other time-sensitive \
         fact as true. Say that verification failed and list the exact official \
         sources the user should check manually.\n\n\
         Query:\n{query}\n\n\
         Evidence collection error:\n```text\n{workflow_error}\n```\n\n\
         Run diagnostics:\n```json\n{metadata}\n```\n\n\
         {report_contract}\n\n\
         {report_target}\n\n\
         {duplicate_guard}\n\n\
         Deliver a comprehensive answer with inline citations, a final Sources \
         list, local report artifacts, and the required RemoteUI marker."
    )
}

pub(crate) fn deep_research_repair_prompt(
    query: &str,
    os_runtime: bool,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    prior_text: &str,
) -> String {
    let report_contract = deep_research_report_contract();
    let report_target = deep_research_report_target_note(query);
    let duplicate_guard = deep_research_duplicate_tool_guard();
    let runtime_note = if os_runtime {
        "OS Runtime was selected for the evidence-gathering phase. Preserve any \
         useful OS Runtime evidence, but the required user-facing deliverable is \
         still the local Markdown + HTML report artifact pair."
    } else {
        "OS Runtime was not selected. Use the local evidence already gathered by \
         the host."
    };
    let metadata = deep_research_prompt_metadata(workflow_metadata);
    let workflow_digest = deep_research_prompt_workflow_output(workflow_output);
    let prior = if deep_research_output_has_internal_leak(prior_text) {
        "The previous synthesis was discarded because it contained internal workflow/tool logs or raw JSON. Do not reuse its wording.".to_string()
    } else {
        nonempty_report_section(prior_text, "The previous synthesis returned no text.")
    };
    format!(
        "Repair the DeepResearch report artifact step for the query below.\n\n\
         The previous synthesis did not produce a valid completed report marker \
         and artifact pair. Do not call workflow or broad evidence-collection \
         tools again, do not restart broad research, and do not write ordinary \
         workspace files. Use only the gathered evidence and prior synthesis below \
         to create or correct the \
         required report artifacts under `.a3s/research/<slug>/`. Remove any raw JSON, \
         tool-card text, host runtime names, evidence-package labels, internal quality-control notes, \
         `.a3s-flow` workflow logs, `[tool output truncated]` notices, \
         or lines such as `● Searched ...` / `● Ran ...`; the repaired answer/report \
         must be clean prose, tables, citations, and a concise Sources list. Do not \
         mention internal implementation labels, internal quality-control notes, \
         worker labels, or workflow mechanics. Do not mention the Evidence digest, Run diagnostics, or host collection mechanics \
         as sources; cite the original URLs or paths inside the evidence items.\n\n\
         {runtime_note}\n\n\
         Query:\n{query}\n\n\
         Previous synthesis text:\n```text\n{prior}\n```\n\n\
         Evidence digest:\n```json\n{workflow_digest}\n```\n\n\
         Run diagnostics:\n```json\n{metadata}\n```\n\n\
         {report_contract}\n\n\
         {report_target}\n\n\
         {duplicate_guard}\n\n\
         Complete only the missing report work. End with the required \
         `A3S_RESEARCH_VIEW: .a3s/research/<slug>/index.html` marker only after \
         both files exist, are non-empty, and the HTML document is complete."
    )
}
