//! A custom `LlmClient` that runs models on a local **Codex / ChatGPT account**
//! login (`~/.codex/auth.json`), talking to the ChatGPT backend's Responses API.
//!
//! a3s-code only ships an OpenAI Chat-Completions client, which can't drive the
//! ChatGPT backend (different wire format). This implements the trait directly:
//! build a Responses-API request, attach the account Bearer + account-id headers,
//! and map the SSE event stream onto a3s-code's `StreamEvent`s.

use a3s_code_core::llm::{
    default_http_client, ContentBlock, HttpClient, LlmClient, LlmResponse, LlmResponseMeta,
    Message, StreamEvent, TokenUsage, ToolDefinition,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const CODEX_BASE: &str = "https://chatgpt.com/backend-api/codex";
const ORIGINATOR: &str = "codex_cli_rs";
const UA: &str = "codex_cli_rs (a3s)";

/// The user-facing Codex models from the local cache (`~/.codex/models_cache.json`),
/// `visibility == "list"`, ordered by the CLI's `priority`. Falls back to a
/// single sane default if the cache is missing.
pub fn codex_models() -> Vec<String> {
    fn from_cache() -> Option<Vec<String>> {
        let home = std::env::var_os("HOME")?;
        let path = std::path::Path::new(&home).join(".codex/models_cache.json");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let mut list: Vec<(i64, String)> = v
            .get("models")?
            .as_array()?
            .iter()
            .filter_map(|m| {
                if m.get("visibility").and_then(|x| x.as_str()) != Some("list") {
                    return None;
                }
                let slug = m.get("slug").and_then(|x| x.as_str())?.to_string();
                let prio = m.get("priority").and_then(|x| x.as_i64()).unwrap_or(999);
                Some((prio, slug))
            })
            .collect();
        list.sort_by_key(|(p, _)| *p);
        let out: Vec<String> = list.into_iter().map(|(_, s)| s).collect();
        (!out.is_empty()).then_some(out)
    }
    from_cache().unwrap_or_else(|| vec!["gpt-5.5".to_string()])
}

pub(crate) fn codex_model_context(model: &str) -> Option<u32> {
    let home = std::env::var_os("HOME")?;
    let path = std::path::Path::new(&home).join(".codex/models_cache.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    v.get("models")?
        .as_array()?
        .iter()
        .find(|m| m.get("slug").and_then(|x| x.as_str()) == Some(model))
        .and_then(parse_model_context)
}

fn parse_model_context(model: &Value) -> Option<u32> {
    const KEYS: &[&str] = &[
        "context_length",
        "max_context_length",
        "max_model_len",
        "context_window",
        "max_input_tokens",
    ];
    for key in KEYS {
        if let Some(n) = model.get(key).and_then(|v| v.as_u64()).filter(|n| *n > 0) {
            return Some(n as u32);
        }
    }
    model
        .get("model_info")
        .and_then(|info| info.get("max_input_tokens"))
        .and_then(|v| v.as_u64())
        .filter(|n| *n > 0)
        .map(|n| n as u32)
}

pub struct CodexClient {
    access_token: String,
    account_id: String,
    model: String,
    session_id: String,
    http: Arc<dyn HttpClient>,
}

impl CodexClient {
    /// Read `~/.codex/auth.json` for the access token + ChatGPT account id and
    /// bind to `model`. Errors if the user isn't logged into the Codex CLI.
    pub fn from_codex_login(model: &str, session_id: &str) -> Result<Self> {
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME unset"))?;
        let path = std::path::Path::new(&home).join(".codex/auth.json");
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("read {} (run `codex login`)", path.display()))?;
        let v: Value = serde_json::from_str(&raw).context("parse ~/.codex/auth.json")?;

        let access_token = v
            .pointer("/tokens/access_token")
            .or_else(|| v.get("access_token"))
            .and_then(|x| x.as_str())
            .ok_or_else(|| anyhow!("no access_token in ~/.codex/auth.json — run `codex login`"))?
            .to_string();

        let account_id = v
            .pointer("/tokens/account_id")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .or_else(|| {
                v.pointer("/tokens/id_token")
                    .and_then(|x| x.as_str())
                    .and_then(account_id_from_id_token)
            })
            .ok_or_else(|| {
                anyhow!("no ChatGPT account id in ~/.codex/auth.json — re-run `codex login`")
            })?;

        Ok(Self {
            access_token,
            account_id,
            model: model.to_string(),
            session_id: session_id.to_string(),
            http: default_http_client(),
        })
    }

    fn build_body(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
        stream: bool,
    ) -> Value {
        json!({
            "model": self.model,
            "instructions": system.unwrap_or(""),
            "input": convert_messages(messages),
            "tools": convert_tools(tools),
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "store": false,
            "stream": stream,
            "prompt_cache_key": self.session_id,
        })
    }
}

#[async_trait]
impl LlmClient for CodexClient {
    async fn complete(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        // The backend only streams; drive the streaming path and wait for Done.
        let mut rx = self
            .complete_streaming(messages, system, tools, CancellationToken::new())
            .await?;
        while let Some(ev) = rx.recv().await {
            if let StreamEvent::Done(resp) = ev {
                return Ok(resp);
            }
        }
        Err(anyhow!("codex stream closed before response.completed"))
    }

    async fn complete_streaming(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
        cancel_token: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamEvent>> {
        let body = self.build_body(messages, system, tools, true);
        let url = format!("{CODEX_BASE}/responses");
        let bearer = format!("Bearer {}", self.access_token);
        let headers = vec![
            ("Authorization", bearer.as_str()),
            ("chatgpt-account-id", self.account_id.as_str()),
            ("OpenAI-Beta", "responses=experimental"),
            ("originator", ORIGINATOR),
            ("session_id", self.session_id.as_str()),
            ("Accept", "text/event-stream"),
            ("User-Agent", UA),
        ];

        let resp = self
            .http
            .post_streaming(&url, headers, &body, cancel_token.clone())
            .await?;
        if !(200..300).contains(&resp.status) {
            return Err(anyhow!(
                "codex /responses HTTP {}: {}",
                resp.status,
                resp.error_body
            ));
        }

        let (tx, rx) = mpsc::channel(128);
        let model = self.model.clone();
        let url2 = url.clone();
        let mut stream = resp.byte_stream;

        tokio::spawn(async move {
            use futures::StreamExt;
            let mut buf = String::new();
            let mut text = String::new();
            let mut reasoning = String::new();
            let mut response_id: Option<String> = None;
            let mut usage = TokenUsage::default();
            // item_id -> (call_id, name, args) — Vec to stay dep-free.
            let mut calls: Vec<(String, (String, String, String))> = Vec::new();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => break,
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(end) = buf.find("\n\n") {
                    let frame: String = buf.drain(..end).collect();
                    buf.drain(..2);
                    for line in frame.lines() {
                        let Some(data) = line
                            .strip_prefix("data: ")
                            .or_else(|| line.strip_prefix("data:"))
                        else {
                            continue;
                        };
                        let Ok(ev) = serde_json::from_str::<Value>(data.trim()) else {
                            continue;
                        };
                        match ev.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                            "response.created" => {
                                response_id = ev
                                    .pointer("/response/id")
                                    .and_then(|x| x.as_str())
                                    .map(str::to_string);
                            }
                            "response.output_text.delta" => {
                                if let Some(d) = ev.get("delta").and_then(|x| x.as_str()) {
                                    text.push_str(d);
                                    let _ = tx.send(StreamEvent::TextDelta(d.to_string())).await;
                                }
                            }
                            "response.reasoning_text.delta"
                            | "response.reasoning_summary_text.delta" => {
                                if let Some(d) = ev.get("delta").and_then(|x| x.as_str()) {
                                    reasoning.push_str(d);
                                    let _ =
                                        tx.send(StreamEvent::ReasoningDelta(d.to_string())).await;
                                }
                            }
                            "response.output_item.added" => {
                                let item = ev.get("item");
                                if item.and_then(|i| i.get("type")).and_then(|t| t.as_str())
                                    == Some("function_call")
                                {
                                    let id = item_str(item, "id");
                                    let call_id = item_str(item, "call_id");
                                    let name = item_str(item, "name");
                                    calls
                                        .push((id, (call_id.clone(), name.clone(), String::new())));
                                    let _ = tx
                                        .send(StreamEvent::ToolUseStart { id: call_id, name })
                                        .await;
                                }
                            }
                            "response.function_call_arguments.delta" => {
                                let item_id =
                                    ev.get("item_id").and_then(|x| x.as_str()).unwrap_or("");
                                if let Some(d) = ev.get("delta").and_then(|x| x.as_str()) {
                                    if let Some(e) = calls.iter_mut().find(|(k, _)| k == item_id) {
                                        e.1 .2.push_str(d);
                                    }
                                    let _ = tx
                                        .send(StreamEvent::ToolUseInputDelta(d.to_string()))
                                        .await;
                                }
                            }
                            "response.output_item.done" => {
                                let item = ev.get("item");
                                if item.and_then(|i| i.get("type")).and_then(|t| t.as_str())
                                    == Some("function_call")
                                {
                                    let id = item_str(item, "id");
                                    let entry = (
                                        item_str(item, "call_id"),
                                        item_str(item, "name"),
                                        item_str(item, "arguments"),
                                    );
                                    if let Some(e) = calls.iter_mut().find(|(k, _)| *k == id) {
                                        e.1 = entry; // authoritative final copy
                                    } else {
                                        calls.push((id, entry));
                                    }
                                }
                            }
                            "response.completed" => {
                                if let Some(u) = ev.pointer("/response/usage") {
                                    usage.prompt_tokens =
                                        u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0)
                                            as usize;
                                    usage.completion_tokens = u
                                        .get("output_tokens")
                                        .and_then(|x| x.as_u64())
                                        .unwrap_or(0)
                                        as usize;
                                    usage.total_tokens =
                                        u.get("total_tokens").and_then(|x| x.as_u64()).unwrap_or(0)
                                            as usize;
                                    usage.cache_read_tokens = u
                                        .pointer("/input_tokens_details/cached_tokens")
                                        .and_then(|x| x.as_u64())
                                        .map(|v| v as usize);
                                }
                                let mut content = Vec::new();
                                if !text.is_empty() {
                                    content.push(ContentBlock::Text {
                                        text: std::mem::take(&mut text),
                                    });
                                }
                                let has_calls = !calls.is_empty();
                                for (_, (call_id, name, args)) in calls.drain(..) {
                                    content.push(ContentBlock::ToolUse {
                                        id: call_id,
                                        name,
                                        input: parse_args(&args),
                                    });
                                }
                                let resp = LlmResponse {
                                    message: Message {
                                        role: "assistant".into(),
                                        content,
                                        reasoning_content: (!reasoning.is_empty())
                                            .then(|| std::mem::take(&mut reasoning)),
                                    },
                                    usage: usage.clone(),
                                    stop_reason: Some(
                                        if has_calls { "tool_calls" } else { "stop" }.into(),
                                    ),
                                    token_logprobs: Vec::new(),
                                    meta: Some(LlmResponseMeta {
                                        provider: Some("codex".into()),
                                        request_model: Some(model.clone()),
                                        request_url: Some(url2.clone()),
                                        response_id: response_id.clone(),
                                        ..Default::default()
                                    }),
                                };
                                let _ = tx.send(StreamEvent::Done(resp)).await;
                                return;
                            }
                            "response.failed" | "error" => {
                                // Drop tx → channel closes, which the core treats as an error.
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

fn item_str(item: Option<&Value>, key: &str) -> String {
    item.and_then(|i| i.get(key))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

fn parse_args(s: &str) -> Value {
    if s.trim().is_empty() {
        return json!({});
    }
    serde_json::from_str(s).unwrap_or_else(|_| json!({}))
}

/// base64url-decode the JWT payload + read the ChatGPT account-id claim.
fn account_id_from_id_token(jwt: &str) -> Option<String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let payload = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(|x| x.as_str())
        .map(str::to_string)
}

fn convert_messages(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    for m in messages {
        for b in &m.content {
            match b {
                ContentBlock::Text { text } => {
                    let kind = if m.role == "assistant" {
                        "output_text"
                    } else {
                        "input_text"
                    };
                    out.push(json!({
                        "type": "message",
                        "role": m.role,
                        "content": [ { "type": kind, "text": text } ],
                    }));
                }
                ContentBlock::ToolUse { id, name, input } => {
                    out.push(json!({
                        "type": "function_call",
                        "name": name,
                        "arguments": input.to_string(),
                        "call_id": id,
                    }));
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => {
                    out.push(json!({
                        "type": "function_call_output",
                        "call_id": tool_use_id,
                        "output": content.as_text(),
                    }));
                }
                ContentBlock::Image { source } => {
                    out.push(json!({
                        "type": "message",
                        "role": m.role,
                        "content": [ {
                            "type": "input_image",
                            "image_url": format!("data:{};base64,{}", source.media_type, source.data),
                        } ],
                    }));
                }
            }
        }
    }
    out
}

fn convert_tools(tools: &[ToolDefinition]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_context_from_cache_shapes() {
        assert_eq!(
            parse_model_context(&json!({"context_length": 200000})),
            Some(200_000)
        );
        assert_eq!(
            parse_model_context(&json!({"model_info": {"max_input_tokens": 1000000}})),
            Some(1_000_000)
        );
        assert_eq!(parse_model_context(&json!({"context_window": 0})), None);
        assert_eq!(parse_model_context(&json!({"slug": "gpt"})), None);
    }
}
