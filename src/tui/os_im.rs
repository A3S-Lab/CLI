//! REST client for the OS instant-messaging API (`/api/v1/im/...`).
//!
//! The TUI chat page (`/im`) is a STANDALONE surface: it talks to the OS IM
//! endpoints directly and never routes chat content through the coding agent's
//! session. Transport is plain REST + polling for now; the OS also exposes a
//! socket.io `/ws/im` gateway for true push, which a later revision can adopt
//! (the CLI has no socket.io client today).

use std::time::Duration;

use crate::a3s_os::os_origin;

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// One message in a conversation.
#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImMessage {
    pub id: String,
    pub conversation_id: String,
    pub sender_id: String,
    /// Sender's display name (server-resolved); may be absent on older servers.
    #[serde(default)]
    pub sender_name: Option<String>,
    pub content: String,
    #[serde(default)]
    pub kind: String,
    pub created_at: String,
}

/// A conversation plus its list read-model (last message + unread count).
#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImConversation {
    pub id: String,
    pub kind: String,
    pub title: Option<String>,
    #[serde(default)]
    pub member_ids: Vec<String>,
    pub last_message: Option<ImMessage>,
    #[serde(default)]
    pub unread_count: i64,
}

/// A person the user may start a chat with (a co-member of one of their orgs).
#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Contact {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub email: String,
}

#[derive(serde::Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(serde::Deserialize)]
struct Page<T> {
    items: Vec<T>,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

fn base(address: &str) -> String {
    format!("{}/api/v1/im/conversations", os_origin(address))
}

/// GET the caller's conversations (newest activity first).
pub(crate) async fn list_conversations(
    address: &str,
    token: &str,
) -> Result<Vec<ImConversation>, String> {
    let resp = client()?
        .get(format!("{}?limit=100", base(address)))
        .bearer_auth(token)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let resp = ok(resp).await?;
    let env: Envelope<Page<ImConversation>> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(env.data.items)
}

/// GET a conversation's messages. The API returns newest-first; we reverse to
/// oldest-first so the chat pane reads top-to-bottom.
pub(crate) async fn history(
    address: &str,
    token: &str,
    conversation_id: &str,
) -> Result<Vec<ImMessage>, String> {
    let resp = client()?
        .get(format!(
            "{}/{conversation_id}/messages?limit=100",
            base(address)
        ))
        .bearer_auth(token)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let resp = ok(resp).await?;
    let env: Envelope<Page<ImMessage>> = resp.json().await.map_err(|e| e.to_string())?;
    let mut items = env.data.items;
    items.reverse();
    Ok(items)
}

/// POST a message into a conversation. Returns the persisted message.
pub(crate) async fn send(
    address: &str,
    token: &str,
    conversation_id: &str,
    content: &str,
) -> Result<ImMessage, String> {
    let resp = client()?
        .post(format!("{}/{conversation_id}/messages", base(address)))
        .bearer_auth(token)
        .json(&serde_json::json!({ "content": content }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let resp = ok(resp).await?;
    let env: Envelope<ImMessage> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(env.data)
}

/// POST to open (or create) the DM with `user_id`.
pub(crate) async fn open_dm(
    address: &str,
    token: &str,
    user_id: &str,
) -> Result<ImConversation, String> {
    let resp = client()?
        .post(format!("{}/dm", base(address)))
        .bearer_auth(token)
        .json(&serde_json::json!({ "userId": user_id }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let resp = ok(resp).await?;
    let env: Envelope<ImConversation> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(env.data)
}

/// Resolve the signed-in user's own id (`GET /api/v1/users/me` → `data.id`), so
/// the chat page can mark and align the user's own messages.
pub(crate) async fn whoami(address: &str, token: &str) -> Result<String, String> {
    let resp = client()?
        .get(format!("{}/api/v1/users/me", os_origin(address)))
        .bearer_auth(token)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let resp = ok(resp).await?;
    let env: Envelope<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
    env.data
        .get("id")
        .and_then(|i| i.as_str())
        .map(String::from)
        .ok_or_else(|| "no id in /users/me".to_string())
}

/// GET the contact directory (org co-members), optionally filtered by `query`.
pub(crate) async fn contacts(
    address: &str,
    token: &str,
    query: &str,
) -> Result<Vec<Contact>, String> {
    let url = format!(
        "{}/api/v1/im/contacts?query={}",
        os_origin(address),
        urlencoding_min(query),
    );
    let resp = client()?
        .get(url)
        .bearer_auth(token)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let resp = ok(resp).await?;
    let env: Envelope<Vec<Contact>> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(env.data)
}

/// Minimal query-string encoding for the search term (space + the few chars that
/// would break a query string). Good enough for a name/username/email filter.
fn urlencoding_min(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            '&' => "%26".to_string(),
            '#' => "%23".to_string(),
            '+' => "%2B".to_string(),
            '?' => "%3F".to_string(),
            _ => c.to_string(),
        })
        .collect()
}

/// Advance the caller's read cursor. Best-effort (errors are non-fatal).
pub(crate) async fn mark_read(
    address: &str,
    token: &str,
    conversation_id: &str,
    message_id: &str,
) -> Result<(), String> {
    let resp = client()?
        .post(format!("{}/{conversation_id}/read", base(address)))
        .bearer_auth(token)
        .json(&serde_json::json!({ "messageId": message_id }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok(resp).await.map(|_| ())
}

/// Turn a non-2xx response into an `Err` carrying the server's message.
async fn ok(resp: reqwest::Response) -> Result<reqwest::Response, String> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
        .unwrap_or_else(|| body.chars().take(120).collect());
    Err(format!("HTTP {}: {msg}", status.as_u16()))
}
