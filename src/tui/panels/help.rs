//! `/help` overlay: the full-screen usage guide.

use super::super::*;

impl App {
    /// Full-screen `/help` panel: a detailed usage guide.
    pub(crate) fn render_help(&self) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let head = |s: &str| Style::new().fg(ACCENT).bold().render(s);
        let row = |k: &str, d: &str| {
            format!(
                "    {}  {}",
                Style::new().fg(TN_FG).bold().render(&format!("{k:<16}")),
                Style::new().fg(TN_GRAY).render(d)
            )
        };
        let mut lines: Vec<String> = vec![
            head("  A3S CODE — help   (Esc to close)"),
            String::new(),
            head("  Slash commands"),
            row("/model", "pick the model"),
            row("/config", "open config.acl in your editor"),
            row("/ide", "file tree + code viewer"),
            row("/git", "status / diff / stage / commit"),
            row(
                "/memory",
                "browse long-term memory (GitLens-style timeline)",
            ),
            row("/top", "live process monitor (Enter to force-kill)"),
            row(
                "/relay",
                "continue a session from a3s-code / Claude / Codex",
            ),
            row("/btw <q>", "ask a background side-question (yellow panel)"),
            row("/help", "this panel"),
            row("/clear", "clear the conversation"),
            row("/auto", "auto-approve tools for this session"),
            row("/exit", "quit"),
            String::new(),
            head("  Input modes"),
            row("! <cmd>", "run a shell command (pink) · Esc leaves"),
            row(
                "? <query>",
                "deep research — goal-driven, auto-continues (cyan)",
            ),
            row("/btw <q>", "side-channel question, kept out of the chat"),
            String::new(),
            head("  Keys"),
            row("Enter", "send · while busy, the message is queued"),
            row("Shift+Tab", "cycle run mode: default → plan → auto"),
            row("↑ / ↓", "recall input history"),
            row(
                "wheel / PgUp / PgDn",
                "scroll the transcript (scrollbar on the right)",
            ),
            row("Shift+End", "jump to the latest output"),
            row("drag", "select transcript text — auto-copies on release"),
            row("Esc", "interrupt the running turn"),
            row("Ctrl+C ×2", "quit"),
            String::new(),
            head("  Run modes"),
            row("default", "asks before file-modifying tools"),
            row("plan", "pinned TODO plan, tracks each step ▶/✔/✗"),
            row("auto", "auto-approves tools"),
            String::new(),
            Style::new()
                .fg(TN_GRAY)
                .render("  Resume a past session:  a3s code resume <id>  (printed on exit)"),
        ];
        for l in &mut lines {
            *l = pad_to(l, width);
        }
        lines.truncate(h);
        while lines.len() < h {
            lines.push(String::new());
        }
        lines.join("\n")
    }
}
