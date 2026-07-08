//! Runtime event projections for the TUI.
//!
//! This is a deliberately small ECS-style island inside the TEA app: runtime
//! events mutate tool/subagent entities by stable ids, while the main `App`
//! renders read-only projections of the current world.

use std::collections::BTreeMap;
use std::time::Instant;

/// One completed tool call this session, retained for `/output`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolCallRecord {
    pub(crate) name: String,
    pub(crate) args: Option<serde_json::Value>,
    pub(crate) output: String,
    pub(crate) exit_code: i32,
}

/// A live tool entity projected from ToolStart/InputDelta/OutputDelta/ToolEnd.
#[derive(Clone, Debug)]
pub(crate) struct ToolRun {
    pub(crate) name: String,
    args_json: String,
    output: String,
}

impl ToolRun {
    pub(crate) fn args(&self) -> Option<serde_json::Value> {
        serde_json::from_str(&self.args_json).ok()
    }

    pub(crate) fn output(&self) -> &str {
        &self.output
    }
}

/// A running or just-finished parallel subagent task.
#[derive(Clone, Debug)]
pub(crate) struct SubagentRun {
    pub(crate) agent: String,
    pub(crate) description: String,
    pub(crate) started: Instant,
    pub(crate) ended: Option<Instant>,
    pub(crate) tokens: u64,
    pub(crate) done: bool,
    pub(crate) success: Option<bool>,
}

#[derive(Clone, Debug)]
pub(crate) struct CompletedTool {
    pub(crate) args: Option<serde_json::Value>,
}

#[derive(Default, Debug)]
pub(crate) struct RuntimeProjection {
    tools: BTreeMap<String, ToolRun>,
    tool_order: Vec<String>,
    latest_tool_id: Option<String>,
    tool_log: Vec<ToolCallRecord>,
    subagents: BTreeMap<String, SubagentRun>,
    subagent_order: Vec<String>,
    subagent_task: Option<String>,
}

impl RuntimeProjection {
    pub(crate) fn clear_turn_entities(&mut self) {
        self.tools.clear();
        self.tool_order.clear();
        self.latest_tool_id = None;
        self.subagents.clear();
        self.subagent_order.clear();
        self.subagent_task = None;
    }

    pub(crate) fn finish_turn_entities(&mut self, now: Instant) {
        self.tools.clear();
        self.tool_order.clear();
        self.latest_tool_id = None;
        for run in self.subagents.values_mut() {
            if !run.done {
                run.done = true;
                run.success = Some(false);
                run.ended = Some(now);
            }
        }
    }

    pub(crate) fn clear_live_tools(&mut self) {
        self.tools.clear();
        self.tool_order.clear();
        self.latest_tool_id = None;
    }

    pub(crate) fn remove_tool(&mut self, id: &str) -> bool {
        let removed = self.tools.remove(id).is_some();
        self.sync_live_tool_order();
        removed
    }

    pub(crate) fn active_tool_count(&self) -> usize {
        self.tools.len()
    }

    pub(crate) fn active_subagent_count(&self) -> usize {
        self.subagents.values().filter(|run| !run.done).count()
    }

    pub(crate) fn tool_log(&self) -> &[ToolCallRecord] {
        &self.tool_log
    }

    pub(crate) fn subagents(&self) -> Vec<&SubagentRun> {
        self.subagent_order
            .iter()
            .filter_map(|id| self.subagents.get(id))
            .collect()
    }

    pub(crate) fn set_subagent_task(&mut self, task: impl Into<String>) {
        let task = task.into();
        if !task.trim().is_empty() {
            self.subagent_task = Some(task);
        }
    }

    pub(crate) fn subagent_task(&self) -> Option<&str> {
        self.subagent_task.as_deref()
    }

    pub(crate) fn live_tool(&self) -> Option<&ToolRun> {
        self.latest_tool_id
            .as_deref()
            .and_then(|id| self.tools.get(id))
            .or_else(|| {
                self.tool_order
                    .iter()
                    .rev()
                    .find_map(|id| self.tools.get(id))
            })
    }

    pub(crate) fn start_tool(&mut self, id: String, name: String) {
        if !self.tools.contains_key(&id) {
            self.tool_order.push(id.clone());
        }
        self.latest_tool_id = Some(id.clone());
        self.tools.insert(
            id.clone(),
            ToolRun {
                name,
                args_json: String::new(),
                output: String::new(),
            },
        );
    }

    pub(crate) fn push_tool_input(&mut self, delta: &str) {
        if let Some(tool) = self
            .latest_tool_id
            .as_deref()
            .and_then(|id| self.tools.get_mut(id))
        {
            tool.args_json.push_str(delta);
        }
    }

