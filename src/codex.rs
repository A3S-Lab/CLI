//! Codex / ChatGPT-account LLM client.
//!
//! This provider reads the local Codex CLI login (`$CODEX_HOME/auth.json` or
//! `~/.codex/auth.json`) and talks to the ChatGPT Codex Responses backend. It
//! exists because the ChatGPT-account backend uses a different wire format from
//! OpenAI chat completions.

use a3s_code_core::llm::{
    default_http_client,
    structured::{NativeStructuredSupport, StructuredDirective},
    ContentBlock, HttpClient, LlmClient, LlmResponse, LlmResponseMeta, Message,
    NonRetryableLlmError, StreamEvent, TokenUsage, ToolDefinition,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const CODEX_BASE: &str = "https://chatgpt.com/backend-api/codex";
const ORIGINATOR: &str = "codex_cli_rs";
const UA: &str = "codex_cli_rs (a3s)";
const CODEX_MODEL_REFRESH_TIMEOUT: Duration = Duration::from_secs(15);
const RESPONSES_LITE_HEADER: &str = "x-openai-internal-codex-responses-lite";
const FALLBACK_CODEX_MODEL: &str = "gpt-5.6-sol";
const CODEX_WIRE_REASONING_EFFORT_ORDER: &[&str] =
    &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

pub(crate) fn codex_home() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        return Some(expand_home_path(PathBuf::from(configured)));
    }
    std::env::var_os("HOME").map(|home| Path::new(&home).join(".codex"))
}

pub(crate) fn codex_auth_path() -> Option<PathBuf> {
    codex_home().map(|home| home.join("auth.json"))
}

fn codex_models_cache_path() -> Option<PathBuf> {
    codex_home().map(|home| home.join("models_cache.json"))
}

fn expand_home_path(path: PathBuf) -> PathBuf {
    let Some(raw) = path.to_str() else {
        return path;
    };
    let Some(rest) = raw.strip_prefix("~/") else {
        return path;
    };
    std::env::var_os("HOME")
        .map(|home| Path::new(&home).join(rest))
        .unwrap_or(path)
}

pub(crate) fn native_reasoning_effort_for_a3s(a3s_effort: &str) -> Option<&'static str> {
    match a3s_effort.trim().to_ascii_lowercase().as_str() {
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" => Some("max"),
        // Native Codex treats Ultra as a product/orchestration tier and maps it
        // to the Responses wire effort `max`. A3S supplies the orchestration.
        "ultracode" => Some("max"),
        _ => None,
    }
}

/// Normalize catalog/product effort names to values accepted by the Responses
/// request. In particular, `ultra` is never a wire value; native Codex sends it
/// as `max` and enables its multi-agent behavior separately.
fn codex_wire_reasoning_effort(effort: &str) -> Option<&'static str> {
    match effort.trim().to_ascii_lowercase().as_str() {
        "none" => Some("none"),
        "minimal" => Some("minimal"),
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" | "ultra" => Some("max"),
        _ => None,
    }
}

fn codex_usage_limit_error(status: u16, body: &str) -> Option<NonRetryableLlmError> {
    codex_usage_limit_error_at(status, body, chrono::Utc::now().timestamp())
}

fn codex_usage_limit_error_at(status: u16, body: &str, now: i64) -> Option<NonRetryableLlmError> {
    if status != 429 {
        return None;
    }

    let payload: Value = serde_json::from_str(body).ok()?;
    let error = payload.get("error")?;
    let is_usage_limit = [error.get("type"), error.get("code")]
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|kind| kind == "usage_limit_reached");
    if !is_usage_limit {
        return None;
    }

    let plan = codex_plan_label(error.get("plan_type"));
    let plan_suffix = plan
        .as_deref()
        .map(|plan| format!(" ({plan} plan)"))
        .unwrap_or_default();
    let mut message = format!("Codex usage limit reached{plan_suffix}.");

    let reset_in_seconds =
        codex_json_u64(error.get("resets_in_seconds")).filter(|seconds| *seconds > 0);
    let reset_at = codex_json_i64(error.get("resets_at")).and_then(|timestamp| {
        let derived_remaining = u64::try_from(timestamp.checked_sub(now)?).ok()?;
        if derived_remaining == 0 {
            return None;
        }
        let local = chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0)?
            .with_timezone(&chrono::Local);
        Some((
            local.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
            reset_in_seconds.unwrap_or(derived_remaining),
        ))
    });

    if let Some((local_time, remaining)) = reset_at {
        message.push_str(&format!(
            " It resets at {local_time} local time (in about {}).",
            format_quota_wait(remaining)
        ));
        message.push_str(" Wait for the reset, or use another provider or account.");
    } else if let Some(remaining) = reset_in_seconds {
        message.push_str(&format!(
            " It resets in about {}. Wait for the reset, or use another provider or account.",
            format_quota_wait(remaining)
        ));
    } else {
        message.push_str(" Try again later, or use another provider or account.");
    }

    Some(NonRetryableLlmError::new(message))
}

