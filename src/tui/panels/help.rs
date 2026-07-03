//! `/help` overlay: the full-screen usage guide.

use super::super::*;

const PARAMETER_HELP_ROWS: &[(&str, &str)] = &[
    (
        "/login <token>",
        "sign in with a copied OS access token instead of browser auth",
    ),
    (
        "/list <query>",
        "open OS digital assets/apps filtered by query",
    ),
    ("/ps <query>", "open OS process services filtered by query"),
    ("/im <userId>", "open or create an OS direct message"),
    ("/ctx <query>", "search past ctx-indexed agent sessions"),
    (
        "/ctx <n>",
        "attach search hit n as context to the next message",
    ),
    (
        "/ctx save <n>",
        "promote search hit n into long-term memory",
    ),
    (
        "/kb add <text>",
        "save a text note into the project knowledge base",
    ),
    (
        "/kb import <path>",
        "preview and import a file or folder into the knowledge base",
    ),
    ("/kb search <query>", "search the project knowledge base"),
    ("/kb open", "browse the knowledge-base vault"),
    (
        "/loop init [name]",
        "create an engineered loop under .a3s/loops",
    ),
    ("/loop run <name>", "run a persisted engineered loop"),
    (
        "/loop audit <name>",
        "inspect loop readiness and missing files",
    ),
    ("/loop logs <name>", "open the loop run log"),
    ("/loop <task>", "quick autonomous loop with auto-continue"),
    ("/agent <description>", "draft a local agent definition"),
    ("/agent off", "leave local agent-development mode"),
    (
        "/flow <description>",
        "draft a flow DAG locally; /flow opens it in OS",
    ),
    (
        "/btw <question>",
        "ask a background side-question outside the main chat",
    ),
];

fn help_row(width: usize, key: &str, desc: &str) -> String {
    let key_width = 20.min(width.saturating_sub(6));
    let desc_width = width.saturating_sub(4 + key_width + 2);
    let key = pad_to(&truncate(key, key_width), key_width);
    let desc = truncate(desc, desc_width);
    format!(
        "    {}  {}",
        Style::new().fg(TN_FG).bold().render(&key),
        Style::new().fg(TN_GRAY).render(&desc)
    )
}

fn help_body_lines(width: usize) -> Vec<String> {
    let head = |s: &str| Style::new().fg(ACCENT).bold().render(s);
    let row = |k: &str, d: &str| help_row(width, k, d);
    let mut lines: Vec<String> = vec![head("  Slash commands")];
    lines.extend(SLASH_COMMANDS.iter().map(|(k, d)| row(k, d)));
    lines.extend([String::new(), head("  Command forms")]);
    lines.extend(PARAMETER_HELP_ROWS.iter().map(|(k, d)| row(k, d)));
    lines.extend([
        String::new(),
        head("  Input modes"),
        row("! <cmd>", "run a shell command directly"),
        row("? <query>", "deep research with goal + auto-continue"),
        row("& <git-url>", "clone and run a read-only code review"),
        row("@<path>", "attach a workspace file from the file picker"),
        row("Ctrl+V", "attach a clipboard image to the next message"),
        String::new(),
        head("  Keys"),
        row("Enter", "send; while busy, the message is queued"),
        row("Shift+Enter", "insert a newline in the input"),
        row("Shift+Tab", "cycle run mode: default -> plan -> auto"),
        row(
            "Up / Down",
            "recall input history; inside menus, move selection",
        ),
        row("PgUp / PgDn", "scroll the transcript or this help panel"),
        row("Shift+End", "jump to the latest transcript output"),
        row("drag", "select transcript text; auto-copies on release"),
        row(
            "Esc",
            "interrupt the running turn; close panels where applicable",
        ),
        row("Ctrl+C x2", "quit"),
        String::new(),
        head("  Panels"),
        row(
            "/ide /config /kb",
            "full-screen file editors and knowledge-base browser",
        ),
        row(
            "/git",
            "status, diff, stage, unstage, commit, and recent log",
        ),
        row(
            "/memory",
            "long-term memory timeline; c opens a ctx-sourced session",
        ),
        row(
            "/model",
            "configured models plus signed-in Claude/Codex/OS gateway tabs",
        ),
        row(
            "/flow /agent /loop",
            "local authoring panels with optional OS-backed runs",
        ),
        String::new(),
        Style::new()
            .fg(TN_GRAY)
            .render("  Resume a past session:  a3s code resume <id>  (printed on exit)"),
    ]);
    lines
}

impl App {
    fn help_max_scroll(&self) -> usize {
        let body_h = (self.height as usize).saturating_sub(2);
        help_body_lines(self.width as usize)
            .len()
            .saturating_sub(body_h)
    }

    pub(crate) fn scroll_help_by(&mut self, delta: isize) {
        let max_scroll = self.help_max_scroll();
        if delta < 0 {
            self.help_scroll = self.help_scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.help_scroll = self
                .help_scroll
                .saturating_add(delta as usize)
                .min(max_scroll);
        }
    }

    /// Full-screen `/help` panel: a detailed usage guide.
    pub(crate) fn render_help(&self) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let body = help_body_lines(width);
        let body_h = h.saturating_sub(2);
        let max_scroll = body.len().saturating_sub(body_h);
        let scroll = self.help_scroll.min(max_scroll);
        let mut lines: Vec<String> = Vec::with_capacity(h);
        lines.push(
            Style::new()
                .fg(ACCENT)
                .bold()
                .render("  A3S CODE help   Esc/Enter close · Up/Down/PgUp/PgDn scroll"),
        );
        lines.extend(body.iter().skip(scroll).take(body_h).cloned());
        if h > 1 {
            let showing_to = (scroll + body_h).min(body.len());
            let footer = if max_scroll > 0 {
                format!(
                    "  showing {}-{} of {}",
                    scroll.saturating_add(1).min(body.len()),
                    showing_to,
                    body.len()
                )
            } else {
                "  all help entries visible".to_string()
            };
            lines.push(Style::new().fg(TN_GRAY).render(&footer));
        }
        for l in &mut lines {
            *l = pad_to(&truncate(l, width), width);
        }
        lines.truncate(h);
        while lines.len() < h {
            lines.push(String::new());
        }
        lines.join("\n")
    }

    pub(crate) fn handle_help_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let body_h = (self.height as usize).saturating_sub(2);
        let max_scroll = self.help_max_scroll();
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q' | 'Q') => {
                self.help_open = false;
                self.help_scroll = 0;
            }
            KeyCode::Up => self.scroll_help_by(-1),
            KeyCode::Down => self.scroll_help_by(1),
            KeyCode::PageUp => self.scroll_help_by(-(body_h.max(1) as isize)),
            KeyCode::PageDown => self.scroll_help_by(body_h.max(1) as isize),
            KeyCode::Home => self.help_scroll = 0,
            KeyCode::End => self.help_scroll = max_scroll,
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_body_includes_every_registered_slash_command() {
        let body = a3s_tui::style::strip_ansi(&help_body_lines(120).join("\n"));
        for (cmd, _) in SLASH_COMMANDS {
            assert!(body.contains(cmd), "{cmd} should be explained in /help");
        }
        assert!(body.contains("/ctx save <n>"));
        assert!(body.contains("/loop run <name>"));
        assert!(body.contains("/agent off"));
    }
}
