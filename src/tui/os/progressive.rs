//! Small shared client for OS progressive capabilities.
//!
//! Asset panels still own their business payloads, but the ViewLink path should
//! be consistent: search/describe/execute with `shaped=true`, then parse `.view`
//! or `viewUrl` through the normal RemoteUI parser.

use futures::StreamExt;

use super::remote_ui;
use crate::system_agents::sanitize_display_text;

const MAX_CAPABILITY_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_CAPABILITY_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_TRAVERSAL_DEPTH: usize = 32;
const MAX_TRAVERSAL_NODES: usize = 4096;
const MAX_OPERATION_CANDIDATES: usize = 256;
const MAX_OPERATION_TEXT_BYTES: usize = 16 * 1024;
const MAX_OPERATION_TEXT_NODES: usize = 128;
const MAX_IDENTIFIER_CHARS: usize = 256;
const MAX_PARAM_NAMES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProgressiveOperation {
    pub(crate) module: String,
    pub(crate) operation: String,
    pub(crate) method: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) score: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProgressiveExecution {
    pub(crate) view: Option<remote_ui::ViewSpec>,
    pub(crate) operation: ProgressiveOperation,
}

pub(crate) async fn execute_first_matching<F>(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    query: &str,
    params: serde_json::Value,
    scorer: F,
) -> Option<ProgressiveExecution>
where
    F: Fn(&str, &str) -> i32,
{
    let candidates = search_operations(client, origin, token, query, scorer).await?;
    for candidate in candidates.into_iter().take(4) {
        let described = describe_operation(client, origin, token, &candidate).await;
        let described_view = described
            .as_ref()
            .and_then(|value| view_spec_from_json(value, origin));
        if let Some(execution) = execute_operation(
            client,
            origin,
            token,
            &candidate,
            shaped_params(described.as_ref(), &params),
            described_view,
        )
        .await
        {
            return Some(execution);
        }
    }
    None
}

pub(crate) async fn search_operations<F>(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    query: &str,
    scorer: F,
) -> Option<Vec<ProgressiveOperation>>
where
    F: Fn(&str, &str) -> i32,
{
    let search = post_capability_json(
        client,
        token,
        &capability_url(origin),
        serde_json::json!({
            "action": "search",
            "query": query,
        }),
    )
    .await?;
    Some(operation_candidates(&search, scorer))
}

pub(crate) async fn describe_operation(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    operation: &ProgressiveOperation,
) -> Option<serde_json::Value> {
    post_capability_json(
        client,
        token,
        &capability_url(origin),
        serde_json::json!({
            "action": "describe",
            "module": operation.module.as_str(),
            "operation": operation.operation.as_str(),
        }),
    )
    .await
}

pub(crate) async fn execute_operation(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    operation: &ProgressiveOperation,
    params: serde_json::Value,
    fallback_view: Option<remote_ui::ViewSpec>,
) -> Option<ProgressiveExecution> {
    let text = post_capability_text(
        client,
        token,
        &capability_url(origin),
        serde_json::json!({
            "action": "execute",
            "module": operation.module.as_str(),
            "operation": operation.operation.as_str(),
            "shaped": true,
            "params": params,
        }),
    )
    .await?;
    let view = remote_ui::find_view_url(&text, Some(origin)).or(fallback_view);
    Some(ProgressiveExecution {
        view,
        operation: operation.clone(),
    })
}

fn capability_url(origin: &str) -> String {
    format!(
        "{}/api/v1/kernel/capabilities",
        origin.trim_end_matches('/')
    )
}

fn shaped_params(
    described: Option<&serde_json::Value>,
    defaults: &serde_json::Value,
) -> serde_json::Value {
    let Some(described) = described else {
        return defaults.clone();
    };
    let names = capability_param_names(described);
    if names.is_empty() {
        return defaults.clone();
    }
    let mut params = serde_json::Map::new();
    for name in names {
        if let Some(value) = pick_default_param(defaults, &name) {
            params.insert(name, value);
        }
    }
    if params.is_empty() {
        defaults.clone()
    } else {
        serde_json::Value::Object(params)
    }
}

