//! Pinned plan/TODO + bottom task tracker + subagent rows, plus relayout.

use super::super::*;
use a3s_tui::components::{Checklist, ChecklistItem, ChecklistStatus};

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
        let width = self.width as usize;
        let cap = width.saturating_sub(8);
        let mut lines = vec![pad_to(
            &Style::new()
                .fg(TN_GRAY)
                .render(&format!("  ─ tasks · ✓ {} done ────────", self.completed)),
            width,
        )];
        if let Some(t) = running {
            lines.push(pad_to(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render(&format!("  ⏳ {}", truncate(t, cap))),
                width,
            ));
        }
        let mut q: Vec<&Queued> = self.queue.iter().collect();
        q.sort_by_key(|x| (x.prio, x.seq));
        for item in q.iter().take(6) {
            lines.push(pad_to(
                &Style::new()
                    .fg(TN_GRAY)
                    .render(&format!("  ▱ {}", truncate(&item.text, cap))),
                width,
            ));
        }
        lines
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
        let now = Instant::now();
        let total = subagents.len();
        let done = subagents.iter().filter(|s| s.done).count();
        let tokens = subagents.iter().map(|s| s.tokens).sum::<u64>();
        let started = subagents.iter().map(|s| s.started).min().unwrap_or(now);
        let ended = if done == total {
            subagents
                .iter()
                .filter_map(|s| s.ended)
                .max()
                .unwrap_or(now)
        } else {
            now
        };
        let elapsed = ended.saturating_duration_since(started);
        let status = if done == total {
            format!("{done}/{total} agents done")
        } else {
            format!(
                "{} running · {done}/{total} done",
                total.saturating_sub(done)
            )
        };
        let right = if tokens > 0 {
            format!(
                "{status} · {} · ↓ {} tokens",
                fmt_elapsed(elapsed),
                fmt_tokens(tokens)
            )
        } else {
            format!("{status} · {}", fmt_elapsed(elapsed))
        };
        let task = self
            .running_task
            .as_deref()
            .unwrap_or("parallel agents")
            .trim();
        let slug = workflow_slug(task);
        let rlen = a3s_tui::style::visible_len(&right);
        let maxleft = width.saturating_sub(rlen + 3).max(8);
        let left = truncate(&format!("  ◯ {slug}  {task}"), maxleft);
        let pad = width.saturating_sub(a3s_tui::style::visible_len(&left) + rlen + 1);
        let mut out = vec![format!(
            "{}{}{}",
            Style::new().fg(ACCENT).bold().render(&left),
            " ".repeat(pad),
            Style::new().fg(TN_GRAY).render(&right),
        )];

        for s in subagents.iter().filter(|s| !s.done).take(4) {
            let el = fmt_elapsed(s.started.elapsed());
            let right = if s.tokens > 0 {
                format!("{el} · ↓ {} tokens", fmt_tokens(s.tokens))
            } else {
                el
            };
            let rlen = a3s_tui::style::visible_len(&right);
            let maxleft = width.saturating_sub(rlen + 3).max(8);
            let left = truncate(&format!("     ◯ {}  {}", s.agent, s.description), maxleft);
            let pad = width.saturating_sub(a3s_tui::style::visible_len(&left) + rlen + 1);
            out.push(format!(
                "{}{}{}",
                Style::new().fg(TN_PURPLE).render(&left),
                " ".repeat(pad),
                Style::new().fg(TN_GRAY).render(&right),
            ));
        }
        out
    }
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

    Checklist::new(items)
        .indent(2)
        .connector(true)
        .pending_color(TN_GRAY)
        .active_color(TN_ORANGE)
        .done_color(TN_GRAY)
        .error_color(TN_RED)
        .text_color(TN_GRAY)
        .view(width.min(u16::MAX as usize) as u16, 8)
        .lines()
        .map(str::to_string)
        .collect()
}

fn plan_checklist_item(text: &str, glyph: char, color: Color) -> ChecklistItem {
    match glyph {
        '✔' => ChecklistItem::new(text)
            .status(ChecklistStatus::Done)
            .color(color),
        '▶' => ChecklistItem::new(text)
            .status(ChecklistStatus::Active)
            .color(TN_ORANGE),
        '✗' => ChecklistItem::new(text)
            .status(ChecklistStatus::Error)
            .color(color),
        _ => ChecklistItem::new(text)
            .status(ChecklistStatus::Pending)
            .color(color),
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
        assert!(lines.iter().any(|line| line.contains("\x1b[9;")));
        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 30),
            "{plain}"
        );
    }
}