    pub(crate) fn push_tool_output(&mut self, id: String, name: String, delta: &str) {
        if !self.tools.contains_key(&id) {
            self.start_tool(id.clone(), name);
        }
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.output.push_str(delta);
        }
        self.latest_tool_id = Some(id);
    }

    pub(crate) fn end_tool(
        &mut self,
        id: &str,
        name: String,
        output: String,
        exit_code: i32,
    ) -> CompletedTool {
        let run = match self.tools.remove(id) {
            Some(run) => Some(run),
            // Some synthetic/tool-guard events can arrive without the same id
            // shape as their start event. Only fall back when the live set is
            // unambiguous; otherwise keep the remaining live tools intact.
            None if self.tools.len() == 1 => self
                .tool_order
                .last()
                .cloned()
                .and_then(|only| self.tools.remove(&only)),
            None => None,
        };
        self.sync_live_tool_order();

        let args = run
            .as_ref()
            .and_then(|run| serde_json::from_str(&run.args_json).ok());
        let logged_output = bounded_log_output(output);
        self.tool_log.push(ToolCallRecord {
            name,
            args: args.clone(),
            output: logged_output,
            exit_code,
        });
        CompletedTool { args }
    }

    pub(crate) fn start_subagent(
        &mut self,
        task_id: String,
        agent: String,
        description: String,
        now: Instant,
    ) {
        if let Some(run) = self.subagents.get_mut(&task_id) {
            run.agent = agent;
            run.description = description;
            if !run.done {
                run.ended = None;
            }
            return;
        }
        self.subagent_order.push(task_id.clone());
        self.subagents.insert(
            task_id.clone(),
            SubagentRun {
                agent,
                description,
                started: now,
                ended: None,
                tokens: 0,
                done: false,
                success: None,
            },
        );
    }

    pub(crate) fn add_subagent_tokens(&mut self, task_id: &str, tokens: u64) {
        if let Some(run) = self.subagents.get_mut(task_id) {
            run.tokens += tokens;
        }
    }

    pub(crate) fn end_subagent(
        &mut self,
        task_id: String,
        agent: String,
        success: bool,
        now: Instant,
    ) {
        if !self.subagents.contains_key(&task_id) {
            self.subagent_order.push(task_id.clone());
            self.subagents.insert(
                task_id.clone(),
                SubagentRun {
                    agent: agent.clone(),
                    description: String::new(),
                    started: now,
                    ended: None,
                    tokens: 0,
                    done: false,
                    success: None,
                },
            );
        }
        if let Some(run) = self.subagents.get_mut(&task_id) {
            run.agent = agent;
            run.done = true;
            run.success = Some(success);
            run.ended = Some(now);
        }
    }

    fn sync_live_tool_order(&mut self) {
        let active_ids = &self.tools;
        self.tool_order
            .retain(|tool_id| active_ids.contains_key(tool_id));
        if self
            .latest_tool_id
            .as_deref()
            .is_some_and(|latest| !active_ids.contains_key(latest))
        {
            self.latest_tool_id = self.tool_order.last().cloned();
        }
    }
}