fn pick_default_param(defaults: &serde_json::Value, name: &str) -> Option<serde_json::Value> {
    let object = defaults.as_object()?;
    if let Some(value) = object.get(name) {
        return Some(value.clone());
    }
    let lower = name.to_ascii_lowercase();
    let aliases = if lower.contains("function") && lower.contains("ref") {
        &["functionRef", "ref", "worker", "name"][..]
    } else if lower == "ref" || lower.ends_with("ref") {
        &["ref", "functionRef", "worker", "name"][..]
    } else if lower.contains("asset") && lower.contains("id") {
        &["assetId", "id"][..]
    } else if lower == "input" || lower.ends_with("input") {
        &["input", "payload", "body"][..]
    } else if lower == "inputs" || lower.ends_with("inputs") {
        &["inputs", "batch", "payloads"][..]
    } else if lower.contains("kind") {
        &["agentKind", "kind"][..]
    } else if lower.contains("config") {
        &["config"][..]
    } else if lower.contains("timeout") {
        &["timeoutMs", "timeout"][..]
    } else if lower.contains("idempotency") {
        &["idempotencyKey"][..]
    } else {
        &[][..]
    };
    aliases.iter().find_map(|alias| object.get(*alias).cloned())
}

pub(crate) fn operation_candidates<F>(
    value: &serde_json::Value,
    scorer: F,
) -> Vec<ProgressiveOperation>
where
    F: Fn(&str, &str) -> i32,
{
    let mut out = Vec::new();
    let mut stack = vec![(value, 0usize)];
    let mut visited = 0usize;
    while let Some((value, depth)) = stack.pop() {
        if visited >= MAX_TRAVERSAL_NODES || out.len() >= MAX_OPERATION_CANDIDATES {
            break;
        }
        if depth > MAX_TRAVERSAL_DEPTH {
            continue;
        }
        visited += 1;
        match value {
            serde_json::Value::Object(obj) => {
                let module = capability_module(obj);
                let operation = first_string(obj, &["operation", "operationName", "name", "id"]);
                if let (Some(module), Some(operation)) = (module, operation) {
                    let text = object_strings(value).to_ascii_lowercase();
                    let score = scorer(&text, &operation.to_ascii_lowercase());
                    if score > 0 {
                        out.push(ProgressiveOperation {
                            module,
                            operation,
                            method: first_string(obj, &["method"]),
                            path: first_string(obj, &["path"]),
                            score,
                        });
                    }
                }
                if depth < MAX_TRAVERSAL_DEPTH {
                    stack.extend(obj.values().rev().map(|child| (child, depth + 1)));
                }
            }
            serde_json::Value::Array(items) if depth < MAX_TRAVERSAL_DEPTH => {
                stack.extend(items.iter().rev().map(|child| (child, depth + 1)));
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.operation.cmp(&b.operation))
    });
    out.dedup_by(|a, b| a.module == b.module && a.operation == b.operation);
    out
}

fn capability_module(obj: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    if let Some(module) = first_string(obj, &["module", "moduleName", "modulePath"]) {
        return Some(module);
    }
    let resource = first_string(obj, &["resource", "resourceName", "path"])?;
    if resource.starts_with('/') {
        let parts = resource
            .trim_start_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        return parts
            .iter()
            .position(|segment| *segment == "api")
            .and_then(|idx| parts.get(idx + 2))
            .or_else(|| parts.first())
            .map(|segment| (*segment).to_string());
    }
    resource
        .split('.')
        .next()
        .filter(|segment| !segment.trim().is_empty())
        .map(str::to_string)
}

fn first_string(obj: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| obj.get(*key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| sanitize_display_text(value, MAX_IDENTIFIER_CHARS))
        .filter(|value| !value.is_empty())
}

fn object_strings(value: &serde_json::Value) -> String {
    let mut out = String::new();
    let mut stack = vec![value];
    let mut visited = 0usize;
    while let Some(value) = stack.pop() {
        if visited >= MAX_OPERATION_TEXT_NODES || out.len() >= MAX_OPERATION_TEXT_BYTES {
            break;
        }
        visited += 1;
        match value {
            serde_json::Value::String(text) => append_bounded_text(&mut out, text),
            serde_json::Value::Array(items) => stack.extend(items.iter().rev()),
            serde_json::Value::Object(obj) => {
                for (key, child) in obj.iter().rev() {
                    append_bounded_text(&mut out, key);
                    stack.push(child);
                }
            }
            _ => {}
        }
    }
    out
}

fn append_bounded_text(output: &mut String, text: &str) {
    if output.len() >= MAX_OPERATION_TEXT_BYTES {
        return;
    }
    output.push(' ');
    let remaining = MAX_OPERATION_TEXT_BYTES.saturating_sub(output.len());
    if text.len() <= remaining {
        output.push_str(text);
        return;
    }
    let mut boundary = remaining.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    output.push_str(&text[..boundary]);
}

