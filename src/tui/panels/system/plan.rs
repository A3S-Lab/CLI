//! Pinned plan/TODO + bottom task tracker + subagent rows, plus relayout.

use super::super::*;
use a3s_code_core::planning::{Task, TaskStatus};
use a3s_tui::components::{
    ChecklistItem, ChecklistStatus, QueuedTask, SubagentRow, SubagentRowStatus,
};

const MAX_PLAN_PANEL_ROWS: usize = 8;

impl App {
    /// a3s-lane task detail lines for the very bottom: the running task plus
    /// each queued message. Empty when there's nothing in flight.
    pub(crate) fn task_lines(&self) -> Vec<String> {
        let running = self
            .running_task
            .as_ref()
            .filter(|_| self.state != State::Idle);
        // Only show the panel when work is actually queued — a lone running
        // task would otherwise resize the viewport every turn (transcript jump).
        if self.queue.is_empty() {
            return Vec::new();
        }
        let theme = agent_chrome_theme();
        let chrome = agent_chrome(&theme);
        let queued = self
            .queue
            .iter()
            .map(|item| {
                chrome
                    .queued_task(item.text.clone())
                    .priority(i32::from(item.prio))
                    .sequence(item.seq)
            })
            .collect::<Vec<_>>();
        task_queue_lines(
            self.completed,
            running.map(String::as_str),
            queued,
            self.width as usize,
        )
    }

    /// Visible transcript rows = the viewport height, mirroring the layout chrome
    /// (separators + status + input + pinned plan/task/subagent rows). Single
    /// source of truth shared by `relayout` and mouse hit-testing.
    pub(crate) fn viewport_rows(&self) -> usize {
        let bottom = self.bottom_pane_projection();
        let dynamic = bottom.dynamic_rows().min(u16::MAX as usize) as u16;
        self.height.saturating_sub(
            super::bottom::FIXED_ROWS_EXCLUDING_INPUT + self.input_height() + dynamic,
        ) as usize
    }

    /// Resize the viewport so the pinned plan panel and the bottom task panel
    /// both fit without covering the transcript. The scrollbar overlays the
    /// final column only while scrolling is available.
    pub(crate) fn relayout(&mut self) {
        self.viewport
            .resize(self.width, self.viewport_rows() as u16);
        self.refresh_transcript_view();
    }

    /// Replace the pinned plan from a planning-mode task list.
    pub(crate) fn set_plan(&mut self, tasks: &[Task]) {
        self.plan.replace(tasks);
        self.relayout();
    }

    /// Update one plan task's status by id (from StepStart/StepEnd events).
    pub(crate) fn set_task_status(&mut self, id: &str, status: TaskStatus) {
        self.plan.update_status(id, status);
        self.refresh_transcript_view();
    }

    /// Apply the canonical Codex `update_plan` arguments to the pinned plan.
    /// Returns false for a partial or invalid argument object so streamed JSON
    /// can be retried when the authoritative ToolExecutionStart arrives.
    pub(crate) fn apply_update_plan_args(&mut self, args: &serde_json::Value) -> bool {
        let Some(tasks) = tasks_from_update_plan_args(args) else {
            return false;
        };
        self.set_plan(&tasks);
        true
    }

    /// The pinned plan/TODO lines, hung under the thinking line with a `⎿`
    /// connector and checkbox glyphs (◻ pending · ◼ in-progress · ☑ done).
    pub(crate) fn plan_lines(&self) -> Vec<String> {
        if self.plan.is_empty() {
            return Vec::new();
        }
        let width = self.width as usize;
        plan_checklist_lines(self.plan.tasks(), width)
    }

