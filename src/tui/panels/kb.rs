//! `/kb` knowledge-base dashboard.
//!
//! The dashboard is the default landing page for `/kb`: it shows what is in the
//! project vault, previews imports before copying files, and makes the explicit
//! note/import/search flows discoverable without dropping the user into a file
//! browser immediately.

use super::super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KbCommand {
    Dashboard,
    Open,
    Add(String),
    Import(String),
    Search(String),
    Legacy(String),
    Usage(&'static str),
}

pub(crate) struct KbSearch {
    pub(crate) query: String,
    pub(crate) hits: Vec<kbutil::SearchHit>,
}

pub(crate) struct KbPanel {
    pub(crate) stats: kbutil::KbStats,
    pub(crate) recent: Vec<String>,
    pub(crate) pending_import: Option<kbutil::ImportPreview>,
    pub(crate) search: Option<KbSearch>,
    pub(crate) note: Option<String>,
    pub(crate) scroll: usize,
}

pub(crate) fn parse_kb_command(rest: &str) -> KbCommand {
    let arg = rest.trim();
    if arg.is_empty() {
        return KbCommand::Dashboard;
    }
    let (head, tail) = arg
        .split_once(char::is_whitespace)
        .map(|(h, t)| (h, t.trim()))
        .unwrap_or((arg, ""));
    match head {
        "open" => {
            if tail.is_empty() {
                KbCommand::Open
            } else {
                KbCommand::Usage("usage: /kb open")
            }
        }
        "add" => {
            if tail.is_empty() {
                KbCommand::Usage("usage: /kb add <text>")
            } else {
                KbCommand::Add(tail.to_string())
            }
        }
        "import" => {
            if tail.is_empty() {
                KbCommand::Usage("usage: /kb import <file|folder>")
            } else {
                KbCommand::Import(tail.to_string())
            }
        }
        "search" => {
            if tail.is_empty() {
                KbCommand::Usage("usage: /kb search <query>")
            } else {
                KbCommand::Search(tail.to_string())
            }
        }
        "help" => KbCommand::Dashboard,
        _ => KbCommand::Legacy(arg.to_string()),
    }
}

fn import_kind_label(kind: kbutil::ImportKind) -> &'static str {
    match kind {
        kbutil::ImportKind::File => "file",
        kbutil::ImportKind::Folder => "folder",
    }
}

fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn kb_usage_hint() -> &'static str {
    "  /kb add <text> · /kb import <file|folder> · /kb search <query> · o browse · c compile · Esc close"
}

fn looks_path_like(s: &str) -> bool {
    s.contains('/')
        || s.contains('\\')
        || s.starts_with('.')
        || s.starts_with('~')
        || s.ends_with(".md")
        || s.ends_with(".txt")
        || s.ends_with(".json")
        || s.ends_with(".yaml")
        || s.ends_with(".yml")
}

impl App {
    pub(crate) fn handle_kb_command(&mut self, rest: &str) -> Option<Cmd<Msg>> {
        self.textarea.clear();
        match parse_kb_command(rest) {
            KbCommand::Dashboard => {
                self.open_kb_dashboard(None);
                None
            }
            KbCommand::Open => {
                self.open_kb_browser();
                None
            }
            KbCommand::Add(text) => {
                let cwd = self.cwd.clone();
                let now = chrono::Utc::now().to_rfc3339();
                Some(cmd::cmd(move || async move {
                    let summary = tokio::task::spawn_blocking(move || {
                        kbutil::add_text_to_kb(&cwd, &text, &now)
                    })
                    .await
                    .unwrap_or_else(|e| format!("✗ /kb add failed: {e}"));
                    Msg::KbAdded(summary)
                }))
            }
            KbCommand::Import(arg) => {
                self.prepare_kb_import(arg, None);
                None
            }
            KbCommand::Search(query) => {
                let hits = kbutil::search_kb(&self.cwd, &query);
                let count = hits.len();
                self.open_kb_dashboard(Some(format!(
                    "search `{}` · {count} hit(s)",
                    truncate(&query, 48)
                )));
                if let Some(kb) = self.kb.as_mut() {
                    kb.search = Some(KbSearch { query, hits });
                }
                None
            }
            KbCommand::Legacy(arg) => {
                let legacy_note = if looks_path_like(&arg) {
                    Some("previewing path; use `/kb import <path>` next time".to_string())
                } else {
                    Some("use `/kb add <text>` to capture a text note".to_string())
                };
                if kbutil::preview_import(&self.cwd, &arg).is_ok() {
                    self.prepare_kb_import(arg, legacy_note);
                } else {
                    self.open_kb_dashboard(legacy_note);
                }
                None
            }
            KbCommand::Usage(usage) => {
                self.open_kb_dashboard(Some(usage.to_string()));
                None
            }
        }
    }

