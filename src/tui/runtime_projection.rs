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
}

impl RuntimeProjection {
    pub(crate) fn clear_turn_entities(&mut self) {
        self.tools.clear();
        self.tool_order.clear();
        self.latest_tool_id = None;
        self.subagents.clear();
        self.subagent_order.clear();
    }

    pub(crate) fn clear_live_tools(&mut self) {
        self.tools.clear();
        self.tool_order.clear();
        self.latest_tool_id = None;
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
            None => self
                .latest_tool_id
                .clone()
                .and_then(|latest| self.tools.remove(&latest)),
        };
        let active_ids = &self.tools;
        self.tool_order
            .retain(|tool_id| active_ids.contains_key(tool_id));
        if self.latest_tool_id.as_deref() == Some(id) {
            self.latest_tool_id = self.tool_order.last().cloned();
        }

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
        if !self.subagents.contains_key(&task_id) {
            self.subagent_order.push(task_id.clone());
        }
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
}