    /// Bottom tracker for parallel subagents (Claude-style): a durable summary
    /// row plus live rows for agents still running.
    pub(crate) fn subagent_lines(&self) -> Vec<String> {
        if self.deep_research_loop.is_some()
            && self
                .deep_research_projection
                .as_ref()
                .is_some_and(|projection| projection.active_children.is_empty())
        {
            return Vec::new();
        }
        let subagents = self.runtime.subagents();
        if subagents.is_empty() {
            return Vec::new();
        }
        let width = self.width as usize;
        let task = self
            .runtime
            .subagent_task()
            .or(self.running_task.as_deref())
            .unwrap_or("parallel agents")
            .trim();
        let theme = agent_chrome_theme();
        let chrome = agent_chrome(&theme);
        let rows = subagents
            .into_iter()
            .map(|s| {
                let elapsed = s
                    .ended
                    .unwrap_or_else(Instant::now)
                    .saturating_duration_since(s.started);
                let status = match s.outcome {
                    Some(runtime_projection::SubagentOutcome::Succeeded) => {
                        SubagentRowStatus::Succeeded
                    }
                    Some(runtime_projection::SubagentOutcome::Failed) => SubagentRowStatus::Failed,
                    Some(runtime_projection::SubagentOutcome::Cancelled) => {
                        SubagentRowStatus::Cancelled
                    }
                    Some(runtime_projection::SubagentOutcome::TrackingLost) => {
                        SubagentRowStatus::TrackingLost
                    }
                    None => SubagentRowStatus::Running,
                };
                chrome
                    .subagent_row(s.agent.clone(), s.description.clone())
                    .status(status)
                    .elapsed_seconds(elapsed.as_secs())
                    .tokens(s.tokens)
            })
            .collect::<Vec<_>>();
        subagent_tracker_lines(task, rows, width)
    }
}

fn subagent_tracker_lines(task: &str, rows: Vec<SubagentRow>, width: usize) -> Vec<String> {
    if width == 0 || rows.is_empty() || rows.iter().all(SubagentRow::is_done) {
        return Vec::new();
    }

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    chrome
        .subagent_tracker(task)
        .show_slug(false)
        .rows(rows.clone())
        .max_running_rows(4)
        .margin(2)
        .child_indent(5)
        .marker("•")
        .accent_color(ACCENT)
        .active_color(ACCENT)
        .muted_color(TN_GRAY)
        .error_color(TN_RED)
        .view(width.min(u16::MAX as usize) as u16)
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>()
}

fn task_queue_lines(
    completed: usize,
    running: Option<&str>,
    queued: Vec<QueuedTask>,
    width: usize,
) -> Vec<String> {
    if width == 0 || queued.is_empty() {
        return Vec::new();
    }

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    let mut queue = chrome
        .task_queue()
        .completed(completed)
        .queued_tasks(queued)
        .margin(2)
        .header_color(TN_GRAY)
        .running_color(TN_YELLOW)
        .queued_color(TN_GRAY);
    if let Some(running) = running {
        queue = queue.running(running);
    }

    queue
        .view(width.min(u16::MAX as usize) as u16)
        .lines()
        .map(str::to_string)
        .collect()
}

fn plan_checklist_lines(plan: &[Task], width: usize) -> Vec<String> {
    if width == 0 || plan.is_empty() {
        return Vec::new();
    }

    let (visible, hidden) = focused_plan_tasks(plan, MAX_PLAN_PANEL_ROWS);
    let items = visible
        .into_iter()
        .map(plan_checklist_item)
        .collect::<Vec<_>>();

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    let mut lines = chrome
        .checklist(items)
        .indent(2)
        .connector(true)
        .active_color(TN_ORANGE)
        .done_color(TN_GRAY)
        .skipped_color(TN_GRAY)
        .cancelled_color(TN_GRAY)
        .strikethrough_done(false)
        .view(
            width.min(u16::MAX as usize) as u16,
            MAX_PLAN_PANEL_ROWS.saturating_sub(usize::from(hidden > 0)),
        )
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if hidden > 0 {
        lines.push(a3s_tui::style::fit_visible(
            &Style::new()
                .fg(TN_GRAY)
                .render(&format!("     … {hidden} more")),
            width,
        ));
    }
    lines
}

