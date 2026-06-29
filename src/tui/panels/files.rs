//! `@` file picker: an IDE-style collapsible tree (folders expand on demand).

use super::super::*;

impl App {
    /// The `@<query>` after the last `@` in the input (no whitespace), if any.
    pub(crate) fn at_query(&self) -> Option<String> {
        let val = self.textarea.value();
        let at = val.rfind('@')?;
        let after = &val[at + 1..];
        if after.contains(char::is_whitespace) {
            None
        } else {
            Some(after.to_string())
        }
    }

    /// Visible tree nodes `(path, depth, is_dir)`: directories are collapsed
    /// unless in `at_expanded`; a non-empty query auto-opens dirs holding a
    /// match so it behaves like a filter. ponytail: rescans `files` per call —
    /// capped at 2000 matches, switch to a cached tree if big repos lag.
    pub(crate) fn at_nodes(&self) -> Vec<(String, usize, bool)> {
        let q = self.at_query().unwrap_or_default().to_lowercase();
        let matches: Vec<&String> = self
            .files
            .iter()
            .filter(|f| q.is_empty() || f.to_lowercase().contains(&q))
            .take(2000)
            .collect();
        let is_open = |dir: &str| -> bool {
            self.at_expanded.contains(dir)
                || (!q.is_empty()
                    && matches
                        .iter()
                        .any(|f| f.starts_with(dir) && f.as_bytes().get(dir.len()) == Some(&b'/')))
        };
        let mut nodes: Vec<(String, usize, bool)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for f in &matches {
            let parts: Vec<&str> = f.split('/').collect();
            let mut visible = true;
            for d in 0..parts.len() - 1 {
                let dir = parts[..=d].join("/");
                let parent_open = d == 0 || is_open(&parts[..d].join("/"));
                if parent_open && seen.insert(dir.clone()) {
                    nodes.push((dir.clone(), d, true));
                }
                if !is_open(&dir) {
                    visible = false;
                    break;
                }
            }
            if visible {
                nodes.push(((*f).clone(), parts.len() - 1, false));
            }
        }
        nodes.sort_by(|a, b| a.0.cmp(&b.0)); // path order = tree order
        nodes
    }

    fn dir_is_open(&self, dir: &str) -> bool {
        self.at_expanded.contains(dir) || self.at_query().is_some_and(|q| !q.is_empty())
    }

    pub(crate) fn file_menu_open(&self) -> bool {
        self.state != State::Awaiting
            && !self.textarea.value().contains('\n')
            && self.at_query().is_some()
            && !self.files.is_empty()
    }

    /// Keys while the picker is open: ↑/↓ move, →/← expand/collapse a folder,
    /// Enter toggles a folder or inserts a file, Esc dismisses.
    pub(crate) fn handle_file_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let nodes = self.at_nodes();
        if nodes.is_empty() {
            return None;
        }
        let last = nodes.len() - 1;
        self.file_sel = self.file_sel.min(last);
        let (path, _, is_dir) = nodes[self.file_sel].clone();
        match key.code {
            KeyCode::Up => {
                self.file_sel = self.file_sel.saturating_sub(1);
                Some(None)
            }
            KeyCode::Down => {
                self.file_sel = (self.file_sel + 1).min(last);
                Some(None)
            }
            KeyCode::Right if is_dir => {
                self.at_expanded.insert(path);
                Some(None)
            }
            KeyCode::Left if is_dir => {
                self.at_expanded.remove(&path);
                Some(None)
            }
            KeyCode::Enter | KeyCode::Tab => {
                if is_dir {
                    // Toggle the folder open/closed instead of inserting.
                    if !self.at_expanded.remove(&path) {
                        self.at_expanded.insert(path);
                    }
                } else if let Some(at) = self.textarea.value().rfind('@') {
                    let val = self.textarea.value();
                    self.touch_workspace_file(&path);
                    self.textarea.set_value(&format!("{}@{path} ", &val[..at]));
                    self.file_sel = 0;
                }
                Some(None)
            }
            KeyCode::Esc => {
                let val = self.textarea.value();
                if let Some(at) = val.rfind('@') {
                    self.textarea.set_value(&val[..at]);
                }
                self.file_sel = 0;
                Some(None)
            }
            _ => None,
        }
    }

    /// Overlay the `@` file tree just above the input box.
    pub(crate) fn overlay_file_menu(&self, composed: String) -> String {
        if !self.file_menu_open() {
            return composed;
        }
        let nodes = self.at_nodes();
        let total = nodes.len();
        if total == 0 {
            return composed;
        }
        let sel = self.file_sel.min(total - 1);
        let width = self.width as usize;
        let max_rows = (self.height as usize).saturating_sub(9).clamp(4, 12);
        let start = if sel < max_rows {
            0
        } else {
            sel + 1 - max_rows
        };
        let end = (start + max_rows).min(total);

        let mut menu = vec![pad_to(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render("  @ file · ↑/↓ · →/← folder · Enter · Esc"),
            width,
        )];
        for (i, (path, depth, is_dir)) in nodes.iter().enumerate().take(end).skip(start) {
            let name = path.rsplit('/').next().unwrap_or(path);
            let indent = "  ".repeat(*depth);
            let raw = if *is_dir {
                let arrow = if self.dir_is_open(path) { "▾" } else { "▸" };
                pad_to(&format!("  {indent}{arrow} {name}/"), width)
            } else {
                pad_to(&format!("  {indent}  {name}"), width)
            };
            menu.push(if i == sel {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
            } else if *is_dir {
                Style::new().fg(TN_CYAN).render(&raw)
            } else {
                Style::new().fg(TN_FG).render(&raw)
            });
        }
        if total > max_rows {
            let up = if start > 0 { "↑" } else { " " };
            let down = if end < total { "↓" } else { " " };
            menu.push(pad_to(
                &Style::new()
                    .fg(TN_GRAY)
                    .render(&format!("  {up}{down} {}/{total}", sel + 1)),
                width,
            ));
        }
        self.overlay_list(composed, &menu)
    }
}
