//! TUI query scope parsing and report synthesis/recovery prompts.

use super::*;

pub(super) fn deep_research_query_is_local_only(query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return false;
    }
    // Compatibility fallback only. Keep this deliberately small and limited
    // to unambiguous user directives; explicit typed scope is authoritative.
    [
        "local-only",
        "local files only",
        "local workspace evidence only",
        "do not use web",
        "don't use web",
        "do not browse",
        "no web",
        "stay offline",
        "仅本地",
        "只使用本地",
        "不要联网",
        "不要上网",
        "不联网",
        "不查外网",
        "不要查外网",
    ]
    .iter()
    .any(|marker| query.contains(marker))
}

pub(super) fn deep_research_inferred_evidence_scope(query: &str) -> DeepResearchEvidenceScope {
    if deep_research_query_is_local_only(query) {
        DeepResearchEvidenceScope::LocalOnly
    } else {
        DeepResearchEvidenceScope::WebAndWorkspace
    }
}

pub(super) fn parse_deep_research_tui_query(
    raw_query: &str,
) -> (String, DeepResearchEvidenceScope) {
    let raw_query = raw_query.trim();
    let mut parts = raw_query.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or_default();
    let remainder = parts.next().unwrap_or_default().trim();
    match first {
        "--local-only" | "--offline" => {
            (remainder.to_string(), DeepResearchEvidenceScope::LocalOnly)
        }
        "--web" => (
            remainder.to_string(),
            DeepResearchEvidenceScope::WebAndWorkspace,
        ),
        _ => (
            raw_query.to_string(),
            deep_research_inferred_evidence_scope(raw_query),
        ),
    }
}

pub(super) fn deep_research_input_scope_hint() -> &'static str {
    "◇ deep research · --web | --local-only"
}

pub(super) fn deep_research_synthesis_prompt(
    query: &str,
    os_runtime: bool,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> String {
    deep_research_synthesis_prompt_with_scope(
        query,
        os_runtime,
        workflow_output,
        workflow_metadata,
        deep_research_inferred_evidence_scope(query),
    )
}

pub(super) fn deep_research_evidence_scope_prompt(
    scope: DeepResearchEvidenceScope,
) -> &'static str {
    match scope {
        DeepResearchEvidenceScope::LocalOnly => {
            "Evidence was collected under the authoritative local_only scope. Evidence collection is now closed. Do not search, fetch, run shell commands, delegate work, or start another workflow. Use only the supplied evidence and state external-evidence gaps transparently."
        }
        DeepResearchEvidenceScope::WebAndWorkspace => {
            "Evidence was collected under the authoritative web_and_workspace scope. Evidence collection is now closed. Do not search, fetch, run shell commands, delegate work, or start another workflow. Use only the supplied evidence and state unresolved gaps transparently."
        }
    }
}

pub(super) fn deep_research_synthesis_prompt_with_scope(
    query: &str,
    os_runtime: bool,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    evidence_scope: DeepResearchEvidenceScope,
) -> String {
    let report_target = deep_research_report_target_note(query);
    let workflow_digest = deep_research_prompt_workflow_output(workflow_output);
    let metadata = deep_research_prompt_metadata(workflow_metadata);
    deep_research_prompts::synthesis_prompt(deep_research_prompts::SynthesisPrompt {
        query,
        os_runtime,
        workflow_digest: &workflow_digest,
        metadata: &metadata,
        report_target: &report_target,
        evidence_scope: deep_research_evidence_scope_prompt(evidence_scope),
    })
}

pub(super) fn deep_research_recovery_prompt(
    query: &str,
    os_runtime: bool,
    workflow_error: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> String {
    deep_research_recovery_prompt_with_scope(
        query,
        os_runtime,
        workflow_error,
        workflow_metadata,
        deep_research_inferred_evidence_scope(query),
    )
}

pub(super) fn deep_research_recovery_prompt_with_scope(
    query: &str,
    os_runtime: bool,
    workflow_error: &str,
    workflow_metadata: Option<&serde_json::Value>,
    evidence_scope: DeepResearchEvidenceScope,
) -> String {
    let report_target = deep_research_report_target_note(query);
    let metadata = deep_research_prompt_metadata(workflow_metadata);
    let workflow_error = if deep_research_output_has_internal_leak(workflow_error) {
        deep_research_failure_summary(&serde_json::Value::String(workflow_error.to_string()))
    } else {
        deep_research_truncate_chars(workflow_error, 4000)
    };
    deep_research_prompts::recovery_prompt(deep_research_prompts::RecoveryPrompt {
        query,
        os_runtime,
        workflow_error: &workflow_error,
        metadata: &metadata,
        report_target: &report_target,
        evidence_scope: deep_research_evidence_scope_prompt(evidence_scope),
    })
}

pub(super) fn deep_research_repair_prompt(
    query: &str,
    os_runtime: bool,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    prior_text: &str,
) -> String {
    deep_research_repair_prompt_with_scope(
        query,
        os_runtime,
        workflow_output,
        workflow_metadata,
        prior_text,
        deep_research_inferred_evidence_scope(query),
    )
}

pub(super) fn deep_research_repair_prompt_with_scope(
    query: &str,
    os_runtime: bool,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    prior_text: &str,
    evidence_scope: DeepResearchEvidenceScope,
) -> String {
    let report_target = deep_research_report_target_note(query);
    let metadata = deep_research_prompt_metadata(workflow_metadata);
    let workflow_digest = deep_research_prompt_workflow_output(workflow_output);
    let prior = if deep_research_output_has_internal_leak(prior_text) {
        "The previous synthesis was discarded because it contained internal workflow/tool logs or raw JSON. Do not reuse its wording.".to_string()
    } else {
        nonempty_report_section(prior_text, "The previous synthesis returned no text.")
    };
    deep_research_prompts::repair_prompt(deep_research_prompts::RepairPrompt {
        query,
        os_runtime,
        workflow_digest: &workflow_digest,
        metadata: &metadata,
        prior: &prior,
        report_target: &report_target,
        evidence_scope: deep_research_evidence_scope_prompt(evidence_scope),
    })
}

pub(super) fn json_contains_tool_evidence(value: &serde_json::Value, tool: &str) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            ((key == "name" || key == "tool" || key == "tool_name") && value.as_str() == Some(tool))
                || json_contains_tool_evidence(value, tool)
        }),
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| json_contains_tool_evidence(item, tool)),
        _ => false,
    }
}

/// The persistent `/goal` north-star for a `?` deep-research task. Kept short
/// since it is prepended to every continuation turn of the long-horizon loop.
pub(super) fn deep_research_goal(query: &str) -> String {
    format!("Deep research — deliver a comprehensive, well-cited report answering: {query}")
}
