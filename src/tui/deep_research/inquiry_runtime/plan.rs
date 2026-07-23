//! A3S Code adapter for the standalone DeepResearch planner.

use a3s_code_core::{AgentEvent, AgentSession};
use serde_json::{Map, Value};
use tokio::sync::mpsc;

#[cfg(test)]
pub(super) use a3s_deep_research::planner::{
    attach_bootstrap_acquisition, validate_plan, workflow_args_with_plan,
};
pub(super) use a3s_deep_research::planner::{
    bootstrap_workflow_args, bound_questions, commit_plan_research_contract, host_fallback_plan,
    host_plan_from_outline, queue_plan_questions, validated_loop_planner, PlannedInquiry,
};

use super::execution::{call_generation_with_progress, generated_object};
use super::{
    InquiryCheckpointWriter, DURABLE_GENERATION_WORKFLOW_GRACE_MS, PLANNER_GENERATION_MAX_ATTEMPTS,
    PLANNER_OUTLINE_ATTEMPT_TIMEOUT_MS,
};

pub(super) async fn generate_plan(
    session: &AgentSession,
    workflow_args: &Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
    checkpoint: &InquiryCheckpointWriter,
) -> Result<PlannedInquiry, String> {
    let planner = validated_loop_planner(workflow_args)?;
    let outline_schema = planner
        .get("output_schema")
        .cloned()
        .ok_or_else(|| "DeepResearch planner contract has no output schema".to_string())?;
    let outline_prompt = required_planner_text(planner, "prompt")?;
    let outline_timeout_ms =
        required_planner_timeout(planner, "timeout_ms")?.min(PLANNER_OUTLINE_ATTEMPT_TIMEOUT_MS);

    let generation_args = serde_json::json!({
        "schema": outline_schema,
        "schema_name": "deep_research_semantic_outline",
        "schema_description": "A bounded semantic retrieval plan for one general-purpose DeepResearch inquiry",
        "prompt": outline_prompt,
        "system": "You are a concise semantic research planner. Return only the requested object and no reasoning.",
        "mode": "auto",
        "max_repair_attempts": 0,
        "include_raw_text": false,
        "timeout_ms": outline_timeout_ms,
    });
    let workflow_timeout_ms = outline_timeout_ms
        .saturating_mul(u64::from(PLANNER_GENERATION_MAX_ATTEMPTS))
        .saturating_add(DURABLE_GENERATION_WORKFLOW_GRACE_MS);
    let execution_timeout_ms = checkpoint
        .pre_review_stage_timeout_ms(workflow_timeout_ms)
        .ok_or_else(|| {
            "the shared inquiry deadline left no outline-planner budget after reserving retrieval review and finalization"
                .to_string()
        })?;
    let generated = call_generation_with_progress(
        session,
        generation_args,
        progress_tx,
        Some(checkpoint),
        "planner-outline",
        execution_timeout_ms,
        PLANNER_GENERATION_MAX_ATTEMPTS,
    )
    .await?;
    let outline = generated_object(&generated)?;
    host_plan_from_outline(workflow_args, outline)
}

fn required_planner_text<'a>(
    planner: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, String> {
    planner
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("DeepResearch planner contract has no non-empty `{field}`"))
}

fn required_planner_timeout(planner: &Map<String, Value>, field: &str) -> Result<u64, String> {
    let value = planner
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("DeepResearch planner contract omitted integer `{field}`"))?;
    if (1_000..=600_000).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "DeepResearch planner contract `{field}` must be between 1000 and 600000"
        ))
    }
}
