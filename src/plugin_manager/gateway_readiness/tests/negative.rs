use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::*;

fn reserve_address() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    address
}

async fn spawn_tool_upstream(body: &'static str) -> (SocketAddr, JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let app = Router::new()
            .route("/healthz", get(|| async { StatusCode::OK }))
            .route("/api", get(move || async move { body }));
        axum::serve(listener, app).await.unwrap();
    });
    (address, task)
}

fn deadline_ms() -> u64 {
    epoch_millis("test.clock").unwrap() + 5_000
}

fn bind_key(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn public_tool_endpoint(gateway: SocketAddr, endpoint_ref: &RuntimeEndpointRef) -> String {
    let binding_id = endpoint_ref
        .as_str()
        .strip_prefix("gateway:managed-services/")
        .unwrap();
    format!("http://{gateway}/_a3s/runtime/{binding_id}/api")
}

#[derive(Clone)]
struct ControlledMcpState {
    protocol: &'static str,
    initialize_delay: Duration,
    initializes: Arc<AtomicUsize>,
    initialized: Arc<AtomicUsize>,
}

async fn controlled_mcp_post(
    State(state): State<ControlledMcpState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    match request.get("method").and_then(serde_json::Value::as_str) {
        Some("initialize") => {
            state.initializes.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(state.initialize_delay).await;
            let mut response = Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request["id"].clone(),
                "result": {
                    "protocolVersion": state.protocol,
                    "capabilities": {},
                    "serverInfo": { "name": "a3s-negative-mcp", "version": "1.0.0" }
                }
            }))
            .into_response();
            response.headers_mut().insert(
                "mcp-session-id",
                axum::http::HeaderValue::from_static("negative-readiness-session"),
            );
            response
        }
        Some("notifications/initialized") => {
            state.initialized.fetch_add(1, Ordering::SeqCst);
            StatusCode::ACCEPTED.into_response()
        }
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn spawn_controlled_mcp(
    protocol: &'static str,
    initialize_delay: Duration,
) -> (SocketAddr, ControlledMcpState, JoinHandle<()>) {
    let state = ControlledMcpState {
        protocol,
        initialize_delay,
        initializes: Arc::new(AtomicUsize::new(0)),
        initialized: Arc::new(AtomicUsize::new(0)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_state = state.clone();
    let task = tokio::spawn(async move {
        let app = Router::new()
            .route("/mcp", post(controlled_mcp_post))
            .with_state(server_state);
        axum::serve(listener, app).await.unwrap();
    });
    (address, state, task)
}

#[tokio::test]
async fn mcp_initialize_rejects_negotiated_protocol_drift() {
    let (address, state, server) = spawn_controlled_mcp("2025-03-26", Duration::ZERO).await;
    let endpoint = format!("http://{address}/mcp");
    let observed_at_ms = epoch_millis("test.clock").unwrap();

    let error = initialize_mcp(
        &endpoint,
        "2025-06-18",
        observed_at_ms,
        Some(Instant::now() + Duration::from_secs(2)),
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, "use.plugin.mcp_initialize_failed");
    assert!(error.message.contains("different protocol"), "{error}");
    assert_eq!(state.initializes.load(Ordering::SeqCst), 1);
    assert_eq!(state.initialized.load(Ordering::SeqCst), 1);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn mcp_initialize_is_bounded_by_the_lifecycle_deadline() {
    let (address, state, server) =
        spawn_controlled_mcp("2025-06-18", Duration::from_millis(250)).await;
    let endpoint = format!("http://{address}/mcp");
    let observed_at_ms = epoch_millis("test.clock").unwrap();

    let error = initialize_mcp(
        &endpoint,
        "2025-06-18",
        observed_at_ms,
        Some(Instant::now() + Duration::from_millis(40)),
    )
    .await
    .unwrap_err();

    assert_eq!(error.code, "use.plugin.mcp_initialize_failed");
    assert!(error.message.contains("exceeded"), "{error}");
    assert!(state.initializes.load(Ordering::SeqCst) <= 1);
    assert_eq!(state.initialized.load(Ordering::SeqCst), 0);
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn bind_key_replay_rejects_request_drift_and_preserves_the_original_route() {
    let (first_upstream, first_server) = spawn_tool_upstream("first").await;
    let (second_upstream, second_server) = spawn_tool_upstream("second").await;
    let gateway_address = reserve_address();
    let temporary = tempfile::tempdir().unwrap();
    let paths = ComponentPaths::for_test(temporary.path());
    let host = GatewayRuntimeServiceHost::start(
        PrivateGatewayConfig {
            address: gateway_address,
        },
        &paths,
    )
    .await
    .unwrap();
    let (surface, plan) = tool_plan();
    let first_endpoint =
        RuntimeServiceEndpoint::node_local_tcp("http", first_upstream.port()).unwrap();
    let first_observation = running_observation(&plan, &first_endpoint);
    let intent = lifecycle_intent();
    let key = bind_key('5');

    let endpoint_ref = host
        .bind_tool_service(
            &intent,
            &surface,
            &plan,
            &first_observation,
            &first_endpoint,
            &key,
            Some(deadline_ms()),
        )
        .await
        .unwrap();
    let second_endpoint =
        RuntimeServiceEndpoint::node_local_tcp("http", second_upstream.port()).unwrap();
    let second_observation = running_observation(&plan, &second_endpoint);
    let error = host
        .bind_tool_service(
            &intent,
            &surface,
            &plan,
            &second_observation,
            &second_endpoint,
            &key,
            Some(deadline_ms()),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, "use.plugin.gateway_bind_failed");
    assert!(error.message.contains("idempotency"), "{error}");
    let client = reqwest_rmcp::Client::builder().no_proxy().build().unwrap();
    let response = client
        .get(public_tool_endpoint(gateway_address, &endpoint_ref))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest_rmcp::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "first");

    host.shutdown().await;
    first_server.abort();
    second_server.abort();
    let _ = first_server.await;
    let _ = second_server.await;
}

#[tokio::test]
async fn binding_rejects_an_endpoint_not_owned_by_the_runtime_observation() {
    let (observed_upstream, observed_server) = spawn_tool_upstream("observed").await;
    let (substituted_upstream, substituted_server) = spawn_tool_upstream("substituted").await;
    let gateway_address = reserve_address();
    let temporary = tempfile::tempdir().unwrap();
    let paths = ComponentPaths::for_test(temporary.path());
    let host = GatewayRuntimeServiceHost::start(
        PrivateGatewayConfig {
            address: gateway_address,
        },
        &paths,
    )
    .await
    .unwrap();
    let (surface, plan) = tool_plan();
    let observed_endpoint =
        RuntimeServiceEndpoint::node_local_tcp("http", observed_upstream.port()).unwrap();
    let observation = running_observation(&plan, &observed_endpoint);
    let substituted_endpoint =
        RuntimeServiceEndpoint::node_local_tcp("http", substituted_upstream.port()).unwrap();

    let error = host
        .bind_tool_service(
            &lifecycle_intent(),
            &surface,
            &plan,
            &observation,
            &substituted_endpoint,
            &bind_key('6'),
            Some(deadline_ms()),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, "use.plugin.gateway_binding_invalid");
    host.shutdown().await;
    observed_server.abort();
    substituted_server.abort();
    let _ = observed_server.await;
    let _ = substituted_server.await;
}

#[tokio::test]
async fn binding_rejects_a_noncanonical_lifecycle_intent() {
    let (upstream, server) = spawn_tool_upstream("ready").await;
    let gateway_address = reserve_address();
    let temporary = tempfile::tempdir().unwrap();
    let paths = ComponentPaths::for_test(temporary.path());
    let host = GatewayRuntimeServiceHost::start(
        PrivateGatewayConfig {
            address: gateway_address,
        },
        &paths,
    )
    .await
    .unwrap();
    let (surface, plan) = tool_plan();
    let endpoint = RuntimeServiceEndpoint::node_local_tcp("http", upstream.port()).unwrap();
    let observation = running_observation(&plan, &endpoint);
    let mut intent = lifecycle_intent();
    intent.schema = "a3s.plugin-lifecycle-intent.invalid".to_string();

    let error = host
        .bind_tool_service(
            &intent,
            &surface,
            &plan,
            &observation,
            &endpoint,
            &bind_key('7'),
            Some(deadline_ms()),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code, "use.plugin.gateway_binding_invalid");
    host.shutdown().await;
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn receipt_retirement_rejects_target_scope_and_generation_substitution() {
    let (upstream, server) = spawn_tool_upstream("ready").await;
    let gateway_address = reserve_address();
    let temporary = tempfile::tempdir().unwrap();
    let paths = ComponentPaths::for_test(temporary.path());
    let host = GatewayRuntimeServiceHost::start(
        PrivateGatewayConfig {
            address: gateway_address,
        },
        &paths,
    )
    .await
    .unwrap();
    let endpoint = RuntimeServiceEndpoint::node_local_tcp("http", upstream.port()).unwrap();

    let (workspace_surface, workspace_plan) = tool_plan_for(PlanScopeKind::Workspace, 7);
    let workspace_observation = running_observation(&workspace_plan, &endpoint);
    let workspace_intent = lifecycle_intent_for(PlanScopeKind::Workspace, 7);
    let workspace_ref = host
        .bind_tool_service(
            &workspace_intent,
            &workspace_surface,
            &workspace_plan,
            &workspace_observation,
            &endpoint,
            &bind_key('8'),
            Some(deadline_ms()),
        )
        .await
        .unwrap();
    let workspace_receipt = tool_receipt(&workspace_plan, &workspace_observation, workspace_ref);

    let (user_surface, user_plan) = tool_plan_for(PlanScopeKind::User, 7);
    let user_observation = running_observation(&user_plan, &endpoint);
    let user_intent = lifecycle_intent_for(PlanScopeKind::User, 7);
    let user_ref = host
        .bind_tool_service(
            &user_intent,
            &user_surface,
            &user_plan,
            &user_observation,
            &endpoint,
            &bind_key('9'),
            Some(deadline_ms()),
        )
        .await
        .unwrap();
    let user_receipt = tool_receipt(&user_plan, &user_observation, user_ref);

    let (next_surface, next_plan) = tool_plan_for(PlanScopeKind::Workspace, 8);
    let next_observation = running_observation(&next_plan, &endpoint);
    let next_intent = lifecycle_intent_for(PlanScopeKind::Workspace, 8);
    let next_ref = host
        .bind_tool_service(
            &next_intent,
            &next_surface,
            &next_plan,
            &next_observation,
            &endpoint,
            &bind_key('a'),
            Some(deadline_ms()),
        )
        .await
        .unwrap();
    let next_receipt = tool_receipt(&next_plan, &next_observation, next_ref);

    let scope_error = host
        .drain_service(
            &workspace_intent,
            &user_receipt,
            &bind_key('b'),
            Some(deadline_ms()),
        )
        .await
        .unwrap_err();
    assert_eq!(
        scope_error.code, "use.plugin.gateway_binding_invalid",
        "{scope_error}"
    );

    let generation_error = host
        .drain_service(
            &workspace_intent,
            &next_receipt,
            &bind_key('c'),
            Some(deadline_ms()),
        )
        .await
        .unwrap_err();
    assert_eq!(generation_error.code, "use.plugin.gateway_binding_invalid");

    let mut target_substitution = workspace_receipt.clone();
    target_substitution.endpoint_ref = user_receipt.endpoint_ref.clone();
    let target_error = host
        .drain_service(
            &workspace_intent,
            &target_substitution,
            &bind_key('d'),
            Some(deadline_ms()),
        )
        .await
        .unwrap_err();
    assert_eq!(target_error.code, "use.plugin.gateway_drain_failed");

    for receipt in [&workspace_receipt, &user_receipt, &next_receipt] {
        let identity = GatewayRuntimeServiceHost::receipt_identity(receipt).unwrap();
        assert!(host
            .gateway()
            .managed_service_status(&identity)
            .unwrap()
            .unwrap()
            .ready());
    }

    host.shutdown().await;
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn managed_gateway_state_has_one_process_owner() {
    let temporary = tempfile::tempdir().unwrap();
    let paths = ComponentPaths::for_test(temporary.path());
    let first = GatewayRuntimeServiceHost::start(
        PrivateGatewayConfig {
            address: reserve_address(),
        },
        &paths,
    )
    .await
    .unwrap();

    let error = GatewayRuntimeServiceHost::start(
        PrivateGatewayConfig {
            address: reserve_address(),
        },
        &paths,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("already owned"), "{error}");

    first.shutdown().await;
    drop(first);
    let recovered = GatewayRuntimeServiceHost::start(
        PrivateGatewayConfig {
            address: reserve_address(),
        },
        &paths,
    )
    .await
    .unwrap();
    recovered.shutdown().await;
}
