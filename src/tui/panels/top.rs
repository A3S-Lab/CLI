//! `/top` process monitor panel; coding-agent rows are highlighted.

use super::super::*;

impl App {
    /// Full-screen `/top` process monitor; coding-agent rows are highlighted.
    pub(crate) fn render_top_panel(&self, rows: &[ProcRow]) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let agents = rows.iter().filter(|r| r.agent.is_some()).count();
        let title = Style::new().fg(ACCENT).bold().render(&format!(
            "  /top — {} processes · {agents} coding agent(s) · Enter to kill",
            rows.len()
        ));
        let mut out = vec![
            pad_to(&title, width),
            pad_to(
                &Style::new().fg(Color::BrightBlack).render(
                    "  PID      CPU%   MEM%   COMMAND                        Esc close · ↑/↓ select",
                ),
                width,
            ),
        ];
        let body = h.saturating_sub(3);
        let start = self
            .top_scroll
            .min(rows.len().saturating_sub(body.min(rows.len())));
        for (i, r) in rows.iter().enumerate().skip(start).take(body) {
            let cmd = truncate(&r.cmd, width.saturating_sub(44).max(10));
            let tag = r.agent.map(|a| format!("   ◀ {a}")).unwrap_or_default();
            let raw = pad_to(
                &format!("  {:<7} {:>5.1}  {:>5.1}   {cmd}{tag}", r.pid, r.cpu, r.mem),
                width,
            );
            // Agent rows wear their brand colour; the selected row inverts it.
            let color = r.agent.map(agent_color).unwrap_or(Color::White);
            let styled = if i == self.top_sel {
                Style::new().fg(Color::Black).bg(color).bold().render(&raw)
            } else if r.agent.is_some() {
                Style::new().fg(color).bold().render(&raw)
            } else {
                Style::new().fg(Color::White).render(&raw)
            };
            out.push(styled);
        }
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
                        .bg(Color::Red)
                        .bold()
                        .render(line);
                    *slot = format!("{}{styled}", " ".repeat(col0));
                }
            }
        }
        out.join("\n")
    }
}
