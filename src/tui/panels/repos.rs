//! The repos-folder project picker shared by `/deploy`, `/run`, and `/review`:
//! Agentic CI/CD (deploy), a dev-mode debug run on the A3S Runtime (run), or
//! a read-only local code review (review) — gateway access auto-configured
//! where the action needs it.
//!
//! Login-gated: registered only while signed in to OS (the target + gateway
//! origin come from the session). Opens a single-select panel over
//! `repo_dir()` (`~/.a3s/repos` — where `&` clones), and on Enter drives the
//! agent with the purpose's directive.

use super::super::*;

/// What Enter does with the picked project: full Agentic CI/CD (`/deploy`),
/// a quick dev-mode debug run on the A3S Runtime (`/run`), or a read-only
/// code review of the local clone (`/review` without a URL).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepoAction {
    Deploy,
    Run,
    Review,
}

/// `/deploy` / `/run` selection panel: the repos-folder projects + cursor.
pub(crate) struct RepoPanel {
    /// Absolute path of the repos root (config `repo_dir`).
    pub(crate) root: std::path::PathBuf,
    /// Immediate subdirectory names (each a deployable project), sorted.
    pub(crate) projects: Vec<String>,
    pub(crate) sel: usize,
    /// Which command opened the picker (drives Enter + the overlay copy).
    pub(crate) action: RepoAction,
}

/// List the immediate subdirectories of `root` (each a deployable project),
/// skipping dotfiles. Sorted for a stable panel.
pub(crate) fn list_projects(root: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            (!n.starts_with('.')).then_some(n)
        })
        .collect();
    v.sort();
    v
}

/// The OS gateway access address a deployed project is exposed at — a
/// deterministic rule so the panel, the directive, and the agent all agree.
/// Path-based route under the gateway origin: `<gateway>/apps/<slug>`.
pub(crate) fn gateway_access_url(gateway_origin: &str, project: &str) -> String {
    format!(
        "{}/apps/{}",
        gateway_origin.trim_end_matches('/'),
        slug(project)
    )
}

/// The dev-mode access address a `/run` instance is exposed at. Same gateway
/// rule as deploys with a `-dev` suffix, so a debug run never shadows the
/// deployed app: `<gateway>/apps/<slug>-dev`.
pub(crate) fn dev_access_url(gateway_origin: &str, project: &str) -> String {
    format!(
        "{}/apps/{}-dev",
        gateway_origin.trim_end_matches('/'),
        slug(project)
    )
}

/// Filesystem-and-URL-safe project slug (ascii lower, `-` separators).
pub(crate) fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() {
        "app".to_string()
    } else {
        s
    }
}

/// The Agentic CI/CD directive for the selected project. Injects the concrete
/// gateway access URL (from the OS session) + the rule, so the agent both runs
/// the pipeline and configures OS gateway access without guessing hosts.
pub(crate) fn deploy_prompt(project_dir: &str, project: &str, access_url: &str) -> String {
    format!(
        "Run Agentic CI/CD for the project at {project_dir} and deploy it to OS, then \
         make it reachable at its gateway access address.\n\
         RemoteUI requirement: before the build/deploy work starts, use the signed-in \
         OS progressive API / Runtime capability to create or open the live CI/CD \
         execution view for this run (GitHub Actions-style timeline, step status, \
         streaming logs, artifacts, and gateway link). Execute the relevant OS \
         operation with `\"shaped\":true` and keep the whole execute response in \
         the tool output so its `.view` / `viewUrl` survives; the TUI will auto-open \
         the first RemoteUI view it sees. Do not print a raw authenticated URL.\n\
         IMPORTANT: {project_dir} is OUTSIDE this session's workspace, so the \
         path-scoped file tools (read/ls/glob/grep/edit) will reject it — use the \
         `bash` tool for everything under it (e.g. `ls`, `cat`, `find`, `sed -n`, and \
         all build/test/deploy commands run with `cd {project_dir}`).\n\
         1. Inspect the project (Dockerfile, chart/, package.json, manifests) with bash \
         to learn how it builds, tests, and runs.\n\
         2. Build it and run its tests/linters. If a step fails, diagnose and fix the \
         minimal cause, then re-run — don't proceed on a red build.\n\
         3. Containerize/deploy it to the OS runtime substrate (use the project's own \
         Dockerfile/Helm chart where present; otherwise the simplest correct path).\n\
         4. Configure the OS gateway so the deployed service is reachable at EXACTLY \
         this access address (the rule is `<gateway-origin>/apps/<project-slug>`): \
         {access_url}\n\
         Use your OS platform capabilities (the signed-in session) for the deploy \
         and gateway steps. When done, report: build/test result, what was deployed, and \
         confirm the service responds at {access_url} (curl it). If any step can't be \
         completed, say which and why — don't claim success you didn't verify.\n\
         Project: {project}"
    )
}

