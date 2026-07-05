//! `/top` agent-activity observation panel. Reuses `a3s top`'s shared
//! process-table view for columns, colours, agent detection, and risk. Enter
//! drills into a coding agent's process subtree.

use super::super::*;
use a3s_tui::components::SectionHeader;

fn top_panel_line(rendered: &str, width: usize) -> String {
    pad_to(&truncate(rendered, width), width)
}

fn top_panel_header(title: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    SectionHeader::new(title)
        .show_separator(false)
        .indent(2)
        .title_color(ACCENT)
        .view(width.min(u16::MAX as usize) as u16, 1)
}

fn top_panel_title(processes: usize, agents: usize, focus: Option<(&str, u32)>) -> String {
    match focus {
        Some((label, pid)) => {
            format!("/top ▸ {label} (pid {pid}) — {processes} activity rows · Esc back")
        }
        None => format!(
            "/top — {processes} agent activity rows · {agents} agent(s) · Enter focus agent · Esc close"
        ),
    }
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

        let title = match self.top_focus {
            Some(pid) => {
                let label = self
                    .top
                    .as_ref()
                    .and_then(|all| all.iter().find(|r| r.pid == pid))
                    .and_then(|r| r.agent.map(|a| a.label()))
                    .unwrap_or("agent");
                top_panel_title(rows.len(), agents, Some((label, pid)))
            }
            None => top_panel_title(rows.len(), agents, None),
        };
        // Body via the shared renderer; the panel has no per-pid history, so the
        // sparkline columns render blank (graceful degradation).
        let hidden = HashSet::new();
        let table = render_process_table(
            &rows,
            &ProcessTableView {
                selected: self.top_sel,
                scroll: self.top_scroll,
                width: self.width,
                height: h.saturating_sub(1).max(1),
                hidden: &hidden,
                history: None,
            },
        );

        let mut out = vec![top_panel_header(&title, width)];
        out.extend(table.lines().map(|line| top_panel_line(line, width)));
        while out.len() < h {
            out.push(String::new());
        }
        out.truncate(h);

        out.join("\n")
    }
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

    #[test]
    fn top_panel_lines_are_width_bounded_with_styles() {
        let line = top_panel_header("/top — many processes · Enter focus agent · Esc close", 28);

        assert!(
            a3s_tui::style::visible_len(&line) <= 28,
            "{}",
            a3s_tui::style::strip_ansi(&line)
        );
    }

    #[test]
    fn top_panel_title_stays_observe_only() {
        let title = top_panel_title(12, 2, None);

        assert!(title.contains("Enter focus agent"), "{title}");
        assert!(!title.contains(&["ki", "ll"].join("")), "{title}");
        assert!(!title.contains("terminate"), "{title}");
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
