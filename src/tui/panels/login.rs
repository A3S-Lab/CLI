//! Account auth for `/model`: detect a local Claude Code / Codex login, persist
//! the active choice, and build an OpenAI-compatible client that injects the
//! account's Bearer token. (There's no `/login` command — `/model` drives this
//! via tabs once a local login is detected.)
//!
//! Codex (ChatGPT account) is the supported path: the token goes as
//! `Authorization: Bearer` to the ChatGPT backend through the OpenAI client
//! (which honors custom headers). Claude is experimental — api.anthropic.com
//! speaks a different wire format than the OpenAI client, so its account token
//! won't work until a dedicated Anthropic client lands.

use super::super::*;
use a3s_code_core::llm::{create_client_with_config, LlmClient, LlmConfig};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum AuthProvider {
    Claude,
    Codex,
}

impl AuthProvider {
    fn key(self) -> &'static str {
        match self {
            AuthProvider::Claude => "claude",
            AuthProvider::Codex => "codex",
        }
    }
    fn from_key(k: &str) -> Option<Self> {
        match k {
            "claude" => Some(AuthProvider::Claude),
            "codex" => Some(AuthProvider::Codex),
            _ => None,
        }
    }
    fn default_model(self) -> &'static str {
        match self {
            AuthProvider::Claude => "claude-sonnet-4-20250514",
            AuthProvider::Codex => "gpt-5-codex",
        }
    }
    /// base_url + headers for the OpenAI-compatible client.
    fn wiring(self, token: &str) -> (String, Vec<(String, String)>) {
        match self {
            AuthProvider::Codex => (
                "https://chatgpt.com/backend-api/codex".into(),
                vec![("Authorization".into(), format!("Bearer {token}"))],
            ),
            AuthProvider::Claude => (
                "https://api.anthropic.com".into(),
                vec![
                    ("Authorization".into(), format!("Bearer {token}")),
                    ("anthropic-version".into(), "2023-06-01".into()),
                    (
                        "anthropic-beta".into(),
                        "oauth-2025-04-20,claude-code-20250219".into(),
                    ),
                ],
            ),
        }
    }
}

// ---- credential store: ~/.a3s/credentials.json (mode 0600) ----
fn creds_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".a3s/credentials.json"))
}

pub(crate) fn load_creds() -> Option<(AuthProvider, String)> {
    let p = creds_path()?;
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
    let provider = AuthProvider::from_key(v.get("provider")?.as_str()?)?;
    let token = v.get("token")?.as_str()?.to_string();
    Some((provider, token))
}

pub(crate) fn save_creds(provider: AuthProvider, token: &str) {
    if let Some(p) = creds_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let body = serde_json::json!({ "provider": provider.key(), "token": token }).to_string();
        if std::fs::write(&p, &body).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
}

/// Reuse an already-signed-in token from the local Claude Code / Codex CLIs, so
/// `/model` can offer those accounts without the user pasting anything.
pub(crate) fn detect_local(provider: AuthProvider) -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let home = std::path::Path::new(&home);
    let read = |rel: &str| -> Option<serde_json::Value> {
        serde_json::from_str(&std::fs::read_to_string(home.join(rel)).ok()?).ok()
    };
    match provider {
        AuthProvider::Codex => {
            let v = read(".codex/auth.json")?;
            v.pointer("/tokens/access_token")
                .or_else(|| v.get("access_token"))
                .and_then(|x| x.as_str())
                .map(String::from)
        }
        AuthProvider::Claude => {
            let v = read(".claude/.credentials.json")?;
            v.pointer("/claudeAiOauth/accessToken")
                .and_then(|x| x.as_str())
                .map(String::from)
        }
    }
}

impl App {
    /// An OpenAI-compatible client carrying the account Bearer token, if signed
    /// in via `/model`. Both providers route through the OpenAI client (it
    /// honors a custom `Authorization` header). The empty default api_key is
    /// never sent — the header wins.
    pub(crate) fn auth_client(&self) -> Option<Arc<dyn LlmClient>> {
        let (provider, token) = self.auth.as_ref()?;
        let model = self
            .model
            .as_ref()
            .map(|m| m.rsplit('/').next().unwrap_or(m).to_string())
            .unwrap_or_else(|| provider.default_model().to_string());
        let (base_url, headers) = provider.wiring(token);
        let cfg = LlmConfig {
            provider: "openai".into(),
            model,
            base_url: Some(base_url),
            headers: headers.into_iter().collect(),
            ..LlmConfig::default()
        };
        Some(create_client_with_config(cfg))
    }
}
