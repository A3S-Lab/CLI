//! Pinned plan/TODO + bottom task tracker + subagent rows, plus relayout.

use super::super::*;
use a3s_tui::components::{ChecklistItem, ChecklistStatus, QueuedTask, SubagentRow};

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
        let n = (self.task_lines().len() + self.plan_lines().len() + self.subagent_lines().len())
            as u16;
        self.height.saturating_sub(6 + self.input_height() + n) as usize
    }

    /// Resize the viewport so the pinned plan panel and the bottom task panel
    /// both fit without covering the transcript. Width reserves one column on the
    /// right for the scrollbar (`append_scrollbar`), so content never clips it.
    pub(crate) fn relayout(&mut self) {
        self.viewport
            .resize(self.width.saturating_sub(1), self.viewport_rows() as u16);
    }

    /// Replace the pinned plan from a planning-mode task list.
    pub(crate) fn set_plan(&mut self, tasks: &[a3s_code_core::planning::Task]) {
        self.plan = tasks
            .iter()
            .map(|t| {
                let (g, c) = task_status_style(t.status);
                (t.id.clone(), t.content.clone(), g, c)
            })
            .collect();
        self.relayout();
    }

    /// Update one plan task's status by id (from StepStart/StepEnd events).
    pub(crate) fn set_task_status(&mut self, id: &str, glyph: char, color: Color) {
        if let Some(t) = self.plan.iter_mut().find(|t| t.0 == id) {
            t.2 = glyph;
            t.3 = color;
        }
    }

    /// The pinned plan/TODO lines, hung under the thinking line with a `⎿`
    /// connector and checkbox glyphs (◻ pending · ◼ in-progress · ☑ done).
    pub(crate) fn plan_lines(&self) -> Vec<String> {
        if self.plan.is_empty() {
            return Vec::new();
        }
        let width = self.width as usize;
        plan_checklist_lines(&self.plan, width)
    }

    /// Bottom tracker for parallel subagents (Claude-style): a durable summary
    /// row plus live rows for agents still running.
    pub(crate) fn subagent_lines(&self) -> Vec<String> {
        let subagents = self.runtime.subagents();
        if subagents.is_empty() {
            return Vec::new();
        }
        let width = self.width as usize;
        let task = self
            .running_task
            .as_deref()
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
                let mut row = chrome
                    .subagent_row(s.agent.clone(), s.description.clone())
                    .elapsed(fmt_elapsed(elapsed))
                    .tokens(s.tokens);
                if s.done {
                    row = row.done(s.success.unwrap_or(true));
                }
                row
            })
            .collect::<Vec<_>>();
        subagent_tracker_lines(task, rows, width)
    }
}

fn subagent_tracker_lines(task: &str, rows: Vec<SubagentRow>, width: usize) -> Vec<String> {
    if width == 0 || rows.is_empty() {
        return Vec::new();
    }

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    chrome
        .subagent_tracker(task)
        .slug(workflow_slug(task))
        .rows(rows)
        .max_running_rows(4)
        .margin(2)
        .child_indent(5)
        .marker("◯")
        .accent_color(ACCENT)
        .active_color(TN_PURPLE)
        .muted_color(TN_GRAY)
        .error_color(TN_RED)
        .view(width.min(u16::MAX as usize) as u16)
        .lines()
        .map(str::to_string)
        .collect()
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

fn plan_checklist_lines(plan: &[(String, String, char, Color)], width: usize) -> Vec<String> {
    if width == 0 || plan.is_empty() {
        return Vec::new();
    }

    let items = plan
        .iter()
        .take(8)
        .map(|(_, text, glyph, color)| plan_checklist_item(text, *glyph, *color))
        .collect::<Vec<_>>();

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    chrome
        .checklist(items)
        .indent(2)
        .connector(true)
        .active_color(TN_ORANGE)
        .done_color(TN_GRAY)
        .strikethrough_done(false)
        .view(width.min(u16::MAX as usize) as u16, 8)
        .lines()
        .map(str::to_string)
        .collect()
}

fn plan_checklist_item(text: &str, glyph: char, color: Color) -> ChecklistItem {
    let text_color = if glyph == '✗' { TN_RED } else { TN_GRAY };
    match glyph {
        '✔' => ChecklistItem::new(text)
            .status(ChecklistStatus::Done)
            .glyph_color(color)
            .text_color(text_color),
        '▶' => ChecklistItem::new(text)
            .status(ChecklistStatus::Active)
            .glyph_color(TN_ORANGE)
            .text_color(text_color),
        '✗' => ChecklistItem::new(text)
            .status(ChecklistStatus::Error)
            .glyph_color(color)
            .text_color(text_color),
        _ => ChecklistItem::new(text)
            .status(ChecklistStatus::Pending)
            .glyph_color(color)
            .text_color(text_color),
    }
}

fn workflow_slug(text: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in text.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
        if slug.len() >= 24 {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "parallel-agents".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(plain.contains("extract-reusable-term"), "{plain}");
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
        assert!(plain[1].contains("⏳ running"), "{plain:?}");
        assert!(plain[2].contains("▱ first queued job"), "{plain:?}");
        assert!(plain[3].contains("▱ later queued job"), "{plain:?}");
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
            (
                "1".to_string(),
                "collect enough evidence for a fairly long task".to_string(),
                '□',
                TN_GRAY,
            ),
            ("2".to_string(), "implement".to_string(), '▶', TN_YELLOW),
            ("3".to_string(), "verify".to_string(), '✔', TN_GRAY),
            ("4".to_string(), "fix failure".to_string(), '✗', TN_RED),
        ];
        let lines = plan_checklist_lines(&plan, 30);
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(lines.len(), 4);
        assert!(plain.contains("⎿  ◻ collect"), "{plain}");
        assert!(plain.contains("◼ implement"), "{plain}");
        assert!(plain.contains("✔ verify"), "{plain}");
        assert!(plain.contains("✗ fix failure"), "{plain}");
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
}
