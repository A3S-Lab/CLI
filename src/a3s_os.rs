//! OS account login helpers for the TUI.

use a3s_code_core::config::OsConfig;
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const OAUTH_CLIENT_ID: &str = "a3s-code";
const OAUTH_SCOPE: &str = "profile offline_access";
const STORE_FILE: &str = "os-auth.json";

/// Built-in `a3s-os-capabilities` skill that drives OS's progressive API —
/// the platform-wide kernel `capabilities` endpoint (POST /api/v1/kernel/
/// capabilities), spanning all domains, not just security. Materialized under
/// `~/.a3s/os-skills/` only when signed in; `{{BASE_URL}}` is replaced with the
/// configured OS address so the agent calls the right endpoint.
const CAPABILITY_SKILL: &str = include_str!("../skills/a3s-os-capabilities.md");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StoredOsSession {
    pub address: String,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    pub login_at_ms: u64,
}

impl StoredOsSession {
    pub(crate) fn display_label(&self) -> String {
        self.account_label
            .clone()
            .unwrap_or_else(|| self.address.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct OsAuthStore {
    #[serde(default)]
    sessions: Vec<StoredOsSession>,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenError {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

pub(crate) async fn login_via_browser(config: OsConfig) -> Result<StoredOsSession> {
    validate_address(&config.address)?;
    let state = random_url_token(32);
    let code_verifier = pkce_verifier();
    let code_challenge = pkce_challenge(&code_verifier);
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("bind local OS login callback")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let authorize_url =
        build_authorization_url(&config.address, &redirect_uri, &state, &code_challenge);
    open_browser(&authorize_url).with_context(|| {
        format!("open browser failed; visit this URL manually to continue: {authorize_url}")
    })?;

    let callback = tokio::time::timeout(CALLBACK_TIMEOUT, wait_for_callback(listener, &state))
        .await
        .map_err(|_| anyhow!("timed out waiting for OS login callback"))??;
    if let Some(error) = callback
        .get("error")
        .filter(|value| !value.trim().is_empty())
    {
        let description = callback
            .get("error_description")
            .map(String::as_str)
            .unwrap_or("OAuth2 authorization failed");
        anyhow::bail!("OS OAuth2 authorization failed: {error}: {description}");
    }
    let code = callback
        .get("code")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("OAuth2 callback did not include an authorization code"))?;
    let token =
        exchange_authorization_code(&config.address, code, &redirect_uri, &code_verifier).await?;
    let session = session_from_token_response(&config.address, token);
    save_session(&session)?;
    Ok(session)
}

pub(crate) fn login_with_token(config: &OsConfig, token: &str) -> Result<StoredOsSession> {
    validate_address(&config.address)?;
    let token = token.trim();
    if token.is_empty() {
        anyhow::bail!("token is empty");
    }
    let session = StoredOsSession {
        address: normalize_address(&config.address),
        access_token: token.to_string(),
        refresh_token: None,
        token_type: Some("Bearer".to_string()),
        expires_at_ms: None,
        account_label: None,
        login_at_ms: now_ms(),
    };
    save_session(&session)?;
    Ok(session)
}

pub(crate) fn logout(config: &OsConfig) -> Result<bool> {
    let path = auth_store_path()?;
    remove_session_at(&path, &normalize_address(&config.address))
}

/// Env vars the agent's `bash` inherits so it can call the progressive API
/// without re-reading `~/.a3s/os-auth.json` on every turn (the shell can't keep
/// state between tool calls, so the address/token would otherwise be looked up
/// each time). `spawn_shell` runs `bash` with the cli's process env (no
/// `env_clear`), so setting them here makes `$A3S_OS_BASE_URL` / `$A3S_OS_TOKEN`
/// available to every command.
pub(crate) const OS_ENV_BASE_URL: &str = "A3S_OS_BASE_URL";
pub(crate) const OS_ENV_TOKEN: &str = "A3S_OS_TOKEN";
pub(crate) const OS_ENV_REFRESH_TOKEN: &str = "A3S_OS_REFRESH_TOKEN";

/// Export the signed-in platform endpoint + tokens to the process env so the
/// agent's shell — and the RemoteUI webview helper, which inherits this env —
/// can use them directly. Called on login and on startup restore. The refresh
/// token lets the webview's seeded session survive an edge-expired access token.
pub(crate) fn export_os_env(session: &StoredOsSession) {
    std::env::set_var(OS_ENV_BASE_URL, &session.address);
    std::env::set_var(OS_ENV_TOKEN, &session.access_token);
    match &session.refresh_token {
        Some(rt) => std::env::set_var(OS_ENV_REFRESH_TOKEN, rt),
        None => std::env::remove_var(OS_ENV_REFRESH_TOKEN),
    }
}

/// Clear the exported platform env (called on /logout).
pub(crate) fn clear_os_env() {
    std::env::remove_var(OS_ENV_BASE_URL);
    std::env::remove_var(OS_ENV_TOKEN);
    std::env::remove_var(OS_ENV_REFRESH_TOKEN);
}

/// The stored session for the configured OS address, if the user logged in
/// on a previous run. This is the load-back half of `save_session`: without it a
/// persisted login is never restored, so the user has to `/login` every launch.
/// Best-effort — any read/parse error is treated as "not signed in".
pub(crate) fn current_session(config: &OsConfig) -> Option<StoredOsSession> {
    let path = auth_store_path().ok()?;
    current_session_at(&path, &normalize_address(&config.address))
}

fn current_session_at(path: &Path, address: &str) -> Option<StoredOsSession> {
    read_store(path)
        .ok()?
        .sessions
        .into_iter()
        .find(|s| s.address == address)
}

fn os_skills_root() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| anyhow!("HOME is not set; cannot install the OS skill"))?;
    Ok(Path::new(&home).join(".a3s").join("os-skills"))
}

/// Materialize the built-in `a3s-os-capabilities` skill (templated with the OS
/// base URL) under `~/.a3s/os-skills/` and return that directory so the caller
/// can add it to the session's skill dirs. Call only when signed in. Best-effort
/// — returns `None` on any I/O error rather than failing the launch.
pub(crate) fn ensure_capability_skill_dir(config: &OsConfig) -> Option<PathBuf> {
    let root = os_skills_root().ok()?;
    ensure_capability_skill_dir_at(&root, config).ok()?;
    Some(root)
}

fn ensure_capability_skill_dir_at(root: &Path, config: &OsConfig) -> Result<()> {
    let skill_dir = root.join("a3s-os-capabilities");
    std::fs::create_dir_all(&skill_dir)?;
    let body = CAPABILITY_SKILL.replace("{{BASE_URL}}", &normalize_address(&config.address));
    std::fs::write(skill_dir.join("SKILL.md"), body)?;
    Ok(())
}

/// Remove the materialized OS skill dir (called on /logout). Best-effort.
pub(crate) fn remove_capability_skill_dir() {
    if let Ok(root) = os_skills_root() {
        let _ = std::fs::remove_dir_all(root);
    }
}

fn session_from_token_response(
    configured_address: &str,
    token: OAuthTokenResponse,
) -> StoredOsSession {
    StoredOsSession {
        address: normalize_address(configured_address),
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        token_type: token.token_type.or_else(|| Some("Bearer".to_string())),
        expires_at_ms: token
            .expires_in
            .map(|seconds| now_ms().saturating_add(seconds.saturating_mul(1000))),
        account_label: None,
        login_at_ms: now_ms(),
    }
}

/// Outcome rendered on the localhost callback page after the OAuth redirect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginOutcome {
    Success,
    NotApproved,
    InvalidState,
}

/// The OS-branded page shown in the browser once the OAuth redirect lands back
/// on the local callback. Returns `(http_status, html_body)`.
fn login_callback_page(outcome: LoginOutcome) -> (&'static str, String) {
    let (status, heading, detail) = match outcome {
        LoginOutcome::Success => (
            "200 OK",
            "OS sign-in successful",
            "You are signed in. You can close this page and return to a3s code.",
        ),
        LoginOutcome::NotApproved => (
            "400 Bad Request",
            "OS sign-in not approved",
            "The authorization was not approved. You can close this page and return to a3s code.",
        ),
        LoginOutcome::InvalidState => (
            "400 Bad Request",
            "Invalid sign-in state",
            "Sign-in state validation failed. Return to a3s code and run /login again.",
        ),
    };
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>OS sign-in</title></head>\
         <body style=\"margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;\
         font-family:system-ui,-apple-system,'PingFang SC','Microsoft YaHei',sans-serif;\
         background:#0f172a;color:#e2e8f0\">\
         <div style=\"text-align:center;padding:2rem\">\
         <h1 style=\"font-size:1.5rem;margin:0 0 .75rem\">{heading}</h1>\
         <p style=\"margin:0;color:#94a3b8\">{detail}</p></div></body></html>"
    );
    (status, body)
}

