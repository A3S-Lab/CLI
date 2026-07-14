//! DeepResearch, tool, plan, and input-history state projections.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DeepResearchLoop {
    pub(super) query: String,
    pub(super) total_layers: usize,
    pub(super) os_runtime: bool,
    pub(super) evidence_scope: DeepResearchEvidenceScope,
    pub(super) started_at: Instant,
    pub(super) phase_started_at: Option<Instant>,
}

impl DeepResearchLoop {
    pub(super) fn verification_prompt(&self, next_layer: usize) -> String {
        let report_target = deep_research_report_target_note(&self.query);
        deep_research_prompts::verification_prompt(deep_research_prompts::VerificationPrompt {
            next_layer,
            total_layers: self.total_layers,
            query: &self.query,
            report_target: &report_target,
        })
    }
}

pub(super) fn deep_research_report_repair_prompt_from_state(
    loop_state: Option<&DeepResearchLoop>,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    review_text: &str,
) -> Option<String> {
    let loop_state = loop_state?;
    Some(deep_research_repair_prompt_with_scope(
        &loop_state.query,
        loop_state.os_runtime,
        workflow_output,
        workflow_metadata,
        review_text,
        loop_state.evidence_scope,
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkflowSubagentBackfill {
    pub(super) task_id: String,
    pub(super) agent: String,
    pub(super) description: String,
    pub(super) success: bool,
}

pub(super) fn workflow_parallel_subagent_backfills(
    metadata: &serde_json::Value,
) -> Vec<WorkflowSubagentBackfill> {
    let Some(steps) = metadata
        .pointer("/dynamic_workflow/snapshot/steps")
        .and_then(serde_json::Value::as_object)
    else {
        return Vec::new();
    };

    let mut backfills = Vec::new();
    for step in steps.values() {
        if step.get("step_name").and_then(serde_json::Value::as_str) != Some("parallel_task") {
            continue;
        }
        let descriptions = step
            .pointer("/input/tasks")
            .and_then(serde_json::Value::as_array)
            .map(|tasks| {
                tasks
                    .iter()
                    .map(|task| {
                        task.get("description")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string()
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let Some(results) = step
            .pointer("/output/metadata/results")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for (index, result) in results.iter().enumerate() {
            let Some(task_id) = result.get("task_id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let agent = result
                .get("agent")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("general")
                .to_string();
            let success = result
                .get("success")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            backfills.push(WorkflowSubagentBackfill {
                task_id: task_id.to_string(),
                agent,
                description: descriptions.get(index).cloned().unwrap_or_default(),
                success,
            });
        }
    }
    backfills
}

pub(super) fn is_new_remote_view(
    last_view: Option<&remote_ui::ViewSpec>,
    spec: &remote_ui::ViewSpec,
) -> bool {
    last_view != Some(spec)
}

pub(super) fn take_pending_tool_label(
    pending_tools: &mut VecDeque<(String, String)>,
    tool_id: &str,
) -> Option<(String, bool)> {
    let index = pending_tools
        .iter()
        .position(|(pending_id, _)| pending_id == tool_id)?;
    let was_front = index == 0;
    pending_tools
        .remove(index)
        .map(|(_, label)| (label, was_front))
}

pub(super) fn take_pending_tools_for_confirmation(
    pending_tools: &mut VecDeque<(String, String)>,
    expected_tool_id: &str,
    take_all: bool,
) -> Vec<(String, String)> {
    if pending_tools
        .front()
        .is_none_or(|(tool_id, _)| tool_id != expected_tool_id)
    {
        return Vec::new();
    }
    if take_all {
        pending_tools.drain(..).collect()
    } else {
        pending_tools.pop_front().into_iter().collect()
    }
}

/// Presentation ownership for a model-requested tool call.
///
/// Most tools own a durable transcript cell. Plan updates instead own the
/// pinned checklist above the input; retaining a second transcript cell would
/// show the same state twice. The runtime projection still tracks the call so
/// duplicate terminal delivery cannot reintroduce it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToolPresentationPolicy {
    Transcript,
    PinnedOnly,
}

pub(super) fn presentation_policy(tool_name: &str) -> ToolPresentationPolicy {
    if tool_name.trim().eq_ignore_ascii_case("update_plan") {
        ToolPresentationPolicy::PinnedOnly
    } else {
        ToolPresentationPolicy::Transcript
    }
}

impl ToolPresentationPolicy {
    pub(super) fn transcript_visible(self) -> bool {
        matches!(self, Self::Transcript)
    }
}

/// Typed materialized view of the active turn's plan.
///
/// Keep semantic task status until the presentation boundary. Storing glyphs
/// and colours here previously collapsed skipped/cancelled back to pending and
/// made synthesis consume UI decoration as domain state.
#[derive(Clone, Debug, Default)]
pub(super) struct PlanProjection {
    tasks: Vec<a3s_code_core::planning::Task>,
}

impl PlanProjection {
    pub(super) fn replace(&mut self, tasks: &[a3s_code_core::planning::Task]) {
        self.tasks = tasks.to_vec();
    }

    pub(super) fn update_status(&mut self, id: &str, status: a3s_code_core::planning::TaskStatus) {
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            task.status = status;
        }
    }

    pub(super) fn tasks(&self) -> &[a3s_code_core::planning::Task] {
        &self.tasks
    }

    pub(super) fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.tasks.clear();
    }
}

pub(super) fn history_recall_value(
    history: &[String],
    position: &mut Option<usize>,
    draft: &mut Option<String>,
    current: &str,
    up: bool,
) -> Option<String> {
    if history.is_empty() {
        return None;
    }

    let pos = match (*position, up) {
        (None, true) => {
            *draft = Some(current.to_string());
            history.len() - 1
        }
        (None, false) => return None,
        (Some(i), true) => i.saturating_sub(1),
        (Some(i), false) => i.saturating_add(1),
    };

    if pos >= history.len() {
        *position = None;
        Some(draft.take().unwrap_or_default())
    } else {
        *position = Some(pos);
        Some(history[pos].clone())
    }
}

pub(super) fn should_exit_prompt_mode(
    state: &State,
    shell_mode: bool,
    research_mode: bool,
    key: &KeyEvent,
) -> bool {
    state != &State::Streaming && (shell_mode || research_mode) && key.code == KeyCode::Esc
}
