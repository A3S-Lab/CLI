//! `/agent` — local multi-turn development for a3s-code agent definitions.
//!
//! Bare `/agent` opens a picker over `agent_dir()` (`~/.a3s/agents` or the
//! `agent_dir` config key). Enter validates the selected Markdown/YAML agent
//! definition and puts the TUI into a local agent-development context. Subsequent
//! user turns are wrapped with the selected file path and editing constraints so
//! the current TUI session can iteratively improve the agent definition.
//!
//! `/agent <natural language>` asks the agent to draft a local Markdown agent
//! definition under `agent_dir()`.

use super::super::*;

#[derive(Clone)]
pub(crate) struct AgentFile {
    pub(crate) rel: String,
    pub(crate) path: std::path::PathBuf,
}

/// `/agent` selection panel: local agent definitions + cursor.
pub(crate) struct AgentPanel {
    /// Absolute path of the agents root (config `agent_dir`).
    pub(crate) root: std::path::PathBuf,
    /// Markdown/YAML files under the root, sorted by relative path.
    pub(crate) agents: Vec<AgentFile>,
    pub(crate) sel: usize,
}

/// The local agent currently being developed in the TUI.
#[derive(Clone)]
pub(crate) struct AgentDevSession {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) rel: String,
    pub(crate) path: std::path::PathBuf,
    pub(crate) root: std::path::PathBuf,
}

/// List local `*.md`, `*.yaml`, and `*.yml` agent definitions recursively,
/// skipping dotfiles and dot-directories. Sorted for a stable picker.
pub(crate) fn list_agents(root: &std::path::Path) -> Vec<AgentFile> {
    let mut out = Vec::new();
    list_agents_inner(root, root, &mut out);
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn list_agents_inner(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<AgentFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            list_agents_inner(root, &path, out);
            continue;
        }
        if !path.is_file() || !is_agent_definition_file(&path) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        out.push(AgentFile { rel, path });
    }
}

fn is_agent_definition_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("md" | "yaml" | "yml")
    )
}

fn parse_agent_definition(
    path: &std::path::Path,
    content: &str,
) -> Result<a3s_code_core::subagent::AgentDefinition, String> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("md") => a3s_code_core::subagent::parse_agent_md(content),
        Some("yaml" | "yml") => a3s_code_core::subagent::parse_agent_yaml(content),
        _ => Err(anyhow::anyhow!("unsupported agent file extension")),
    }
    .map_err(|e| e.to_string())
}

fn agent_picker_header(total: usize, root: &std::path::Path, width: usize) -> String {
    truncate(
        &format!(
            "  ◇ agent — pick a definition ({total} in {})",
            root.to_string_lossy()
        ),
        width,
    )
}

fn agent_picker_hint(width: usize) -> String {
    truncate("  ↑/↓ select · Enter develop locally · Esc cancel", width)
}

fn agent_picker_row(name: &str, width: usize) -> String {
    pad_to(&truncate(&format!("  {name}"), width), width)
}

/// Directive for `/agent <description>`: create a local Markdown agent definition.
pub(crate) fn agent_gen_prompt(description: &str, dir: &str) -> String {
    format!(
        "Create one a3s-code agent definition from the description below and save it under \
         {dir}. This is a SMALL single-file task: do it directly in this turn — do NOT \
         plan, delegate, or fan out subagents.\n\
         Description: {description}\n\
         IMPORTANT: {dir} is OUTSIDE this session's workspace, so the path-scoped file \
         tools will reject it — use the `bash` tool (`mkdir -p {dir}`, then write the \
         file with a heredoc).\n\
         The file MUST be Markdown with YAML frontmatter, because a3s-code can load \
         `.md`, `.yaml`, and `.yml` agents but Markdown gives VibeCoding a readable \
         prompt body. Use this exact shape:\n\
         ---\n\
         name: <kebab-case-agent-name>\n\
         description: <one-line trigger/purpose>\n\
         tools: Read, Grep, Glob, Bash\n\
         max_steps: 30\n\
         ---\n\
         <system prompt for the agent>\n\
         Rules: make `name` kebab-case and stable; keep `description` one line and \
         action-oriented; choose a conservative tools list (omit tools that are not \
         needed); write a practical system prompt with scope, workflow, and success \
         criteria; do not include secrets.\n\
         Save as {dir}/<kebab-case-agent-name>.md (if that file exists, append -2, \
         -3, …). Validate with `test -s \"$FILE\" && sed -n '1,40p' \"$FILE\"` \
         (always pass the file path — never run a command that waits on stdin). Then \
         report the saved path and tell the user `/agent` starts local interactive \
         development for it."
    )
}