    pub(crate) fn open_kb_dashboard(&mut self, note: Option<String>) {
        self.kb = Some(KbPanel {
            stats: kbutil::kb_stats(&self.cwd),
            recent: kbutil::recent_sources(&self.cwd, 8),
            pending_import: None,
            search: None,
            note,
            scroll: 0,
        });
    }

    pub(crate) fn open_kb_browser(&mut self) {
        let root = kbutil::kb_dir(&self.cwd);
        if !root.is_dir() {
            self.open_kb_dashboard(Some(
                "KB is empty · add sources with `/kb add` or `/kb import`".to_string(),
            ));
            return;
        }
        let mut ide = Ide::browse(ide_children(&root, 0), "knowledge base");
        ide.kb_root = Some(root);
        self.kb = None;
        self.ide = Some(ide);
    }

    fn prepare_kb_import(&mut self, arg: String, note: Option<String>) {
        match kbutil::preview_import(&self.cwd, &arg) {
            Ok(preview) if preview.addable == 0 => {
                self.open_kb_dashboard(Some(format!(
                    "nothing importable in {} · KB stores text files only",
                    truncate(&arg, 48)
                )));
            }
            Ok(preview) => {
                self.open_kb_dashboard(note.or_else(|| {
                    Some("import preview ready · Enter confirms, Esc cancels".to_string())
                }));
                if let Some(kb) = self.kb.as_mut() {
                    kb.pending_import = Some(preview);
                }
            }
            Err(e) => self.open_kb_dashboard(Some(format!("✗ /kb import: {e}"))),
        }
    }

    fn confirm_kb_import(&mut self) -> Option<Cmd<Msg>> {
        let preview = self.kb.as_mut()?.pending_import.take()?;
        let arg = preview.arg;
        if let Some(kb) = self.kb.as_mut() {
            kb.note = Some(format!("importing {}…", truncate(&arg, 48)));
        }
        let cwd = self.cwd.clone();
        let now = chrono::Utc::now().to_rfc3339();
        Some(cmd::cmd(move || async move {
            let summary =
                tokio::task::spawn_blocking(move || kbutil::import_to_kb(&cwd, &arg, &now))
                    .await
                    .unwrap_or_else(|e| format!("✗ /kb import failed: {e}"));
            Msg::KbAdded(summary)
        }))
    }

