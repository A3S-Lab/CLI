use std::path::Path;

use a3s_code_core::dynamic_workflow::recover_dynamic_workflow_step_output;
use a3s_deep_research::engine::WorkflowOutput;
use serde_json::Value;

const INITIAL_RETRIEVAL_CHECKPOINT_STEP_ID: &str = "checkpoint_initial_retrieval";
const INITIAL_RETRIEVAL_RECOVERY_WARNING: &str = "The optional supplemental retrieval pass did not finish before the shared stage deadline; the durable initial closed-evidence checkpoint was recovered.";

pub(crate) async fn recover_initial_retrieval_checkpoint(
    workspace: &Path,
    arguments: &Value,
) -> Option<WorkflowOutput> {
    let run_id = arguments.get("run_id")?.as_str()?;
    let query = arguments.pointer("/input/query")?.as_str()?;
    let mut output = recover_dynamic_workflow_step_output(
        workspace,
        run_id,
        query,
        INITIAL_RETRIEVAL_CHECKPOINT_STEP_ID,
    )
    .await
    .ok()??;
    if output.get("query").and_then(Value::as_str) != Some(query)
        || output.get("mode").and_then(Value::as_str) != Some("inquiry_collection")
    {
        return None;
    }
    let research = output.get_mut("research")?.as_object_mut()?;
    let warnings = research
        .entry("warnings")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()?;
    let errors = warnings
        .entry("collection_errors")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()?;
    if !errors
        .iter()
        .any(|error| error.as_str() == Some(INITIAL_RETRIEVAL_RECOVERY_WARNING))
    {
        errors.push(Value::String(
            INITIAL_RETRIEVAL_RECOVERY_WARNING.to_string(),
        ));
    }
    Some(WorkflowOutput {
        output: serde_json::to_string(&output).ok()?,
        metadata: Some(serde_json::json!({
            "dynamic_workflow": {
                "run_id": run_id,
                "recovered_step": INITIAL_RETRIEVAL_CHECKPOINT_STEP_ID,
                "recovered_initial_retrieval": true
            }
        })),
    })
}