/// The `/run` directive: a quick DEV-MODE start of the selected project on
/// the A3S Runtime — speed over rigor (no CI), debug-friendly, and it must
/// end by reporting the concrete access address.
pub(crate) fn run_prompt(project_dir: &str, project: &str, access_url: &str) -> String {
    format!(
        "Start the project at {project_dir} in DEVELOPMENT mode on the A3S Runtime for \
         quick debugging, and make it reachable at its dev access address.\n\
         RemoteUI requirement: before starting the dev process, use the signed-in OS \
         progressive API / Runtime capability to create or open the live Runtime run \
         view for this session (GitHub Actions-style timeline, step status, streaming \
         logs, health checks, and access link). Execute the relevant OS operation with \
         `\"shaped\":true` and keep the whole execute response in the tool output so \
         its `.view` / `viewUrl` survives; the TUI will auto-open the first RemoteUI \
         view it sees. Do not print a raw authenticated URL.\n\
         IMPORTANT: {project_dir} is OUTSIDE this session's workspace, so the \
         path-scoped file tools (read/ls/glob/grep/edit) will reject it — use the \
         `bash` tool for everything under it (with `cd {project_dir}`).\n\
         1. Quickly inspect the project with bash (README, package.json, Cargo.toml, \
         Dockerfile…) to learn its dev entrypoint (`npm run dev`, `cargo run`, …) and \
         port. Prioritize SPEED: skip full builds, tests, and linters — this is a \
         debug run, not CI/CD.\n\
         2. Start it in development mode on the A3S Runtime via your OS platform \
         capabilities (the signed-in session), with dev settings on (debug logging, \
         hot reload where the project supports it).\n\
         3. Expose it so it responds at EXACTLY this dev access address (the rule is \
         `<gateway-origin>/apps/<project-slug>-dev`, never the production `/apps/\
         <project-slug>` route): {access_url}\n\
         4. Verify with curl, then report on its own line `Access: {access_url}`, plus \
         how it was started and where its logs are. If it cannot start, report the \
         failing command and its error — don't claim success you didn't verify.\n\
         Project: {project}"
    )
}

fn repo_picker_header(
    action: RepoAction,
    total: usize,
    root: &std::path::Path,
    width: usize,
) -> String {
    let (icon_title, _) = repo_picker_copy(action);
    truncate(
        &format!(
            "  {icon_title} — pick a project ({total} in {})",
            root.to_string_lossy()
        ),
        width,
    )
}

fn repo_picker_hint(action: RepoAction, width: usize) -> String {
    let (_, enter_hint) = repo_picker_copy(action);
    truncate(&format!("  ↑/↓ select · {enter_hint} · Esc cancel"), width)
}

fn repo_picker_copy(action: RepoAction) -> (&'static str, &'static str) {
    match action {
        RepoAction::Deploy => ("🚀 deploy", "Enter run Agentic CI/CD"),
        RepoAction::Run => ("▶ run", "Enter start dev mode on A3S Runtime"),
        RepoAction::Review => ("🔎 review", "Enter review (read-only)"),
    }
}

fn repo_picker_project_row(name: &str, width: usize) -> String {
    pad_to(&truncate(&format!("  {name}"), width), width)
}

