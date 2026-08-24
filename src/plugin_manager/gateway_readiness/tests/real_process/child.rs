use std::net::{Ipv4Addr, SocketAddr};

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use super::backend::{
    QualificationServiceKind, CHILD_GENERATION_ENV, CHILD_KIND_ENV, CHILD_MARKER_ENV,
    CHILD_PORT_ENV, CHILD_UNIT_ID_ENV,
};

#[derive(Clone)]
struct ChildState {
    kind: QualificationServiceKind,
    generation: u64,
    unit_id: String,
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_service_child() {
    if std::env::var(CHILD_MARKER_ENV).as_deref() != Ok("1") {
        return;
    }
    let kind = match std::env::var(CHILD_KIND_ENV).as_deref() {
        Ok("tool") => QualificationServiceKind::Tool,
        Ok("mcp") => QualificationServiceKind::Mcp,
        value => panic!("invalid qualification service kind: {value:?}"),
    };
    let port = required_positive_u16(CHILD_PORT_ENV);
    let generation = std::env::var(CHILD_GENERATION_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .expect("qualification generation must be positive");
    let unit_id = std::env::var(CHILD_UNIT_ID_ENV)
        .ok()
        .filter(|value| !value.is_empty() && value.len() <= 255)
        .expect("qualification unit ID must be bounded");
    let state = ChildState {
        kind,
        generation,
        unit_id,
    };
    let app = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/api", get(tool_get))
        .route("/mcp", post(mcp_post))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .await
        .expect("qualification child must bind its reserved port");
    axum::serve(listener, app)
        .await
        .expect("qualification child server failed");
}

fn required_positive_u16(name: &str) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| panic!("{name} must be a positive port"))
}

async fn tool_get(State(state): State<ChildState>) -> Response {
    if state.kind != QualificationServiceKind::Tool {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(serde_json::json!({
        "kind": "tool",
        "generation": state.generation,
        "unitId": state.unit_id,
    }))
    .into_response()
}

async fn mcp_post(
    State(state): State<ChildState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    if state.kind != QualificationServiceKind::Mcp {
        return StatusCode::NOT_FOUND.into_response();
    }
    let method = request.get("method").and_then(serde_json::Value::as_str);
    match method {
        Some("initialize")
            if request["params"]["protocolVersion"]
                == serde_json::Value::String("2025-06-18".to_string()) =>
        {
            let mut response = Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request["id"].clone(),
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "a3s-cli-runtime-qualification",
                        "version": "1.0.0"
                    }
                }
            }))
            .into_response();
            response.headers_mut().insert(
                "mcp-session-id",
                HeaderValue::from_str(&format!("qualification-{}", state.generation))
                    .expect("qualification MCP session ID must be valid"),
            );
            response
        }
        Some("notifications/initialized") => StatusCode::ACCEPTED.into_response(),
        Some("tools/list") => Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": request["id"].clone(),
            "result": {
                "tools": [{
                    "name": "generation",
                    "description": "Return the exact Runtime generation.",
                    "inputSchema": { "type": "object", "properties": {} }
                }]
            }
        }))
        .into_response(),
        Some("tools/call") if request["params"]["name"] == "generation" => {
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request["id"].clone(),
                "result": {
                    "content": [{
                        "type": "text",
                        "text": format!("{}:{}", state.generation, state.unit_id)
                    }],
                    "isError": false
                }
            }))
            .into_response()
        }
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}
