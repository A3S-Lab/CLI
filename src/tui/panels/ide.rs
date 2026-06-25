//! `/ide` panel: file-tree navigation + in-place file editing/preview.

use super::super::*;

impl App {
    /// Handle a key while the `/ide` panel is open. Returns true if consumed.
    pub(crate) fn ide_key(&mut self, key: &KeyEvent) -> bool {
        if self.ide.is_none() {
            return false;
        }
        // Esc leaves the editor first (back to the tree), then closes the panel.
        if key.code == KeyCode::Esc {
            let editing = self.ide.as_ref().is_some_and(|i| i.focus_editor);
            if editing {
                if let Some(i) = self.ide.as_mut() {
                    i.focus_editor = false;
                }
            } else {
                self.ide = None;
            }
            return true;
        }
        let h = self.height as usize;
        let w = self.width as usize;
        let ide = self.ide.as_mut().unwrap();
        match key.code {
            // Editor focused: full text editing of the open file.
            _ if ide.focus_editor && ide.file.is_some() => {
                let body = h.saturating_sub(2);
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let f = ide.file.as_mut().unwrap();
                if f.image {
                    return true; // image preview is read-only
                }
                let nlines = f.lines.len();
                match key.code {
                    // Ctrl+S saves to disk.
                    KeyCode::Char('s') if ctrl => {
                        let content = format!("{}\n", f.lines.join("\n"));
                        if std::fs::write(&f.path, content).is_ok() {
                            f.dirty = false;
                        }
                    }
                    KeyCode::Up => f.row = f.row.saturating_sub(1),
                    KeyCode::Down => f.row = (f.row + 1).min(nlines.saturating_sub(1)),
                    KeyCode::Left => {
                        if f.col > 0 {
                            f.col -= 1;
                        } else if f.row > 0 {
                            f.row -= 1;
                            f.col = f.lines[f.row].chars().count();
                        }
                    }
                    KeyCode::Right => {
                        let len = f.lines.get(f.row).map_or(0, |l| l.chars().count());
                        if f.col < len {
                            f.col += 1;
                        } else if f.row + 1 < nlines {
                            f.row += 1;
                            f.col = 0;
                        }
                    }
                    KeyCode::Home => f.col = 0,
                    KeyCode::End => f.col = f.lines.get(f.row).map_or(0, |l| l.chars().count()),
                    KeyCode::PageUp => f.row = f.row.saturating_sub(body),
                    KeyCode::PageDown => f.row = (f.row + body).min(nlines.saturating_sub(1)),
                    KeyCode::Char(c) => {
                        let b = char_byte(&f.lines[f.row], f.col);
                        f.lines[f.row].insert(b, c);
                        f.col += 1;
                        f.dirty = true;
                    }
                    KeyCode::Tab => {
                        let b = char_byte(&f.lines[f.row], f.col);
                        f.lines[f.row].insert_str(b, "    ");
                        f.col += 4;
                        f.dirty = true;
                    }
                    KeyCode::Enter => {
                        let b = char_byte(&f.lines[f.row], f.col);
                        let right = f.lines[f.row].split_off(b);
                        f.lines.insert(f.row + 1, right);
                        f.row += 1;
                        f.col = 0;
                        f.dirty = true;
                    }
                    KeyCode::Backspace => {
                        if f.col > 0 {
                            let b0 = char_byte(&f.lines[f.row], f.col - 1);
                            let b1 = char_byte(&f.lines[f.row], f.col);
                            f.lines[f.row].replace_range(b0..b1, "");
                            f.col -= 1;
                            f.dirty = true;
                        } else if f.row > 0 {
                            let cur = f.lines.remove(f.row);
                            f.row -= 1;
                            f.col = f.lines[f.row].chars().count();
                            f.lines[f.row].push_str(&cur);
                            f.dirty = true;
                        }
                    }
                    KeyCode::Delete => {
                        let len = f.lines[f.row].chars().count();
                        if f.col < len {
                            let b0 = char_byte(&f.lines[f.row], f.col);
                            let b1 = char_byte(&f.lines[f.row], f.col + 1);
                            f.lines[f.row].replace_range(b0..b1, "");
                            f.dirty = true;
                        } else if f.row + 1 < nlines {
                            let next = f.lines.remove(f.row + 1);
                            f.lines[f.row].push_str(&next);
                            f.dirty = true;
                        }
                    }
                    _ => {}
                }
                // Clamp cursor column + scroll the cursor into view.
                let len = f.lines.get(f.row).map_or(0, |l| l.chars().count());
                f.col = f.col.min(len);
                if f.row < f.scroll {
                    f.scroll = f.row;
                } else if f.row >= f.scroll + body {
                    f.scroll = f.row + 1 - body;
                }
                return true;
            }
            // Tree focused: Tab enters the editor.
            KeyCode::Tab => ide.focus_editor = !ide.focus_editor,
            KeyCode::Up | KeyCode::Char('k') => ide.sel = ide.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                ide.sel = (ide.sel + 1).min(ide.entries.len().saturating_sub(1))
            }
            // ← jumps to the parent directory entry.
            KeyCode::Left => {
                if let Some(d) = ide.entries.get(ide.sel).map(|e| e.depth) {
                    if d > 0 {
                        let mut j = ide.sel;
                        while j > 0 && ide.entries[j].depth >= d {
                            j -= 1;
                        }
                        ide.sel = j;
                    }
                }
            }
            // Enter/→ toggles a directory or opens a file.
            KeyCode::Enter | KeyCode::Right if !ide.entries.is_empty() => {
                let sel = ide.sel.min(ide.entries.len() - 1);
                let (is_dir, expanded, depth, path) = {
                    let e = &ide.entries[sel];
                    (e.is_dir, e.expanded, e.depth, e.path.clone())
                };
                if is_dir && expanded {
                    ide.entries[sel].expanded = false;
                    let mut j = sel + 1;
                    while j < ide.entries.len() && ide.entries[j].depth > depth {
                        j += 1;
                    }
                    ide.entries.drain(sel + 1..j);
                } else if is_dir {
                    ide.entries[sel].expanded = true;
                    let at = sel + 1;
                    for (k, c) in ide_children(&path, depth + 1).into_iter().enumerate() {
                        ide.entries.insert(at + k, c);
                    }
                } else if is_image_path(&path) {
                    let tw = (w / 3).clamp(16, 38);
                    let lines =
                        render_image_file(&path, w.saturating_sub(tw + 4), h.saturating_sub(3))
                            .unwrap_or_else(|| vec!["<cannot decode image>".into()]);
                    ide.file = Some(IdeFile {
                        path,
                        lines,
                        scroll: 0,
                        row: 0,
                        col: 0,
                        dirty: false,
                        image: true,
                    });
                    ide.focus_editor = false; // read-only; keep tree focus
                } else {
                    let lines: Vec<String> = std::fs::read_to_string(&path)
                        .unwrap_or_else(|err| format!("<cannot read: {err}>"))
                        .replace('\t', "    ")
                        .lines()
                        .map(String::from)
                        .collect();
                    ide.file = Some(IdeFile {
                        path,
                        lines: if lines.is_empty() {
                            vec![String::new()]
                        } else {
                            lines
                        },
                        scroll: 0,
                        row: 0,
                        col: 0,
                        dirty: false,
                        image: false,
                    });
                    ide.focus_editor = true;
                }
            }
            _ => {}
        }
        // Keep the tree selection within the visible window.
        let body = h.saturating_sub(2);
        if ide.sel < ide.tree_scroll {
            ide.tree_scroll = ide.sel;
        } else if body > 0 && ide.sel >= ide.tree_scroll + body {
            ide.tree_scroll = ide.sel + 1 - body;
        }
        true
    }

    /// Full-screen `/ide`: file tree on the left, file viewer on the right.
    pub(crate) fn render_ide(&self, ide: &Ide) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let tw = (width / 3).clamp(16, 38);
        let body = h.saturating_sub(2);
        let fname = ide
            .file
            .as_ref()
            .map(|f| {
                let p = f
                    .path
                    .strip_prefix(&self.cwd)
                    .unwrap_or(&f.path)
                    .to_string_lossy()
                    .into_owned();
                if f.dirty {
                    format!("{p} ●")
                } else {
                    p
                }
            })
            .unwrap_or_else(|| "(no file)".into());
        let hint = if ide.focus_editor {
            "edit · Ctrl+S save · Esc back to tree"
        } else {
            "Tab edit · ↑↓ nav · Enter open · Esc close"
        };
        let mut out = vec![
            pad_to(
                &Style::new()
                    .fg(ACCENT)
                    .bold()
                    .render(&format!("  IDE — {fname}    {hint}")),
                width,
            ),
            pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render(&"─".repeat(width)),
                width,
            ),
        ];
        let sep = Style::new().fg(Color::BrightBlack).render(" │ ");
        for i in 0..body {
            let left = if let Some(e) = ide.entries.get(ide.tree_scroll + i) {
                let icon = if e.is_dir {
                    if e.expanded {
                        "▾"
                    } else {
                        "▸"
                    }
                } else {
                    "·"
                };
                let plain = pad_to(
                    &truncate(&format!(" {}{icon} {}", "  ".repeat(e.depth), e.name), tw),
                    tw,
                );
                if ide.tree_scroll + i == ide.sel && !ide.focus_editor {
                    Style::new().fg(Color::Black).bg(ACCENT).render(&plain)
                } else if e.is_dir {
                    Style::new().fg(ACCENT).render(&plain)
                } else {
                    Style::new().fg(Color::White).render(&plain)
                }
            } else {
                " ".repeat(tw)
            };
            let right = if let Some(f) = &ide.file {
                if f.image {
                    // Pre-rendered half-block rows; show raw, no line numbers.
                    f.lines.get(f.scroll + i).cloned().unwrap_or_default()
                } else if let Some(line) = f.lines.get(f.scroll + i) {
                    let lineno = f.scroll + i;
                    let num = Style::new()
                        .fg(if ide.focus_editor && lineno == f.row {
                            Color::Yellow
                        } else {
                            Color::BrightBlack
                        })
                        .render(&format!("{:>4} ", lineno + 1));
                    // Truncate the plain line first, then syntax-highlight it.
                    let plain = truncate(line, width.saturating_sub(tw + 8).max(8));
                    format!("{num}{}", highlight_code(&plain, lang_of(&f.path)))
                } else {
                    String::new()
                }
            } else if i == 0 {
                Style::new()
                    .fg(Color::BrightBlack)
                    .render("  ← pick a file to view")
            } else {
                String::new()
            };
            out.push(format!("{left}{sep}{right}"));
        }
        out.truncate(h);
        while out.len() < h {
            out.push(String::new());
        }
        out.join("\n")
    }
}