fn capability_param_names(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(value, 0usize)];
    let mut visited = 0usize;
    while let Some((value, depth)) = stack.pop() {
        if visited >= MAX_TRAVERSAL_NODES || out.len() >= MAX_PARAM_NAMES {
            break;
        }
        if depth > MAX_TRAVERSAL_DEPTH {
            continue;
        }
        visited += 1;
        if let serde_json::Value::Object(obj) = value {
            for pointer in [
                "/params/properties",
                "/parameters/properties",
                "/inputSchema/properties",
                "/schema/properties",
                "/operation/parameters/properties",
                "/data/operation/inputSchema/body/properties",
                "/data/operation/inputSchema/properties",
            ] {
                if let Some(properties) = value.pointer(pointer).and_then(|v| v.as_object()) {
                    for name in properties.keys() {
                        if out.len() >= MAX_PARAM_NAMES {
                            break;
                        }
                        let name = sanitize_display_text(name, MAX_IDENTIFIER_CHARS);
                        if !name.is_empty() {
                            out.push(name);
                        }
                    }
                }
            }
            if depth < MAX_TRAVERSAL_DEPTH {
                stack.extend(obj.values().rev().map(|child| (child, depth + 1)));
            }
        } else if let serde_json::Value::Array(items) = value {
            if depth < MAX_TRAVERSAL_DEPTH {
                stack.extend(items.iter().rev().map(|child| (child, depth + 1)));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

async fn post_capability_json(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: serde_json::Value,
) -> Option<serde_json::Value> {
    let text = post_capability_text(client, token, url, body).await?;
    serde_json::from_str(&text).ok()
}

async fn post_capability_text(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: serde_json::Value,
) -> Option<String> {
    let body = serde_json::to_vec(&body).ok()?;
    if body.len() > MAX_CAPABILITY_REQUEST_BYTES {
        return None;
    }
    let resp = client
        .post(url)
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .ok()?;
    let status = resp.status();
    let text = bounded_response_text(resp).await?;
    if !status.is_success() || envelope_is_error(&text) {
        return None;
    }
    Some(text)
}

async fn bounded_response_text(response: reqwest::Response) -> Option<String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_CAPABILITY_RESPONSE_BYTES as u64)
    {
        return None;
    }
    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(MAX_CAPABILITY_RESPONSE_BYTES);
    let mut body = Vec::with_capacity(initial_capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if body.len().saturating_add(chunk.len()) > MAX_CAPABILITY_RESPONSE_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).ok()
}

fn envelope_is_error(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return true;
    };
    let code = value
        .get("code")
        .and_then(|v| v.as_i64())
        .or_else(|| value.get("statusCode").and_then(|v| v.as_i64()))
        .unwrap_or(200);
    code >= 400
}