async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<BTreeMap<String, String>> {
    // Browsers don't make exactly one request to the redirect URI: Chrome opens
    // speculative *preconnect* sockets (no bytes sent) and fetches /favicon.ico,
    // and any of those can land before the real ?code=...&state=... redirect.
    // Accepting only once meant a preconnect/favicon consumed the single accept
    // (empty read -> missing-state bail, or a 10s read-timeout error), the
    // listener was dropped, and the real callback then hit a dead port — the
    // "redirects back but can't be reached" bug. So loop: skip non-OAuth
    // requests (replying so the socket closes cleanly) and keep the listener
    // alive until the OAuth params actually arrive. The outer CALLBACK_TIMEOUT
    // bounds the whole wait.
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut buf = [0_u8; 8192];
        // A preconnect socket sends nothing: a per-connection read timeout (or a
        // 0-byte read) must NOT kill the flow — just move on to the next socket.
        let n = match tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => n,
            _ => continue,
        };
        let request = String::from_utf8_lossy(&buf[..n]);
        let target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");
        let query = target.split_once('?').map(|(_, query)| query).unwrap_or("");
        let params = parse_query(query);

        // Not the OAuth redirect (favicon, "/", a bare preconnect that did send a
        // request line): acknowledge it and keep waiting for the real callback.
        if !params.contains_key("state")
            && !params.contains_key("code")
            && !params.contains_key("error")
        {
            let _ = stream
                .write_all(b"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n")
                .await;
            continue;
        }

        let state = params.get("state").cloned().unwrap_or_default();
        let error = params.get("error").filter(|value| !value.trim().is_empty());
        let outcome = if state == expected_state && error.is_none() {
            LoginOutcome::Success
        } else if state == expected_state {
            LoginOutcome::NotApproved
        } else {
            LoginOutcome::InvalidState
        };
        let (status, body) = login_callback_page(outcome);
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;

        if state != expected_state {
            anyhow::bail!("login callback state mismatch");
        }
        return Ok(params);
    }
}