fn agent_description(def: &a3s_code_core::subagent::AgentDefinition) -> String {
    let desc = def.description.trim();
    if desc.chars().count() >= 10 {
        desc.to_string()
    } else {
        format!("A3S Code agent definition for {}", def.name)
    }
}

pub(crate) fn agent_dev_prompt(session: &AgentDevSession, request: &str) -> String {
    format!(
        "You are in A3S Code local agent-development mode.\n\
         Current agent: {name}\n\
         Description: {description}\n\
         Definition file: {path}\n\
         Agents root: {root}\n\n\
         User request:\n{request}\n\n\
         Work on this local agent definition iteratively. Read the current file from disk before \
         editing; if normal file tools cannot access it because it is outside the workspace, use \
         non-interactive bash commands with the full quoted path. Keep the definition valid for \
         a3s-code: Markdown agents need YAML frontmatter followed by the system prompt; YAML/YML \
         agents must remain valid YAML. Preserve or improve the stable agent `name`, trigger \
         `description`, tools, model, max_steps, workflow guidance, and success criteria according \
         to the user's request. Do not open OS, WebIDE, RemoteUI, or browser pages for this local \
         agent-dev turn. Validate the file after edits by printing its first relevant lines and, \
         when practical, parsing or sanity-checking the frontmatter/YAML. End with a concise \
         summary of changes and any next suggested improvement.\n\n\
         The TUI remains in agent-development mode for `{name}` after this turn; the user can \
         press Esc or run `/agent off` to return to normal mode.",
        name = session.name.as_str(),
        description = session.description.as_str(),
        path = session.path.display(),
        root = session.root.display(),
    )
}

pub(crate) fn agent_goal_label(session: &AgentDevSession, goal: &str) -> String {
    format!(
        "Agent `{}` goal: {}",
        session.name,
        goal.trim().trim_end_matches('.')
    )
}

pub(crate) fn agent_loop_prompt(session: &AgentDevSession, loop_prompt: &str) -> String {
    format!(
        "You are running A3S Code loop engineering inside local /agent development mode.\n\
         Active agent: {name}\n\
         Description: {description}\n\
         Definition file: {path}\n\
         Agents root: {root}\n\n\
         Agent-loop rules:\n\
         - Keep this loop scoped to the active agent definition and its loop artifacts.\n\
         - Stay local: do not open OS, WebIDE, RemoteUI, browser pages, or OS workflow designer.\n\
         - Read the current agent definition before proposing or applying changes.\n\
         - Use maker/checker passes: one pass improves the definition, a separate pass verifies \
         frontmatter/YAML validity, tool scope, trigger description, workflow guidance, and \
         success criteria.\n\
         - Validate the file after edits when practical, then summarize report paths and changes.\n\n\
         {loop_prompt}",
        name = session.name.as_str(),
        description = session.description.as_str(),
        path = session.path.display(),
        root = session.root.display(),
    )
}