fn codex_plan_label(value: Option<&Value>) -> Option<String> {
    let value = value?.as_str()?.trim();
    if value.is_empty()
        || value.len() > 32
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return None;
    }

    let words = value
        .split(['-', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            let mut chars = lower.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>();
    (!words.is_empty()).then(|| words.join(" "))
}

fn codex_json_i64(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn codex_json_u64(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn format_quota_wait(seconds: u64) -> String {
    let minutes = seconds.saturating_add(59) / 60;
    if minutes >= 24 * 60 {
        let days = minutes / (24 * 60);
        let hours = (minutes % (24 * 60)) / 60;
        if hours == 0 {
            format!("{days}d")
        } else {
            format!("{days}d {hours}h")
        }
    } else if minutes >= 60 {
        let hours = minutes / 60;
        let minutes = minutes % 60;
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {minutes}m")
        }
    } else {
        format!("{}m", minutes.max(1))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexModel {
    pub(crate) slug: String,
    pub(crate) context_window: Option<u32>,
    use_responses_lite: bool,
    default_reasoning_effort: Option<String>,
    supported_reasoning_efforts: Vec<String>,
}

impl CodexModel {
    fn fallback() -> Self {
        Self {
            slug: FALLBACK_CODEX_MODEL.to_string(),
            context_window: None,
            // GPT-5.6 account models use the Codex Responses Lite contract.
            use_responses_lite: true,
            default_reasoning_effort: Some("low".to_string()),
            supported_reasoning_efforts: ["low", "medium", "high", "xhigh", "max"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }

    /// Resolve an A3S effort profile to the closest native effort supported by
    /// this Codex account model. `ultracode` keeps its A3S orchestration meaning
    /// while using Codex's highest Responses wire effort (`max`, or a lower cap).
    pub(crate) fn resolve_reasoning_effort(&self, a3s_effort: &str) -> Option<String> {
        let target = native_reasoning_effort_for_a3s(a3s_effort)?;
        if self
            .supported_reasoning_efforts
            .iter()
            .any(|effort| effort == target)
        {
            return Some(target.to_string());
        }

        let target_rank = reasoning_effort_rank(target)?;
        let known_supported = self
            .supported_reasoning_efforts
            .iter()
            .filter_map(|effort| reasoning_effort_rank(effort).map(|rank| (rank, effort)));
        if let Some((_, effort)) = known_supported
            .clone()
            .filter(|(rank, _)| *rank <= target_rank)
            .max_by_key(|(rank, _)| *rank)
        {
            return Some(effort.clone());
        }
        if let Some((_, effort)) = known_supported.min_by_key(|(rank, _)| *rank) {
            return Some(effort.clone());
        }

        self.default_reasoning_effort.clone().filter(|default| {
            reasoning_effort_rank(default).is_some()
                && self
                    .supported_reasoning_efforts
                    .iter()
                    .any(|effort| effort == default)
        })
    }
}

fn reasoning_effort_rank(effort: &str) -> Option<usize> {
    CODEX_WIRE_REASONING_EFFORT_ORDER
        .iter()
        .position(|candidate| *candidate == effort)
}

fn cached_catalog_value() -> Option<Value> {
    let path = codex_models_cache_path()?;
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn normalized_effort(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|effort| !effort.is_empty())
        .map(str::to_ascii_lowercase)
}

fn parse_supported_reasoning_efforts(model: &Value) -> Vec<String> {
    let Some(levels) = model
        .get("supported_reasoning_levels")
        .or_else(|| model.get("supported_reasoning_efforts"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    levels
        .iter()
        .filter_map(|level| {
            normalized_effort(level).or_else(|| level.get("effort").and_then(normalized_effort))
        })
        .filter(|effort| seen.insert(effort.clone()))
        .collect()
}

fn parse_default_reasoning_effort(model: &Value) -> Option<String> {
    model
        .get("default_reasoning_level")
        .or_else(|| model.get("default_reasoning_effort"))
        .and_then(normalized_effort)
}

fn parse_model_catalog(value: &Value) -> Vec<CodexModel> {
    let Some(models) = value.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut models = models
        .iter()
        .enumerate()
        .filter_map(|(index, model)| {
            // Match Codex's own picker semantics. Hidden entries such as
            // `codex-auto-review` are internal, not account-selectable models.
            if model.get("visibility").and_then(Value::as_str) != Some("list") {
                return None;
            }
            let slug = model
                .get("slug")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|slug| !slug.is_empty())?;
            let priority = model
                .get("priority")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX);
            Some((
                priority,
                index,
                CodexModel {
                    slug: slug.to_string(),
                    context_window: parse_model_context(model),
                    use_responses_lite: model
                        .get("use_responses_lite")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    default_reasoning_effort: parse_default_reasoning_effort(model),
                    supported_reasoning_efforts: parse_supported_reasoning_efforts(model),
                },
            ))
        })
        .collect::<Vec<_>>();
    models.sort_by_key(|(priority, index, _)| (*priority, *index));

    let mut seen = HashSet::new();
    models
        .into_iter()
        .filter_map(|(_, _, model)| seen.insert(model.slug.clone()).then_some(model))
        .collect()
}

pub(crate) fn cached_codex_models() -> Vec<CodexModel> {
    let models = cached_catalog_value()
        .as_ref()
        .map(parse_model_catalog)
        .unwrap_or_default();
    if models.is_empty() {
        vec![CodexModel::fallback()]
    } else {
        models
    }
}

/// Refresh the picker catalog through the installed Codex CLI. This delegates
/// client-version negotiation, token refresh, account entitlements, ETags, and
/// cache persistence to the owner of the login instead of duplicating them.
pub(crate) async fn refresh_codex_models() -> Result<Vec<CodexModel>> {
    let mut command = tokio::process::Command::new("codex");
    command.args(["debug", "models"]).kill_on_drop(true);
    let output = tokio::time::timeout(CODEX_MODEL_REFRESH_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            anyhow!(
                "`codex debug models` timed out after {} seconds",
                CODEX_MODEL_REFRESH_TIMEOUT.as_secs()
            )
        })?
        .context("run `codex debug models`")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("Codex could not refresh the signed-in account's model catalog");
        return Err(anyhow!(
            "`codex debug models` exited with {}: {}",
            output.status,
            detail.trim()
        ));
    }

    let value: Value =
        serde_json::from_slice(&output.stdout).context("parse `codex debug models` output")?;
    let models = parse_model_catalog(&value);
    if models.is_empty() {
        return Err(anyhow!(
            "the signed-in Codex account returned no picker-visible models"
        ));
    }
    Ok(models)
}

fn cached_codex_model(model: &str) -> Option<CodexModel> {
    cached_catalog_value()
        .as_ref()
        .map(parse_model_catalog)
        .unwrap_or_default()
        .into_iter()
        .find(|candidate| candidate.slug == model)
        .or_else(|| (model == FALLBACK_CODEX_MODEL).then(CodexModel::fallback))
}

pub(crate) fn codex_model_context(model: &str) -> Option<u32> {
    cached_codex_model(model).and_then(|model| model.context_window)
}

pub(crate) fn has_codex_login() -> bool {
    let Some(path) = codex_home().map(|home| home.join("auth.json")) else {
        return false;
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|value| {
            value
                .pointer("/tokens/access_token")
                .and_then(Value::as_str)
                .is_some_and(|token| !token.is_empty())
                || value
                    .get("access_token")
                    .and_then(Value::as_str)
                    .is_some_and(|token| !token.is_empty())
        })
}

fn positive_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .filter(|value| *value > 0)
        .and_then(|value| u32::try_from(value).ok())
}

fn parse_model_context(model: &Value) -> Option<u32> {
    const KEYS: &[&str] = &[
        "context_length",
        "max_context_length",
        "max_context_window",
        "max_model_len",
        "context_window",
        "max_input_tokens",
    ];
    for key in KEYS {
        if let Some(context) = model.get(key).and_then(positive_u32) {
            return Some(context);
        }
    }
    model
        .get("model_info")
        .and_then(|info| info.get("max_input_tokens"))
        .and_then(positive_u32)
}

#[derive(Clone)]
pub(crate) struct CodexClient {
    access_token: String,
    account_id: String,
    model: String,
    session_id: String,
    use_responses_lite: bool,
    reasoning_effort: Option<String>,
    forced_tool_choice: Option<String>,
    http: Arc<dyn HttpClient>,
}

impl CodexClient {
    /// Read Codex's auth cache and bind it to `model`.
    pub(crate) fn from_codex_login(model: &str, session_id: &str) -> Result<Self> {
        let path = codex_auth_path().ok_or_else(|| anyhow!("HOME unset and CODEX_HOME unset"))?;
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("read {} (run `codex login`)", path.display()))?;
        let value: Value =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;

        let access_token = value
            .pointer("/tokens/access_token")
            .or_else(|| value.get("access_token"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow!("no access_token in {} — run `codex login`", path.display()))?
            .to_string();

        let account_id = value
            .pointer("/tokens/account_id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| {
                value
                    .pointer("/tokens/id_token")
                    .and_then(|value| value.as_str())
                    .and_then(account_id_from_id_token)
            })
            .ok_or_else(|| {
                anyhow!(
                    "no ChatGPT account id in {} — re-run `codex login`",
                    path.display()
                )
            })?;

        Ok(Self {
            access_token,
            account_id,
            model: model.to_string(),
            session_id: session_id.to_string(),
            use_responses_lite: cached_codex_model(model)
                .is_some_and(|model| model.use_responses_lite),
            // Preserve the backend default for callers that do not opt into an
            // A3S profile. The TUI always materializes an explicit profile.
            reasoning_effort: None,
            forced_tool_choice: None,
            http: default_http_client(),
        })
    }

    pub(crate) fn from_codex_login_with_effort(
        model: &str,
        session_id: &str,
        a3s_effort: &str,
    ) -> Result<Self> {
        Ok(Self::from_codex_login(model, session_id)?.with_a3s_effort(a3s_effort))
    }

    /// Clone this authenticated client with the native effort resolved from the
    /// latest signed-in account catalog. Keeping the configured effort immutable
    /// makes `/effort` transactional: a failed session rebuild cannot mutate the
    /// client retained by the old active session.
    pub(crate) fn with_a3s_effort(&self, a3s_effort: &str) -> Self {
        let mut client = self.clone();
        client.reasoning_effort = self.resolve_reasoning_effort(a3s_effort);
        client
    }

    pub(crate) fn resolve_reasoning_effort(&self, a3s_effort: &str) -> Option<String> {
        cached_codex_model(&self.model).and_then(|model| model.resolve_reasoning_effort(a3s_effort))
    }

    #[cfg(test)]
    fn configured_reasoning_effort(&self) -> Option<&str> {
        self.reasoning_effort.as_deref()
    }

    fn build_body(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
        stream: bool,
    ) -> Value {
        let mut input = convert_messages(messages);
        let tools = convert_tools(tools);
        let reasoning_effort = self
            .reasoning_effort
            .as_deref()
            .and_then(codex_wire_reasoning_effort);
        if self.use_responses_lite {
            let mut reasoning = json!({ "context": "all_turns" });
            if let Some(effort) = reasoning_effort {
                reasoning["effort"] = json!(effort);
            }
            let mut prefix = vec![json!({
                "type": "additional_tools",
                "role": "developer",
                "tools": tools,
            })];
            if let Some(instructions) = system.filter(|instructions| !instructions.is_empty()) {
                prefix.push(json!({
                    "type": "message",
                    "role": "developer",
                    "content": [{ "type": "input_text", "text": instructions }],
                }));
            }
            prefix.append(&mut input);
            return json!({
                "model": self.model,
                "input": prefix,
                "tool_choice": self.tool_choice(),
                "parallel_tool_calls": false,
                "reasoning": reasoning,
                "store": false,
                "stream": stream,
                "prompt_cache_key": self.session_id,
            });
        }

        let mut body = json!({
            "model": self.model,
            "instructions": system.unwrap_or(""),
            "input": input,
            "tools": tools,
            "tool_choice": self.tool_choice(),
            "parallel_tool_calls": false,
            "store": false,
            "stream": stream,
            "prompt_cache_key": self.session_id,
        });
        if let Some(effort) = reasoning_effort {
            body["reasoning"] = json!({ "effort": effort });
        }
        body
    }

    fn tool_choice(&self) -> Value {
        self.forced_tool_choice
            .as_ref()
            .map(|name| json!({ "type": "function", "name": name }))
            .unwrap_or_else(|| json!("auto"))
    }

    fn request_headers<'a>(&'a self, bearer: &'a str) -> Vec<(&'static str, &'a str)> {
        let mut headers = vec![
            ("Authorization", bearer),
            ("chatgpt-account-id", self.account_id.as_str()),
            ("OpenAI-Beta", "responses=experimental"),
            ("originator", ORIGINATOR),
            ("session_id", self.session_id.as_str()),
            ("Accept", "text/event-stream"),
            ("User-Agent", UA),
        ];
        if self.use_responses_lite {
            headers.push((RESPONSES_LITE_HEADER, "true"));
        }
        headers
    }
}

#[async_trait]
impl LlmClient for CodexClient {
    fn fork_for_session(&self, session_id: &str) -> Option<Arc<dyn LlmClient>> {
        let mut client = self.clone();
        client.session_id = session_id.to_string();
        Some(Arc::new(client))
    }

    async fn complete(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        let mut rx = self
            .complete_streaming(messages, system, tools, CancellationToken::new())
            .await?;
        while let Some(event) = rx.recv().await {
            if let StreamEvent::Done(response) = event {
                return Ok(response);
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
        let headers = self.request_headers(&bearer);

        let response = self
            .http
            .post_streaming(&url, headers, &body, cancel_token.clone())
            .await?;
        if !(200..300).contains(&response.status) {
            if let Some(error) = codex_usage_limit_error(response.status, &response.error_body) {
                return Err(error.into());
            }
            return Err(anyhow!(
                "codex /responses HTTP {}: {}",
                response.status,
                response.error_body
            ));
        }

        let (tx, rx) = mpsc::channel(128);
        let model = self.model.clone();
        let request_url = url.clone();
        let mut stream = response.byte_stream;

        tokio::spawn(async move {
            use futures::StreamExt;
            let mut buf = String::new();
            let mut text = String::new();
            let mut reasoning = String::new();
            let mut response_id: Option<String> = None;
            let mut usage = TokenUsage::default();
            let mut calls: Vec<(String, (String, String, String))> = Vec::new();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
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
                        let Ok(event) = serde_json::from_str::<Value>(data.trim()) else {
                            continue;
                        };
                        match event
                            .get("type")
                            .and_then(|kind| kind.as_str())
                            .unwrap_or("")
                        {
                            "response.created" => {
                                response_id = event
                                    .pointer("/response/id")
                                    .and_then(|value| value.as_str())
                                    .map(str::to_string);
                            }
                            "response.output_text.delta" => {
                                if let Some(delta) = event.get("delta").and_then(|v| v.as_str()) {
                                    text.push_str(delta);
                                    let _ =
                                        tx.send(StreamEvent::TextDelta(delta.to_string())).await;
                                }
                            }
                            "response.reasoning_text.delta"
                            | "response.reasoning_summary_text.delta" => {
                                if let Some(delta) = event.get("delta").and_then(|v| v.as_str()) {
                                    reasoning.push_str(delta);
                                    let _ = tx
                                        .send(StreamEvent::ReasoningDelta(delta.to_string()))
                                        .await;
                                }
                            }
                            "response.output_item.added" => {
                                let item = event.get("item");
                                if item
                                    .and_then(|value| value.get("type"))
                                    .and_then(|value| value.as_str())
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
                                    event.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
                                if let Some(delta) = event.get("delta").and_then(|v| v.as_str()) {
                                    let call_id = if let Some(entry) =
                                        calls.iter_mut().find(|(key, _)| key == item_id)
                                    {
                                        entry.1 .2.push_str(delta);
                                        (!entry.1 .0.is_empty()).then(|| entry.1 .0.clone())
                                    } else {
                                        None
                                    };
                                    let _ = tx
                                        .send(StreamEvent::ToolUseInputDelta {
                                            id: call_id,
                                            delta: delta.to_string(),
                                        })
                                        .await;
                                }
                            }
                            "response.output_item.done" => {
                                let item = event.get("item");
                                if item
                                    .and_then(|value| value.get("type"))
                                    .and_then(|value| value.as_str())
                                    == Some("function_call")
                                {
                                    let id = item_str(item, "id");
                                    let entry = (
                                        item_str(item, "call_id"),
                                        item_str(item, "name"),
                                        item_str(item, "arguments"),
                                    );
                                    if let Some(existing) =
                                        calls.iter_mut().find(|(key, _)| *key == id)
                                    {
                                        existing.1 = entry;
                                    } else {
                                        calls.push((id, entry));
                                    }
                                }
                            }
                            "response.completed" => {
                                if let Some(raw_usage) = event.pointer("/response/usage") {
                                    usage.prompt_tokens = raw_usage
                                        .get("input_tokens")
                                        .and_then(|value| value.as_u64())
                                        .unwrap_or(0)
                                        as usize;
                                    usage.completion_tokens = raw_usage
                                        .get("output_tokens")
                                        .and_then(|value| value.as_u64())
                                        .unwrap_or(0)
                                        as usize;
                                    usage.total_tokens = raw_usage
                                        .get("total_tokens")
                                        .and_then(|value| value.as_u64())
                                        .unwrap_or(0)
                                        as usize;
                                    usage.cache_read_tokens = raw_usage
                                        .pointer("/input_tokens_details/cached_tokens")
                                        .and_then(|value| value.as_u64())
                                        .map(|value| value as usize);
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
                                let response = LlmResponse {
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
                                        request_url: Some(request_url.clone()),
                                        response_id: response_id.clone(),
                                        ..Default::default()
                                    }),
                                };
                                let _ = tx.send(StreamEvent::Done(response)).await;
                                return;
                            }
                            "response.failed" | "error" => return,
                            _ => {}
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    fn native_structured_support(&self) -> NativeStructuredSupport {
        NativeStructuredSupport::ForcedTool
    }

    async fn complete_structured(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
        directive: &StructuredDirective,
    ) -> Result<LlmResponse> {
        let mut client = self.clone();
        client.forced_tool_choice = directive.force_tool.clone();
        client.complete(messages, system, tools).await
    }

    async fn complete_streaming_structured(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
        directive: &StructuredDirective,
        cancel_token: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamEvent>> {
        let mut client = self.clone();
        client.forced_tool_choice = directive.force_tool.clone();
        client
            .complete_streaming(messages, system, tools, cancel_token)
            .await
    }
}

fn item_str(item: Option<&Value>, key: &str) -> String {
    item.and_then(|value| value.get(key))
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string()
}

fn parse_args(value: &str) -> Value {
    if value.trim().is_empty() {
        return json!({});
    }
    serde_json::from_str(value).unwrap_or_else(|_| json!({}))
}

fn account_id_from_id_token(jwt: &str) -> Option<String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let payload = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn convert_messages(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    for message in messages {
        for block in &message.content {
            match block {
                ContentBlock::Text { text } => {
                    let kind = if message.role == "assistant" {
                        "output_text"
                    } else {
                        "input_text"
                    };
                    out.push(json!({
                        "type": "message",
                        "role": message.role,
                        "content": [{"type": kind, "text": text}],
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
                        "role": message.role,
                        "content": [{
                            "type": "input_image",
                            "image_url": format!(
                                "data:{};base64,{}",
                                source.media_type, source.data
                            ),
                        }],
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
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_code_core::llm::{HttpResponse, StreamingHttpResponse};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ErrorStreamingHttp {
        status: u16,
        body: String,
        calls: AtomicUsize,
    }

    struct SseStreamingHttp {
        stream: String,
    }

    #[async_trait]
    impl HttpClient for ErrorStreamingHttp {
        async fn post(
            &self,
            _url: &str,
            _headers: Vec<(&str, &str)>,
            _body: &Value,
            _cancel_token: CancellationToken,
        ) -> Result<HttpResponse> {
            anyhow::bail!("unexpected non-streaming HTTP call")
        }

        async fn post_streaming(
            &self,
            _url: &str,
            _headers: Vec<(&str, &str)>,
            _body: &Value,
            _cancel_token: CancellationToken,
        ) -> Result<StreamingHttpResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(StreamingHttpResponse {
                status: self.status,
                retry_after: None,
                byte_stream: Box::pin(futures::stream::empty()),
                error_body: self.body.clone(),
            })
        }
    }

    #[async_trait]
    impl HttpClient for SseStreamingHttp {
        async fn post(
            &self,
            _url: &str,
            _headers: Vec<(&str, &str)>,
            _body: &Value,
            _cancel_token: CancellationToken,
        ) -> Result<HttpResponse> {
            anyhow::bail!("unexpected non-streaming HTTP call")
        }

        async fn post_streaming(
            &self,
            _url: &str,
            _headers: Vec<(&str, &str)>,
            _body: &Value,
            _cancel_token: CancellationToken,
        ) -> Result<StreamingHttpResponse> {
            Ok(StreamingHttpResponse {
                status: 200,
                retry_after: None,
                byte_stream: Box::pin(futures::stream::iter(vec![Ok(self.stream.clone().into())])),
                error_body: String::new(),
            })
        }
    }

    fn client(use_responses_lite: bool, reasoning_effort: Option<&str>) -> CodexClient {
        CodexClient {
            access_token: "token".to_string(),
            account_id: "account".to_string(),
            model: if use_responses_lite {
                "gpt-5.6-sol"
            } else {
                "gpt-5.5"
            }
            .to_string(),
            session_id: "session".to_string(),
            use_responses_lite,
            reasoning_effort: reasoning_effort.map(str::to_string),
            forced_tool_choice: None,
            http: default_http_client(),
        }
    }

    fn model_with_efforts(default: Option<&str>, supported: &[&str]) -> CodexModel {
        CodexModel {
            slug: "test-model".to_string(),
            context_window: None,
            use_responses_lite: false,
            default_reasoning_effort: default.map(str::to_string),
            supported_reasoning_efforts: supported
                .iter()
                .map(|effort| (*effort).to_string())
                .collect(),
        }
    }

    fn tool() -> ToolDefinition {
        ToolDefinition {
            name: "read".to_string(),
            description: "Read a file".to_string(),
            parameters: json!({"type": "object"}),
        }
    }

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
        assert_eq!(
            parse_model_context(&json!({"context_window": u64::MAX})),
            None
        );
    }

    #[test]
    fn parses_all_picker_visible_models_in_priority_order() {
        let models = parse_model_catalog(&json!({
            "models": [
                {
                    "slug": "gpt-5.6-terra",
                    "visibility": "list",
                    "priority": 2,
                    "context_window": 372000,
                    "use_responses_lite": true
                },
                {
                    "slug": "codex-auto-review",
                    "visibility": "hide",
                    "priority": 0,
                    "context_window": 372000
                },
                {
                    "slug": "gpt-5.6-sol",
                    "visibility": "list",
                    "priority": 1,
                    "context_window": 372000,
                    "use_responses_lite": true,
                    "default_reasoning_level": " LOW ",
                    "supported_reasoning_levels": [
                        {"effort": "low"},
                        {"effort": "MEDIUM"},
                        {"effort": "low"},
                        {"effort": "high"},
                        {"effort": "xhigh"},
                        {"effort": "max"},
                        {"effort": "ultra"}
                    ],
                    "supported_in_api": false
                },
                {
                    "slug": "gpt-5.5",
                    "visibility": "list",
                    "priority": 0,
                    "context_window": 272000
                },
                {
                    "slug": "gpt-5.6-sol",
                    "visibility": "list",
                    "priority": 99
                }
            ]
        }));

        assert_eq!(
            models
                .iter()
                .map(|model| model.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["gpt-5.5", "gpt-5.6-sol", "gpt-5.6-terra"]
        );
        let sol = models
            .iter()
            .find(|model| model.slug == "gpt-5.6-sol")
            .unwrap();
        assert_eq!(sol.context_window, Some(372_000));
        assert!(sol.use_responses_lite);
        assert_eq!(sol.default_reasoning_effort.as_deref(), Some("low"));
        assert_eq!(
            sol.supported_reasoning_efforts,
            ["low", "medium", "high", "xhigh", "max", "ultra"]
        );
    }

    #[test]
    fn parses_alternate_reasoning_capability_shape() {
        let models = parse_model_catalog(&json!({
            "models": [{
                "slug": "alternate",
                "visibility": "list",
                "default_reasoning_effort": " XHIGH ",
                "supported_reasoning_efforts": [" xhigh ", {"effort": "MAX"}]
            }]
        }));

        assert_eq!(models[0].default_reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(models[0].supported_reasoning_efforts, ["xhigh", "max"]);
    }

    #[test]
    fn resolves_a3s_effort_against_model_capabilities() {
        let sol = model_with_efforts(
            Some("low"),
            &["low", "medium", "high", "xhigh", "max", "ultra"],
        );
        let luna = model_with_efforts(Some("medium"), &["low", "medium", "high", "xhigh", "max"]);
        let legacy = model_with_efforts(Some("medium"), &["low", "medium", "high", "xhigh"]);

        assert_eq!(
            sol.resolve_reasoning_effort("high").as_deref(),
            Some("high")
        );
        assert_eq!(
            sol.resolve_reasoning_effort("ultracode").as_deref(),
            Some("max")
        );
        assert_eq!(
            luna.resolve_reasoning_effort("ultracode").as_deref(),
            Some("max")
        );
        assert_eq!(
            legacy.resolve_reasoning_effort("max").as_deref(),
            Some("xhigh")
        );
        assert_eq!(
            legacy.resolve_reasoning_effort("ultracode").as_deref(),
            Some("xhigh")
        );

        let medium_only = model_with_efforts(Some("medium"), &["medium"]);
        assert_eq!(
            medium_only.resolve_reasoning_effort("low").as_deref(),
            Some("medium")
        );
        let below_low = model_with_efforts(Some("minimal"), &["none", "minimal"]);
        assert_eq!(
            below_low.resolve_reasoning_effort("low").as_deref(),
            Some("minimal")
        );
        assert_eq!(
            model_with_efforts(Some("medium"), &[]).resolve_reasoning_effort("high"),
            None
        );
        assert_eq!(sol.resolve_reasoning_effort("unknown"), None);
        assert_eq!(native_reasoning_effort_for_a3s("ultracode"), Some("max"));
        assert_eq!(codex_wire_reasoning_effort("ultra"), Some("max"));
    }

    #[test]
    fn standard_responses_request_keeps_top_level_instructions_and_tools() {
        let client = client(false, Some("xhigh"));
        let body = client.build_body(&[Message::user("hello")], Some("system"), &[tool()], true);

        assert_eq!(body["instructions"], "system");
        assert_eq!(body["tools"][0]["name"], "read");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["reasoning"]["effort"], "xhigh");
        assert!(!client
            .request_headers("Bearer token")
            .iter()
            .any(|(name, _)| *name == RESPONSES_LITE_HEADER));
    }

    #[test]
    fn responses_lite_moves_instructions_and_tools_into_input() {
        let client = client(true, Some("ultra"));
        let body = client.build_body(&[Message::user("hello")], Some("system"), &[tool()], true);

        assert!(body.get("instructions").is_none());
        assert!(body.get("tools").is_none());
        assert_eq!(body["input"][0]["type"], "additional_tools");
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(body["input"][0]["tools"][0]["name"], "read");
        assert_eq!(body["input"][1]["role"], "developer");
        assert_eq!(body["input"][1]["content"][0]["text"], "system");
        assert_eq!(body["input"][2]["role"], "user");
        assert_eq!(body["reasoning"]["context"], "all_turns");
        assert_eq!(body["reasoning"]["effort"], "max");
        assert!(!body.to_string().contains("ultra"));
        assert!(client
            .request_headers("Bearer token")
            .iter()
            .any(|(name, value)| *name == RESPONSES_LITE_HEADER && *value == "true"));
    }

    #[test]
    fn unresolved_effort_is_omitted_from_request_bodies() {
        let standard = client(false, None).build_body(&[], None, &[], false);
        let lite = client(true, None).build_body(&[], None, &[], false);
        let invalid = client(false, Some("not-a-wire-effort")).build_body(&[], None, &[], false);

        assert!(standard.get("reasoning").is_none());
        assert_eq!(lite["reasoning"]["context"], "all_turns");
        assert!(lite["reasoning"].get("effort").is_none());
        assert!(invalid.get("reasoning").is_none());
    }

    #[test]
    fn structured_requests_force_the_named_function_for_both_codex_transports() {
        for responses_lite in [false, true] {
            let mut client = client(responses_lite, Some("low"));
            client.forced_tool_choice = Some("emit_research_plan".to_string());
            let body = client.build_body(&[], None, &[tool()], true);

            assert_eq!(body["tool_choice"]["type"], "function");
            assert_eq!(body["tool_choice"]["name"], "emit_research_plan");
            assert_eq!(
                client.native_structured_support(),
                NativeStructuredSupport::ForcedTool
            );
        }
    }

    #[test]
    fn usage_limit_payload_becomes_a_friendly_terminal_error() {
        let body = r#"{
            "error": {
                "type": "usage_limit_reached",
                "message": "The usage limit has been reached",
                "plan_type": "pro",
                "resets_at": 1783656812,
                "resets_in_seconds": 9893
            }
        }"#;

        let error = codex_usage_limit_error_at(429, body, 1783646919)
            .expect("usage limits should be terminal");
        let message = error.to_string();
        assert!(message.contains("Codex usage limit reached (Pro plan)."));
        assert!(message.contains("It resets at "), "{message}");
        assert!(message.contains(" local time"), "{message}");
        assert!(message.contains("in about 2h 45m"), "{message}");
        assert!(message.contains("another provider or account"), "{message}");
        assert!(!message.contains("usage_limit_reached"), "{message}");
        assert!(!message.contains("resets_in_seconds"), "{message}");
    }

    #[test]
    fn usage_limit_payload_degrades_gracefully_when_reset_is_missing() {
        let error = codex_usage_limit_error_at(
            429,
            r#"{"error":{"code":"usage_limit_reached","plan_type":"plus"}}"#,
            1783646919,
        )
        .expect("the error code shape should also be recognized");

        assert_eq!(
            error.to_string(),
            "Codex usage limit reached (Plus plan). Try again later, or use another provider or account."
        );
    }

    #[test]
    fn usage_limit_prefers_server_relative_reset_over_local_clock_delta() {
        let error = codex_usage_limit_error_at(
            429,
            r#"{"error":{"type":"usage_limit_reached","resets_at":"1783656812","resets_in_seconds":"90"}}"#,
            1783646919,
        )
        .expect("numeric strings should be accepted");
        let message = error.to_string();

        assert!(message.contains("2026-"), "{message}");
        assert!(message.contains(":32 "), "{message}");
        assert!(message.contains("in about 2m"), "{message}");
        assert!(!message.contains("2h 45m"), "{message}");
    }

    #[test]
    fn ordinary_rate_limits_and_malformed_payloads_remain_retryable() {
        assert!(codex_usage_limit_error_at(
            429,
            r#"{"error":{"type":"rate_limit_exceeded"}}"#,
            1783646919,
        )
        .is_none());
        assert!(codex_usage_limit_error_at(429, "not json", 1783646919).is_none());
        assert!(codex_usage_limit_error_at(
            500,
            r#"{"error":{"type":"usage_limit_reached"}}"#,
            1783646919,
        )
        .is_none());
    }

    #[tokio::test]
    async fn usage_limit_http_response_returns_non_retryable_marker() {
        let http = Arc::new(ErrorStreamingHttp {
            status: 429,
            body: r#"{"error":{"type":"usage_limit_reached","plan_type":"pro","resets_in_seconds":90}}"#
                .to_string(),
            calls: AtomicUsize::new(0),
        });
        let mut client = client(true, Some("max"));
        client.http = http.clone();

        let error = match client
            .complete_streaming(&[], None, &[], CancellationToken::new())
            .await
        {
            Ok(_) => panic!("usage limits must fail before opening a stream"),
            Err(error) => error,
        };

        assert!(error.downcast_ref::<NonRetryableLlmError>().is_some());
        assert_eq!(
            error.to_string(),
            "Codex usage limit reached (Pro plan). It resets in about 2m. Wait for the reset, or use another provider or account."
        );
        assert_eq!(http.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn streaming_tool_argument_deltas_keep_interleaved_call_ids() -> Result<()> {
        let frames = [
            r#"data: {"type":"response.output_item.added","item":{"type":"function_call","id":"item_a","call_id":"call_a","name":"read"}}"#,
            r#"data: {"type":"response.output_item.added","item":{"type":"function_call","id":"item_b","call_id":"call_b","name":"search"}}"#,
            r#"data: {"type":"response.function_call_arguments.delta","item_id":"item_b","delta":"{\"query\":\"beta\"}"}"#,
            r#"data: {"type":"response.function_call_arguments.delta","item_id":"item_a","delta":"{\"path\":\"alpha\"}"}"#,
            r#"data: {"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}}"#,
        ];
        let mut client = client(false, Some("high"));
        client.http = Arc::new(SseStreamingHttp {
            stream: frames.join("\n\n") + "\n\n",
        });
        let mut rx = client
            .complete_streaming(&[], None, &[], CancellationToken::new())
            .await?;

        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::ToolUseStart { id, name })
                if id == "call_a" && name == "read"
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::ToolUseStart { id, name })
                if id == "call_b" && name == "search"
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::ToolUseInputDelta { id: Some(id), delta })
                if id == "call_b" && delta == r#"{"query":"beta"}"#
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::ToolUseInputDelta { id: Some(id), delta })
                if id == "call_a" && delta == r#"{"path":"alpha"}"#
        ));
        let Some(StreamEvent::Done(response)) = rx.recv().await else {
            panic!("expected done event");
        };
        let calls = response.tool_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "call_a");
        assert_eq!(calls[0].args, json!({"path": "alpha"}));
        assert_eq!(calls[1].id, "call_b");
        assert_eq!(calls[1].args, json!({"query": "beta"}));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a live Codex login and consumes account quota"]
    async fn real_gpt_5_6_sol_native_effort_tool_smoke() -> Result<()> {
        let client = CodexClient::from_codex_login_with_effort(
            "gpt-5.6-sol",
            "a3s-sol-effort-smoke",
            "high",
        )?;
        assert_eq!(client.configured_reasoning_effort(), Some("high"));
        let echo = ToolDefinition {
            name: "echo".to_string(),
            description: "Echo the supplied text".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "additionalProperties": false
            }),
        };

        let response = client
            .complete(
                &[Message::user(
                    "Call the echo tool exactly once with the text `sol-ready`. Do not answer in plain text.",
                )],
                Some("Follow the user's tool instruction exactly."),
                &[echo],
            )
            .await?;

        let call_id = response
            .message
            .content
            .iter()
            .find_map(|block| match block {
                ContentBlock::ToolUse { id, name, input }
                    if name == "echo" && input["text"] == "sol-ready" =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
            .expect("Sol should request the echo tool");

        let ultracode_client = client.with_a3s_effort("ultracode");
        assert_eq!(client.configured_reasoning_effort(), Some("high"));
        assert_eq!(ultracode_client.configured_reasoning_effort(), Some("max"));
        let final_response = ultracode_client
            .complete(
                &[
                    Message::user(
                        "Call the echo tool exactly once with the text `sol-ready`. Do not answer in plain text.",
                    ),
                    response.message,
                    Message::tool_result(&call_id, "sol-ready", false),
                ],
                Some("After the tool result, confirm it briefly."),
                &[tool()],
            )
            .await?;
        assert!(!final_response.message.text().trim().is_empty());
        assert!(!final_response
            .message
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolUse { .. })));
        Ok(())
    }
}
