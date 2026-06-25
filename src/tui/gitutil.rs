//! Git helpers and the `/git` panel data model.

/// Current git branch of `dir` (cheap: parse `.git/HEAD`), if any.
pub(crate) fn git_branch(dir: &str) -> Option<String> {
    let head = std::fs::read_to_string(format!("{dir}/.git/HEAD")).ok()?;
    head.strip_prefix("ref: refs/heads/")
        .map(|b| b.trim().to_string())
}

/// A changed file in the `/git` panel: porcelain X (staged) + Y (unstaged).
#[derive(Clone)]
pub(crate) struct GitFile {
    pub(crate) x: char,
    pub(crate) y: char,
    pub(crate) path: String,
}

impl GitFile {
    pub(crate) fn staged(&self) -> bool {
        self.x != ' ' && self.x != '?'
    }
    pub(crate) fn untracked(&self) -> bool {
        self.x == '?'
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum GitView {
    Status,
    Log,
}

/// State of the `/git` full-screen panel (a small gitui-style view).
pub(crate) struct Git {
    pub(crate) files: Vec<GitFile>,
    pub(crate) sel: usize,
    /// Right-pane content: the selected file's diff, or the selected commit's
    /// details in the Log view.
    pub(crate) diff: Vec<String>,
    pub(crate) diff_scroll: usize,
    pub(crate) log: Vec<String>,
    pub(crate) log_sel: usize,
    pub(crate) view: GitView,
    /// `Some` while the user is typing a commit message.
    pub(crate) commit_input: Option<String>,
    pub(crate) note: String,
}

/// Run a git subcommand in `repo`, returning stdout (+ stderr on failure).
pub(crate) async fn run_git(repo: String, args: Vec<String>) -> String {
    match tokio::process::Command::new("git")
        .current_dir(&repo)
        .args(&args)
        .output()
        .await
    {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            if !o.status.success() {
                s.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            s
        }
        Err(e) => format!("git error: {e}"),
    }
}

/// Working-tree status (porcelain) + recent log for the `/git` panel.
pub(crate) async fn git_status_log(repo: String) -> (Vec<GitFile>, Vec<String>) {
    let status = run_git(
        repo.clone(),
        vec![
            "status".into(),
            "--porcelain=v1".into(),
            "--untracked-files=all".into(),
        ],
    )
    .await;
    let files = status
        .lines()
        .filter_map(|l| {
            let b = l.as_bytes();
            if b.len() < 4 {
                return None;
            }
            Some(GitFile {
                x: b[0] as char,
                y: b[1] as char,
                path: l[3..].to_string(),
            })
        })
        .collect();
    let log = run_git(
        repo,
        vec![
            "log".into(),
            "--oneline".into(),
            "-n".into(),
            "30".into(),
            "--no-color".into(),
        ],
    )
    .await
    .lines()
    .map(String::from)
    .collect();
    (files, log)
}

/// Diff for one file (whole change vs HEAD; untracked shown as all-added).
pub(crate) async fn git_diff_file(repo: String, file: GitFile) -> Vec<String> {
    let args = if file.untracked() {
        vec![
            "diff".into(),
            "--no-color".into(),
            "--no-index".into(),
            "--".into(),
            "/dev/null".into(),
            file.path.clone(),
        ]
    } else {
        vec![
            "diff".into(),
            "--no-color".into(),
            "HEAD".into(),
            "--".into(),
            file.path.clone(),
        ]
    };
    run_git(repo, args)
        .await
        .lines()
        .map(String::from)
        .collect()
}