fn bounded_log_output(output: String) -> String {
    if output.len() > 8192 {
        let mut s: String = output.chars().take(8000).collect();
        s.push_str("\n… (output truncated — see transcript)");
        s
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_projection_tracks_live_tool_by_id_and_records_log() {
        let mut projection = RuntimeProjection::default();

        projection.start_tool("t1".into(), "bash".into());
        projection.push_tool_input(r#"{"command":"echo hi"}"#);
        projection.push_tool_output("t1".into(), "bash".into(), "hi\n");

        let live = projection.live_tool().expect("live tool");
        assert_eq!(live.name, "bash");
        assert_eq!(live.args().unwrap()["command"], "echo hi");
        assert_eq!(live.output(), "hi\n");
        assert_eq!(projection.active_tool_count(), 1);

        let completed = projection.end_tool("t1", "bash".into(), "hi\n".into(), 0);
        assert_eq!(completed.args.unwrap()["command"], "echo hi");
        assert_eq!(projection.active_tool_count(), 0);
        assert_eq!(projection.tool_log().len(), 1);
        assert_eq!(projection.tool_log()[0].output, "hi\n");
    }

    #[test]
    fn unknown_tool_end_uses_single_live_tool_as_legacy_fallback() {
        let mut projection = RuntimeProjection::default();

        projection.start_tool("actual".into(), "bash".into());
        projection.push_tool_input(r#"{"command":"pwd"}"#);

        let completed = projection.end_tool("missing", "bash".into(), "/work\n".into(), 0);

        assert_eq!(completed.args.unwrap()["command"], "pwd");
        assert_eq!(projection.active_tool_count(), 0);
        assert!(projection.live_tool().is_none());
        assert_eq!(projection.tool_log()[0].output, "/work\n");
    }

    #[test]
    fn unknown_tool_end_does_not_clear_ambiguous_live_tools() {
        let mut projection = RuntimeProjection::default();

        projection.start_tool("a".into(), "bash".into());
        projection.push_tool_input(r#"{"command":"cargo test"}"#);
        projection.start_tool("b".into(), "grep".into());
        projection.push_tool_input(r#"{"pattern":"TODO"}"#);

        let completed = projection.end_tool("missing", "bash".into(), "done\n".into(), 0);

        assert!(completed.args.is_none());
        assert_eq!(projection.active_tool_count(), 2);
        assert_eq!(projection.live_tool().unwrap().name, "grep");
        assert_eq!(projection.tool_log().len(), 1);
        assert_eq!(projection.tool_log()[0].output, "done\n");
    }

    #[test]
    fn remove_tool_clears_only_the_matching_live_tool() {
        let mut projection = RuntimeProjection::default();

        projection.start_tool("a".into(), "bash".into());
        projection.start_tool("b".into(), "read".into());

        assert!(projection.remove_tool("b"));
        assert_eq!(projection.active_tool_count(), 1);
        assert_eq!(projection.live_tool().unwrap().name, "bash");

        assert!(!projection.remove_tool("missing"));
        assert_eq!(projection.active_tool_count(), 1);
    }

    #[test]
    fn subagent_projection_counts_only_running_agents() {
        let mut projection = RuntimeProjection::default();
        let now = Instant::now();

        projection.start_subagent("a".into(), "explore".into(), "inspect".into(), now);
        projection.start_subagent("b".into(), "review".into(), "audit".into(), now);
        projection.add_subagent_tokens("a", 12);
        projection.end_subagent("a".into(), "explore".into(), true, now);

        assert_eq!(projection.active_subagent_count(), 1);
        let runs = projection.subagents();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].tokens, 12);
        assert!(runs[0].done);
        assert!(!runs[1].done);
    }

    #[test]
    fn subagent_projection_preserves_first_start_time_on_duplicate_start() {
        let mut projection = RuntimeProjection::default();
        let first = Instant::now();
        let duplicate = first + std::time::Duration::from_secs(5);
        let end = first + std::time::Duration::from_secs(8);

        projection.start_subagent(
            "a".into(),
            "explore".into(),
            "first description".into(),
            first,
        );
        projection.start_subagent(
            "a".into(),
            "general".into(),
            "refreshed description".into(),
            duplicate,
        );
        projection.end_subagent("a".into(), "general".into(), true, end);

        let runs = projection.subagents();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].agent, "general");
        assert_eq!(runs[0].description, "refreshed description");
        assert_eq!(
            runs[0]
                .ended
                .unwrap()
                .saturating_duration_since(runs[0].started),
            std::time::Duration::from_secs(8)
        );
    }

    #[test]
    fn finish_turn_entities_keeps_completed_subagent_summary() {
        let mut projection = RuntimeProjection::default();
        let start = Instant::now();
        let finish = start + std::time::Duration::from_secs(4);

        projection.set_subagent_task("DeepResearch market map");
        projection.start_tool("tool".into(), "parallel_task".into());
        projection.start_subagent("a".into(), "researcher".into(), "inspect".into(), start);
        projection.start_subagent("b".into(), "reviewer".into(), "audit".into(), start);
        projection.end_subagent("a".into(), "researcher".into(), true, finish);

        projection.finish_turn_entities(finish);

        assert_eq!(projection.active_tool_count(), 0);
        assert_eq!(projection.active_subagent_count(), 0);
        let runs = projection.subagents();
        assert_eq!(runs.len(), 2);
        assert!(runs[0].done);
        assert_eq!(runs[0].success, Some(true));
        assert!(runs[1].done);
        assert_eq!(runs[1].success, Some(false));
        assert_eq!(runs[1].ended, Some(finish));
        assert_eq!(projection.subagent_task(), Some("DeepResearch market map"));
    }

    #[test]
    fn clear_turn_entities_resets_subagent_task_title() {
        let mut projection = RuntimeProjection::default();
        projection.set_subagent_task("DeepResearch market map");
        projection.clear_turn_entities();
        assert_eq!(projection.subagent_task(), None);
    }
}