fn view_spec_from_json(value: &serde_json::Value, origin: &str) -> Option<remote_ui::ViewSpec> {
    let text = serde_json::to_string(value).ok()?;
    remote_ui::find_view_url(&text, Some(origin))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn operation_candidates_score_and_sort_progressive_results() {
        let value = serde_json::json!({
            "data": {
                "results": [
                    {
                        "name": "FunctionController_batch",
                        "resource": "functions.batch",
                        "path": "/api/v1/functions/{ref}/batch",
                        "method": "POST",
                        "description": "Function as a Service batch MCP tools"
                    },
                    {
                        "name": "AssetController_list",
                        "resource": "assets",
                        "path": "/api/v1/assets",
                        "method": "GET"
                    }
                ]
            }
        });
        let candidates = operation_candidates(&value, |text, operation| {
            let mut score = 0;
            if text.contains("function") {
                score += 5;
            }
            if text.contains("mcp") {
                score += 5;
            }
            if operation.contains("batch") {
                score += 10;
            }
            score
        });
        assert_eq!(candidates[0].operation, "FunctionController_batch");
        assert_eq!(candidates[0].module, "functions");
        assert_eq!(candidates[0].method.as_deref(), Some("POST"));
    }

    #[test]
    fn operation_candidates_parse_dotted_and_rest_resources() {
        let value = serde_json::json!({
            "data": {
                "results": [
                    {
                        "name": "AgentDebugRunController_runAgentic",
                        "resource": "runtimes.agent_debug_runs.agentic",
                        "path": "/api/v1/runtimes/agent-debug-runs/agentic",
                        "method": "POST",
                        "description": "Agent as a Service run for agentic agent assets"
                    },
                    {
                        "name": "AgentBuildController_triggerAgentBuild",
                        "resource": "/api/v1/assets/{owner}/{name}/build/agent",
                        "path": "/api/v1/assets/{owner}/{name}/build/agent",
                        "method": "POST",
                        "description": "Trigger application agent build"
                    }
                ]
            }
        });
        let candidates = operation_candidates(&value, |text, operation| {
            let mut score = 0;
            if text.contains("agent as a service") || text.contains("agent") {
                score += 5;
            }
            if operation.contains("agentbuild") {
                score += 10;
            }
            score
        });

        assert_eq!(candidates[0].module, "assets");
        assert_eq!(candidates[1].module, "runtimes");
    }

    #[test]
    fn shaped_params_follow_described_schema_when_present() {
        let described = serde_json::json!({
            "data": {
                "operation": {
                    "inputSchema": {
                        "body": {
                            "properties": {
                                "ref": {},
                                "input": {},
                                "agentKind": {}
                            }
                        }
                    }
                }
            }
        });
        let defaults = serde_json::json!({
            "ref": "mcp-weather",
            "input": {"mode": "debug"},
            "inputs": [{"mode": "test"}],
            "agentKind": "tool",
            "extra": "ignored"
        });
        let params = shaped_params(Some(&described), &defaults);
        assert_eq!(params["ref"], "mcp-weather");
        assert_eq!(params["agentKind"], "tool");
        assert_eq!(params["input"]["mode"], "debug");
        assert!(params.get("extra").is_none());
    }

    #[test]
    fn progressive_traversal_is_bounded_and_terminal_safe() {
        let operations = (0..(MAX_OPERATION_CANDIDATES + 64))
            .map(|idx| {
                serde_json::json!({
                    "module": "functions\u{202e}",
                    "operation": format!("run-{idx}\u{1b}]8;;https://evil.invalid\u{7}hidden\u{1b}]8;;\u{7}"),
                    "description": "function execution"
                })
            })
            .collect::<Vec<_>>();
        let candidates = operation_candidates(&serde_json::json!(operations), |_, _| 1);
        assert_eq!(candidates.len(), MAX_OPERATION_CANDIDATES);
        for candidate in candidates {
            for text in [candidate.module, candidate.operation] {
                assert!(!text.contains('\u{1b}'), "{text:?}");
                assert!(!text.contains('\u{202e}'), "{text:?}");
                assert!(!text.contains("evil.invalid"), "{text:?}");
            }
        }

        let mut too_deep = serde_json::json!({
            "module": "functions",
            "operation": "too-deep",
            "description": "function execution"
        });
        for _ in 0..=MAX_TRAVERSAL_DEPTH {
            too_deep = serde_json::json!({"child": too_deep});
        }
        assert!(operation_candidates(&too_deep, |_, _| 1).is_empty());
    }

    #[test]
    fn progressive_parameter_discovery_is_bounded() {
        let properties = (0..(MAX_PARAM_NAMES + 64))
            .map(|idx| (format!("field-{idx}"), serde_json::json!({})))
            .collect::<serde_json::Map<_, _>>();
        let described = serde_json::json!({
            "data": {"operation": {"inputSchema": {"properties": properties}}}
        });
        let names = capability_param_names(&described);
        assert!(names.len() <= MAX_PARAM_NAMES, "{}", names.len());
    }

    #[tokio::test]
    async fn progressive_http_rejects_oversized_requests_before_network_io() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/capabilities", listener.local_addr().unwrap());
        let client = reqwest::Client::new();
        let output = post_capability_text(
            &client,
            "token",
            &url,
            serde_json::json!({"payload": "x".repeat(MAX_CAPABILITY_REQUEST_BYTES + 1)}),
        )
        .await;
        assert!(output.is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), listener.accept())
                .await
                .is_err(),
            "an oversized request must be rejected before opening a connection"
        );
    }

    #[tokio::test]
    async fn progressive_http_rejects_oversized_response_content_length() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/capabilities", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0u8; 4096];
            let _ = socket.read(&mut request).await;
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_CAPABILITY_RESPONSE_BYTES + 1
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.flush().await.unwrap();
        });

        let output = post_capability_text(
            &reqwest::Client::new(),
            "token",
            &url,
            serde_json::json!({"action": "search"}),
        )
        .await;
        assert!(output.is_none());
    }
}