    pub(crate) fn handle_kb_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        match key.code {
            KeyCode::Esc => {
                if self
                    .kb
                    .as_ref()
                    .and_then(|kb| kb.pending_import.as_ref())
                    .is_some()
                {
                    if let Some(kb) = self.kb.as_mut() {
                        kb.pending_import = None;
                        kb.note = Some("import cancelled".to_string());
                    }
                } else {
                    self.kb = None;
                }
                None
            }
            KeyCode::Enter => self.confirm_kb_import(),
            KeyCode::Char('o') => {
                self.open_kb_browser();
                None
            }
            KeyCode::Char('r') => {
                self.open_kb_dashboard(Some("refreshed".to_string()));
                None
            }
            KeyCode::Char('a') => {
                self.kb = None;
                self.textarea.set_value("/kb add ");
                None
            }
            KeyCode::Char('i') => {
                self.kb = None;
                self.textarea.set_value("/kb import ");
                None
            }
            KeyCode::Char('s') => {
                self.kb = None;
                self.textarea.set_value("/kb search ");
                None
            }
            KeyCode::Char('c') => {
                self.kb = None;
                self.on_submit("Use your `okf` skill.".to_string())
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(kb) = self.kb.as_mut() {
                    kb.scroll = kb.scroll.saturating_sub(1);
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(kb) = self.kb.as_mut() {
                    kb.scroll += 1;
                }
                None
            }
            KeyCode::PageUp => {
                if let Some(kb) = self.kb.as_mut() {
                    kb.scroll = kb.scroll.saturating_sub(10);
                }
                None
            }
            KeyCode::PageDown => {
                if let Some(kb) = self.kb.as_mut() {
                    kb.scroll += 10;
                }
                None
            }
            KeyCode::Char('g') => {
                if let Some(kb) = self.kb.as_mut() {
                    kb.scroll = 0;
                }
                None
            }
            _ => None,
        }
    }

    pub(crate) fn render_kb(&self, kb: &KbPanel) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let mut out = vec![
            pad_to(
                &Style::new().fg(ACCENT).bold().render(&format!(
                    "  /kb — knowledge base · {} source(s) · {} concept(s) · {} import(s) · {}",
                    kb.stats.sources,
                    kb.stats.concepts,
                    kb.stats.imports,
                    fmt_bytes(kb.stats.bytes)
                )),
                width,
            ),
            pad_to(&Style::new().fg(TN_GRAY).render(&"─".repeat(width)), width),
        ];

        if let Some(note) = &kb.note {
            let color = if note.starts_with('✗') {
                TN_RED
            } else if note.contains("cancel") || note.contains("usage") {
                TN_YELLOW
            } else {
                TN_CYAN
            };
            out.push(pad_to(
                &Style::new().fg(color).render(&format!("  {note}")),
                width,
            ));
            out.push(String::new());
        }

        if let Some(preview) = &kb.pending_import {
            self.render_kb_import_preview(preview, &mut out, width);
            out.push(String::new());
        }

        if let Some(search) = &kb.search {
            self.render_kb_search(search, kb.scroll, &mut out, width, h);
        } else {
            self.render_kb_recent(kb, &mut out, width);
        }