impl App {
    pub(crate) fn exit_agent_dev(&mut self) {
        match self.agent_dev.take() {
            Some(session) => self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  agent dev off — {} ({})",
                session.name, session.rel
            ))),
            None => self.push_line(&Style::new().fg(TN_GRAY).render("  agent dev is not active")),
        }
        self.relayout();
    }

    /// Open the `/agent` picker.
    pub(crate) fn open_agent_panel(&mut self) {
        let root = agent_dir();
        let agents = list_agents(&root);
        if agents.is_empty() {
            self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  no agents in {} — draft one with `/agent <description>` first",
                root.display()
            )));
            return;
        }
        self.agent_picker = Some(AgentPanel {
            root,
            agents,
            sel: 0,
        });
    }

    /// Keys while the `/agent` picker is open — consumes everything.
    pub(crate) fn handle_agent_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let p = self.agent_picker.as_mut()?;
        let last = p.agents.len().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => p.sel = p.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => p.sel = (p.sel + 1).min(last),
            KeyCode::Esc => self.agent_picker = None,
            KeyCode::Enter => {
                let panel = self.agent_picker.take()?;
                let picked = panel.agents.get(panel.sel.min(last))?.clone();
                let source =
                    match std::fs::read_to_string(&picked.path) {
                        Ok(s) => s,
                        Err(e) => {
                            self.push_line(&Style::new().fg(TN_RED).render(&format!(
                                "  could not read {}: {e}",
                                picked.path.display()
                            )));
                            return None;
                        }
                    };
                let def = match parse_agent_definition(&picked.path, &source) {
                    Ok(def) => def,
                    Err(e) => {
                        self.push_line(&Style::new().fg(TN_RED).render(&format!(
                            "  {} is not a valid agent definition — fix it (or redraft with /agent <description>): {e}",
                            picked.rel
                        )));
                        return None;
                    }
                };
                self.agent_dev = Some(AgentDevSession {
                    name: def.name.clone(),
                    description: agent_description(&def),
                    rel: picked.rel.clone(),
                    path: picked.path.clone(),
                    root: panel.root,
                });
                self.push_line(&gutter(
                    TN_CYAN,
                    &format!(
                        "◇ agent dev: {} ({}) · Esc or /agent off returns to normal mode",
                        def.name, picked.rel
                    ),
                ));
                self.relayout();
            }
            _ => {}
        }
        None
    }

    /// Overlay the `/agent` picker above the input.
    pub(crate) fn overlay_agent_menu(&self, composed: String) -> String {
        let Some(p) = self.agent_picker.as_ref() else {
            return composed;
        };
        let width = self.width as usize;
        let total = p.agents.len();
        let mut menu = vec![
            pad_to(
                &Style::new()
                    .fg(ACCENT)
                    .bold()
                    .render(&agent_picker_header(total, &p.root, width)),
                width,
            ),
            pad_to(
                &Style::new().fg(TN_GRAY).render(&agent_picker_hint(width)),
                width,
            ),
        ];
        let sel = p.sel.min(total.saturating_sub(1));
        let max_rows = (self.height as usize).saturating_sub(8).clamp(3, 12);
        let start = if sel < max_rows {
            0
        } else {
            sel + 1 - max_rows
        };
        let end = (start + max_rows).min(total);
        for (row, agent) in p.agents.iter().enumerate().take(end).skip(start) {
            let raw = agent_picker_row(&agent.rel, width);
            menu.push(if row == sel {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
            } else {
                Style::new().fg(TN_FG).render(&raw)
            });
        }
        if total > max_rows {
            menu.push(pad_to(
                &Style::new()
                    .fg(TN_GRAY)
                    .render(&format!("  {}/{total}", sel + 1)),
                width,
            ));
        }
        self.overlay_list(composed, &menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("a3s-{name}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn lists_agent_files_recursively_sorted_skipping_dotfiles() {
        let root = temp_root("agents");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::create_dir_all(root.join(".secret")).unwrap();
        std::fs::write(root.join("zeta.yaml"), "name: z\ndescription: z").unwrap();
        std::fs::write(root.join("alpha.md"), "---\nname: a\n---\nbody").unwrap();
        std::fs::write(root.join("nested/beta.yml"), "name: b\ndescription: b").unwrap();
        std::fs::write(root.join(".hidden.md"), "x").unwrap();
        std::fs::write(root.join(".secret/gamma.md"), "x").unwrap();
        std::fs::write(root.join("notes.txt"), "x").unwrap();

        let agents = list_agents(&root);
        let rels: Vec<_> = agents.into_iter().map(|a| a.rel).collect();
        assert_eq!(rels, vec!["alpha.md", "nested/beta.yml", "zeta.yaml"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_agent_definition_with_core_parser() {
        let root = temp_root("agent-parse");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("reviewer.md");
        let body = r#"---
name: reviewer
description: Review changes
---
Be precise.
"#;
        let def = parse_agent_definition(&path, body).unwrap();
        assert_eq!(def.name, "reviewer");
        assert_eq!(def.prompt.as_deref(), Some("Be precise."));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn agent_picker_rows_fit_fixed_width() {
        let root = std::path::PathBuf::from(
            "/Users/example/.a3s/agents/a/path/that/is/far/too/long/for/a/picker/header",
        );
        let header = agent_picker_header(9, &root, 40);
        let hint = agent_picker_hint(40);
        let row = agent_picker_row(
            "nested/very-long-agent-file-name-that-would-overflow-the-panel.md",
            40,
        );
        assert!(a3s_tui::style::visible_len(&header) <= 40, "{header}");
        assert!(a3s_tui::style::visible_len(&hint) <= 40, "{hint}");
        assert_eq!(a3s_tui::style::visible_len(&row), 40);
        assert!(row.contains('…'), "{row}");
    }

    #[test]
    fn agent_gen_prompt_carries_format_rules_and_dir() {
        let p = agent_gen_prompt("review rust diffs", "/Users/x/.a3s/agents");
        assert!(p.contains("review rust diffs"));
        assert!(p.contains("/Users/x/.a3s/agents"));
        assert!(p.contains("YAML frontmatter"));
        assert!(p.contains("name: <kebab-case-agent-name>"));
        assert!(p.contains("OUTSIDE this session's workspace") && p.contains("bash"));
        assert!(p.contains("never run a command that waits on stdin"));
    }

    #[test]
    fn agent_dev_prompt_keeps_work_local_and_names_exit_path() {
        let session = AgentDevSession {
            name: "code-reviewer".into(),
            description: "Review code changes carefully".into(),
            rel: "review/code-reviewer.md".into(),
            path: std::path::PathBuf::from("/Users/x/.a3s/agents/review/code-reviewer.md"),
            root: std::path::PathBuf::from("/Users/x/.a3s/agents"),
        };
        let p = agent_dev_prompt(&session, "add a security checklist");
        assert!(p.contains("code-reviewer"), "{p}");
        assert!(p.contains("add a security checklist"), "{p}");
        assert!(
            p.contains("/Users/x/.a3s/agents/review/code-reviewer.md"),
            "{p}"
        );
        assert!(p.contains("Do not open OS"), "{p}");
        assert!(p.contains("/agent off") && p.contains("Esc"), "{p}");
    }

    #[test]
    fn agent_goal_label_scopes_goal_to_active_agent() {
        let session = AgentDevSession {
            name: "code-reviewer".into(),
            description: "Review code changes carefully".into(),
            rel: "review/code-reviewer.md".into(),
            path: std::path::PathBuf::from("/Users/x/.a3s/agents/review/code-reviewer.md"),
            root: std::path::PathBuf::from("/Users/x/.a3s/agents"),
        };

        assert_eq!(
            agent_goal_label(&session, "tighten security checks."),
            "Agent `code-reviewer` goal: tighten security checks"
        );
    }

    #[test]
    fn agent_loop_prompt_keeps_engineered_loop_local_and_agent_scoped() {
        let session = AgentDevSession {
            name: "code-reviewer".into(),
            description: "Review code changes carefully".into(),
            rel: "review/code-reviewer.md".into(),
            path: std::path::PathBuf::from("/Users/x/.a3s/agents/review/code-reviewer.md"),
            root: std::path::PathBuf::from("/Users/x/.a3s/agents"),
        };
        let p = agent_loop_prompt(&session, "Run this A3S Code engineered loop.");

        assert!(p.contains("loop engineering inside local /agent"), "{p}");
        assert!(p.contains("code-reviewer"), "{p}");
        assert!(
            p.contains("/Users/x/.a3s/agents/review/code-reviewer.md"),
            "{p}"
        );
        assert!(p.contains("Stay local"), "{p}");
        assert!(p.contains("do not open OS, WebIDE, RemoteUI"), "{p}");
        assert!(p.contains("maker/checker"), "{p}");
        assert!(p.contains("Run this A3S Code engineered loop."), "{p}");
    }
}
