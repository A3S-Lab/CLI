//! Detect a local Claude Code / Codex login so `/model` can surface those
//! accounts as tabs.

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum AuthProvider {
    Claude,
    Codex,
}

/// True when the local Claude Code / Codex CLI has a stored login.
pub(crate) fn has_local_login(provider: AuthProvider) -> bool {
    match provider {
        AuthProvider::Claude => crate::claude::has_claude_login(),
        AuthProvider::Codex => crate::codex::has_codex_login(),
    }
}
