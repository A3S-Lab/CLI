//! Pinned plan/TODO + bottom task tracker + subagent rows, plus relayout.

use super::super::*;

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
                .fg(Color::BrightBlack)
                .render(&format!("  ─ tasks · ✓ {} done ────────", self.completed)),
            width,
        )];
        if let Some(t) = running {
            lines.push(pad_to(
                &Style::new()
                    .fg(Color::Yellow)
                    .render(&format!("  ⏳ {}", truncate(t, cap))),
                width,
            ));
        }
        let mut q: Vec<&Queued> = self.queue.iter().collect();
        q.sort_by_key(|x| (x.prio, x.seq));
        for item in q.iter().take(6) {
            lines.push(pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render(&format!("  ▱ {}", truncate(&item.text, cap))),
                width,
            ));
        }
        lines
    }

    /// Resize the viewport so the pinned plan panel and the bottom task panel
    /// both fit without covering the transcript.
    pub(crate) fn relayout(&mut self) {
        let n = (self.task_lines().len() + self.plan_lines().len() + self.subagent_lines().len())
            as u16;
        self.viewport.resize(
            self.width,
            self.height.saturating_sub(6 + self.input_height() + n),
        );
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
        let cap = width.saturating_sub(10);
        let mut lines = Vec::new();
        for (i, (_, text, glyph, color)) in self.plan.iter().take(8).enumerate() {
            // Map the status glyph to a checkbox; done tasks get struck through,
            // in-progress is orange (glyph + text).
            let (boxc, bcolor, done, inprog) = match glyph {
                '✔' => ('✔', TN_GREEN, true, false),
                '▶' => ('◼', TN_ORANGE, false, true),
                '✗' => ('✗', TN_RED, false, false),
                _ => ('◻', Color::BrightBlack, false, false),
            };
            let text_style = if done {
                Style::new().fg(*color).strikethrough()
            } else if inprog {
                Style::new().fg(TN_ORANGE)
            } else {
                Style::new().fg(*color)
            };
            // ⎿ on the first row; align the rest under the checkbox.
            let conn = if i == 0 { "⎿  " } else { "   " };
            lines.push(pad_to(
                &format!(
                    "  {conn}{} {}",
                    Style::new().fg(bcolor).render(&boxc.to_string()),
                    text_style.render(&truncate(text, cap)),
                ),
                width,
            ));
        }
        lines
    }

    /// Bottom tracker for running parallel subagents (Claude-style): one row per
    /// task with the agent type, description, elapsed time, and tokens.
    pub(crate) fn subagent_lines(&self) -> Vec<String> {
        if self.subagents.is_empty() {
            return Vec::new();
        }
        let width = self.width as usize;
        let mut out = vec![pad_to(
            &Style::new().fg(Color::White).bold().render("  ⏺ main"),
            width,
        )];
        for s in &self.subagents {
            let secs = s.started.elapsed().as_secs();
            let el = if secs >= 60 {
                format!("{}m {}s", secs / 60, secs % 60)
            } else {
                format!("{secs}s")
            };
            let right = if s.tokens > 0 {
                format!("{el} · ↓ {} tokens", fmt_tokens(s.tokens))
            } else {
                el
            };
            let glyph = if s.done { '●' } else { '◯' };
            let rlen = a3s_tui::style::visible_len(&right);
            let maxleft = width.saturating_sub(rlen + 3).max(8);
            let left = truncate(
                &format!("  {glyph} {}  {}", s.agent, s.description),
                maxleft,
            );
            let pad = width.saturating_sub(a3s_tui::style::visible_len(&left) + rlen + 1);
            out.push(format!(
                "{}{}{}",
                Style::new().fg(Color::Magenta).render(&left),
                " ".repeat(pad),
                Style::new().fg(Color::BrightBlack).render(&right),
            ));
        }
        out
    }
}
