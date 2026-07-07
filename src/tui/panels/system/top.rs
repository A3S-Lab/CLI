//! `/top` agent-activity observation panel. Reuses `a3s top`'s shared
//! process-table view for columns, colours, agent detection, and risk. Enter
//! drills into a coding agent's process subtree.

use super::super::*;
use a3s_tui::components::DataTableMsg;
use a3s_tui::event::MouseEvent;

use crate::top::process_data_table;

fn top_panel_line(rendered: &str, width: usize) -> String {
    pad_to(&truncate(rendered, width), width)
}

fn top_panel_header(
    processes: usize,
    agents: usize,
    focus: Option<(&str, u32)>,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }

    let (left, center, right) = match focus {
        Some((label, pid)) => (
            format!("  /top ▸ {label} (pid {pid})"),
            format!("{processes} activity rows"),
            "Esc back".to_string(),
        ),
        None => (
            "  /top".to_string(),
            format!("{processes} activity rows · {agents} agent(s)"),
            if width < 40 {
                "Esc close".to_string()
            } else {
                "Enter focus agent · Esc close".to_string()
            },
        ),
    };

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    chrome
        .status_bar()
        .left(left)
        .center(center)
        .right(right)
        .fg(ACCENT)
        .no_bg()
        .bold(true)
        .view(width.min(u16::MAX as usize) as u16)
}

impl App {
    /// Rows currently shown in `/top`: the focused agent's process subtree, or
    /// coding-agent roots plus their descendants when not focused.
    pub(crate) fn top_rows(&self) -> Vec<ProcessRow> {
        let Some(all) = &self.top else {
            return Vec::new();
        };
        match self.top_focus {
            Some(root) => process_subtree(all, root),
            None => agent_activity_rows(all),
        }
    }

    /// Full-screen `/top` observation view; coding-agent rows are highlighted and can be
    /// drilled into. The body is rendered by the shared `a3s top` renderer.
    pub(crate) fn render_top_panel(&self) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let rows = self.top_rows();
        let agents = rows.iter().filter(|r| r.agent.is_some()).count();

        let focus = match self.top_focus {
            Some(pid) => {
                let label = self
                    .top
                    .as_ref()
                    .and_then(|all| all.iter().find(|r| r.pid == pid))
                    .and_then(|r| r.agent.map(|a| a.label()))
                    .unwrap_or("agent");
                Some((label, pid))
            }
            None => None,
        };
        // Body via the shared renderer. The panel keeps a short per-pid history
        // so the shared Sparkline columns show live CPU/MEM trends.
        let hidden = HashSet::new();
        let history = |pid: u32| self.top_history.values(pid);
        let table = render_process_table(
            &rows,
            &ProcessTableView {
                selected: self.top_sel,
                scroll: self.top_scroll,
                width: self.width,
                height: h.saturating_sub(1).max(1),
                hidden: &hidden,
                history: Some(&history),
            },
        );

        let mut out = vec![top_panel_header(rows.len(), agents, focus, width)];
        out.extend(table.lines().map(|line| top_panel_line(line, width)));
        while out.len() < h {
            out.push(String::new());
        }
        out.truncate(h);

        out.join("\n")
    }

    pub(crate) fn handle_top_mouse(&mut self, mouse: &MouseEvent) -> Option<Cmd<Msg>> {
        let rows = self.top_rows();
        let total = rows.len();
        if total == 0 || self.width == 0 || self.height == 0 {
            return None;
        }

        let hidden = HashSet::new();
        let history = |pid: u32| self.top_history.values(pid);
        let table_height = (self.height as usize).saturating_sub(1).max(1);
        let mut table = process_data_table(
            &rows,
            &ProcessTableView {
                selected: self.top_sel.min(total - 1),
                scroll: self.top_scroll,
                width: self.width,
                height: table_height,
                hidden: &hidden,
                history: Some(&history),
            },
        );
        table.set_y_offset(1);
        let before = table.selected_index().unwrap_or(0).min(total - 1);

        match table.handle_mouse(mouse, table_height) {
            Some(DataTableMsg::Selected(index)) => {
                self.top_sel = index.min(total - 1);
                self.top_scroll = table.scroll_offset();
            }
            None => {
                let after = table.selected_index().unwrap_or(before).min(total - 1);
                if after != before {
                    self.top_sel = after;
                    self.top_scroll = table.scroll_offset();
                }
            }
        }
        None
    }
}

