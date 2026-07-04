//! Lightweight git context for the status bar.

/// Current git branch of `dir` (cheap: parse `.git/HEAD`), if any.
pub(crate) fn git_branch(dir: &str) -> Option<String> {
    let head = std::fs::read_to_string(format!("{dir}/.git/HEAD")).ok()?;
    head.strip_prefix("ref: refs/heads/")
        .map(|b| b.trim().to_string())
}
