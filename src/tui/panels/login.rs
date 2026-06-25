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

/// The Claude model(s) configured in the local Claude Code login
/// (`~/.claude.json`), found by walking the JSON for `"model": "claude-…"`.
pub(crate) fn claude_models() -> Vec<String> {
    fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(map) => {
                for (k, val) in map {
                    if k == "model" {
                        if let Some(s) = val.as_str() {
                            if s.starts_with("claude") && !out.iter().any(|m| m == s) {
                                out.push(s.to_string());
                            }
                        }
                    }
                    walk(val, out);
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let path = std::path::Path::new(&home).join(".claude.json");
        if let Ok(txt) = std::fs::read_to_string(path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                walk(&v, &mut out);
            }
        }
    }
    if out.is_empty() {
        out.push("claude-sonnet-4".to_string());
    }
    out
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
