//! `/git` panel: read-only status, diff, and recent-log views.

use super::super::*;
use a3s_tui::components::{
    GitPanel, GitPanelView as TuiGitPanelView, GitStatusFile as TuiGitStatusFile,
};

impl App {
    fn git_readonly_footer() -> &'static str {
        "  ↑↓ select · Tab log/status · PgUp/PgDn scroll · r refresh · Esc"
    }

    /// Spawn a diff fetch for the currently selected `/git` file.
    pub(crate) fn git_load_diff(&self) -> Option<Cmd<Msg>> {
        let g = self.git.as_ref()?;
        let file = g.files.get(g.sel)?.clone();
        let workspace = self.cwd.clone();
        Some(cmd::cmd(move || async move {
            Msg::GitDiff(git_diff_file(workspace, file).await)
        }))
    }

    /// Spawn a `git show` for the selected commit (Log view's right pane).
    pub(crate) fn git_load_commit(&self) -> Option<Cmd<Msg>> {
        let g = self.git.as_ref()?;
        let hash = g.log.get(g.log_sel)?.split_whitespace().next()?.to_string();
        let workspace = self.cwd.clone();
        Some(cmd::cmd(move || async move {
            let out = run_git(
                workspace,
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
        let workspace = self.cwd.clone();
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
                KeyCode::Char('r') => {
                    return Some(cmd::cmd(move || async move {
                        let (f, l) = git_status_log(workspace).await;
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

    /// Full-screen `/git` panel: read-only status + diff / log.
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
        let panel = GitPanel::new(self.branch.as_deref().unwrap_or("(detached)"))
            .files(files)
            .selected_file(g.sel)
            .log_entries(g.log.clone())
            .selected_log(g.log_sel)
            .active_view(view)
            .diff_lines(g.diff.clone())
            .diff_scroll(g.diff_scroll)
            .note(g.note.as_str())
            .footer_text(Self::git_readonly_footer())
            .accent_color(ACCENT)
            .muted_color(TN_GRAY)
            .status_colors(TN_GREEN, TN_YELLOW, TN_RED)
            .diff_colors(TN_CYAN, TN_GREEN, TN_RED, TN_GRAY)
            .fill_height(true);

        panel.view(self.width, self.height as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_footer_stays_read_only() {
        let footer = App::git_readonly_footer();

        assert!(footer.contains("refresh"), "{footer}");
        assert!(!footer.contains(&["st", "age"].join("")), "{footer}");
        assert!(!footer.contains(&["com", "mit"].join("")), "{footer}");
    }
}
