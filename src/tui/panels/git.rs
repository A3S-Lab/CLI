//! `/git` panel: status/diff/log views, staging, and commit input.

use super::super::*;

impl App {
    /// Spawn a diff fetch for the currently selected `/git` file.
    pub(crate) fn git_load_diff(&self) -> Option<Cmd<Msg>> {
        let g = self.git.as_ref()?;
        let file = g.files.get(g.sel)?.clone();
        let repo = self.cwd.clone();
        Some(cmd::cmd(move || async move {
            Msg::GitDiff(git_diff_file(repo, file).await)
        }))
    }

    /// Spawn a `git show` for the selected commit (Log view's right pane).
    pub(crate) fn git_load_commit(&self) -> Option<Cmd<Msg>> {
        let g = self.git.as_ref()?;
        let hash = g.log.get(g.log_sel)?.split_whitespace().next()?.to_string();
        let repo = self.cwd.clone();
        Some(cmd::cmd(move || async move {
            let out = run_git(
                repo,
                vec![
                    "show".into(),
                    "--no-color".into(),
                    "--stat".into(),
                    "-p".into(),
                    hash,
                ],
            )
            .await;
            Msg::GitDiff(out.lines().map(String::from).collect())
        }))
    }

    /// Handle a key while the `/git` panel is open.
    pub(crate) fn git_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let repo = self.cwd.clone();
        // Commit-message input mode.
        if self.git.as_ref().is_some_and(|g| g.commit_input.is_some()) {
            let g = self.git.as_mut().unwrap();
            let inp = g.commit_input.as_mut().unwrap();
            match key.code {
                KeyCode::Esc => g.commit_input = None,
                KeyCode::Backspace => {
                    inp.pop();
                }
                KeyCode::Char(c) => inp.push(c),
                KeyCode::Enter => {
                    let m = inp.trim().to_string();
                    g.commit_input = None;
                    if !m.is_empty() {
                        g.note = "committing…".into();
                        return Some(cmd::cmd(move || async move {
                            run_git(repo.clone(), vec!["commit".into(), "-m".into(), m]).await;
                            let (f, l) = git_status_log(repo).await;
                            Msg::GitStatus(f, l)
                        }));
                    }
                }
                _ => {}
            }
            return None;
        }
        if key.code == KeyCode::Esc {
            self.git = None;
            return None;
        }
        let mut reload = false;
        let mut reload_commit = false;
        {
            let g = self.git.as_mut()?;
            let last = g.files.len().saturating_sub(1);
            let last_commit = g.log.len().saturating_sub(1);
            let log_view = g.view == GitView::Log;
            match key.code {
                KeyCode::Tab => {
                    g.diff_scroll = 0;
                    if log_view {
                        g.view = GitView::Status;
                        reload = true;
                    } else {
                        g.view = GitView::Log;
                        reload_commit = true;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if log_view {
                        g.log_sel = g.log_sel.saturating_sub(1);
                        g.diff_scroll = 0;
                        reload_commit = true;
                    } else {
                        g.sel = g.sel.saturating_sub(1);
                        reload = true;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if log_view {
                        g.log_sel = (g.log_sel + 1).min(last_commit);
                        g.diff_scroll = 0;
                        reload_commit = true;
                    } else {
                        g.sel = (g.sel + 1).min(last);
                        reload = true;
                    }
                }
                KeyCode::PageUp => g.diff_scroll = g.diff_scroll.saturating_sub(15),
                KeyCode::PageDown => g.diff_scroll += 15,
                // Space / s toggles staging of the selected file.
                KeyCode::Char(' ') | KeyCode::Char('s') => {
                    if let Some(f) = g.files.get(g.sel) {
                        let path = f.path.clone();
                        let unstage = f.staged() && f.y == ' ';
                        g.note = "…".into();
                        return Some(cmd::cmd(move || async move {
                            let args = if unstage {
                                vec![
                                    "reset".into(),
                                    "-q".into(),
                                    "HEAD".into(),
                                    "--".into(),
                                    path,
                                ]
                            } else {
                                vec!["add".into(), "--".into(), path]
                            };
                            run_git(repo.clone(), args).await;
                            let (f, l) = git_status_log(repo).await;
                            Msg::GitStatus(f, l)
                        }));
                    }
                }
                KeyCode::Char('u') => {
                    if let Some(f) = g.files.get(g.sel) {
                        let path = f.path.clone();
                        return Some(cmd::cmd(move || async move {
                            run_git(
                                repo.clone(),
                                vec![
                                    "reset".into(),
                                    "-q".into(),
                                    "HEAD".into(),
                                    "--".into(),
                                    path,
                                ],
                            )
                            .await;
                            let (f, l) = git_status_log(repo).await;
                            Msg::GitStatus(f, l)
                        }));
                    }
                }
                KeyCode::Char('a') => {
                    g.note = "staging all…".into();
                    return Some(cmd::cmd(move || async move {
                        run_git(repo.clone(), vec!["add".into(), "-A".into()]).await;
                        let (f, l) = git_status_log(repo).await;
                        Msg::GitStatus(f, l)
                    }));
                }
                KeyCode::Char('c') => g.commit_input = Some(String::new()),
                KeyCode::Char('r') => {
                    return Some(cmd::cmd(move || async move {
                        let (f, l) = git_status_log(repo).await;
                        Msg::GitStatus(f, l)
                    }));
                }
                _ => {}
            }
        }
        if reload {
            return self.git_load_diff();
        }
        if reload_commit {
            return self.git_load_commit();
        }
        None
    }

    /// Full-screen `/git` panel (gitui-style): status + diff / log + commit.
    pub(crate) fn render_git(&self, g: &Git) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let branch = self.branch.as_deref().unwrap_or("(detached)");
        let tab = |label: &str, active: bool| {
            if active {
                Style::new()
                    .fg(Color::Black)
                    .bg(ACCENT)
                    .bold()
                    .render(&format!(" {label} "))
            } else {
                Style::new().fg(TN_GRAY).render(&format!(" {label} "))
            }
        };
        let logtab = if g.log.is_empty() {
            "Log".to_string()
        } else {
            format!("Log ({})", g.log.len())
        };
        let header = format!(
            "  git · {branch}   {} {}  {}   {}",
            tab("Status", g.view == GitView::Status),
            tab(&logtab, g.view == GitView::Log),
            Style::new()
                .fg(ACCENT)
                .render("⇄ Tab to switch · commits in Log"),
            Style::new().fg(TN_GRAY).render(&g.note)
        );
        let mut out = vec![
            pad_to(&header, width),
            pad_to(&Style::new().fg(TN_GRAY).render(&"─".repeat(width)), width),
        ];
        let body = h.saturating_sub(3);

        if g.view == GitView::Log {
            if g.log.is_empty() {
                let msg = if g.note.is_empty() {
                    "  no commits in this repository yet"
                } else {
                    "  loading commits…"
                };
                out.push(pad_to(&Style::new().fg(TN_GRAY).render(msg), width));
                out.truncate(h);
                while out.len() < h {
                    out.push(String::new());
                }
                return out.join("\n");
            }
            // Two columns: the commit list (selectable) + the selected commit's
            // details (`git show`) on the right.
            let tw = (width / 3).clamp(20, 46);
            let sep = Style::new().fg(TN_GRAY).render(" │ ");
            // keep the selected commit visible
            let start = g.log_sel.saturating_sub(body.saturating_sub(1));
            for i in 0..body {
                let ci = start + i;
                let left = if let Some(line) = g.log.get(ci) {
                    let (hash, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
                    let raw = pad_to(&truncate(&format!(" {hash}  {rest}"), tw), tw);
                    if ci == g.log_sel {
                        Style::new().fg(Color::Black).bg(TN_YELLOW).render(&raw)
                    } else {
                        format!(
                            "{}{}",
                            Style::new()
                                .fg(TN_YELLOW)
                                .render(&pad_to(&format!(" {hash} "), hash.len() + 2)),
                            truncate(rest, tw.saturating_sub(hash.len() + 3))
                        )
                    }
                } else {
                    " ".repeat(tw)
                };
                let right = if let Some(line) = g.diff.get(g.diff_scroll + i) {
                    let st = if line.starts_with("@@") {
                        Style::new().fg(TN_CYAN)
                    } else if line.starts_with("commit ") {
                        Style::new().fg(TN_YELLOW).bold()
                    } else if line.starts_with('+') {
                        Style::new().fg(TN_GREEN)
                    } else if line.starts_with('-') {
                        Style::new().fg(TN_RED)
                    } else if line.starts_with("diff ") || line.starts_with("index ") {
                        Style::new().fg(TN_GRAY)
                    } else {
                        Style::new()
                    };
                    st.render(&truncate(line, width.saturating_sub(tw + 4)))
                } else {
                    String::new()
                };
                out.push(format!("{left}{sep}{right}"));
            }
        } else {
            let tw = (width / 3).clamp(20, 46);
            let sep = Style::new().fg(TN_GRAY).render(" │ ");
            // Scroll the file list so the selection stays visible (mirrors the Log
            // view); previously it rendered from index 0 and the highlight could
            // scroll off the bottom and become unreachable.
            let start = g.sel.saturating_sub(body.saturating_sub(1));
            for i in 0..body {
                let fi = start + i;
                // left: file list
                let left = if let Some(f) = g.files.get(fi) {
                    let mark = format!("{}{}", f.x, f.y);
                    let raw = pad_to(&truncate(&format!(" {mark}  {}", f.path), tw), tw);
                    let color = if f.untracked() {
                        TN_RED
                    } else if f.staged() {
                        TN_GREEN
                    } else {
                        TN_YELLOW
                    };
                    if fi == g.sel {
                        Style::new().fg(Color::Black).bg(color).render(&raw)
                    } else {
                        Style::new().fg(color).render(&raw)
                    }
                } else if fi == 0 && g.files.is_empty() {
                    pad_to(&Style::new().fg(TN_GRAY).render("  working tree clean"), tw)
                } else {
                    " ".repeat(tw)
                };
                // right: diff
                let right = if let Some(line) = g.diff.get(g.diff_scroll + i) {
                    let st = if line.starts_with("@@") {
                        Style::new().fg(TN_CYAN)
                    } else if line.starts_with('+') {
                        Style::new().fg(TN_GREEN)
                    } else if line.starts_with('-') {
                        Style::new().fg(TN_RED)
                    } else if line.starts_with("diff ")
                        || line.starts_with("index ")
                        || line.starts_with("--- ")
                        || line.starts_with("+++ ")
                    {
                        Style::new().fg(TN_GRAY)
                    } else {
                        Style::new()
                    };
                    st.render(&truncate(line, width.saturating_sub(tw + 4)))
                } else {
                    String::new()
                };
                out.push(format!("{left}{sep}{right}"));
            }
        }

        // Bottom row: commit input, or the key hints.
        let bottom = if let Some(msg) = &g.commit_input {
            Style::new().fg(TN_YELLOW).bold().render(&format!(
                "  commit message: {msg}_   (Enter commit · Esc cancel)"
            ))
        } else {
            Style::new().fg(TN_GRAY).render(
                "  ↑↓ select · Space/s stage · u unstage · a stage-all · c commit · Tab log · r refresh · Esc",
            )
        };
        while out.len() + 1 < h {
            out.push(String::new());
        }
        out.push(pad_to(&bottom, width));
        out.truncate(h);
        out.join("\n")
    }
}
