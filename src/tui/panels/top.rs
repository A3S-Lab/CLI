//! `/top` process monitor panel. Reuses `a3s top`'s shared process-table view
//! so the panel and the standalone monitor agree on columns, colours, agent
//! detection, and risk. Enter drills into a coding agent's process subtree.

use super::super::*;

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

    /// Full-screen `/top` monitor; coding-agent rows are highlighted and can be
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
                Style::new().fg(ACCENT).bold().render(&format!(
                    "  /top ▸ {label} (pid {pid}) — {} processes · Esc back · K kill",
                    rows.len()
                ))
            }
            None => Style::new().fg(ACCENT).bold().render(&format!(
                "  /top — {} processes · {agents} agent(s) · Enter focus agent · K kill · Esc close",
                rows.len()
            )),
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

        let mut out = vec![pad_to(&title, width)];
        out.extend(table.lines().map(str::to_string));
        while out.len() < h {
            out.push(String::new());
        }
        out.truncate(h);

        // Force-kill confirmation: a bright dialog box centred on the panel.
        if let Some((pid, cmd)) = &self.top_kill {
            let bw = 44.min(width.saturating_sub(2)).max(20);
            let inner = bw - 2;
            let vis = a3s_tui::style::visible_len;
            let center = |s: &str| {
                let pad = inner.saturating_sub(vis(s)) / 2;
                format!("{}{s}{}", " ".repeat(pad), " ".repeat(inner - pad - vis(s)))
            };
            let bx = [
                format!("┌{}┐", "─".repeat(inner)),
                format!("│{}│", center("⚠  FORCE-KILL THIS PROCESS?")),
                format!("│{}│", center("")),
                format!("│{}│", center(&format!("PID {pid}"))),
                format!("│{}│", center(&truncate(cmd, inner.saturating_sub(4)))),
                format!("│{}│", center("")),
                format!("│{}│", center("[ Y ] yes        [ N ] no")),
                format!("└{}┘", "─".repeat(inner)),
            ];
            let row0 = h.saturating_sub(bx.len()) / 2;
            let col0 = width.saturating_sub(bw) / 2;
            for (k, line) in bx.iter().enumerate() {
                if let Some(slot) = out.get_mut(row0 + k) {
                    let styled = Style::new()
                        .fg(Color::BrightWhite)
                        .bg(TN_RED)
                        .bold()
                        .render(line);
                    *slot = format!("{}{styled}", " ".repeat(col0));
                }
            }
        }
        out.join("\n")
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