/// Coding-agent roots plus all transitive children by ppid, preserving input order.
fn agent_activity_rows(rows: &[ProcessRow]) -> Vec<ProcessRow> {
    let roots = rows
        .iter()
        .filter(|row| row.agent.is_some())
        .map(|row| row.pid)
        .collect::<HashSet<_>>();
    if roots.is_empty() {
        return Vec::new();
    }
    process_forest(rows, roots)
}

/// All processes in `root`'s subtree (root + transitive children by ppid),
/// preserving the input order.
fn process_subtree(rows: &[ProcessRow], root: u32) -> Vec<ProcessRow> {
    process_forest(rows, HashSet::from([root]))
}

fn process_forest(rows: &[ProcessRow], roots: HashSet<u32>) -> Vec<ProcessRow> {
    let mut included = roots;
    loop {
        let mut added = false;
        for r in rows {
            if !included.contains(&r.pid) && included.contains(&r.ppid) {
                included.insert(r.pid);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    rows.iter()
        .filter(|r| included.contains(&r.pid))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::top::AgentKind;

    fn row(pid: u32, ppid: u32, command: &str) -> ProcessRow {
        ProcessRow {
            pid,
            ppid,
            cpu_pct: 0.0,
            mem_pct: 0.0,
            elapsed: "00:01".into(),
            cwd: None,
            command: command.into(),
            agent: None,
            risk: crate::top::Risk::Low,
        }
    }

    fn row_with_usage(pid: u32, cpu_pct: f32, mem_pct: f32) -> ProcessRow {
        let mut row = row(pid, 1, "a3s code");
        row.cpu_pct = cpu_pct;
        row.mem_pct = mem_pct;
        row.agent = Some(AgentKind::A3sCode);
        row
    }

    #[test]
    fn top_panel_lines_are_width_bounded_with_styles() {
        let line = top_panel_header(123, 4, None, 28);
        let plain = a3s_tui::style::strip_ansi(&line);

        assert!(a3s_tui::style::visible_len(&line) <= 28, "{}", plain);
        assert!(plain.ends_with("Esc close"), "{plain}");
        assert!(line.contains("\x1b["), "status header should carry styling");
    }

    #[test]
    fn top_panel_header_stays_observe_only() {
        let title = a3s_tui::style::strip_ansi(&top_panel_header(12, 2, None, 80));

        assert!(title.contains("Enter focus agent"), "{title}");
        assert!(!title.contains(&["ki", "ll"].join("")), "{title}");
        assert!(!title.contains("terminate"), "{title}");
    }

    #[test]
    fn focused_top_panel_header_preserves_back_action() {
        let header = top_panel_header(8, 1, Some(("a3s", 4242)), 32);
        let plain = a3s_tui::style::strip_ansi(&header);

        assert!(plain.contains("/top ▸ a3s"), "{plain}");
        assert!(plain.ends_with("Esc back"), "{plain}");
        assert_eq!(a3s_tui::style::visible_len(&header), 32);
    }

    #[test]
    fn process_subtree_includes_transitive_children() {
        let rows = vec![
            row(10, 1, "root"),
            row(11, 10, "child"),
            row(12, 11, "grandchild"),
        ];

        let pids = process_subtree(&rows, 10)
            .into_iter()
            .map(|row| row.pid)
            .collect::<Vec<_>>();
        assert_eq!(pids, vec![10, 11, 12]);
    }

    #[test]
    fn agent_activity_rows_hide_unrelated_processes() {
        let mut agent = row(20, 1, "a3s code");
        agent.agent = Some(AgentKind::A3sCode);
        let rows = vec![
            row(10, 1, "unrelated-server"),
            agent,
            row(21, 20, "agent-child"),
            row(11, 10, "unrelated-child"),
        ];

        let pids = agent_activity_rows(&rows)
            .into_iter()
            .map(|row| row.pid)
            .collect::<Vec<_>>();
        assert_eq!(pids, vec![20, 21]);
    }

    #[test]
    fn agent_activity_rows_include_transitive_agent_children() {
        let mut agent = row(30, 1, "codex");
        agent.agent = Some(AgentKind::Codex);
        let rows = vec![
            agent,
            row(31, 30, "shell"),
            row(32, 31, "test runner"),
            row(40, 1, "database"),
        ];

        let pids = agent_activity_rows(&rows)
            .into_iter()
            .map(|row| row.pid)
            .collect::<Vec<_>>();
        assert_eq!(pids, vec![30, 31, 32]);
    }

    #[test]
    fn top_process_history_keeps_recent_samples_and_prunes_missing_pids() {
        let mut history = TopProcessHistory::default();

        for sample in 0..(TOP_HISTORY_LIMIT + 4) {
            history.observe(&[
                row_with_usage(42, sample as f32, (sample * 2) as f32),
                row_with_usage(99, 1.0, 2.0),
            ]);
        }
        history.observe(&[row_with_usage(42, 99.0, 88.0)]);

        let (cpu, mem) = history.values(42);
        assert_eq!(cpu.len(), TOP_HISTORY_LIMIT);
        assert_eq!(mem.len(), TOP_HISTORY_LIMIT);
        assert_eq!(cpu.last().copied(), Some(99.0));
        assert_eq!(mem.last().copied(), Some(88.0));
        assert!(history.values(99).0.is_empty());
    }

    #[test]
    fn top_process_table_uses_history_for_sparkline_columns() {
        let row = row_with_usage(42, 70.0, 30.0);
        let mut history = TopProcessHistory::default();
        history.observe(&[row_with_usage(42, 10.0, 5.0)]);
        history.observe(std::slice::from_ref(&row));

        let hidden = HashSet::new();
        let lookup = |pid| history.values(pid);
        let table = render_process_table(
            &[row],
            &ProcessTableView {
                selected: 0,
                scroll: 0,
                width: 96,
                height: 4,
                hidden: &hidden,
                history: Some(&lookup),
            },
        );
        let plain = a3s_tui::style::strip_ansi(&table);

        assert!(plain.contains("CPU"), "{plain}");
        assert!(plain.contains("MEM"), "{plain}");
        assert!(
            !plain.contains("········"),
            "history-fed sparklines should not use the empty placeholder: {plain}"
        );
        assert!(
            plain.chars().any(|ch| matches!(ch, '▁'..='█')),
            "expected sparkline bar glyphs: {plain}"
        );
    }

    #[test]
    fn top_process_table_mouse_wheel_moves_selection() {
        use a3s_tui::event::MouseEventKind;

        let rows = (0..4)
            .map(|index| row(100 + index, 1, &format!("process-{index}")))
            .collect::<Vec<_>>();
        let hidden = HashSet::new();
        let mut table = process_data_table(
            &rows,
            &ProcessTableView {
                selected: 0,
                scroll: 0,
                width: 80,
                height: 5,
                hidden: &hidden,
                history: None,
            },
        );
        table.set_y_offset(1);

        let msg = table.handle_mouse(
            &MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 3,
                modifiers: a3s_tui::KeyModifiers::NONE,
            },
            5,
        );

        assert_eq!(msg, None);
        assert_eq!(table.selected_index(), Some(1));
        assert_eq!(table.scroll_offset(), 0);
    }

    #[test]
    fn top_process_table_click_selects_visible_body_row() {
        use a3s_tui::event::{MouseButton, MouseEventKind};

        let rows = (0..5)
            .map(|index| row(200 + index, 1, &format!("process-{index}")))
            .collect::<Vec<_>>();
        let hidden = HashSet::new();
        let mut table = process_data_table(
            &rows,
            &ProcessTableView {
                selected: 2,
                scroll: 2,
                width: 80,
                height: 4,
                hidden: &hidden,
                history: None,
            },
        );
        table.set_y_offset(1);

        let msg = table.handle_mouse(
            &MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 0,
                row: 4,
                modifiers: a3s_tui::KeyModifiers::NONE,
            },
            4,
        );

        assert_eq!(msg, Some(DataTableMsg::Selected(3)));
        assert_eq!(table.selected_index(), Some(3));
        assert_eq!(table.scroll_offset(), 2);
    }
}
