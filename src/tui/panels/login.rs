//! `/login`: full-screen account sign-in (Phase 0 — paste a token).
//!
//! Codex (ChatGPT account) is the supported path: the pasted token is injected
//! as `Authorization: Bearer` to the ChatGPT backend through the OpenAI-compatible
//! client (which honors custom headers). Claude is experimental — api.anthropic.com
//! speaks a different wire format than the OpenAI client, so account tokens won't
//! work until a dedicated Anthropic client lands.

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

fn provider_at(sel: usize) -> AuthProvider {
    if sel == 0 {
        AuthProvider::Claude
    } else {
        AuthProvider::Codex
    }
}

pub(crate) enum LoginPhase {
    Pick,
    Paste,
    Done(String),
    Error(String),
}

pub(crate) struct Login {
    pub(crate) sel: usize, // 0 Claude, 1 Codex
    pub(crate) phase: LoginPhase,
    pub(crate) input: String,
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

fn save_creds(provider: AuthProvider, token: &str) {
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
/// `/login` doesn't make the user paste anything when they're already logged in.
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
    /// An OpenAI-compatible client carrying the OAuth Bearer token, if signed in.
    /// Both providers route through the OpenAI client (it honors a custom
    /// `Authorization` header). The empty default api_key is never sent — the
    /// header wins.
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

    /// Keys while the `/login` panel is open.
    pub(crate) fn login_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let login = self.login.as_mut()?;
        match &login.phase {
            LoginPhase::Pick => match key.code {
                KeyCode::Up => login.sel = login.sel.saturating_sub(1),
                KeyCode::Down => login.sel = (login.sel + 1).min(1),
                KeyCode::Enter => {
                    let provider = provider_at(login.sel);
                    // Reuse the local Claude Code / Codex login if present;
                    // otherwise fall back to pasting a token.
                    if let Some(token) = detect_local(provider) {
                        self.finish_login(provider, token, "local login");
                    } else if let Some(l) = self.login.as_mut() {
                        l.phase = LoginPhase::Paste;
                    }
                }
                KeyCode::Esc => self.login = None,
                _ => {}
            },
            LoginPhase::Paste => match key.code {
                KeyCode::Char(c) => login.input.push(c),
                KeyCode::Backspace => {
                    login.input.pop();
                }
                KeyCode::Esc => self.login = None,
                KeyCode::Enter => {
                    let provider = provider_at(login.sel);
                    let token = login.input.trim().to_string();
                    if token.is_empty() {
                        login.phase = LoginPhase::Error("no token entered".into());
                    } else {
                        self.finish_login(provider, token, "pasted token");
                    }
                }
                _ => {}
            },
            LoginPhase::Done(_) | LoginPhase::Error(_) => self.login = None,
        }
        None
    }

    /// Persist the token, set it active, rebuild the session, and report.
    fn finish_login(&mut self, provider: AuthProvider, token: String, via: &str) {
        save_creds(provider, &token);
        self.auth = Some((provider, token));
        match self.rebuild_session(None) {
            Ok((s, _)) => {
                self.session = Arc::new(s);
                let who = if provider == AuthProvider::Codex {
                    "Codex"
                } else {
                    "Claude (experimental)"
                };
                if let Some(l) = self.login.as_mut() {
                    l.phase = LoginPhase::Done(format!("signed in · {who} · {via}"));
                }
            }
            Err(e) => {
                if let Some(l) = self.login.as_mut() {
                    l.phase = LoginPhase::Error(e);
                }
            }
        }
    }

    /// Full-screen `/login` render (mirrors the other panels' full-height view).
    pub(crate) fn render_login(&self, login: &Login) -> String {
        let w = self.width as usize;
        let mut lines: Vec<String> = vec![
            String::new(),
            format!("  {}", Style::new().fg(ACCENT).bold().render("Sign in")),
            String::new(),
        ];
        match &login.phase {
            LoginPhase::Pick => {
                for i in 0..2 {
                    let provider = provider_at(i);
                    let label = if i == 0 {
                        "Claude account (subscription)"
                    } else {
                        "Codex / ChatGPT account"
                    };
                    let detected = detect_local(provider).is_some();
                    let note = match (i, detected) {
                        (0, true) => "✓ found local login · experimental",
                        (0, false) => "paste a token · experimental",
                        (_, true) => "✓ using your local Codex login",
                        (_, false) => "paste a token",
                    };
                    let marker = if i == login.sel { "❯" } else { " " };
                    let row = if i == login.sel {
                        Style::new()
                            .fg(Color::BrightWhite)
                            .bold()
                            .render(&format!("  {marker} {label}"))
                    } else {
                        Style::new()
                            .fg(Color::White)
                            .render(&format!("  {marker} {label}"))
                    };
                    let note_color = if detected {
                        Color::Green
                    } else {
                        Color::BrightBlack
                    };
                    lines.push(format!(
                        "{row}  {}",
                        Style::new().fg(note_color).render(note)
                    ));
                }
                lines.push(String::new());
                lines.push(format!(
                    "  {}",
                    Style::new()
                        .fg(Color::BrightBlack)
                        .render("↑/↓ select · enter · esc cancel")
                ));
            }
            LoginPhase::Paste => {
                let (who, hint) = if login.sel == 0 {
                    ("Claude", "run `claude setup-token` and paste it here")
                } else {
                    ("Codex", "paste your Codex / ChatGPT access token")
                };
                lines.push(format!(
                    "  {} {}",
                    Style::new().bold().render(who),
                    Style::new()
                        .fg(Color::BrightBlack)
                        .render(&format!("— {hint}"))
                ));
                lines.push(String::new());
                let masked = "•".repeat(login.input.chars().count().min(48));
                lines.push(format!("  {}▏", Style::new().fg(ACCENT).render(&masked)));
                lines.push(String::new());
                lines.push(format!(
                    "  {}",
                    Style::new()
                        .fg(Color::BrightBlack)
                        .render("enter to save · esc cancel")
                ));
            }
            LoginPhase::Done(m) => {
                lines.push(format!(
                    "  {} {m}",
                    Style::new().fg(Color::Green).bold().render("✓")
                ));
                lines.push(String::new());
                lines.push(format!(
                    "  {}",
                    Style::new().fg(Color::BrightBlack).render("press any key")
                ));
            }
            LoginPhase::Error(m) => {
                lines.push(format!(
                    "  {} {m}",
                    Style::new().fg(Color::Red).bold().render("✗")
                ));
                lines.push(String::new());
                lines.push(format!(
                    "  {}",
                    Style::new().fg(Color::BrightBlack).render("press any key")
                ));
            }
        }
        while lines.len() < self.height as usize {
            lines.push(String::new());
        }
        lines.truncate(self.height as usize);
        lines
            .iter()
            .map(|l| pad_to(l, w))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
