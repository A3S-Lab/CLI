//! `/help` overlay: the full-screen usage guide.

use super::super::*;
use a3s_tui::components::{HelpPanel, HelpSection};

const PARAMETER_HELP_ROWS: &[(&str, &str)] = &[
    (
        "/login <token>",
        "sign in with a copied OS access token instead of browser auth",
    ),
    (
        "/use [status]",
        "inspect the live Use registry, MCP connections, Skills, and providers",
    ),
    (
        "/use repair",
        "show explicit repair commands without installing or changing anything",
    ),
    (
        "/use plugin",
        "enable/disable Claude skills & plugins (alias of /plugin)",
    ),
    (
        "/use packages",
        "review enable/disable plans for Use packages (alias of /packages)",
    ),
    ("/use reload", "re-scan skills/plugins (alias of /reload)"),
    (
        "/copy [transcript]",
        "copy the latest response or the complete semantic session Markdown",
    ),
    (
        "/export [path]",
        "atomically create a Markdown session file inside the workspace",
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
    (
        "/ctx memory",
        "open the memory graph (preferred over /memory)",
    ),
    (
        "/ctx kb […]",
        "personal knowledge base hub (preferred over /kb)",
    ),
    (
        "/ctx sleep [focus]",
        "consolidate today's work into memory (preferred over /sleep)",
    ),
    (
        "/ctx evolution",
        "review learned preferences and skills (preferred over /evolution)",
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
];

fn parameter_help_group(command: &str) -> &'static str {
    if command.starts_with("/ctx") || command.starts_with("/kb") {
        "Context & knowledge forms"
    } else if command.starts_with("/loop") {
        "Workflow forms"
    } else {
        "Session & sharing forms"
    }
}

fn help_panel() -> HelpPanel {
    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    let mut panel = chrome
        .help_panel_without_title()
        .section(
            HelpSection::new("Start here")
                .row(
                    "/",
                    "browse commands; keep typing to search names and descriptions",
                )
                .row("/status", "inspect this session without changing it")
                .row("/help", "open this complete grouped reference")
                .row("! <cmd>", "run a shell command directly")
                .row(
                    "/research <query>",
                    "start deep research (primary) · status|explain|replay|diff for diagnostics",
                )
                .row(
                    "? <query>",
                    "shortcut for /research · prefer the slash form · `--local-only` for offline evidence",
                )
                .row(
                    "@<path>",
                    "reference a workspace file; image files become visual attachments",
                )
                .row("$<skill>", "mention Skill · Alt/Option+Enter sticks until Esc or /unstick")
                .row("/unstick", "clear sticky skill mode")
                .row(
                    "Ctrl+V",
                    "attach image · chips above the prompt · submitted turns show half-block previews (click to enlarge)",
                ),
        )
        .section(
            HelpSection::new("Keys")
                .row(
                    "Enter",
                    "send; while busy, append the message to the FIFO queue",
                )
                .row(
                    "Ctrl+O",
                    "Send now: cancel the active turn and promote this prompt",
                )
                .row(
                    "Ctrl+J",
                    "insert a newline (reliable when the terminal remaps Shift+Enter)",
                )
                .row("Shift+Enter", "insert a newline when the terminal reports Shift")
                .row(
                    "Shift+Tab",
                    "cycle agent → plan → reviewer → auto → yolo (/ask+/plan → plan; /auto /yolo /reviewer aliases)",
                )
                .row(
                    "Up / Down",
                    "recall session history (↑ from first multiline row); inside menus, move selection",
                )
                .row(
                    "Paste",
                    "small text inserts inline; large dumps become pills (click expands, × removes)",
                )
                .row("PgUp / PgDn", "scroll the transcript or this help panel")
                .row("Shift+End", "jump to the latest transcript output")
                .row(
                    "Ctrl+T",
                    "expand full tool output and diffs (complete transcript)",
                )
                .row("Ctrl+R", "fuzzy-search prompts from the current session")
                .row(
                    "Ctrl+G",
                    "review DiffView for the latest turn's file edits (←→ files, i instruct)",
                )
                .row(
                    "Ctrl+B",
                    "inspect delegated tasks, recent output, and safe cancellation",
                )
                .row(
                    "wheel / drag",
                    "scroll; select transcript text and copy on release",
                )
                .row(
                    "Esc",
                    "interrupt the running turn; close panels; clear sticky skill on empty composer",
                )
                .row("Ctrl+C x2", "quit"),
        );

    for group in SlashCommandGroup::ALL {
        let mut section = HelpSection::new(group.label());
        for (command, description) in SLASH_COMMANDS
            .iter()
            .filter(|(command, _)| slash_command_group(command) == group)
        {
            section.add_row(*command, *description);
        }
        panel = panel.section(section);
    }

    for title in [
        "Session & sharing forms",
        "Context & knowledge forms",
        "Workflow forms",
    ] {
        let mut section = HelpSection::new(title);
        for (command, description) in PARAMETER_HELP_ROWS
            .iter()
            .filter(|(command, _)| parameter_help_group(command) == title)
        {
            section.add_row(*command, *description);
        }
        panel = panel.section(section);
    }

    panel
        .section(
            HelpSection::new("Panels")
                .row(
                    "/ide /config /kb",
                    "full-screen file editors and knowledge-base browser",
                )
                .row(
                    "editor keys",
                    "Cmd/Ctrl+V paste text, Ctrl+S save, Ctrl+Z undo, Esc normal/tree",
                )
                .row(
                    "/memory",
                    "memory graph with entities, tiers, aliases, and forget candidates",
                )
                .row(
                    "/permissions",
                    "M cycles the next-turn mode; exact grants remain searchable and revocable",
                )
                .row(
                    "/sandbox",
                    "inspect verified host Bash sandbox readiness and fail-closed status",
                )
                .row(
                    "/display",
                    "cycle status-meter density between default, compact, and zen",
                )
                .row(
                    "/model",
                    "configured models plus Claude/Codex/Kimi/WorkBuddy/OS gateway tabs",
                )
                .row(
                    "/loop",
                    "engineered-loop dashboard and scheduled local runs",
                )
                .row(
                    "/use",
                    "integrations hub for plugins, packages, and Use registry status",
                ),
        )
        .section(HelpSection::new("Resume").row("resume", "a3s code resume <id> after exit"))
        .key_width(20)
        .indent(4)
        .gap(2)
        .section_color(ACCENT)
        .key_color(TN_FG)
        .description_color(TN_GRAY)
        .footer_color(TN_GRAY)
}

fn help_body_lines(width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    help_panel()
        .view(width.min(u16::MAX as usize) as u16, usize::MAX)
        .lines()
        .map(str::to_string)
        .collect()
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
        lines.push(format!(
            "{}{}",
            Style::new().fg(ACCENT).bold().render("  A3S Code"),
            Style::new()
                .fg(TN_GRAY)
                .render(" help · Esc/Enter close · Up/Down/PgUp/PgDn scroll")
        ));
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
        assert!(body.contains("/loop"));
        assert!(body.contains("/use"));
        assert!(body.contains("/research <query>"));
        assert!(body.contains("shortcut for /research"));
        assert!(body.contains("--local-only"));
        assert!(body.contains("$<skill>"));
        assert!(body.contains("Ctrl+T"));
        assert!(body.contains("expand full tool output and diffs"));
        assert!(!body.contains("uses runtime when signed in"));
        assert!(!body.contains(&format!("OS-backed {}", "runs")));
        assert!(!body.contains("/kb open"));
        assert!(!body.contains("/kb dashboard"));
        assert!(!body.contains("/kb list"));
        assert!(!body.contains("/loop log <name>"));
        let removed_commands = [
            "im", "run", "deploy", "list", "ps", "workflow", "agent", "flow", "mcp", "skill", "okf",
        ]
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
    fn help_body_starts_with_orientation_and_keeps_shortcuts_unique() {
        let plain = a3s_tui::style::strip_ansi(&help_body_lines(120).join("\n"));
        let start = plain.find("Start here").expect("quick-start section");
        let keys = plain.find("Keys").expect("key section");
        let workflow = plain.find("Workflow").expect("workflow command group");
        assert!(start < keys && keys < workflow, "{plain}");

        for heading in [
            "Workflow",
            "Session & control",
            "Context & memory",
            "Assets & services",
            "System & interface",
            "Session & sharing forms",
            "Context & knowledge forms",
            "Workflow forms",
        ] {
            assert!(plain.contains(heading), "missing help group {heading}");
        }
        assert_eq!(plain.matches("Ctrl+B").count(), 1, "{plain}");
        assert_eq!(plain.matches("Ctrl+R").count(), 1, "{plain}");
        assert_eq!(plain.matches("Ctrl+G").count(), 1, "{plain}");
        assert!(
            plain.contains("agent → plan → reviewer → auto → yolo"),
            "Shift+Tab help must list the full mode ring including reviewer: {plain}"
        );
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