fn plan_checklist_item(task: &Task) -> ChecklistItem {
    match task.status {
        TaskStatus::Completed => ChecklistItem::new(&task.content)
            .status(ChecklistStatus::Done)
            .glyph_color(TN_GRAY)
            .text_color(TN_GRAY),
        TaskStatus::InProgress => ChecklistItem::new(&task.content)
            .status(ChecklistStatus::Active)
            .glyph_color(TN_ORANGE)
            .text_color(TN_GRAY),
        TaskStatus::Failed => ChecklistItem::new(&task.content)
            .status(ChecklistStatus::Error)
            .glyph_color(TN_RED)
            .text_color(TN_RED),
        TaskStatus::Skipped => ChecklistItem::new(&task.content)
            .status(ChecklistStatus::Skipped)
            .glyph_color(TN_GRAY)
            .text_color(TN_GRAY),
        TaskStatus::Cancelled => ChecklistItem::new(&task.content)
            .status(ChecklistStatus::Cancelled)
            .glyph_color(TN_GRAY)
            .text_color(TN_GRAY),
        TaskStatus::Pending => ChecklistItem::new(&task.content)
            .status(ChecklistStatus::Pending)
            .glyph_color(TN_GRAY)
            .text_color(TN_GRAY),
    }
}

fn focused_plan_tasks(plan: &[Task], max_rows: usize) -> (Vec<&Task>, usize) {
    if plan.len() <= max_rows {
        return (plan.iter().collect(), 0);
    }

    // Reserve one row for an explicit overflow summary. Keep every focal row
    // when it fits, then fill the remaining budget with its nearest context.
    let task_budget = max_rows.saturating_sub(1).max(1);
    let mut selected = plan
        .iter()
        .enumerate()
        .filter_map(|(index, task)| {
            matches!(
                task.status,
                TaskStatus::InProgress | TaskStatus::Failed | TaskStatus::Cancelled
            )
            .then_some(index)
        })
        .collect::<Vec<_>>();

    if selected.len() > task_budget {
        selected = selected.into_iter().rev().take(task_budget).collect();
    }

    let mut candidates = (0..plan.len())
        .filter(|index| !selected.contains(index))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        candidates.sort_unstable();
    } else {
        candidates.sort_by_key(|candidate| {
            let distance = selected
                .iter()
                .map(|focus| focus.abs_diff(*candidate))
                .min()
                .unwrap_or(usize::MAX);
            (distance, *candidate)
        });
    }
    selected.extend(
        candidates
            .into_iter()
            .take(task_budget.saturating_sub(selected.len())),
    );
    selected.sort_unstable();

    let hidden = plan.len().saturating_sub(selected.len());
    (
        selected.into_iter().map(|index| &plan[index]).collect(),
        hidden,
    )
}

fn tasks_from_update_plan_args(args: &serde_json::Value) -> Option<Vec<Task>> {
    let rows = args.get("plan")?.as_array()?;
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let content = row.get("step")?.as_str()?.trim();
            if content.is_empty() {
                return None;
            }
            let status = update_plan_status(row.get("status")?.as_str()?)?;
            let id = row
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("codex-plan-{}", index + 1));
            let mut task = Task::new(id, content);
            task.status = status;
            Some(task)
        })
        .collect()
}

