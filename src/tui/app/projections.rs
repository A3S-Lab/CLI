//! DeepResearch, tool, plan, and input-history state projections.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DeepResearchLoop {
    pub(super) query: String,
    pub(super) evidence_scope: DeepResearchEvidenceScope,
    pub(super) started_at: Instant,
}

pub(super) fn is_new_remote_view(
    last_view: Option<&remote_ui::ViewSpec>,
    spec: &remote_ui::ViewSpec,
) -> bool {
    last_view != Some(spec)
}

pub(super) fn take_pending_tool_approval(
    pending_tools: &mut VecDeque<PendingToolApproval>,
    tool_id: &str,
) -> Option<(PendingToolApproval, bool)> {
    let index = pending_tools
        .iter()
        .position(|pending| pending.tool_id == tool_id)?;
    let was_front = index == 0;
    pending_tools
        .remove(index)
        .map(|pending| (pending, was_front))
}

pub(super) fn take_pending_tool_for_confirmation(
    pending_tools: &mut VecDeque<PendingToolApproval>,
    expected_tool_id: &str,
) -> Option<PendingToolApproval> {
    if pending_tools
        .front()
        .is_none_or(|pending| pending.tool_id != expected_tool_id)
    {
        return None;
    }
    pending_tools.pop_front()
}

/// A parked `ask_user` question. It is not a permission approval and not a steer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingUserQuestion {
    pub(super) question_id: String,
    pub(super) question: String,
    pub(super) options: Vec<String>,
    pub(super) allow_free_text: bool,
    pub(super) selected: usize,
    /// Composer owns the reply. True when there are no options, or the user
    /// chose the typed-reply row.
    pub(super) composing: bool,
    pub(super) stashed_composer: Option<String>,
    pub(super) line: String,
}

impl PendingUserQuestion {
    /// Option picker owns keys. A typed reply does not.
    pub(super) fn owns_picker(&self) -> bool {
        !self.options.is_empty() && !self.composing
    }
}

pub(super) fn project_user_question(
    question_id: &str,
    question: &str,
    options: &[String],
    allow_free_text: bool,
) -> PendingUserQuestion {
    let mut line = format!("Question · {question}");
    if !options.is_empty() {
        line.push_str(" · ");
        line.push_str(&options.join(" | "));
    }
    if options.is_empty() {
        line.push_str(" · reply to answer; this is not an approval");
    } else {
        line.push_str(" · choose an option; this is not an approval");
    }
    PendingUserQuestion {
        question_id: question_id.to_string(),
        question: question.to_string(),
        options: options.to_vec(),
        allow_free_text,
        selected: 0,
        composing: options.is_empty(),
        stashed_composer: None,
        line,
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
    omitted_tasks: usize,
}

pub(super) const MAX_PROJECTED_PLAN_TASKS: usize = 256;
pub(super) const MAX_PLAN_TASK_CONTENT_CHARS: usize = 480;

impl PlanProjection {
    pub(super) fn replace(&mut self, tasks: &[a3s_code_core::planning::Task]) {
        self.replace_with_total(tasks, tasks.len());
    }

    pub(super) fn replace_with_total(
        &mut self,
        tasks: &[a3s_code_core::planning::Task],
        total_tasks: usize,
    ) {
        self.tasks = tasks
            .iter()
            .take(MAX_PROJECTED_PLAN_TASKS)
            .map(project_plan_task)
            .collect();
        self.omitted_tasks = total_tasks
            .max(tasks.len())
            .saturating_sub(self.tasks.len());
    }

    pub(super) fn update_status(&mut self, id: &str, status: a3s_code_core::planning::TaskStatus) {
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            task.status = status;
        }
    }

    pub(super) fn tasks(&self) -> &[a3s_code_core::planning::Task] {
        &self.tasks
    }

    pub(super) fn omitted_tasks(&self) -> usize {
        self.omitted_tasks
    }

    pub(super) fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.tasks.clear();
        self.omitted_tasks = 0;
    }
}

fn project_plan_task(task: &a3s_code_core::planning::Task) -> a3s_code_core::planning::Task {
    let content =
        crate::sanitization::sanitize_display_text(&task.content, MAX_PLAN_TASK_CONTENT_CHARS);
    let mut projected = a3s_code_core::planning::Task::new(
        task.id.clone(),
        if content.is_empty() {
            "Untitled step".to_string()
        } else {
            content
        },
    );
    projected.status = task.status;
    projected
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

/// Whether ↑/↓ should drive session prompt history instead of caret motion.
///
/// Cursor-like grammar: single-line drafts always recall; multiline drafts keep
/// caret motion except ↑ on the first row; once browsing, both arrows navigate.
pub(super) fn should_recall_prompt_history(
    up: bool,
    multiline: bool,
    browsing: bool,
    cursor_row: usize,
) -> bool {
    browsing || !multiline || (up && cursor_row == 0)
}

/// Plan-mode write rejection. Distinct from an approval prompt: there is
/// nothing to grant, and the line must not look like a permission request.
pub(super) fn project_mode_denial(tool_name: &str, reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        format!("Mode denial · {tool_name} · plan is read-only; this is not an approval")
    } else {
        format!("Mode denial · {tool_name} · {reason} · this is not an approval")
    }
}

pub(super) fn permission_denied_transcript(mode: Mode, tool_name: &str, reason: &str) -> String {
    if mode == Mode::Plan {
        project_mode_denial(tool_name, reason)
    } else {
        format!("Permission denied: {reason}")
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

#[cfg(test)]
mod tests {
    use super::{permission_denied_transcript, project_mode_denial, project_user_question, Mode};

    #[test]
    fn user_question_renders_as_a_question_not_an_approval_or_steer() {
        let projected = project_user_question(
            "ask-session-1",
            "Which name?",
            &["left".to_string(), "right".to_string()],
            false,
        );
        assert!(projected.line.starts_with("Question ·"));
        assert!(projected.line.contains("left | right"));
        assert!(projected.line.contains("choose an option"));
        assert!(projected.line.contains("not an approval"));
        assert!(projected.owns_picker());
        assert!(!projected.line.contains("Awaiting approval"));
        assert!(!projected.line.to_ascii_lowercase().contains("steer"));
        assert!(!projected.line.to_ascii_lowercase().contains("permission"));
        let typed = project_user_question("ask-session-2", "What token?", &[], true);
        assert!(typed.composing);
        assert!(!typed.owns_picker());
        assert!(typed.line.contains("reply to answer"));
    }

    #[test]
    fn denied_plan_writes_render_as_mode_denial_not_an_approval_prompt() {
        let line = permission_denied_transcript(Mode::Plan, "write", "workspace write");
        assert_eq!(line, project_mode_denial("write", "workspace write"));
        assert!(line.starts_with("Mode denial · write ·"));
        assert!(line.contains("this is not an approval"));
        assert!(!line.contains("Awaiting approval"));
        assert!(!line.to_ascii_lowercase().contains("permission"));
        let ordinary = permission_denied_transcript(Mode::Default, "bash", "host shell");
        assert!(ordinary.starts_with("Permission denied:"));
        assert!(!ordinary.starts_with("Mode denial"));
    }
}
