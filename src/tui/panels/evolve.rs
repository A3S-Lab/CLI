//! `/evolve` — pick a repo from the repos folder and enter a multi-round
//! interactive development session with a standing auto-improvement goal.
//!
//! Login-gated. Two steps: a project picker over `repo_dir()` (`~/.a3s/repos`),
//! then the user types the improvement direction, which becomes a persistent
//! `/goal` north-star (prepended to every turn) plus a `/loop` auto-continue
//! budget — so the agent iteratively develops + improves the repo across turns
//! while the user keeps steering.

use super::super::*;

/// `/evolve` project picker: the projects under the repos folder + cursor.
pub(crate) struct EvolvePanel {
    pub(crate) root: std::path::PathBuf,
    pub(crate) projects: Vec<String>,
    pub(crate) sel: usize,
}

/// Persistent `/goal` north-star for an evolve session (prepended to every
/// turn — kept short). `direction` is the user's improvement intent.
pub(crate) fn evolve_goal(project: &str, project_dir: &str, direction: &str) -> String {
    format!(
        "Iteratively develop and improve the project `{project}` (at {project_dir}) toward: \
         {direction}. Each turn make one concrete, verified improvement and keep the repo \
         building/passing; continue until the direction is fully met."
    )
}

/// First-turn directive that opens the evolve session.
pub(crate) fn evolve_prompt(project_dir: &str, project: &str, direction: &str) -> String {
    format!(
        "Start a multi-round development session on the project `{project}` at {project_dir}. \
         Improvement direction: {direction}.\n\
         {project_dir} is OUTSIDE this session's workspace, so the path-scoped file tools \
         (read/ls/glob/grep/edit/write) will reject it — use the `bash` tool (`ls`, `cat`, \
         `sed -n`, `find`, `grep`, and edits via `bash` heredocs / `sed -i`) for everything \
         under it, running commands with `cd {project_dir}`.\n\
         This turn: (1) get oriented — build the project and run its tests/linters via bash \
         to establish a green baseline (fix trivial breakage first); (2) make the FIRST \
         concrete improvement toward the direction; (3) verify it (re-build / re-test); \
         (4) summarize what changed and what you'll do next. Keep changes small and always \
         leave the repo working. Do not claim a step passed unless you ran it."
    )
}

impl App {
    /// Open the `/evolve` project picker (login-gated by the caller).
    pub(crate) fn open_evolve_panel(&mut self) {
        let root = repo_dir();
        let projects = panels::repos::list_projects(&root);
        if projects.is_empty() {
            self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  no projects in {} — clone one with `& <git-url>` first",
                root.display()
            )));
            return;
        }
        self.evolve = Some(EvolvePanel {
            root,
            projects,
            sel: 0,
        });
    }

    /// Keys while the `/evolve` picker is open. Enter selects a repo and primes
    /// the evolve input mode (the next submit is the improvement direction).
    pub(crate) fn handle_evolve_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let p = self.evolve.as_mut()?;
        let last = p.projects.len().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => p.sel = p.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => p.sel = (p.sel + 1).min(last),
            KeyCode::Esc => self.evolve = None,
            KeyCode::Enter => {
                let panel = self.evolve.take()?;
                let project = panel.projects.get(panel.sel.min(last))?.clone();
                let dir = panel.root.join(&project);
                self.evolve_target = Some((dir, project.clone()));
                self.evolve_mode = true;
                self.push_line(&Style::new().fg(TN_GREEN).render(&format!(
                    "  ⟲ evolving {project} — 输入改进目标后回车开始多轮自动改进（Esc 取消）"
                )));
            }
            _ => {}
        }
        None
    }

    /// A submit while the evolve input mode is primed: set the standing goal +
    /// loop budget and kick off the first development turn.
    pub(crate) fn submit_evolve(&mut self, direction: &str) -> Option<Cmd<Msg>> {
        self.evolve_mode = false;
        let (dir, project) = self.evolve_target.take()?;
        let direction = direction.trim();
        if direction.is_empty() {
            self.push_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render("  evolve 已取消（未输入改进目标）"),
            );
            return None;
        }
        let dir_s = dir.to_string_lossy().into_owned();
        self.goal = Some(evolve_goal(&project, &dir_s, direction));
        self.goal_since = Some(Instant::now());
        self.engage_autonomy(8); // auto mode + auto-continue budget across rounds
        self.messages.push(gutter(
            TN_GREEN,
            &Style::new()
                .bold()
                .render(&format!("⟲ evolve {project}: {direction}")),
        ));
        self.push_line(&Style::new().fg(TN_GRAY).render(
            "  🎯 goal set · ↻ 多轮自动改进（继续输入可随时引导 · Esc 停 · /goal clear 清除）",
        ));
        let prompt = evolve_prompt(&dir_s, &project, direction);
        let display = format!("⟲ evolve {project}");
        // No attachments on the synthetic kickoff; the repo lives outside the
        // workspace so this is a bash-driven dev turn.
        if self.state == State::Idle {
            return self.start_stream_inner(prompt, display, true, false, false);
        }
        self.seq += 1;
        self.queue.push(Queued {
            prio: 1,
            seq: self.seq,
            text: prompt,
        });
        self.push_line(&Style::new().fg(TN_GRAY).render("    ⋯ queued"));
        None
    }

    /// Overlay the `/evolve` picker above the input.
    pub(crate) fn overlay_evolve_menu(&self, composed: String) -> String {
        let Some(p) = self.evolve.as_ref() else {
            return composed;
        };
        let width = self.width as usize;
        let total = p.projects.len();
        let mut menu = vec![
            pad_to(
                &Style::new().fg(TN_GREEN).bold().render(&format!(
                    "  ⟲ evolve — pick a repo ({} in {})",
                    total,
                    truncate(&p.root.to_string_lossy(), width.saturating_sub(28))
                )),
                width,
            ),
            pad_to(
                &Style::new()
                    .fg(TN_GRAY)
                    .render("  ↑/↓ select · Enter set improvement goal · Esc cancel"),
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
        for (row, name) in p.projects.iter().enumerate().take(end).skip(start) {
            let raw = pad_to(&format!("  {name}"), width);
            menu.push(if row == sel {
                Style::new().fg(Color::Black).bg(TN_GREEN).render(&raw)
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

    #[test]
    fn evolve_goal_is_a_repo_scoped_north_star() {
        let g = evolve_goal("risk-reporter", "/repos/risk-reporter", "add rate limiting");
        assert!(g.contains("risk-reporter") && g.contains("/repos/risk-reporter"));
        assert!(g.contains("add rate limiting"));
        assert!(g.contains("Iteratively") && g.contains("keep the repo building"));
    }

    #[test]
    fn evolve_prompt_directs_bash_and_verified_iteration() {
        let p = evolve_prompt("/repos/svc", "svc", "harden auth");
        assert!(p.contains("/repos/svc") && p.contains("harden auth"));
        // Out-of-workspace fix + verified-iteration contract.
        assert!(p.contains("OUTSIDE this session's workspace") && p.contains("bash"));
        assert!(p.contains("green baseline") && p.contains("verify"));
    }
}
