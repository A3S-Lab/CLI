//! Host-side DeepResearch workflow source, budgets, and safety envelope.

use super::*;

const MAX_DEEP_RESEARCH_TRACKS: usize = 4;

/// PTC source used by the `?` DeepResearch workflow. The workflow function is
/// deterministic and only schedules work; side effects live in Flow steps.
pub(super) fn deep_research_workflow_source() -> &'static str {
    static SOURCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SOURCE.get_or_init(|| {
        compact_workflow_source(concat!(
            include_str!("workflow/retrieval_foundation.js"),
            include_str!("workflow/retrieval_web.js"),
            include_str!("workflow/retrieval_selection.js"),
            include_str!("workflow/retrieval_reduction.js"),
            include_str!("workflow/retrieval_materialization.js"),
            include_str!("workflow/retrieval_loop.js"),
            include_str!("workflow/retrieval_local.js"),
            include_str!("workflow/retrieval_local_collection.js"),
            include_str!("workflow/retrieval_execution.js"),
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
    pub(super) max_tracks: usize,
    pub(super) max_steps_per_task: usize,
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
pub(super) fn deep_research_workflow_args(query: &str) -> serde_json::Value {
    let mut args =
        deep_research_workflow_args_with_scope(query, deep_research_inferred_evidence_scope(query));
    let tracks = serde_json::json!([{
        "id": "fixture.facts",
        "title": "Fixture facts",
        "focus": "Collect the primary facts required by this deterministic test.",
        "material": true,
        "questions": ["What primary evidence establishes the fixture fact?"],
        "completion_criteria": ["A traceable source establishes the fixture fact."],
        "evidence_requirements": {
            "primary_source_required": false,
            "independent_corroboration_required": false
        }
    }, {
        "id": "fixture.corroboration",
        "title": "Fixture corroboration",
        "focus": "Collect one independent corroborating source for this deterministic test.",
        "material": false,
        "questions": ["Which independent source corroborates the fixture fact?"],
        "completion_criteria": ["Independent support is retained or explicitly bounded."],
        "evidence_requirements": {
            "primary_source_required": false,
            "independent_corroboration_required": true
        }
    }]);
    args["input"]["research_plan"] = serde_json::json!({
        "report_title": "Fixture Research Report",
        "freshness_required": false,
        "workspace_evidence_required": false,
        "tracks": tracks,
        "search_queries": [],
        "seed_urls": [],
        "budget": {
            "retrieval_timeout_ms": 90000,
            "direct_searches": 2,
            "direct_fetches": 2
        },
        "stop_conditions": ["fixture evidence satisfies the existing test gate"]
    });
    args["input"]["research_plan_fixture"] = serde_json::Value::Bool(true);
    args
}

pub(super) fn deep_research_workflow_args_with_scope(
    query: &str,
    evidence_scope: DeepResearchEvidenceScope,
) -> serde_json::Value {
    deep_research_workflow_args_for_budget(query, evidence_scope, deep_research_default_budget())
}

pub(super) fn deep_research_safety_envelope(
    evidence_scope: DeepResearchEvidenceScope,
    budget: BudgetPlan,
) -> DeepResearchSafetyEnvelope {
    // These values are query-agnostic safety ceilings. The semantic planner
    // chooses the tracks and the bounded per-pass retrieval budget.
    DeepResearchSafetyEnvelope {
        max_tracks: MAX_DEEP_RESEARCH_TRACKS,
        max_steps_per_task: budget.deep_research_child_steps.clamp(1, 2),
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

pub(crate) fn deep_research_workflow_timeout_tool_result(
    workspace: &Path,
    args: &serde_json::Value,
    message: String,
) -> Result<ToolCallResult, String> {
    let Some(mut recovered) = recover_deep_research_workflow_run_from_store(workspace, args) else {
        return Err(message);
    };
    let mut output = recovered.output.unwrap_or(message);
    if deep_research_host_managed_inquiry(args) {
        stamp_host_inquiry_authority_text(&mut output);
        if let Some(snapshot_output) = recovered
            .metadata
            .pointer_mut("/dynamic_workflow/snapshot/output")
        {
            stamp_host_inquiry_authority_value(snapshot_output);
        }
    }
    Ok(ToolCallResult {
        name: "dynamic_workflow".to_string(),
        output,
        exit_code: recovered.exit_code,
        metadata: Some(recovered.metadata),
        error_kind: None,
    })
}

pub(super) fn deep_research_host_managed_inquiry(args: &serde_json::Value) -> bool {
    args.pointer("/input/inquiry_host_managed")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

fn stamp_host_inquiry_authority_text(output: &mut String) {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(output) else {
        return;
    };
    stamp_host_inquiry_authority_value(&mut value);
    if let Ok(encoded) = serde_json::to_string(&value) {
        *output = encoded;
    }
}

fn stamp_host_inquiry_authority_value(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let execution = object
        .entry("execution")
        .or_insert_with(|| serde_json::json!({}));
    let Some(execution) = execution.as_object_mut() else {
        return;
    };
    execution.insert(
        "terminal_authority".to_string(),
        serde_json::Value::String("host_inquiry_reducer".to_string()),
    );
}

pub(super) fn deep_research_workflow_args_for_budget(
    query: &str,
    evidence_scope: DeepResearchEvidenceScope,
    budget: BudgetPlan,
) -> serde_json::Value {
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
        safety.max_tracks,
    );
    serde_json::json!({
        "source": deep_research_workflow_source(),
        "input": {
            "query": query,
            "inquiry_host_managed": true,
            "current_date": current_date,
            "run_started_at_ms": run_started_at_ms,
            "loop_contract": loop_contract,
            "evidence_scope": match evidence_scope {
                DeepResearchEvidenceScope::LocalOnly => "local_only",
                DeepResearchEvidenceScope::WebAndWorkspace => "web_and_workspace",
            },
            "local_max_steps": safety.max_steps_per_task,
            "workflow_timeout_ms": safety.workflow_timeout_ms,
        },
        "limits": {
            "timeoutMs": safety.workflow_timeout_ms,
            "maxToolCalls": safety.workflow_max_tool_calls,
            "maxOutputBytes": safety.workflow_max_output_bytes
        }
    })
}

pub(super) const DEEP_RESEARCH_PROMPT_SUCCESS_OUTPUT_LIMIT: usize = 1200;
#[cfg(test)]
pub(super) const DEEP_RESEARCH_PROMPT_TEXT_LIMIT: usize = 12_000;
pub(super) const DEEP_RESEARCH_MAX_DIGEST_EVIDENCE: usize = 18;
pub(super) const DEEP_RESEARCH_MAX_DIGEST_SOURCES: usize = 12;
pub(super) const DEEP_RESEARCH_MAX_DIGEST_STRINGS: usize = 12;

#[cfg(test)]
mod terminal_authority_tests {
    use super::*;

    #[test]
    fn host_managed_args_and_recovered_output_retain_terminal_authority() {
        let args = deep_research_workflow_args_with_scope(
            "source-backed answer",
            DeepResearchEvidenceScope::WebAndWorkspace,
        );
        assert!(deep_research_host_managed_inquiry(&args));

        let mut output = serde_json::json!({
            "mode": "direct_web",
            "checker": {"decision": "finalize"}
        })
        .to_string();
        stamp_host_inquiry_authority_text(&mut output);
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            value.pointer("/execution/terminal_authority"),
            Some(&serde_json::json!("host_inquiry_reducer"))
        );
        assert!(validated_inquiry_projection(&value).is_err());
    }
}
