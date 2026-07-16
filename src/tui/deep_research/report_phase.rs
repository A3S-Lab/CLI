use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub(crate) struct DelayedDeepResearchReportTool {
    pub(crate) name: String,
    pub(crate) args_json: String,
    pub(crate) authoritative_args: Option<serde_json::Value>,
    pub(crate) output: String,
}

impl DelayedDeepResearchReportTool {
    pub(crate) fn args(&self) -> Option<serde_json::Value> {
        self.authoritative_args
            .clone()
            .or_else(|| serde_json::from_str(&self.args_json).ok())
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ReportPhaseToolBuffer {
    tools: HashMap<String, DelayedDeepResearchReportTool>,
    latest_tool_id: Option<String>,
}

impl ReportPhaseToolBuffer {
    pub(crate) fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.tools.clear();
        self.latest_tool_id = None;
    }

    pub(crate) fn start(&mut self, id: String, name: String) {
        self.latest_tool_id = Some(id.clone());
        self.tools.insert(
            id,
            DelayedDeepResearchReportTool {
                name,
                args_json: String::new(),
                authoritative_args: None,
                output: String::new(),
            },
        );
    }

    pub(crate) fn push_input(&mut self, id: Option<&str>, delta: &str) -> bool {
        let Some(id) = id.or(self.latest_tool_id.as_deref()) else {
            return false;
        };
        let Some(tool) = self.tools.get_mut(id) else {
            return false;
        };
        tool.args_json.push_str(delta);
        true
    }

    pub(crate) fn set_args(
        &mut self,
        id: &str,
        name: String,
        args: serde_json::Value,
        allow_start: bool,
    ) -> bool {
        if !self.tools.contains_key(id) {
            if !allow_start {
                return false;
            }
            self.start(id.to_string(), name);
        }
        let Some(tool) = self.tools.get_mut(id) else {
            return false;
        };
        tool.authoritative_args = Some(args);
        self.latest_tool_id = Some(id.to_string());
        true
    }

    pub(crate) fn push_output_or_start(
        &mut self,
        id: String,
        name: String,
        delta: &str,
        allow_start: bool,
    ) -> bool {
        if !self.tools.contains_key(&id) {
            if !allow_start {
                return false;
            }
            self.start(id.clone(), name);
        }
        if let Some(tool) = self.tools.get_mut(&id) {
            tool.output.push_str(delta);
            self.latest_tool_id = Some(id);
            true
        } else {
            false
        }
    }

    pub(crate) fn take_or_synthetic(
        &mut self,
        id: &str,
        name: String,
        authoritative_args: Option<serde_json::Value>,
        allow_synthetic: bool,
    ) -> Option<DelayedDeepResearchReportTool> {
        let mut tool = self.tools.remove(id);
        self.sync_latest_after_remove(id);
        if let Some(tool) = tool.as_mut() {
            if authoritative_args.is_some() {
                tool.authoritative_args = authoritative_args.clone();
            }
        }
        tool.or_else(|| {
            allow_synthetic.then_some(DelayedDeepResearchReportTool {
                name,
                args_json: String::new(),
                authoritative_args,
                output: String::new(),
            })
        })
    }

    pub(crate) fn remove(&mut self, id: &str) -> bool {
        let removed = self.tools.remove(id).is_some();
        if removed {
            self.sync_latest_after_remove(id);
        }
        removed
    }

    fn sync_latest_after_remove(&mut self, id: &str) {
        if self.latest_tool_id.as_deref() == Some(id) {
            self.latest_tool_id = self.tools.keys().next().cloned();
        }
    }
}

pub(crate) fn suppress_tool_output(
    _tool_name: &str,
    output: &str,
    _args: Option<&serde_json::Value>,
) -> bool {
    let lower = output.to_ascii_lowercase();
    // Hide only the redundant denial noise. Report-phase tools are delayed to
    // keep streamed Markdown coherent, but any tool that actually executed must
    // remain visible to the user, including non-report writes and shell calls.
    lower.contains("permission denied: tool") || lower.contains("blocked by permission policy")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppresses_invalid_tool_noise() {
        assert!(
            suppress_tool_output(
                "write",
                "Permission denied: Tool 'write' is blocked by permission policy.",
                Some(&serde_json::json!({"file_path": "README.md", "content": "oops"})),
            ),
            "denied report-phase writes should not be rendered as user-visible tool cards"
        );
        assert!(
            !suppress_tool_output(
                "read",
                "outside report path",
                Some(&serde_json::json!({"file_path": "README.md"})),
            ),
            "executed non-report reads during report synthesis must remain visible"
        );
        assert!(
            !suppress_tool_output(
                "bash",
                "command completed",
                Some(&serde_json::json!({"command": "pwd"})),
            ),
            "executed shell commands must never be hidden from the user"
        );
        assert!(
            !suppress_tool_output(
                "write",
                "Wrote .a3s/research/topic/report.md",
                Some(&serde_json::json!({
                    "file_path": ".a3s/research/topic/report.md",
                    "content": "# Report"
                })),
            ),
            "real report artifact writes remain visible"
        );
        assert!(
            !suppress_tool_output(
                "read",
                "contents",
                Some(&serde_json::json!({"file_path": ".a3s/research/topic/report.md"})),
            ),
            "real report artifact reads remain available for diagnostics"
        );
    }

    #[test]
    fn buffers_tool_chunks_by_id() {
        let mut buffer = ReportPhaseToolBuffer::default();

        buffer.start("tool-a".to_string(), "read".to_string());
        assert!(buffer.push_input(Some("tool-a"), r#"{"file_path":"#));
        assert!(buffer.push_input(Some("tool-a"), r#""README.md"}"#));
        assert!(buffer.push_output_or_start(
            "tool-a".to_string(),
            "read".to_string(),
            "not found",
            false,
        ));

        let delayed = buffer
            .take_or_synthetic("tool-a", "read".to_string(), None, false)
            .expect("buffered tool should be returned");
        assert_eq!(delayed.name, "read");
        assert_eq!(delayed.args().unwrap()["file_path"], "README.md");
        assert_eq!(delayed.output, "not found");
        assert!(
            buffer
                .take_or_synthetic("tool-a", "read".to_string(), None, false)
                .is_none(),
            "take should remove the buffered tool"
        );
    }

    #[test]
    fn interleaved_tools_never_borrow_a_different_call_id() {
        let mut buffer = ReportPhaseToolBuffer::default();
        buffer.start("tool-a".to_string(), "read".to_string());
        buffer.start("tool-b".to_string(), "grep".to_string());
        assert!(buffer.push_input(Some("tool-a"), r#"{"file_path":"a.rs"}"#));
        assert!(buffer.push_input(Some("tool-b"), r#"{"pattern":"TODO"}"#));

        assert!(buffer
            .take_or_synthetic("missing", "read".to_string(), None, false)
            .is_none());
        let first = buffer
            .take_or_synthetic("tool-a", "read".to_string(), None, false)
            .unwrap();
        let second = buffer
            .take_or_synthetic("tool-b", "grep".to_string(), None, false)
            .unwrap();
        assert_eq!(first.args().unwrap()["file_path"], "a.rs");
        assert_eq!(second.args().unwrap()["pattern"], "TODO");
    }
}
