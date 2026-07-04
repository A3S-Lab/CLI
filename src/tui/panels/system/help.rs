//! `/help` overlay: the full-screen usage guide.

use super::super::*;

const PARAMETER_HELP_ROWS: &[(&str, &str)] = &[
    (
        "/login <token>",
        "sign in with a copied OS access token instead of browser auth",
    ),
    ("/ctx <query>", "search past ctx-indexed agent sessions"),
    (
        "/ctx <n>",
        "attach search hit n as context to the next message",
    ),
    (
        "/ctx save <n>",
        "promote search hit n into long-term memory",
    ),
    ("/kb", "open the local personal knowledge base"),
    (
        "/kb add <text>",
        "save a text note into the local personal knowledge base",
    ),
    (
        "/kb import <path>",
        "preview and import a file or folder into the local knowledge base",
    ),
    (
        "/kb search <query>",
        "search the local personal knowledge base",
    ),
    ("/kb vault", "browse the local knowledge-base vault"),
    (
        "/okf <description>",
        "draft an OKF knowledge package prototype",
    ),
    (
        "/okf",
        "select an OKF package and enter local development mode",
    ),
    ("/okf off", "leave local OKF-package development mode"),
    ("/okf clone <git-url>", "clone an OKF package asset"),
    (
        "/okf review/publish/deploy",
        "review, publish, or deploy OKF packages through OS Knowledge service",
    ),
    (
        "/okf status",
        "check the OS knowledge asset and runtime binding",
    ),
    (
        "/okf list [query]",
        "list OS knowledge assets; filters before opening",
    ),
    (
        "/okf activity [query]",
        "inspect related Runtime indexing/evaluation activity",
    ),
    (
        "/loop init [name]",
        "create an engineered loop under .a3s/loops",
    ),
    (
        "/loop run <name>",
        "run an engineered loop with Runtime evidence when enabled",
    ),
    (
        "/loop audit <name>",
        "inspect loop readiness and missing files",
    ),
    ("/loop logs <name>", "open the loop run log"),
    ("/loop <task>", "quick autonomous loop with auto-continue"),
    ("/agent <description>", "draft a local agent definition"),
    (
        "/agent clone <git-url>",
        "clone an agent asset into agent_dir",
    ),
    ("/agent off", "leave local agent-development mode"),
    (
        "/agent review",
        "review the selected agent; opens selector first when needed",
    ),
    (
        "/agent list [query]",
        "list OS agent assets; filters before opening",
    ),
    (
        "/agent activity [query]",
        "inspect Runtime activity for the selected agent",
    ),
    (
        "/agent publish agentic",
        "publish and sync the active local agentic OS asset",
    ),
    (
        "/agent publish application",
        "publish and sync the active local application OS asset",
    ),
    (
        "/agent publish tool",
        "publish the active local tool agent through OS Function as a Service",
    ),
    (
        "/agent run",
        "start the active local agent through OS Agent as a Service",
    ),
    (
        "/agent deploy",
        "deploy the active local application agent through OS Agent as a Service",
    ),
    (
        "/agent open [agentic|application|tool]",
        "observe the OS agent asset without mutating it",
    ),
    (
        "/agent logs [agentic|application|tool]",
        "observe service log ViewLinks for the selected agent kind",
    ),
    (
        "/agent status [agentic|application|tool]",
        "check OS asset config/runtime binding without starting it",
    ),
    ("/mcp <description>", "draft a local MCP server asset"),
    ("/mcp clone <git-url>", "clone an MCP asset into mcp_dir"),
    ("/mcp off", "leave local MCP-development mode"),
    (
        "/mcp review",
        "review the selected MCP asset; opens selector first when needed",
    ),
    (
        "/mcp list [query]",
        "list OS MCP assets; filters before opening",
    ),
    (
        "/mcp activity [query]",
        "inspect Runtime activity for the selected MCP asset",
    ),
    (
        "/mcp publish",
        "publish the active MCP asset through OS Function as a Service",
    ),
    (
        "/mcp deploy",
        "sync the serving MCP runtime binding for OS Function as a Service",
    ),
    (
        "/mcp debug",
        "publish and invoke the active MCP asset through OS Function as a Service",
    ),
    (
        "/mcp test",
        "publish and batch-test MCP tools through OS Function as a Service",
    ),
    ("/mcp open", "observe the OS MCP asset without mutating it"),
    (
        "/mcp logs",
        "observe OS Function as a Service logs through ViewLinks",
    ),
    (
        "/mcp status",
        "check OS MCP asset and runtime binding without mutating it",
    ),
    (
        "/flow <description>",
        "draft a workflow DAG; /flow publishes it through OS Workflow as a Service",
    ),
    (
        "/flow clone <git-url>",
        "clone a workflow asset into flow_dir",
    ),
    (
        "/flow review [file]",
        "review a workflow DAG; select by file when needed",
    ),
    (
        "/flow list [query]",
        "list OS workflow assets; filters before opening",
    ),
    (
        "/flow activity [query]",
        "inspect Runtime activity for a workflow",
    ),
    (
        "/flow workflow",
        "view the latest dynamic workflow artifact read-only",
    ),
    (
        "/flow publish",
        "publish the selected workflow through OS Workflow as a Service",
    ),
    (
        "/flow run",
        "select and run a workflow through OS Workflow as a Service",
    ),
    (
        "/flow deploy",
        "publish and open the workflow deployment/run surface",
    ),
    (
        "/flow open",
        "open the existing OS workflow designer without mutating it",
    ),
    (
        "/flow logs",
        "open OS Workflow as a Service logs for the workflow",
    ),
    (
        "/flow status",
        "check OS workflow asset and runtime binding status",
    ),
    ("/skill <description>", "draft a local skill asset"),
    (
        "/skill clone <git-url>",
        "clone a skill asset into skill_dir",
    ),
    ("/skill off", "leave local skill-development mode"),
    (
        "/skill review",
        "review the selected skill; opens selector first when needed",
    ),
    (
        "/skill list [query]",
        "list OS skill assets; filters before opening",
    ),
    (
        "/skill activity [query]",
        "inspect related Function as a Service activity",
    ),
    (
        "/skill publish",
        "publish the selected skill through OS Function as a Service",
    ),
    (
        "/skill deploy",
        "publish the selected skill's serving Function as a Service binding",
    ),
    (
        "/skill open",
        "observe the OS skill asset without mutating it",
    ),
    (
        "/skill status",
        "check OS skill asset and runtime binding without mutating it",
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
        row(
            "? <query>",
            "deep research with Runtime-backed evidence when signed in",
        ),
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
        row("/git", "read-only status, diff, and recent log"),
        row(
            "/memory",
            "memory graph with entities, tiers, aliases, and forget candidates",
        ),
        row(
            "/model",
            "configured models plus signed-in Claude/Codex/OS gateway tabs",
        ),
        row(
            "/flow /agent /mcp",
            "team asset authoring with local dev and typed OS services",
        ),
        row(
            "/skill /okf /loop",
            "skill, OKF package, and engineered-loop authoring",
        ),
        String::new(),
        row("resume", "a3s code resume <id> after exit"),
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

    fn contains_cjk(s: &str) -> bool {
        s.chars().any(|ch| {
            ('\u{3400}'..='\u{4dbf}').contains(&ch)
                || ('\u{4e00}'..='\u{9fff}').contains(&ch)
                || ('\u{f900}'..='\u{faff}').contains(&ch)
        })
    }

    fn help_has_command_key(body: &str, key: &str) -> bool {
        body.lines().any(|line| {
            let line = line.trim_start();
            let Some(rest) = line.strip_prefix(key) else {
                return false;
            };
            rest.is_empty() || rest.starts_with(char::is_whitespace)
        })
    }

    fn has_word(haystack: &str, needle: &str) -> bool {
        haystack
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .any(|word| word == needle)
    }

    #[test]
    fn help_body_includes_every_registered_slash_command() {
        let body = a3s_tui::style::strip_ansi(&help_body_lines(120).join("\n"));
        for (cmd, _) in SLASH_COMMANDS {
            assert!(
                help_has_command_key(&body, cmd),
                "{cmd} should be explained in /help"
            );
        }
        assert!(body.contains("/ctx save <n>"));
        assert!(body.contains("/kb vault"));
        assert!(body.contains("/okf clone"));
        assert!(body.contains("/loop run <name>"));
        assert!(body.contains("/agent off"));
        assert!(body.contains("/agent clone"));
        assert!(body.contains("/skill clone"));
        assert!(body.contains("typed OS services"));
        assert!(!body.contains(&format!("OS-backed {}", "runs")));
        assert!(!body.contains("/kb open"));
        assert!(!body.contains("/kb dashboard"));
        assert!(!body.contains("/kb list"));
        assert!(!body.contains("/loop log <name>"));
        let unsupported_asset_forms = [
            ("agent", "debug"),
            ("mcp", "run"),
            ("flow", "debug"),
            ("skill", "run"),
            ("skill", "debug"),
            ("skill", "logs"),
            ("okf", "run"),
            ("okf", "debug"),
            ("okf", "logs"),
            ("okf", "open"),
            ("okf", "dashboard"),
            ("okf", "add"),
            ("okf", "import"),
            ("okf", "search"),
            ("okf", "vault"),
            ("agent", "view"),
            ("agent", "remote"),
            ("agent", "os"),
            ("agent", "dashboard"),
            ("mcp", "view"),
            ("mcp", "remote"),
            ("mcp", "os"),
            ("mcp", "dashboard"),
            ("flow", "view"),
            ("flow", "remote"),
            ("flow", "os"),
            ("flow", "dashboard"),
            ("skill", "view"),
            ("skill", "remote"),
            ("skill", "os"),
            ("skill", "dashboard"),
            ("agent", "ps"),
            ("mcp", "ps"),
            ("flow", "ps"),
            ("skill", "ps"),
            ("okf", "ps"),
        ]
        .into_iter()
        .map(|(family, action)| format!("/{family} {action}"));
        for form in unsupported_asset_forms {
            assert!(
                !help_has_command_key(&body, form.as_str()),
                "{form} should stay out of /help"
            );
        }
        let removed_commands = ["im", "run", "deploy", "review", "list", "ps", "workflow"]
            .into_iter()
            .map(|name| format!("/{name}"))
            .chain([
                format!("/{}{}", "evo", "lve"),
                format!("/{}{}", "evo", "love"),
                format!("/{}{}", "re", "po"),
            ]);
        for removed in removed_commands {
            assert!(
                !help_has_command_key(&body, removed.as_str()),
                "{removed} should stay out of /help"
            );
        }
        assert!(!help_has_command_key(&body, "/plugins"));
        assert!(!help_has_command_key(&body, "/quit"));
    }

    #[test]
    fn help_body_is_english_only_and_width_bounded() {
        let width = 64;
        let body = help_body_lines(width).join("\n");
        let plain = a3s_tui::style::strip_ansi(&body);

        assert!(
            !contains_cjk(&plain),
            "help text should stay English-only:\n{plain}"
        );
        for line in body.lines() {
            assert!(
                a3s_tui::style::visible_len(line) <= width,
                "help line should stay within width {width}: {:?}",
                a3s_tui::style::strip_ansi(line)
            );
        }
    }

    #[test]
    fn help_body_does_not_surface_repository_workspace_management() {
        let body = a3s_tui::style::strip_ansi(&help_body_lines(120).join("\n"));
        let plain = body.to_ascii_lowercase();

        assert!(
            !has_word(&plain, "repo"),
            "A3S Code should not expose source-workspace controls in /help:\n{body}"
        );
        assert!(
            !has_word(&plain, "repository"),
            "A3S Code should not expose source-workspace controls in /help:\n{body}"
        );
    }
}
