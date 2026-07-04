//! `/top` process observation panel. Reuses `a3s top`'s shared process-table view
//! so the panel and the standalone monitor agree on columns, colours, agent
//! detection, and risk. Enter drills into a coding agent's process subtree.

use super::super::*;

fn top_panel_line(rendered: &str, width: usize) -> String {
    pad_to(&truncate(rendered, width), width)
}

fn top_panel_title(processes: usize, agents: usize, focus: Option<(&str, u32)>) -> String {
    match focus {
        Some((label, pid)) => {
            format!("  /top ▸ {label} (pid {pid}) — {processes} processes · Esc back")
        }
        None => format!(
            "  /top — {processes} processes · {agents} agent(s) · Enter focus agent · Esc close"
        ),
    }
}

impl App {
    /// Rows currently shown in `/top`: the focused agent's process subtree, or
    /// all processes when not focused.
    pub(crate) fn top_rows(&self) -> Vec<ProcessRow> {
        let Some(all) = &self.top else {
            return Vec::new();
        };
        match self.top_focus {
            Some(root) => process_subtree(all, root),
            None => all.clone(),
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
        let title = Style::new().fg(ACCENT).bold().render(&title);

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

        let mut out = vec![top_panel_line(&title, width)];
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
        let line = top_panel_line(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render("  /top — many processes · Enter focus agent · Esc close"),
            28,
        );

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
}

/// All processes in `root`'s subtree (root + transitive children by ppid),
/// preserving the input order.
// ponytail: O(n²) fixpoint over the process list; n is small (host processes).
fn process_subtree(rows: &[ProcessRow], root: u32) -> Vec<ProcessRow> {
    let mut included: HashSet<u32> = HashSet::from([root]);
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
