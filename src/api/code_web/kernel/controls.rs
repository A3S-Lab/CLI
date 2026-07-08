use serde_json::{json, Value};

use crate::api::code_web::state::CodeWebSessionControls;
use crate::budget::{self, BudgetProfile, BudgetWorkload};

pub(super) fn effort_levels_json() -> Vec<Value> {
    budget::effort_levels_json()
}

pub(super) fn normalize_effort(value: &str) -> Option<&'static BudgetProfile> {
    budget::normalize_effort(value)
}

pub(super) fn controls_json(
    session_id: &str,
    controls: &CodeWebSessionControls,
    context_limit: Option<u32>,
) -> Value {
    let profile = normalize_effort(&controls.effort)
        .or_else(|| normalize_effort(budget::DEFAULT_CODE_WEB_EFFORT_ID))
        .expect("medium effort profile must exist");
    let plan = budget::budget_plan_for_profile(profile, context_limit, BudgetWorkload::Interactive);
    json!({
        "sessionId": session_id,
        "effort": profile.id,
        "goal": controls.goal.clone(),
        "effortLevel": budget::effort_profile_json(profile),
        "effortLevels": effort_levels_json(),
        "budget": budget::budget_plan_json(&plan),
    })
}

pub(super) fn normalize_goal(value: &str) -> Option<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub(super) fn compose_controlled_prompt(
    controls: &CodeWebSessionControls,
    user_prompt: &str,
) -> String {
    if user_prompt.trim_start().starts_with('/') {
        return user_prompt.to_string();
    }

    let mut directives = Vec::new();
    if let Some(profile) = normalize_effort(&controls.effort) {
        if let Some(guideline) = profile.guideline {
            directives.push(guideline.to_string());
        }
    }
    if let Some(goal) = controls.goal.as_deref().and_then(normalize_goal) {
        directives.push(format!(
            "[session goal] Keep this north-star goal in mind for this turn: {goal}"
        ));
    }

    if directives.is_empty() {
        user_prompt.to_string()
    } else {
        format!(
            "{}\n\nUser request:\n{}",
            directives.join("\n\n"),
            user_prompt
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::{BudgetWorkload, DEFAULT_CODE_WEB_EFFORT_ID};

    #[test]
    fn controls_json_exposes_budget_for_selected_effort() {
        let controls = CodeWebSessionControls {
            effort: "xhigh".to_string(),
            goal: Some("finish the migration".to_string()),
        };
        let value = controls_json("session-1", &controls, Some(32_768));
        let budget =
            budget::budget_plan_for_effort_id("xhigh", Some(32_768), BudgetWorkload::Interactive);

        assert_eq!(value["sessionId"], "session-1");
        assert_eq!(value["effort"], "xhigh");
        assert_eq!(value["goal"], "finish the migration");
        assert_eq!(
            value["budget"]["maxToolRounds"].as_u64(),
            Some(budget.max_tool_rounds as u64)
        );
        assert_eq!(
            value["budget"]["maxParallelTasks"].as_u64(),
            Some(budget.max_parallel_tasks as u64)
        );
        assert_eq!(
            value["budget"]["workflowMaxToolCalls"].as_u64(),
            Some(budget.workflow_max_tool_calls as u64)
        );
        assert_eq!(
            value["budget"]["autoCompactThreshold"].as_f64(),
            Some(budget.auto_compact_threshold as f64)
        );
    }

    #[test]
    fn controls_json_falls_back_to_default_budget_for_unknown_effort() {
        let controls = CodeWebSessionControls {
            effort: "warp-drive".to_string(),
            goal: None,
        };
        let value = controls_json("session-2", &controls, None);
        let budget = budget::budget_plan_for_effort_id(
            DEFAULT_CODE_WEB_EFFORT_ID,
            None,
            BudgetWorkload::Interactive,
        );

        assert_eq!(value["effort"], DEFAULT_CODE_WEB_EFFORT_ID);
        assert_eq!(
            value["budget"]["maxToolRounds"].as_u64(),
            Some(budget.max_tool_rounds as u64)
        );
        assert_eq!(
            value["budget"]["maxParallelTasks"].as_u64(),
            Some(budget.max_parallel_tasks as u64)
        );
    }
}