async fn exchange_authorization_code(
    address: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<OAuthTokenResponse> {
    let token_url = build_token_url(address);
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .context("build OS OAuth2 HTTP client")?;
    let response = client
        .post(&token_url)
        .header("accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", OAUTH_CLIENT_ID),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .with_context(|| format!("send OAuth2 token request to {token_url}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("read OAuth2 token response body")?;

    if !status.is_success() {
        if let Ok(error) = serde_json::from_str::<OAuthTokenError>(&body) {
            let description = error.error_description.unwrap_or_default();
            anyhow::bail!(
                "OAuth2 token exchange failed at {token_url} (HTTP {}): {} {}",
                status.as_u16(),
                error.error,
                description
            );
        }
        anyhow::bail!(
            "OAuth2 token exchange failed at {token_url} (HTTP {}): {body}",
            status.as_u16()
        );
    }

    serde_json::from_str::<OAuthTokenResponse>(&body)
        .with_context(|| format!("parse OAuth2 token response from {token_url}"))
}

/// Refresh proactively this many ms before the access token expires, so an
/// in-flight progressive-API call never races an expiry.
const REFRESH_SKEW_MS: u64 = 120_000; // 2 minutes

/// True if the session both *can* be refreshed (has a refresh token + known
/// expiry) and *should* be now (expiry within `REFRESH_SKEW_MS`, or already past).
pub(crate) fn needs_refresh(session: &StoredOsSession) -> bool {
    session.refresh_token.is_some()
        && session
            .expires_at_ms
            .is_some_and(|exp| now_ms().saturating_add(REFRESH_SKEW_MS) >= exp)
}

/// Exchange the stored refresh token for a fresh access token and persist the
/// updated session. Preserves the existing refresh token / account label when the
/// server doesn't re-issue them (refresh-token rotation is optional in OAuth2).
/// Call only when [`needs_refresh`] is true.
pub(crate) async fn refresh_session(session: &StoredOsSession) -> Result<StoredOsSession> {
    let refresh_token = session
        .refresh_token
        .clone()
        .ok_or_else(|| anyhow!("no refresh token to refresh with"))?;
    let token = exchange_refresh_token(&session.address, &refresh_token).await?;
    let mut next = session_from_token_response(&session.address, token);
    if next.refresh_token.is_none() {
        next.refresh_token = Some(refresh_token);
    }
    if next.account_label.is_none() {
        next.account_label = session.account_label.clone();
    }
    save_session(&next)?;
    Ok(next)
}

async fn exchange_refresh_token(address: &str, refresh_token: &str) -> Result<OAuthTokenResponse> {
    let token_url = build_token_url(address);
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .context("build OS OAuth2 HTTP client")?;
    let response = client
        .post(&token_url)
        .header("accept", "application/json")
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", OAUTH_CLIENT_ID),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .with_context(|| format!("send OAuth2 refresh request to {token_url}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("read OAuth2 refresh response body")?;

    if !status.is_success() {
        if let Ok(error) = serde_json::from_str::<OAuthTokenError>(&body) {
            let description = error.error_description.unwrap_or_default();
            anyhow::bail!(
                "OAuth2 refresh failed at {token_url} (HTTP {}): {} {}",
                status.as_u16(),
                error.error,
                description
            );
        }
        anyhow::bail!(
            "OAuth2 refresh failed at {token_url} (HTTP {}): {body}",
            status.as_u16()
        );
    }

    serde_json::from_str::<OAuthTokenResponse>(&body)
        .with_context(|| format!("parse OAuth2 refresh response from {token_url}"))
}

/// The OS origin (`scheme://host[:port]`). The unified AI gateway's
/// OpenAI-compatible API is host-absolute (`/v1/chat/completions`), so we route
/// to the origin, not the (possibly path-suffixed) configured platform address.
pub(crate) fn os_origin(address: &str) -> String {
    match address.find("://") {
        Some(scheme_end) => {
            let after = &address[scheme_end + 3..];
            let end = after
                .find('/')
                .map_or(address.len(), |j| scheme_end + 3 + j);
            address[..end].to_string()
        }
        None => address.trim_end_matches('/').to_string(),
    }
}

/// One model advertised by the OS unified AI gateway, plus its real context
/// window when the gateway reports it (so the CLI can size auto-compact + the
/// status bar correctly instead of assuming a default).
#[derive(Clone, Debug)]
pub(crate) struct GatewayModel {
    pub id: String,
    pub context: Option<u32>,
}

/// List the OS unified AI gateway's models via the OpenAI-compatible
/// `GET {origin}/v1/models` (Bearer = the OS token). The gateway is
/// "gateway-managed" (it holds the real provider keys; callers send only the OS
/// token + a model id).
///
/// Returns a precise `Err` on failure so the `/model` picker can say WHY the
/// gateway is unavailable — in particular it distinguishes an HTML/SPA response
/// (the OS origin doesn't proxy `/v1/*` to the gateway) from a genuine empty
/// list, an auth error, or an unreachable host. `Ok(vec![])` means the gateway
/// really has no models configured.
pub(crate) async fn fetch_gateway_models(
    address: &str,
    token: &str,
) -> Result<Vec<GatewayModel>, String> {
    // Go through the OS backend's authenticated proxy (`/api/v1/llm/*`), which
    // validates the OS token then forwards to the internal gateway. `/api` is
    // reverse-proxied, unlike a bare `/v1`.
    let url = format!("{}/api/v1/llm/models", os_origin(address));
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .bearer_auth(token)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("request to {url} failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("{url} returned HTTP {}", status.as_u16()));
    }
    // OpenAI shape: { "data": [ { "id": "...", <optional context field> }, ... ] }.
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
        if text.trim_start().starts_with('<') {
            format!(
                "{url} returned HTML, not JSON — the OS build may predate the LLM gateway proxy (/api/v1/llm)"
            )
        } else {
            format!("{url} returned a non-JSON response")
        }
    })?;
    let data = json
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| format!("{url} response had no `data` array"))?;
    Ok(data
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(|i| i.as_str())?.to_string();
            Some(GatewayModel {
                id,
                context: parse_gateway_context(m),
            })
        })
        .collect())
}

