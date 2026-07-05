//! `@` file picker: an IDE-style collapsible tree (folders expand on demand).

use super::super::*;
use a3s_tui::components::{TreePicker, TreePickerItem};

fn file_menu_lines<F>(
    nodes: &[(String, usize, bool)],
    selected: usize,
    width: usize,
    height: usize,
    dir_is_open: F,
) -> Vec<String>
where
    F: Fn(&str) -> bool,
{
    let total = nodes.len();
    if total == 0 || width == 0 {
        return Vec::new();
    }

    let selected = selected.min(total - 1);
    let max_items = height.saturating_sub(9).clamp(4, 12);
    let scroll = selected.saturating_add(1).saturating_sub(max_items);
    let items = nodes
        .iter()
        .map(|(path, depth, is_dir)| {
            let name = path.rsplit('/').next().unwrap_or(path);
            if *is_dir {
                TreePickerItem::branch(format!("{name}/"))
                    .depth(*depth)
                    .open(dir_is_open(path))
                    .color(TN_CYAN)
            } else {
                TreePickerItem::leaf(name).depth(*depth).color(TN_FG)
            }
        })
        .collect::<Vec<_>>();

    TreePicker::new("@ file · ↑/↓ · →/← folder · Enter · Esc")
        .items(items)
        .selected(selected)
        .scroll(scroll)
        .max_items(max_items)
        .show_scroll(total > max_items)
        .indent(2)
        .depth_indent(2)
        .markers("▾", "▸", " ")
        .title_color(ACCENT)
        .branch_color(TN_CYAN)
        .leaf_color(TN_FG)
        .muted_color(TN_GRAY)
        .selected_colors(Color::BrightWhite, ACCENT)
        .view(width.min(u16::MAX as usize) as u16, max_items + 2)
        .lines()
        .map(str::to_string)
        .collect()
}

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
    /// capped at 2000 matches, switch to a cached tree if large workspaces lag.
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
        let menu = file_menu_lines(&nodes, sel, width, self.height as usize, |path| {
            self.dir_is_open(path)
        });
        self.overlay_list(composed, &menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn file_menu_lines_use_bounded_tree_picker_rows() {
        let nodes = vec![
            ("src".to_string(), 0, true),
            ("src/tui".to_string(), 1, true),
            ("src/tui/mod.rs".to_string(), 2, false),
            ("README.md".to_string(), 0, false),
        ];
        let open = HashSet::from(["src".to_string()]);
        let lines = file_menu_lines(&nodes, 2, 32, 20, |path| open.contains(path));
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(plain.contains("@ file"), "{plain}");
        assert!(plain.contains("▾ src/"), "{plain}");
        assert!(plain.contains("▸ tui/"), "{plain}");
        assert!(plain.contains("mod.rs"), "{plain}");
        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 32),
            "{plain}"
        );
        assert!(
            lines.iter().any(|line| line.contains("\x1b[")),
            "tree picker rows should carry styling"
        );
    }

    #[test]
    fn file_menu_lines_scrolls_selected_item_into_view() {
        let nodes = (0..16)
            .map(|idx| (format!("file-{idx}.rs"), 0, false))
            .collect::<Vec<_>>();
        let lines = file_menu_lines(&nodes, 14, 36, 16, |_| false);
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(plain.contains("file-14.rs"), "{plain}");
        assert!(plain.contains("↑↓ 15/16"), "{plain}");
    }
}
