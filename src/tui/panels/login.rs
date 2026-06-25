//! Detect a local Claude Code / Codex login so `/model` can surface those
//! accounts as tabs.
//!
//! NOTE: a3s-code only ships an OpenAI-Chat-Completions client, which can't
//! drive Anthropic's `/v1/messages` or the ChatGPT backend, so these accounts
//! can't actually run yet — the `/model` account tabs are informational and
//! point you at an API key in `config.acl`. Detection only (no token use).

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum AuthProvider {
    Claude,
    Codex,
}

impl AuthProvider {
    pub(crate) fn label(self) -> &'static str {
        match self {
            AuthProvider::Claude => "Claude Code",
            AuthProvider::Codex => "Codex",
        }
    }
}

/// True when the local Claude Code / Codex CLI has a stored login.
pub(crate) fn has_local_login(provider: AuthProvider) -> bool {
    let Some(home) = std::env::var_os("HOME") else {
        return false;
    };
    let home = std::path::Path::new(&home);
    let read = |rel: &str| -> Option<serde_json::Value> {
        serde_json::from_str(&std::fs::read_to_string(home.join(rel)).ok()?).ok()
    };
    match provider {
        AuthProvider::Codex => read(".codex/auth.json")
            .map(|v| v.pointer("/tokens/access_token").is_some() || v.get("access_token").is_some())
            .unwrap_or(false),
        AuthProvider::Claude => read(".claude/.credentials.json")
            .map(|v| v.pointer("/claudeAiOauth/accessToken").is_some())
            .unwrap_or(false),
    }
}
