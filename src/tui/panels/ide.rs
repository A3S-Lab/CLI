//! `/ide` panel: file-tree navigation + in-place file editing/preview.

use super::super::*;

impl App {
    /// Handle a key while the `/ide` panel is open. Returns true if consumed.
    pub(crate) fn ide_key(&mut self, key: &KeyEvent) -> bool {
        if self.ide.is_none() {
            return false;
        }
        // Esc: in Insert mode (or with a pending operator) it returns to a clean
        // Normal mode and stays in the editor; in Normal it leaves the editor back
        // to the tree; from the tree it closes the panel.
        if key.code == KeyCode::Esc {
            if let Some(f) = self
                .ide
                .as_mut()
                .filter(|i| i.focus_editor)
                .and_then(|i| i.file.as_mut())
            {
                if f.mode == EditMode::Insert || f.pending.is_some() {
                    f.mode = EditMode::Normal;
                    f.pending = None;
                    f.clamp_col();
                    return true;
                }
            }
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
        let workspace_manifest = self.workspace_manifest.clone();
        let workspace = self.cwd.clone();
        let ide = self.ide.as_mut().unwrap();
        match key.code {
            // Editor focused: full text editing of the open file.
            _ if ide.focus_editor && ide.file.is_some() => {
                let body = h.saturating_sub(2);
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                // Ctrl+S saves to disk with explicit footer feedback (success or
                // the OS error) — handled before the long-lived &mut file borrow.
                if ctrl && matches!(key.code, KeyCode::Char('s' | 'S')) {
                    let msg = {
                        let f = ide.file.as_mut().unwrap();
                        if f.image || f.readonly {
                            "(read-only)".to_string()
                        } else {
                            let content = format!("{}\n", f.lines.join("\n"));
                            match std::fs::write(&f.path, content) {
                                Ok(()) => {
                                    f.dirty = false;
                                    touch_workspace_file_path_for_manifest(
                                        &workspace_manifest,
                                        &workspace,
                                        &f.path,
                                    );
                                    "✔ saved".to_string()
                                }
                                Err(e) => format!("✗ save failed: {e}"),
                            }
                        }
                    };
                    ide.flash = Some(msg);
                    return true;
                }
                ide.flash = None; // any edit/nav key dismisses the save flash
                                  // Editor content width = right pane minus the line-number gutter;
                                  // must match `render_ide` so the cursor's horizontal scroll tracks.
                let tw = (w / 3).clamp(16, 38);
                let content_w = w.saturating_sub(tw + 8).max(8);
                let f = ide.file.as_mut().unwrap();
                if f.image {
                    return true; // image preview is read-only
                }
                // Vim-aligned editing/navigation lives on the buffer itself.
                f.edit_key(key, body, content_w);
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
                    touch_workspace_file_path_for_manifest(&workspace_manifest, &workspace, &path);
                    ide.file = Some(IdeFile::new(path, lines, true, true));
                    ide.focus_editor = false; // read-only; keep tree focus
                } else {
                    let lines: Vec<String> = std::fs::read_to_string(&path)
                        .unwrap_or_else(|err| format!("<cannot read: {err}>"))
                        .replace('\t', "    ")
                        .lines()
                        .map(String::from)
                        .collect();
                    touch_workspace_file_path_for_manifest(&workspace_manifest, &workspace, &path);
                    ide.file = Some(IdeFile::new(path, lines, false, false));
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
        let readonly = ide.file.as_ref().is_some_and(|f| f.readonly);
        let mode = ide.file.as_ref().map(|f| f.mode);
        let hint: String = if let Some(flash) = ide.flash.as_deref() {
            flash.to_string()
        } else if readonly {
            "read-only · NORMAL · hjkl/↑↓ move · gg/G top/bottom · Esc back".to_string()
        } else if ide.focus_editor {
            match mode {
                Some(EditMode::Insert) => "-- INSERT -- · Esc normal · Ctrl+S save".to_string(),
                _ => "-- NORMAL -- · i insert · dd/dw/x cut · u undo · Ctrl+S save · Esc tree"
                    .to_string(),
            }
        } else {
            "Tab edit · ↑↓ nav · Enter open · Esc close".to_string()
        };
        let mut out = vec![
            pad_to(
                &Style::new()
                    .fg(ACCENT)
                    .bold()
                    .render(&format!("  IDE — {fname}    {hint}")),
                width,
            ),
            pad_to(&Style::new().fg(TN_GRAY).render(&"─".repeat(width)), width),
        ];
        let sep = Style::new().fg(TN_GRAY).render(" │ ");
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
                    Style::new().fg(TN_FG).render(&plain)
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
                    let cur_row = ide.focus_editor && lineno == f.row;
                    let num = Style::new()
                        .fg(if cur_row { TN_YELLOW } else { TN_GRAY })
                        .render(&format!("{:>4} ", lineno + 1));
                    let cw = width.saturating_sub(tw + 8).max(8);
                    // Horizontal scroll: render only the window [hscroll, hscroll+cw)
                    // of the line so long lines scroll sideways instead of being
                    // truncated off-screen.
                    let window = slice_cols(line, f.hscroll, f.hscroll + cw);
                    let body_str = if cur_row {
                        // Active line renders plain + a block cursor so the column
                        // (and the horizontal scroll position) stays visible.
                        render_cursor_line(&window, f.display_col().saturating_sub(f.hscroll))
                    } else {
                        highlight_code(&window, lang_of(&f.path))
                    };
                    format!("{num}{body_str}")
                } else {
                    String::new()
                }
            } else if i == 0 {
                Style::new().fg(TN_GRAY).render("  ← pick a file to view")
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

/// Vim-aligned editing for an open `IdeFile`. Normal mode navigates and operates;
/// Insert mode types. Arrows / Home / End / PgUp / PgDn work in both modes, and
/// Insert mode also honours the readline shortcuts traditional editors bind
/// (Ctrl+A/E/K/U/W). Read-only buffers allow navigation but block every edit.
impl IdeFile {
    /// Char count of the current line.
    fn cur_len(&self) -> usize {
        self.lines.get(self.row).map_or(0, |l| l.chars().count())
    }

    /// Clamp the cursor column into the current line. Normal mode rests *on* the
    /// last char (col ≤ len-1); Insert may sit *after* it (col ≤ len).
    fn clamp_col(&mut self) {
        let len = self.cur_len();
        let max = if self.mode == EditMode::Insert {
            len
        } else {
            len.saturating_sub(1)
        };
        self.col = self.col.min(max);
    }

    /// Column of the first non-blank char on the current line (vim `^`).
    fn first_nonblank(&self) -> usize {
        self.lines.get(self.row).map_or(0, |l| {
            l.chars().position(|c| !c.is_whitespace()).unwrap_or(0)
        })
    }

    /// vim word classes: 0 = whitespace, 1 = word char, 2 = punctuation.
    fn char_class(c: char) -> u8 {
        if c.is_whitespace() {
            0
        } else if c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
    }

    fn row_chars(&self) -> Vec<char> {
        self.lines
            .get(self.row)
            .map_or(Vec::new(), |l| l.chars().collect())
    }

    /// Start of the next word on this line (vim `w`, within-line).
    fn next_word(&self) -> usize {
        let line = self.row_chars();
        let n = line.len();
        let mut c = self.col;
        if c >= n {
            return n;
        }
        let cls = Self::char_class(line[c]);
        if cls != 0 {
            while c < n && Self::char_class(line[c]) == cls {
                c += 1;
            }
        }
        while c < n && Self::char_class(line[c]) == 0 {
            c += 1;
        }
        c
    }

    /// Start of the previous word on this line (vim `b`, within-line).
    fn prev_word(&self) -> usize {
        let line = self.row_chars();
        let mut c = self.col;
        if c == 0 {
            return 0;
        }
        c -= 1;
        while c > 0 && Self::char_class(line[c]) == 0 {
            c -= 1;
        }
        let cls = Self::char_class(line[c]);
        while c > 0 && Self::char_class(line[c - 1]) == cls {
            c -= 1;
        }
        c
    }

    /// End of the current/next word on this line (vim `e`, within-line).
    fn word_end(&self) -> usize {
        let line = self.row_chars();
        let n = line.len();
        if n == 0 {
            return 0;
        }
        let mut c = self.col;
        if c + 1 >= n {
            return n - 1;
        }
        c += 1;
        while c < n && Self::char_class(line[c]) == 0 {
            c += 1;
        }
        if c >= n {
            return n - 1;
        }
        let cls = Self::char_class(line[c]);
        while c + 1 < n && Self::char_class(line[c + 1]) == cls {
            c += 1;
        }
        c
    }

    /// Leading whitespace of the current line (for `o`/`O` auto-indent).
    fn leading_ws(&self) -> String {
        self.lines.get(self.row).map_or(String::new(), |l| {
            l.chars().take_while(|c| c.is_whitespace()).collect()
        })
    }

    /// Snapshot the buffer + cursor for `u`.
    // ponytail: whole-buffer snapshots, bounded — fine for config-sized files
    fn snapshot(&mut self) {
        if self.undo.len() >= 200 {
            self.undo.remove(0);
        }
        self.undo.push((self.lines.clone(), self.row, self.col));
    }

    fn undo(&mut self) {
        if let Some((lines, row, col)) = self.undo.pop() {
            self.lines = lines;
            self.row = row.min(self.lines.len().saturating_sub(1));
            self.col = col;
            self.dirty = true;
            self.clamp_col();
        }
    }

    /// Handle one key in the focused editor. Ctrl+S (save) is handled by the
    /// caller before this; Esc (Insert→Normal / leave) likewise.
    pub(crate) fn edit_key(&mut self, key: &KeyEvent, body: usize, content_w: usize) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let nlines = self.lines.len();
        // A pending Normal-mode operator (d/c/g/y) consumes the next key — even an
        // arrow — instead of the shared navigation below.
        let pending = self.mode == EditMode::Normal && self.pending.is_some();
        let mut handled = false;
        if !pending {
            handled = true;
            match key.code {
                KeyCode::Up => self.row = self.row.saturating_sub(1),
                KeyCode::Down => self.row = (self.row + 1).min(nlines.saturating_sub(1)),
                KeyCode::Left => self.move_left(),
                KeyCode::Right => self.move_right(),
                KeyCode::Home => self.col = 0,
                KeyCode::End => self.col = self.cur_len(),
                KeyCode::PageUp => self.row = self.row.saturating_sub(body),
                KeyCode::PageDown => self.row = (self.row + body).min(nlines.saturating_sub(1)),
                _ => handled = false,
            }
        }
        if !handled {
            match self.mode {
                EditMode::Insert => self.insert_key(key, ctrl),
                EditMode::Normal => self.normal_key(key),
            }
        }
        // Clamp the cursor + scroll it into view, vertically AND horizontally.
        self.clamp_col();
        if self.row < self.scroll {
            self.scroll = self.row;
        } else if body > 0 && self.row >= self.scroll + body {
            self.scroll = self.row + 1 - body;
        }
        // Horizontal: keep the cursor's display column within the content width,
        // so long lines scroll sideways instead of being truncated off-screen.
        let cur_x = self.display_col();
        if cur_x < self.hscroll {
            self.hscroll = cur_x;
        } else if content_w > 0 && cur_x >= self.hscroll + content_w {
            self.hscroll = cur_x + 1 - content_w;
        }
    }

    /// The cursor's display column (sum of glyph widths before `col`), so
    /// horizontal scrolling and the block cursor land correctly with wide (CJK)
    /// chars, not just ASCII.
    fn display_col(&self) -> usize {
        self.lines.get(self.row).map_or(0, |l| {
            l.chars()
                .take(self.col)
                .map(|c| a3s_tui::style::visible_len(&c.to_string()))
                .sum()
        })
    }

    fn move_left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.cur_len();
        }
    }

    fn move_right(&mut self) {
        let len = self.cur_len();
        if self.col < len {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    // ── Insert mode ──────────────────────────────────────────────────────────
    fn insert_key(&mut self, key: &KeyEvent, ctrl: bool) {
        if self.readonly {
            return;
        }
        if ctrl {
            // readline-style shortcuts traditional editors bind in insert mode.
            match key.code {
                KeyCode::Char('a' | 'A') => self.col = 0,
                KeyCode::Char('e' | 'E') => self.col = self.cur_len(),
                KeyCode::Char('k' | 'K') => {
                    self.snapshot();
                    self.kill_to_eol();
                }
                KeyCode::Char('u' | 'U') => {
                    self.snapshot();
                    self.kill_to_bol();
                }
                KeyCode::Char('w' | 'W') => {
                    self.snapshot();
                    self.delete_word_back();
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char(c) => self.insert_char(c),
            KeyCode::Tab => self.insert_str("    "),
            KeyCode::Enter => self.split_line(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete_forward(),
            _ => {}
        }
    }

    fn insert_char(&mut self, c: char) {
        let b = char_byte(&self.lines[self.row], self.col);
        self.lines[self.row].insert(b, c);
        self.col += 1;
        self.dirty = true;
    }
    fn insert_str(&mut self, s: &str) {
        let b = char_byte(&self.lines[self.row], self.col);
        self.lines[self.row].insert_str(b, s);
        self.col += s.chars().count();
        self.dirty = true;
    }
    fn split_line(&mut self) {
        let b = char_byte(&self.lines[self.row], self.col);
        let right = self.lines[self.row].split_off(b);
        self.lines.insert(self.row + 1, right);
        self.row += 1;
        self.col = 0;
        self.dirty = true;
    }
    fn backspace(&mut self) {
        if self.col > 0 {
            let b0 = char_byte(&self.lines[self.row], self.col - 1);
            let b1 = char_byte(&self.lines[self.row], self.col);
            self.lines[self.row].replace_range(b0..b1, "");
            self.col -= 1;
            self.dirty = true;
        } else if self.row > 0 {
            let cur = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.cur_len();
            self.lines[self.row].push_str(&cur);
            self.dirty = true;
        }
    }
    fn delete_forward(&mut self) {
        let len = self.cur_len();
        if self.col < len {
            let b0 = char_byte(&self.lines[self.row], self.col);
            let b1 = char_byte(&self.lines[self.row], self.col + 1);
            self.lines[self.row].replace_range(b0..b1, "");
            self.dirty = true;
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
            self.dirty = true;
        }
    }
    fn kill_to_eol(&mut self) {
        let b = char_byte(&self.lines[self.row], self.col);
        if !self.lines[self.row].split_off(b).is_empty() {
            self.dirty = true;
        }
    }
    fn kill_to_bol(&mut self) {
        let b = char_byte(&self.lines[self.row], self.col);
        self.lines[self.row].replace_range(0..b, "");
        self.col = 0;
        self.dirty = true;
    }
    fn delete_word_back(&mut self) {
        let start = self.prev_word();
        let from = if start < self.col {
            start
        } else {
            self.col.saturating_sub(1)
        };
        if from < self.col {
            let b0 = char_byte(&self.lines[self.row], from);
            let b1 = char_byte(&self.lines[self.row], self.col);
            self.lines[self.row].replace_range(b0..b1, "");
            self.col = from;
            self.dirty = true;
        }
    }

    // ── Normal mode ──────────────────────────────────────────────────────────
    fn normal_key(&mut self, key: &KeyEvent) {
        if let Some(op) = self.pending.take() {
            self.apply_operator(op, key);
            return;
        }
        let ch = match key.code {
            KeyCode::Char(c) => c,
            _ => return,
        };
        let ro = self.readonly;
        match ch {
            // motions
            'h' => self.col = self.col.saturating_sub(1),
            'l' => {
                if self.col + 1 < self.cur_len() {
                    self.col += 1;
                }
            }
            'j' => self.row = (self.row + 1).min(self.lines.len().saturating_sub(1)),
            'k' => self.row = self.row.saturating_sub(1),
            'w' => self.col = self.next_word(),
            'b' => self.col = self.prev_word(),
            'e' => self.col = self.word_end(),
            '0' => self.col = 0,
            '^' => self.col = self.first_nonblank(),
            '$' => self.col = self.cur_len().saturating_sub(1),
            'G' => self.row = self.lines.len().saturating_sub(1),
            // operator / prefix — wait for the second key (g/d/c/y; r = replace one char)
            'g' | 'd' | 'c' | 'y' => self.pending = Some(ch),
            'r' if !ro => self.pending = Some('r'),
            // inline edits
            'x' if !ro => {
                self.snapshot();
                self.delete_char_under();
            }
            'J' if !ro => {
                self.snapshot();
                self.join_line();
            }
            '~' if !ro => {
                self.snapshot();
                self.toggle_case();
            }
            'D' if !ro => {
                self.snapshot();
                self.delete_to_eol();
            }
            'C' if !ro => {
                self.snapshot();
                self.delete_to_eol();
                self.mode = EditMode::Insert;
            }
            'p' if !ro => {
                self.snapshot();
                self.paste(true);
            }
            'P' if !ro => {
                self.snapshot();
                self.paste(false);
            }
            'u' if !ro => self.undo(),
            // enter insert
            'i' if !ro => {
                self.snapshot();
                self.mode = EditMode::Insert;
            }
            'a' if !ro => {
                self.snapshot();
                self.col = (self.col + 1).min(self.cur_len());
                self.mode = EditMode::Insert;
            }
            'I' if !ro => {
                self.snapshot();
                self.col = self.first_nonblank();
                self.mode = EditMode::Insert;
            }
            'A' if !ro => {
                self.snapshot();
                self.col = self.cur_len();
                self.mode = EditMode::Insert;
            }
            'o' if !ro => {
                self.snapshot();
                self.open_line(true);
            }
            'O' if !ro => {
                self.snapshot();
                self.open_line(false);
            }
            _ => {}
        }
    }

    /// Second key of a two-stroke Normal command (the operator/prefix is `op`).
    fn apply_operator(&mut self, op: char, key: &KeyEvent) {
        let ch = match key.code {
            KeyCode::Char(c) => c,
            _ => return, // arrow/other after an operator just cancels it
        };
        let ro = self.readonly;
        match (op, ch) {
            ('g', 'g') => self.row = 0,
            ('d', 'd') if !ro => {
                self.snapshot();
                self.delete_line(true);
            }
            ('d', 'w') if !ro => {
                self.snapshot();
                self.delete_word();
            }
            ('d', '$') if !ro => {
                self.snapshot();
                self.delete_to_eol();
            }
            ('c', 'c') if !ro => {
                self.snapshot();
                self.clear_line();
                self.mode = EditMode::Insert;
            }
            ('c', 'w') if !ro => {
                self.snapshot();
                self.delete_word();
                self.mode = EditMode::Insert;
            }
            ('c', '$') if !ro => {
                self.snapshot();
                self.delete_to_eol();
                self.mode = EditMode::Insert;
            }
            ('y', 'y') => self.yank_line(),
            // `r<char>` — replace the char under the cursor in place.
            ('r', c) if !ro => {
                self.snapshot();
                self.replace_char(c);
            }
            _ => {}
        }
    }

    /// `r<char>` — replace the glyph under the cursor (vim, stays in Normal).
    fn replace_char(&mut self, ch: char) {
        let len = self.cur_len();
        if self.col < len {
            let b0 = char_byte(&self.lines[self.row], self.col);
            let b1 = char_byte(&self.lines[self.row], self.col + 1);
            self.lines[self.row].replace_range(b0..b1, &ch.to_string());
            self.dirty = true;
        }
    }

    /// `J` — join the next line onto this one with a single separating space
    /// (cursor lands at the join, vim-style).
    fn join_line(&mut self) {
        if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            let cur = &mut self.lines[self.row];
            self.col = cur.chars().count();
            if !cur.is_empty() && !next.trim_start().is_empty() {
                cur.push(' ');
            }
            cur.push_str(next.trim_start());
            self.dirty = true;
        }
    }

    /// `~` — toggle the case of the glyph under the cursor and advance.
    fn toggle_case(&mut self) {
        let len = self.cur_len();
        if self.col < len {
            if let Some(ch) = self.lines[self.row].chars().nth(self.col) {
                let flipped: String = if ch.is_uppercase() {
                    ch.to_lowercase().collect()
                } else {
                    ch.to_uppercase().collect()
                };
                let b0 = char_byte(&self.lines[self.row], self.col);
                let b1 = char_byte(&self.lines[self.row], self.col + 1);
                self.lines[self.row].replace_range(b0..b1, &flipped);
                self.col = (self.col + 1).min(self.cur_len().saturating_sub(1));
                self.dirty = true;
            }
        }
    }

    fn delete_char_under(&mut self) {
        let len = self.cur_len();
        if self.col < len {
            let b0 = char_byte(&self.lines[self.row], self.col);
            let b1 = char_byte(&self.lines[self.row], self.col + 1);
            self.clip = self.lines[self.row][b0..b1].to_string();
            self.clip_linewise = false;
            self.lines[self.row].replace_range(b0..b1, "");
            self.dirty = true;
        }
    }
    fn delete_to_eol(&mut self) {
        let b = char_byte(&self.lines[self.row], self.col);
        let removed = self.lines[self.row].split_off(b);
        if !removed.is_empty() {
            self.clip = removed;
            self.clip_linewise = false;
            self.dirty = true;
        }
    }
    fn delete_line(&mut self, yank: bool) {
        if yank {
            self.clip = self.lines[self.row].clone();
            self.clip_linewise = true;
        }
        if self.lines.len() > 1 {
            self.lines.remove(self.row);
            if self.row >= self.lines.len() {
                self.row = self.lines.len() - 1;
            }
        } else {
            self.lines[0].clear();
        }
        self.col = 0;
        self.dirty = true;
    }
    fn delete_word(&mut self) {
        let to = self.next_word().min(self.cur_len());
        if to > self.col {
            let b0 = char_byte(&self.lines[self.row], self.col);
            let b1 = char_byte(&self.lines[self.row], to);
            self.clip = self.lines[self.row][b0..b1].to_string();
            self.clip_linewise = false;
            self.lines[self.row].replace_range(b0..b1, "");
            self.dirty = true;
        }
    }
    fn clear_line(&mut self) {
        self.clip = self.lines[self.row].clone();
        self.clip_linewise = true;
        self.lines[self.row].clear();
        self.col = 0;
        self.dirty = true;
    }
    fn yank_line(&mut self) {
        self.clip = self.lines[self.row].clone();
        self.clip_linewise = true;
    }
    fn paste(&mut self, after: bool) {
        if self.clip.is_empty() && !self.clip_linewise {
            return;
        }
        if self.clip_linewise {
            let at = if after { self.row + 1 } else { self.row };
            self.lines.insert(at, self.clip.clone());
            self.row = at;
            self.col = 0;
        } else {
            let col = if after && self.cur_len() > 0 {
                (self.col + 1).min(self.cur_len())
            } else {
                self.col
            };
            let b = char_byte(&self.lines[self.row], col);
            self.lines[self.row].insert_str(b, &self.clip);
            self.col = col + self.clip.chars().count().saturating_sub(1);
        }
        self.dirty = true;
    }
    fn open_line(&mut self, below: bool) {
        let indent = self.leading_ws();
        let at = if below { self.row + 1 } else { self.row };
        self.lines.insert(at, indent);
        self.row = at;
        self.col = self.cur_len();
        self.mode = EditMode::Insert;
        self.dirty = true;
    }
}

/// Render an editor line's visible window in plain text with a block cursor at
/// `cursor_col` (display columns; a space when the cursor sits past end-of-line).
/// Plain — no syntax colour — so the inverse cursor cell is unambiguous on the
/// active line.
fn render_cursor_line(window: &str, cursor_col: usize) -> String {
    let before = slice_cols(window, 0, cursor_col);
    let at = slice_cols(window, cursor_col, cursor_col + 1);
    let at = if at.is_empty() { " ".to_string() } else { at };
    let after = slice_cols(window, cursor_col + 1, usize::MAX);
    format!(
        "{before}{}{after}",
        Style::new().fg(Color::Black).bg(TN_FG).render(&at)
    )
}

#[cfg(test)]
mod vim_tests {
    use super::*;

    fn buf(lines: &[&str]) -> IdeFile {
        IdeFile::new(
            std::path::PathBuf::from("t"),
            lines.iter().map(|s| s.to_string()).collect(),
            false,
            false,
        )
    }
    fn ro(lines: &[&str]) -> IdeFile {
        IdeFile::new(
            std::path::PathBuf::from("t"),
            lines.iter().map(|s| s.to_string()).collect(),
            false,
            true,
        )
    }
    fn k(c: char) -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
        }
    }
    /// Feed a sequence of plain Char keys (covers multi-key ops like dd/gg/dw).
    fn feed(f: &mut IdeFile, s: &str) {
        for c in s.chars() {
            f.edit_key(&k(c), 20, 80);
        }
    }

    #[test]
    fn opens_in_normal_and_navigates_without_typing() {
        let mut f = buf(&["foo", "bar"]);
        assert_eq!(f.mode, EditMode::Normal);
        feed(&mut f, "j");
        assert_eq!(f.row, 1);
        feed(&mut f, "$");
        assert_eq!(f.col, 2); // last char of "bar"
        feed(&mut f, "0");
        assert_eq!(f.col, 0);
        assert_eq!(f.lines, vec!["foo", "bar"]); // motions never inserted
    }

    #[test]
    fn word_motions_w_e_b() {
        let mut f = buf(&["foo bar baz"]);
        feed(&mut f, "w");
        assert_eq!(f.col, 4); // start of "bar"
        feed(&mut f, "e");
        assert_eq!(f.col, 6); // end of "bar"
        feed(&mut f, "b");
        assert_eq!(f.col, 4); // back to start of "bar"
    }

    #[test]
    fn gg_and_capital_g_jump() {
        let mut f = buf(&["a", "b", "c"]);
        feed(&mut f, "G");
        assert_eq!(f.row, 2);
        feed(&mut f, "gg");
        assert_eq!(f.row, 0);
    }

    #[test]
    fn insert_mode_types_literally() {
        let mut f = buf(&["bc"]);
        feed(&mut f, "i");
        assert_eq!(f.mode, EditMode::Insert);
        feed(&mut f, "a"); // literal char in Insert, not "append"
        assert_eq!(f.lines[0], "abc");
        assert_eq!(f.col, 1);
    }

    #[test]
    fn append_at_end_of_line() {
        let mut f = buf(&["ab"]);
        feed(&mut f, "A");
        assert_eq!(f.mode, EditMode::Insert);
        assert_eq!(f.col, 2);
        feed(&mut f, "c");
        assert_eq!(f.lines[0], "abc");
    }

    #[test]
    fn o_opens_indented_line_below_in_insert() {
        let mut f = buf(&["  foo"]);
        feed(&mut f, "o");
        assert_eq!(f.mode, EditMode::Insert);
        assert_eq!(f.row, 1);
        assert_eq!(f.lines[1], "  "); // indent carried
        feed(&mut f, "x");
        assert_eq!(f.lines[1], "  x");
    }

    #[test]
    fn x_deletes_char_under_cursor() {
        let mut f = buf(&["abc"]);
        feed(&mut f, "x");
        assert_eq!(f.lines[0], "bc");
        assert_eq!(f.col, 0);
    }

    #[test]
    fn dw_deletes_word() {
        let mut f = buf(&["foo bar"]);
        feed(&mut f, "dw");
        assert_eq!(f.lines[0], "bar");
    }

    #[test]
    fn dd_then_p_moves_a_line() {
        let mut f = buf(&["one", "two", "three"]);
        feed(&mut f, "dd");
        assert_eq!(f.lines, vec!["two", "three"]);
        feed(&mut f, "p"); // paste "one" below current line
        assert_eq!(f.lines, vec!["two", "one", "three"]);
    }

    #[test]
    fn u_undoes_a_change() {
        let mut f = buf(&["abc"]);
        feed(&mut f, "x");
        assert_eq!(f.lines[0], "bc");
        feed(&mut f, "u");
        assert_eq!(f.lines[0], "abc");
    }

    #[test]
    fn ctrl_w_kills_word_back_in_insert() {
        let mut f = buf(&["foo bar"]);
        feed(&mut f, "A"); // insert mode at EOL
        f.edit_key(
            &KeyEvent {
                code: KeyCode::Char('w'),
                modifiers: KeyModifiers::CONTROL,
            },
            20,
            80,
        );
        assert_eq!(f.lines[0], "foo ");
    }

    #[test]
    fn readonly_navigates_but_blocks_edits() {
        let mut f = ro(&["abc", "def"]);
        feed(&mut f, "j");
        assert_eq!(f.row, 1);
        feed(&mut f, "x"); // edit blocked
        assert_eq!(f.lines, vec!["abc", "def"]);
        feed(&mut f, "i"); // can't enter insert on a read-only buffer
        assert_eq!(f.mode, EditMode::Normal);
    }

    #[test]
    fn horizontal_scroll_follows_the_cursor() {
        let mut f = buf(&["abcdefghijklmnopqrst"]); // 20 cols
                                                    // content width 8 → moving to EOL (col 19) scrolls the window right.
        f.edit_key(&k('$'), 20, 8);
        assert_eq!(f.col, 19);
        assert_eq!(f.hscroll, 12); // 19 + 1 - 8
                                   // back to col 0 → scrolled fully left again.
        f.edit_key(&k('0'), 20, 8);
        assert_eq!(f.hscroll, 0);
    }

    #[test]
    fn render_cursor_line_marks_the_active_column() {
        let out = render_cursor_line("hello", 1);
        assert!(out.starts_with('h')); // text before the cursor is plain
        assert!(out.contains('\u{1b}')); // the cursor cell is styled (inverse)
                                         // cursor past end-of-line still renders a block (a styled space).
        assert!(render_cursor_line("hi", 5).contains('\u{1b}'));
    }

    #[test]
    fn vim_join_replace_and_toggle_case() {
        let mut f = buf(&["foo", "bar"]);
        feed(&mut f, "J"); // join the next line with a space
        assert_eq!(f.lines, vec!["foo bar"]);

        let mut f = buf(&["abc"]);
        feed(&mut f, "rx"); // replace the char under the cursor
        assert_eq!(f.lines[0], "xbc");

        let mut f = buf(&["aB"]);
        feed(&mut f, "~"); // toggle case + advance
        assert_eq!(f.lines[0], "AB");
    }
}