impl App {
    /// Open the `/deploy` / `/run` project picker (login-gated by the caller).
    pub(crate) fn open_repo_picker(&mut self, action: RepoAction) {
        let root = repo_dir();
        let projects = list_projects(&root);
        if projects.is_empty() {
            self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  no projects in {} — clone one with `& <git-url>` first",
                root.display()
            )));
            return;
        }
        self.repo_picker = Some(RepoPanel {
            root,
            projects,
            sel: 0,
            action,
        });
    }

    /// Keys while the `/deploy` picker is open — consumes everything so nothing
    /// leaks to the input behind the overlay.
    pub(crate) fn handle_repo_picker_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let p = self.repo_picker.as_mut()?;
        let last = p.projects.len().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => p.sel = p.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => p.sel = (p.sel + 1).min(last),
            KeyCode::Esc => self.repo_picker = None,
            KeyCode::Enter => {
                let panel = self.repo_picker.take()?;
                let project = panel.projects.get(panel.sel.min(last))?.clone();
                let dir = panel.root.join(&project);
                // Local review: same `&` flow minus the clone — arm the report
                // gate + the loop that drives the agent to the fenced report.
                // Review state is App-wide, so it must not start mid-turn
                // (mirrors the `&` handler's idle requirement).
                if panel.action == RepoAction::Review {
                    if self.state != State::Idle {
                        self.push_line(&Style::new().fg(TN_YELLOW).render(
                            "  a code review can't start while a turn is running — press Esc to stop first",
                        ));
                        return None;
                    }
                    let prompt =
                        super::review::local_review_prompt(&dir.to_string_lossy(), &project);
                    let label = format!("🔎 review: {project}");
                    self.review = None;
                    self.review_open = false;
                    self.messages
                        .push(gutter(TN_PURPLE, &Style::new().bold().render(&label)));
                    self.push_line(&Style::new().fg(TN_GRAY).render(
                        "  deep read-only inspection → issue checklist to pick fixes (no auto-fix · Esc stops)",
                    ));
                    self.review_pending = true;
                    self.engage_autonomy(8);
                    return self.start_stream_inner(prompt, label, true, true, false);
                }
                // Gateway origin = the signed-in OS origin (GATEWAY_URL == the
                // web origin the user reaches the platform through).
                let gateway = self
                    .os_session
                    .as_ref()
                    .map(|s| crate::a3s_os::os_origin(&s.address))
                    .unwrap_or_default();
                let (prompt, label, hint) = match panel.action {
                    RepoAction::Deploy => {
                        let access = gateway_access_url(&gateway, &project);
                        let prompt = deploy_prompt(&dir.to_string_lossy(), &project, &access);
                        let label = format!("🚀 Agentic CI/CD: {project} → {access}");
                        let hint = format!(
                            "  build → test → deploy → gateway access {access} · RemoteUI opens live CI/CD view"
                        );
                        (prompt, label, hint)
                    }
                    RepoAction::Run => {
                        let access = dev_access_url(&gateway, &project);
                        let prompt = run_prompt(&dir.to_string_lossy(), &project, &access);
                        let label = format!("▶ dev run: {project} → {access}");
                        let hint = format!(
                            "  dev mode on A3S Runtime · debug run → access {access} · RemoteUI opens live run view"
                        );
                        (prompt, label, hint)
                    }
                    RepoAction::Review => unreachable!("handled above"),
                };
                self.arm_runtime_view_auto_open(panel.action, project.clone());
                self.messages
                    .push(gutter(ACCENT, &Style::new().bold().render(&label)));
                self.push_line(&Style::new().fg(TN_GRAY).render(&hint));
                self.engage_autonomy(8);
                if self.state == State::Idle {
                    return self.start_stream_inner(prompt, label, true, false, false);
                }
                self.seq += 1;
                self.queue.push(Queued {
                    prio: 1,
                    seq: self.seq,
                    text: prompt,
                    display: label,
                });
                self.push_line(&Style::new().fg(TN_GRAY).render("    ⋯ queued"));
            }
            _ => {}
        }
        None
    }

    /// Overlay the `/deploy` picker above the input.
    pub(crate) fn overlay_repo_picker(&self, composed: String) -> String {
        let Some(p) = self.repo_picker.as_ref() else {
            return composed;
        };
        let width = self.width as usize;
        let total = p.projects.len();
        let mut menu = vec![
            pad_to(
                &Style::new()
                    .fg(ACCENT)
                    .bold()
                    .render(&repo_picker_header(p.action, total, &p.root, width)),
                width,
            ),
            pad_to(
                &Style::new()
                    .fg(TN_GRAY)
                    .render(&repo_picker_hint(p.action, width)),
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
            let raw = repo_picker_project_row(name, width);
            menu.push(if row == sel {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
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
    fn lists_project_dirs_sorted_skipping_dotfiles() {
        let root = std::env::temp_dir().join(format!("a3s-deploy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("zeta")).unwrap();
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("readme.txt"), "x").unwrap(); // file, not a project
        let ps = list_projects(&root);
        assert_eq!(ps, vec!["alpha", "zeta"]); // sorted, dotdir + file excluded
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn gateway_access_url_follows_the_rule() {
        assert_eq!(
            gateway_access_url("http://180.163.156.38:49164", "risk-reporter"),
            "http://180.163.156.38:49164/apps/risk-reporter"
        );
        // Trailing slash + non-ascii name → slugged, single path.
        assert_eq!(
            gateway_access_url("http://host/", "My App 2"),
            "http://host/apps/my-app-2"
        );
    }

    #[test]
    fn dev_access_url_follows_the_dev_rule() {
        assert_eq!(
            dev_access_url("http://180.163.156.38:49164", "risk-reporter"),
            "http://180.163.156.38:49164/apps/risk-reporter-dev"
        );
        // Trailing slash + non-ascii name → slugged, single path, -dev suffix.
        assert_eq!(
            dev_access_url("http://host/", "My App 2"),
            "http://host/apps/my-app-2-dev"
        );
    }

    #[test]
    fn run_prompt_is_dev_mode_speed_first_and_reports_access() {
        let url = dev_access_url("http://os", "svc");
        let p = run_prompt("/repos/svc", "svc", &url);
        assert!(p.contains("/repos/svc"));
        assert!(p.contains("http://os/apps/svc-dev")); // the concrete dev address
        assert!(p.contains("<gateway-origin>/apps/<project-slug>-dev")); // the rule
        assert!(p.contains("DEVELOPMENT mode") && p.contains("A3S Runtime"));
        assert!(p.contains("RemoteUI requirement") && p.contains("GitHub Actions-style"));
        assert!(p.contains("\"shaped\":true") && p.contains(".view") && p.contains("viewUrl"));
        // Speed-first: a debug run must not turn into full CI/CD.
        assert!(p.contains("skip full builds, tests, and linters"));
        // The deliverable: the access address on its own line.
        assert!(p.contains("Access: http://os/apps/svc-dev"));
        // Out-of-workspace fix: steer to bash like the deploy prompt.
        assert!(p.contains("OUTSIDE this session's workspace") && p.contains("bash"));
    }

    #[test]
    fn deploy_prompt_carries_dir_gateway_and_rule() {
        let url = gateway_access_url("http://os", "svc");
        let p = deploy_prompt("/repos/svc", "svc", &url);
        assert!(p.contains("/repos/svc"));
        assert!(p.contains("http://os/apps/svc")); // the concrete access address, twice
        assert!(p.contains("<gateway-origin>/apps/<project-slug>")); // the rule
        assert!(p.contains("run its tests") && p.contains("gateway"));
        assert!(p.contains("RemoteUI requirement") && p.contains("GitHub Actions-style"));
        assert!(p.contains("\"shaped\":true") && p.contains(".view") && p.contains("viewUrl"));
        // Out-of-workspace fix: the directive must steer the agent to bash
        // (the file tools are workspace-scoped and reject ~/.a3s/repos).
        assert!(p.contains("OUTSIDE this session's workspace") && p.contains("bash"));
    }

    #[test]
    fn picker_rows_fit_fixed_width_for_long_project_names() {
        let row = repo_picker_project_row(
            "very-long-project-name-that-would-otherwise-spill-across-the-terminal",
            28,
        );
        assert_eq!(a3s_tui::style::visible_len(&row), 28);
        assert!(row.contains('…'), "{row}");
    }

    #[test]
    fn picker_header_and_hint_fit_fixed_width() {
        let root = std::path::PathBuf::from(
            "/Users/example/.a3s/repos/a/path/that/is/far/too/long/for/a/picker/header",
        );
        for action in [RepoAction::Deploy, RepoAction::Run, RepoAction::Review] {
            let header = repo_picker_header(action, 42, &root, 42);
            let hint = repo_picker_hint(action, 42);
            assert!(a3s_tui::style::visible_len(&header) <= 42, "{header}");
            assert!(a3s_tui::style::visible_len(&hint) <= 42, "{hint}");
        }
    }
}
