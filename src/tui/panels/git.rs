//! `/git` panel: status/diff/log views, staging, and commit input.

use super::super::*;
use a3s_tui::components::{
    GitPanel, GitPanelView as TuiGitPanelView, GitStatusFile as TuiGitStatusFile,
};

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
        let view = match g.view {
            GitView::Status => TuiGitPanelView::Status,
            GitView::Log => TuiGitPanelView::Log,
        };
        let files = g
            .files
            .iter()
            .map(|file| TuiGitStatusFile::new(file.x, file.y, file.path.clone()))
            .collect::<Vec<_>>();
        let mut panel = GitPanel::new(self.branch.as_deref().unwrap_or("(detached)"))
            .files(files)
            .selected_file(g.sel)
            .log_entries(g.log.clone())
            .selected_log(g.log_sel)
            .active_view(view)
            .diff_lines(g.diff.clone())
            .diff_scroll(g.diff_scroll)
            .note(g.note.as_str())
            .accent_color(ACCENT)
            .muted_color(TN_GRAY)
            .status_colors(TN_GREEN, TN_YELLOW, TN_RED)
            .diff_colors(TN_CYAN, TN_GREEN, TN_RED, TN_GRAY)
            .fill_height(true);

        if let Some(input) = g.commit_input.as_deref() {
            panel = panel.commit_input(input);
        }

        panel.view(self.width, self.height as usize)
    }
}