/// Opportunistically read a model's context window from the several field names
/// OpenAI-compatible gateways use (LiteLLM / one-api / vLLM differ). Absent →
/// `None`, and the caller keeps its default. Never fails.
fn parse_gateway_context(m: &serde_json::Value) -> Option<u32> {
    const KEYS: &[&str] = &[
        "context_length",
        "max_context_length",
        "max_model_len",
        "context_window",
        "max_input_tokens",
    ];
    for key in KEYS {
        if let Some(n) = m.get(key).and_then(|v| v.as_u64()).filter(|n| *n > 0) {
            return Some(n as u32);
        }
    }
    // LiteLLM nests it under `model_info`.
    m.get("model_info")
        .and_then(|mi| mi.get("max_input_tokens"))
        .and_then(|v| v.as_u64())
        .filter(|n| *n > 0)
        .map(|n| n as u32)
}

fn build_authorization_url(
    address: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> String {
    let base = normalize_address(address);
    let authorize =
        if base.contains('?') || base.trim_end_matches('/').ends_with("/oauth/authorize") {
            base
        } else {
            format!("{}/oauth/authorize", base.trim_end_matches('/'))
        };
    let sep = if authorize.contains('?') { '&' } else { '?' };
    format!(
        "{authorize}{sep}response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256&scope={}",
        percent_encode(OAUTH_CLIENT_ID),
        percent_encode(redirect_uri),
        percent_encode(state),
        percent_encode(code_challenge),
        percent_encode(OAUTH_SCOPE)
    )
}

fn build_token_url(address: &str) -> String {
    let base = normalize_address(address);
    if base.ends_with("/api/v1") {
        format!("{base}/oauth/token")
    } else {
        format!("{}/api/v1/oauth/token", base.trim_end_matches('/'))
    }
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(url).status();
    #[cfg(target_os = "linux")]
    let status = std::process::Command::new("xdg-open").arg(url).status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .status();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let status: std::io::Result<std::process::ExitStatus> = Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        "unsupported OS",
    ));

    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(anyhow!("browser opener exited with {status}")),
        Err(error) => Err(error.into()),
    }
}