        while out.len() + 4 < h {
            out.push(String::new());
        }
        out.push(pad_to(
            &Style::new().fg(TN_GRAY).render(kb_usage_hint()),
            width,
        ));
        out.push(pad_to(
            &Style::new()
                .fg(TN_GRAY)
                .render("  a add · i import · s search · r refresh"),
            width,
        ));
        out.truncate(h);
        while out.len() < h {
            out.push(String::new());
        }
        out.join("\n")
    }

    fn render_kb_import_preview(
        &self,
        preview: &kbutil::ImportPreview,
        out: &mut Vec<String>,
        width: usize,
    ) {
        out.push(pad_to(
            &Style::new().fg(TN_YELLOW).bold().render("  Import preview"),
            width,
        ));
        out.push(pad_to(
            &format!(
                "  {} · {}",
                Style::new()
                    .fg(TN_FG)
                    .render(import_kind_label(preview.kind)),
                Style::new().fg(TN_GRAY).render(&truncate(
                    &preview.path.display().to_string(),
                    width.saturating_sub(12)
                ))
            ),
            width,
        ));
        let mut meta = format!(
            "{} text file(s) · {} · {} skipped",
            preview.addable,
            fmt_bytes(preview.bytes),
            preview.skipped
        );
        if preview.capped {
            meta.push_str(" · capped");
        }
        out.push(pad_to(
            &Style::new().fg(TN_GRAY).render(&format!("  {meta}")),
            width,
        ));
        out.push(pad_to(
            &Style::new()
                .fg(TN_CYAN)
                .render("  Enter confirm import · Esc cancel"),
            width,
        ));
    }

    fn render_kb_search(
        &self,
        search: &KbSearch,
        scroll: usize,
        out: &mut Vec<String>,
        width: usize,
        height: usize,
    ) {
        out.push(pad_to(
            &Style::new().fg(TN_CYAN).bold().render(&format!(
                "  Search `{}` · {} hit(s)",
                truncate(&search.query, 40),
                search.hits.len()
            )),
            width,
        ));
        if search.hits.is_empty() {
            out.push(pad_to(
                &Style::new()
                    .fg(TN_GRAY)
                    .render("  no matches yet — try a shorter term or import more sources"),
                width,
            ));
            return;
        }
        let room = height.saturating_sub(out.len() + 4).max(1);
        let start = scroll.min(search.hits.len().saturating_sub(1));
        for (idx, hit) in search.hits.iter().enumerate().skip(start).take(room) {
            let path_budget = (width / 2).clamp(20, 56);
            let path = truncate(&format!("{}:{}", hit.path, hit.line), path_budget);
            let snippet_budget = width.saturating_sub(path_budget + 9);
            let snippet = truncate(&hit.snippet, snippet_budget);
            out.push(pad_to(
                &format!(
                    "  {:>2}. {}  {}",
                    idx + 1,
                    Style::new().fg(TN_FG).render(&path),
                    Style::new().fg(TN_GRAY).render(&snippet)
                ),
                width,
            ));
        }
        if search.hits.len() > room {
            out.push(pad_to(
                &Style::new().fg(TN_GRAY).render(&format!(
                    "  {}/{} · ↑↓/jk/PgUp/PgDn scroll · g top",
                    start + 1,
                    search.hits.len()
                )),
                width,
            ));
        }
    }

    fn render_kb_recent(&self, kb: &KbPanel, out: &mut Vec<String>, width: usize) {
        out.push(pad_to(
            &Style::new().fg(TN_FG).bold().render("  Recent sources"),
            width,
        ));
        if kb.recent.is_empty() {
            out.push(pad_to(
                &Style::new().fg(TN_GRAY).render(
                    "  no sources yet — start with `/kb add <text>` or `/kb import <file|folder>`",
                ),
                width,
            ));
        } else {
            for source in &kb.recent {
                out.push(pad_to(
                    &format!(
                        "  {} {}",
                        Style::new().fg(TN_CYAN).render("•"),
                        Style::new()
                            .fg(TN_FG)
                            .render(&truncate(source, width.saturating_sub(5)))
                    ),
                    width,
                ));
            }
        }

        out.push(String::new());
        out.push(pad_to(
            &Style::new().fg(TN_FG).bold().render("  Workflow"),
            width,
        ));
        for line in [
            "  add captures a note as Markdown",
            "  import previews text files before copying",
            "  search scans sources and generated concept pages",
            "  compile runs the OKF skill to turn sources into linked concepts",
        ] {
            out.push(pad_to(&Style::new().fg(TN_GRAY).render(line), width));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_explicit_kb_subcommands() {
        assert_eq!(parse_kb_command(""), KbCommand::Dashboard);
        assert_eq!(parse_kb_command(" open "), KbCommand::Open);
        assert_eq!(
            parse_kb_command(" add hello world "),
            KbCommand::Add("hello world".into())
        );
        assert_eq!(
            parse_kb_command(" import docs "),
            KbCommand::Import("docs".into())
        );
        assert_eq!(
            parse_kb_command(" search runtime "),
            KbCommand::Search("runtime".into())
        );
        assert!(matches!(parse_kb_command("add"), KbCommand::Usage(_)));
    }

    #[test]
    fn legacy_args_are_not_parsed_as_adds() {
        assert_eq!(
            parse_kb_command("some pasted note"),
            KbCommand::Legacy("some pasted note".into())
        );
    }

    #[test]
    fn byte_format_is_compact() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert_eq!(fmt_bytes(2048), "2.0 KiB");
        assert_eq!(fmt_bytes(2_097_152), "2.0 MiB");
    }
}
