//! `@` file picker: query parsing, candidates, key handling, overlay.

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

    /// Workspace files matching the current `@` query (substring match).
    pub(crate) fn file_candidates(&self) -> Vec<String> {
        let Some(q) = self.at_query() else {
            return Vec::new();
        };
        let q = q.to_lowercase();
        // Sorted so same-directory files group together for the tree view; the
        // overlay scrolls a window, so we can keep plenty for browsing.
        let mut v: Vec<String> = self
            .files
            .iter()
            .filter(|f| q.is_empty() || f.to_lowercase().contains(&q))
            .take(400)
            .cloned()
            .collect();
        v.sort();
        v
    }

    pub(crate) fn file_menu_open(&self) -> bool {
        self.state != State::Awaiting
            && !self.textarea.value().contains('\n')
            && self.at_query().is_some()
            && !self.file_candidates().is_empty()
    }

    /// Keys while the `@` file picker is open: ↑/↓ select, Enter/Tab insert,
    /// Esc dismiss (drops the trailing `@query`).
    pub(crate) fn handle_file_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let cands = self.file_candidates();
        if cands.is_empty() {
            return None;
        }
        let last = cands.len() - 1;
        self.file_sel = self.file_sel.min(last);
        match key.code {
            KeyCode::Up => {
                self.file_sel = self.file_sel.saturating_sub(1);
                Some(None)
            }
            KeyCode::Down => {
                self.file_sel = (self.file_sel + 1).min(last);
                Some(None)
            }
            KeyCode::Enter | KeyCode::Tab => {
                let val = self.textarea.value();
                if let Some(at) = val.rfind('@') {
                    let picked = &cands[self.file_sel];
                    self.textarea
                        .set_value(&format!("{}@{picked} ", &val[..at]));
                }
                self.file_sel = 0;
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

    /// Overlay the `@` file picker just above the input box.
    pub(crate) fn overlay_file_menu(&self, composed: String) -> String {
        if !self.file_menu_open() {
            return composed;
        }
        let cands = self.file_candidates();
        let total = cands.len();
        if total == 0 {
            return composed;
        }
        let sel = self.file_sel.min(total - 1);
        let width = self.width as usize;
        // Build a real multi-level tree: emit each ancestor directory once,
        // indented by depth, then the file under it. `disp` rows carry the
        // candidate index for files (None for directory headers).
        let mut disp: Vec<(String, Option<usize>)> = Vec::new();
        let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut sel_line = 0usize;
        for (ci, cand) in cands.iter().enumerate() {
            let parts: Vec<&str> = cand.split('/').collect();
            for d in 0..parts.len().saturating_sub(1) {
                let dir_path = parts[..=d].join("/");
                if emitted.insert(dir_path) {
                    let indent = "  ".repeat(d);
                    disp.push((
                        pad_to(
                            &format!(
                                "  {indent}{}",
                                Style::new()
                                    .fg(Color::Cyan)
                                    .render(&format!("{}/", parts[d]))
                            ),
                            width,
                        ),
                        None,
                    ));
                }
            }
            let depth = parts.len().saturating_sub(1);
            let base = parts.last().copied().unwrap_or(cand.as_str());
            if ci == sel {
                sel_line = disp.len();
            }
            disp.push((format!("  {}{base}", "  ".repeat(depth)), Some(ci)));
        }

        let max_rows = (self.height as usize).saturating_sub(9).clamp(4, 12);
        let nlines = disp.len();
        let start = if sel_line < max_rows {
            0
        } else {
            sel_line + 1 - max_rows
        };
        let end = (start + max_rows).min(nlines);
        let mut menu = vec![pad_to(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render("  @ file · ↑/↓ · Enter insert · Esc"),
            width,
        )];
        for (line, ci) in disp.iter().take(end).skip(start) {
            if *ci == Some(sel) {
                menu.push(Style::new().fg(Color::BrightWhite).bg(ACCENT).render(line));
            } else if ci.is_some() {
                menu.push(pad_to(&Style::new().fg(Color::White).render(line), width));
            } else {
                menu.push(line.clone()); // directory header (pre-styled + padded)
            }
        }
        if nlines > max_rows {
            let up = if start > 0 { "↑" } else { " " };
            let down = if end < nlines { "↓" } else { " " };
            menu.push(pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render(&format!("  {up}{down} {}/{total}", sel + 1)),
                width,
            ));
        }
        self.overlay_list(composed, &menu)
    }
}