fn save_session(session: &StoredOsSession) -> Result<()> {
    let path = auth_store_path()?;
    save_session_at(&path, session)
}

fn save_session_at(path: &Path, session: &StoredOsSession) -> Result<()> {
    let mut store = read_store(path)?;
    store
        .sessions
        .retain(|item| item.address != session.address);
    store.sessions.push(session.clone());
    store.sessions.sort_by(|a, b| a.address.cmp(&b.address));
    write_store(path, &store)
}

fn remove_session_at(path: &Path, address: &str) -> Result<bool> {
    let mut store = read_store(path)?;
    let before = store.sessions.len();
    store.sessions.retain(|item| item.address != address);
    let removed = store.sessions.len() != before;
    if store.sessions.is_empty() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(removed);
    }
    write_store(path, &store)?;
    Ok(removed)
}

fn read_store(path: &Path) -> Result<OsAuthStore> {
    if !path.exists() {
        return Ok(OsAuthStore::default());
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read OS auth store {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse OS auth store {}", path.display()))
}

fn write_store(path: &Path, store: &OsAuthStore) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(store)?;
    std::fs::write(path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn auth_store_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| anyhow!("HOME is not set; cannot store OS login"))?;
    Ok(Path::new(&home).join(".a3s").join(STORE_FILE))
}

fn validate_address(address: &str) -> Result<()> {
    let address = address.trim();
    if address.starts_with("https://") || address.starts_with("http://") {
        Ok(())
    } else {
        anyhow::bail!("OS address must start with http:// or https://");
    }
}

fn normalize_address(address: &str) -> String {
    address.trim().trim_end_matches('/').to_string()
}

fn parse_query(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            Some((percent_decode(key)?, percent_decode(value)?))
        })
        .collect()
}

fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'%' => return None,
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn pkce_verifier() -> String {
    random_url_token(32)
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn random_url_token(bytes_len: usize) -> String {
    let mut bytes = vec![0_u8; bytes_len];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gateway_context_from_common_fields() {
        let j = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        // OpenAI-compatible variants seen across gateways.
        assert_eq!(parse_gateway_context(&j(r#"{"context_length":200000}"#)), Some(200000));
        assert_eq!(parse_gateway_context(&j(r#"{"max_model_len":32768}"#)), Some(32768));
        assert_eq!(parse_gateway_context(&j(r#"{"context_window":128000}"#)), Some(128000));
        // LiteLLM nests it.
        assert_eq!(parse_gateway_context(&j(r#"{"model_info":{"max_input_tokens":1000000}}"#)), Some(1_000_000));
        // Absent or zero → None so the caller keeps its default.
        assert_eq!(parse_gateway_context(&j(r#"{"id":"m"}"#)), None);
        assert_eq!(parse_gateway_context(&j(r#"{"context_length":0}"#)), None);
    }

    #[test]
    fn os_origin_strips_any_path_for_the_gateway() {
        // The gateway endpoint is host-absolute (/v1/chat/completions), so the
        // OpenAI base must be the bare origin regardless of the platform path.
        assert_eq!(
            os_origin("https://os.example.com"),
            "https://os.example.com"
        );
        assert_eq!(
            os_origin("https://os.example.com/"),
            "https://os.example.com"
        );
        assert_eq!(
            os_origin("https://os.example.com/api/v1"),
            "https://os.example.com"
        );
        assert_eq!(os_origin("http://10.0.0.1:3888/x"), "http://10.0.0.1:3888");
    }

    #[test]
    fn authorization_url_uses_oauth2_code_flow_with_pkce() {
        let url = build_authorization_url(
            "https://os.example.test/",
            "http://127.0.0.1:1234/callback",
            "state 1",
            "challenge-abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOP",
        );

        assert!(url.starts_with("https://os.example.test/oauth/authorize?"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=a3s-code"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1234%2Fcallback"));
        assert!(url.contains("state=state%201"));
        assert!(url.contains("code_challenge=challenge-abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOP"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("scope=profile%20offline_access"));
    }

    #[test]
    fn token_url_targets_standard_oauth2_token_endpoint() {
        assert_eq!(
            build_token_url("https://os.example.test/"),
            "https://os.example.test/api/v1/oauth/token"
        );
        assert_eq!(
            build_token_url("https://os.example.test/api/v1"),
            "https://os.example.test/api/v1/oauth/token"
        );
    }

    #[test]
    fn builds_session_from_oauth2_token_response() {
        let session = session_from_token_response(
            "https://os.example.test",
            OAuthTokenResponse {
                access_token: "tok 1".to_string(),
                refresh_token: Some("ref".to_string()),
                token_type: Some("Bearer".to_string()),
                expires_in: Some(60),
            },
        );

        assert_eq!(session.address, "https://os.example.test");
        assert_eq!(session.access_token, "tok 1");
        assert_eq!(session.refresh_token.as_deref(), Some("ref"));
        assert!(session.expires_at_ms.is_some());
    }

    #[test]
    fn needs_refresh_only_when_expiring_with_a_refresh_token() {
        let base = StoredOsSession {
            address: "https://os.example.test".to_string(),
            access_token: "a".to_string(),
            refresh_token: Some("r".to_string()),
            token_type: None,
            expires_at_ms: None,
            account_label: None,
            login_at_ms: 0,
        };
        // Unknown expiry → can't tell it's expiring → don't refresh.
        assert!(!needs_refresh(&base));
        // Far in the future → not yet.
        assert!(!needs_refresh(&StoredOsSession {
            expires_at_ms: Some(now_ms() + 3_600_000),
            ..base.clone()
        }));
        // Inside the skew window (or already past) → refresh.
        assert!(needs_refresh(&StoredOsSession {
            expires_at_ms: Some(now_ms() + 10_000),
            ..base.clone()
        }));
        // Expiring but no refresh token → nothing we can do.
        assert!(!needs_refresh(&StoredOsSession {
            refresh_token: None,
            expires_at_ms: Some(now_ms() + 10_000),
            ..base.clone()
        }));
    }

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        let challenge = pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn store_replaces_and_removes_sessions_by_address() {
        let dir = tempfile_dir("a3s-os-auth-test");
        let path = dir.join(STORE_FILE);
        let first = StoredOsSession {
            address: "https://os.example.test".to_string(),
            access_token: "one".to_string(),
            refresh_token: None,
            token_type: Some("Bearer".to_string()),
            expires_at_ms: None,
            account_label: None,
            login_at_ms: 1,
        };
        let second = StoredOsSession {
            access_token: "two".to_string(),
            login_at_ms: 2,
            ..first.clone()
        };

        save_session_at(&path, &first).unwrap();
        save_session_at(&path, &second).unwrap();
        let store = read_store(&path).unwrap();
        assert_eq!(store.sessions.len(), 1);
        assert_eq!(store.sessions[0].access_token, "two");

        assert!(remove_session_at(&path, "https://os.example.test").unwrap());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn current_session_restores_persisted_login_and_clears_on_logout() {
        let dir = tempfile_dir("a3s-os-auth-restore");
        let path = dir.join(STORE_FILE);
        let addr = "https://os.example.test";

        // Nothing stored yet → signed out.
        assert!(current_session_at(&path, addr).is_none());

        let session = StoredOsSession {
            address: addr.to_string(),
            access_token: "tok".to_string(),
            refresh_token: None,
            token_type: Some("Bearer".to_string()),
            expires_at_ms: None,
            account_label: Some("alice".to_string()),
            login_at_ms: 1,
        };
        save_session_at(&path, &session).unwrap();

        // Persisted login is restored across "runs".
        let restored = current_session_at(&path, addr).expect("login should be remembered");
        assert_eq!(restored.access_token, "tok");
        assert_eq!(restored.display_label(), "alice");

        // A different address does not match.
        assert!(current_session_at(&path, "https://other.example").is_none());

        // /logout clears the remembered login.
        assert!(remove_session_at(&path, addr).unwrap());
        assert!(current_session_at(&path, addr).is_none());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn capability_skill_materializes_templated_and_is_discoverable() {
        let dir = tempfile_dir("a3s-os-skill");
        let config = OsConfig {
            address: "https://os.example.test/".to_string(),
        };
        ensure_capability_skill_dir_at(&dir, &config).unwrap();

        // The cli skill loader discovers it by name (this is "in effect").
        let skills = crate::tui::skills::load_skills(std::slice::from_ref(&dir));
        assert!(
            skills.iter().any(|(n, _)| n == "a3s-os-capabilities"),
            "a3s-os-capabilities skill not discovered: {skills:?}"
        );

        // Base URL templated in; no placeholder left.
        let md = std::fs::read_to_string(dir.join("a3s-os-capabilities/SKILL.md")).unwrap();
        assert!(md.contains("https://os.example.test"));
        assert!(!md.contains("{{BASE_URL}}"));

        // Definitive "in effect": the *core* skill loader (stricter than the cli's
        // menu parser — validates kind + fail-secure allowed-tools + 10KiB body)
        // accepts it. If this parsed to None the skill would silently not load.
        let skill = a3s_code_core::skills::Skill::parse(&md)
            .expect("core skill loader must accept the materialized SKILL.md");
        assert_eq!(skill.name, "a3s-os-capabilities");
        assert!(
            skill.allowed_tools.is_some(),
            "allowed-tools must parse (fail-secure) so the skill is usable"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn login_callback_page_is_chinese_and_branded() {
        for outcome in [
            LoginOutcome::Success,
            LoginOutcome::NotApproved,
            LoginOutcome::InvalidState,
        ] {
            let (_, body) = login_callback_page(outcome);
            assert!(body.contains("OS"), "missing OS branding: {outcome:?}");
            assert!(
                body.contains("sign-in") || body.contains("sign in"),
                "page should describe the sign-in outcome: {outcome:?}"
            );
            assert!(body.starts_with("<!doctype html>"), "not an HTML page");
            assert!(body.contains("charset=\"utf-8\""), "missing utf-8 charset");
        }
        let (status, body) = login_callback_page(LoginOutcome::Success);
        assert_eq!(status, "200 OK");
        assert!(body.contains("sign-in successful"));
        assert_eq!(
            login_callback_page(LoginOutcome::InvalidState).0,
            "400 Bad Request"
        );
    }

    // Regression: a browser preconnect (empty socket) and a favicon request
    // arriving BEFORE the real ?code=...&state=... redirect must not kill the
    // callback — the listener has to survive them. This is the "redirects back
    // but can't be reached" bug.
    #[tokio::test]
    async fn wait_for_callback_survives_preconnect_and_favicon() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpStream;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move { wait_for_callback(listener, "state-xyz").await });

        // 1) preconnect: open then immediately close, sending no bytes (EOF read).
        TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        // 2) favicon: a real request line but no OAuth params.
        let mut fav = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        fav.write_all(b"GET /favicon.ico HTTP/1.1\r\nhost: x\r\n\r\n")
            .await
            .unwrap();
        // 3) the real OAuth redirect.
        let mut cb = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        cb.write_all(b"GET /callback?code=abc&state=state-xyz HTTP/1.1\r\nhost: x\r\n\r\n")
            .await
            .unwrap();

        let params = task.await.unwrap().expect("callback should succeed");
        assert_eq!(params.get("code").map(String::as_str), Some("abc"));
        assert_eq!(params.get("state").map(String::as_str), Some("state-xyz"));
    }

    fn tempfile_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{name}-{}-{}", std::process::id(), now_ms()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}