fn update_plan_status(status: &str) -> Option<TaskStatus> {
    match status.trim().to_ascii_lowercase().as_str() {
        "pending" => Some(TaskStatus::Pending),
        "in_progress" | "in-progress" | "active" => Some(TaskStatus::InProgress),
        "completed" | "complete" | "done" => Some(TaskStatus::Completed),
        "failed" | "error" => Some(TaskStatus::Failed),
        "skipped" => Some(TaskStatus::Skipped),
        "cancelled" | "canceled" => Some(TaskStatus::Cancelled),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: usize, content: impl Into<String>, status: TaskStatus) -> Task {
        let mut task = Task::new(id.to_string(), content);
        task.status = status;
        task
    }

    #[test]
    fn subagent_tracker_lines_use_shared_component_and_fit_width() {
        let lines = subagent_tracker_lines(
            "Extract reusable terminal components",
            vec![
                SubagentRow::new("planner", "map panels")
                    .done(true)
                    .elapsed("0.8s")
                    .tokens(900),
                SubagentRow::new("coder", "build tracker")
                    .elapsed("1.4s")
                    .tokens(1_500),
            ],
            72,
        );
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(lines.len(), 2);
        assert!(plain.contains("Extract reusable term"), "{plain}");
        assert!(!plain.contains("extract-reusable-term"), "{plain}");
        assert!(plain.contains("1 running · 1/2 done"), "{plain}");
        assert!(plain.contains("↓ 2.4k tokens"), "{plain}");
        assert!(plain.contains("coder  build tracker"), "{plain}");
        assert!(!plain.contains("planner  map panels"), "{plain}");
        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 72),
            "{plain}"
        );
    }

    #[test]
    fn task_queue_lines_use_shared_component_and_sort_queue() {
        let lines = task_queue_lines(
            3,
            Some("running a deliberately long job that must fit"),
            vec![
                QueuedTask::new("later queued job").priority(4).sequence(2),
                QueuedTask::new("first queued job").priority(1).sequence(9),
            ],
            34,
        );
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>();

        assert_eq!(lines.len(), 4);
        assert!(plain[0].contains("tasks · ✓ 3 done"), "{plain:?}");
        assert!(plain[1].contains("● running"), "{plain:?}");
        assert!(plain[2].contains("◦ first queued job"), "{plain:?}");
        assert!(plain[3].contains("◦ later queued job"), "{plain:?}");
        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 34),
            "{plain:?}"
        );
    }

    #[test]
    fn plan_checklist_lines_use_shared_component_and_fit_width() {
        let plan = vec![
            task(
                1,
                "collect enough evidence for a fairly long task",
                TaskStatus::Pending,
            ),
            task(2, "implement", TaskStatus::InProgress),
            task(3, "verify", TaskStatus::Completed),
            task(4, "fix failure", TaskStatus::Failed),
            task(5, "optional", TaskStatus::Skipped),
            task(6, "obsolete", TaskStatus::Cancelled),
        ];
        let lines = plan_checklist_lines(&plan, 30);
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(lines.len(), 6);
        assert!(plain.contains("⎿  ◻ collect"), "{plain}");
        assert!(plain.contains("◼ implement"), "{plain}");
        assert!(plain.contains("✔ verify"), "{plain}");
        assert!(plain.contains("✗ fix failure"), "{plain}");
        assert!(plain.contains("↷ optional"), "{plain}");
        assert!(plain.contains("⊘ obsolete"), "{plain}");
        assert!(
            lines
                .iter()
                .any(|line| line.contains(&format!("\x1b[{}m◼", TN_ORANGE.fg_ansi()))),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains(&format!("\x1b[{}mimplement", TN_GRAY.fg_ansi()))),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|line| line.contains("\x1b[9;")),
            "pinned plan rows should keep completed text readable: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 30),
            "{plain}"
        );
    }

    #[test]
    fn ninth_active_task_is_kept_with_explicit_hidden_count() {
        let mut plan = (1..=9)
            .map(|id| task(id, format!("step {id}"), TaskStatus::Completed))
            .collect::<Vec<_>>();
        plan[8].status = TaskStatus::InProgress;

        let lines = plan_checklist_lines(&plan, 48);
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(lines.len(), MAX_PLAN_PANEL_ROWS);
        assert!(plain.contains("◼ step 9"), "{plain}");
        assert!(plain.contains("… 2 more"), "{plain}");
        assert!(!plain.contains("✔ step 1"), "{plain}");
    }

    #[test]
    fn codex_update_plan_schema_preserves_terminal_status_semantics() {
        let tasks = tasks_from_update_plan_args(&serde_json::json!({
            "explanation": "Keep the panel current.",
            "plan": [
                {"step": "inspect", "status": "completed"},
                {"step": "optional", "status": "skipped"},
                {"step": "obsolete", "status": "cancelled"}
            ]
        }))
        .expect("valid Codex update_plan arguments");

        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].status, TaskStatus::Completed);
        assert_eq!(tasks[1].status, TaskStatus::Skipped);
        assert_eq!(tasks[2].status, TaskStatus::Cancelled);
        assert!(tasks_from_update_plan_args(&serde_json::json!({
            "plan": [{"step": "partial", "status": "unknown"}]
        }))
        .is_none());
    }

    #[test]
    fn update_plan_is_pinned_only_and_fresh_turn_clears_projection() {
        assert_eq!(
            presentation_policy("update_plan"),
            ToolPresentationPolicy::PinnedOnly
        );
        assert!(!presentation_policy("UPDATE_PLAN").transcript_visible());
        assert!(presentation_policy("shell").transcript_visible());

        let mut projection = PlanProjection::default();
        projection.replace(&[task(1, "old turn", TaskStatus::InProgress)]);
        projection.clear();
        assert!(projection.is_empty());
        projection.replace(&[task(1, "queued fresh turn", TaskStatus::Pending)]);
        assert_eq!(projection.tasks()[0].content, "queued fresh turn");
    }

    #[test]
    fn pinned_update_plan_skips_transcript_but_keeps_terminal_deduplication() {
        let id = "call-update-plan".to_string();
        let name = "update_plan".to_string();
        let args = serde_json::json!({
            "plan": [{"step": "verify", "status": "in_progress"}]
        });
        let policy = presentation_policy(&name);
        let mut transcript = Transcript::default();
        let mut runtime = RuntimeProjection::default();

        transcript.start_tool(id.clone(), name.clone(), policy.transcript_visible());
        runtime.prepare_tool(id.clone(), name.clone());
        transcript.start_tool_execution(
            id.clone(),
            name.clone(),
            args.clone(),
            policy.transcript_visible(),
        );
        runtime.start_execution(id.clone(), name.clone(), args.clone());
        let completed = runtime.end_tool(&id, name, Some(args.clone()), "ok".into(), 0);
        transcript.discard_tool(&id);

        assert!(transcript
            .render_transcript_with_activity(80, 76, false)
            .is_empty());
        assert_eq!(completed.args.as_ref(), Some(&args));
        assert_eq!(completed.state, ToolCallState::Succeeded);
        assert!(completed.first_terminal);
    }

    #[test]
    fn interrupted_pinned_update_plan_stays_out_of_transcript() {
        let id = "call-interrupted-plan".to_string();
        let name = "update_plan".to_string();
        let mut transcript = Transcript::default();
        let mut runtime = RuntimeProjection::default();

        transcript.start_tool(id.clone(), name.clone(), false);
        runtime.prepare_tool(id.clone(), name);
        let completed = runtime
            .interrupt_unfinished_tools()
            .into_iter()
            .next()
            .expect("active plan tool");
        transcript.discard_tool(&completed.id);
        transcript.interrupt_unfinished_tools();

        assert!(transcript
            .render_transcript_with_activity(80, 76, false)
            .is_empty());
        assert_eq!(completed.state, ToolCallState::Interrupted);
        assert!(completed.first_terminal);
    }

    #[test]
    fn subagent_tracker_lines_clear_when_all_agents_finish() {
        let lines = subagent_tracker_lines(
            "DeepResearch runtime comparison",
            vec![
                SubagentRow::new("planner", "map sources")
                    .done(true)
                    .elapsed("0.8s")
                    .tokens(900),
                SubagentRow::new("reviewer", "cross-check claims")
                    .done(false)
                    .elapsed("1.4s")
                    .tokens(1_500),
            ],
            72,
        );
        assert!(
            lines.is_empty(),
            "completed agents must leave no pinned status rows"
        );
    }
}
