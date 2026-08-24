use std::net::SocketAddr;
use std::sync::Arc;

use a3s_use::plugin_lifecycle::{
    PluginLifecycleIntent, PluginMcpLifecycleHost, PluginToolLifecycleHost,
    RuntimePluginSurfaceLifecycleHost,
};
use a3s_use::plugin_runtime::{
    RuntimeBindingReceipt, RuntimeBindingStore, RuntimeServiceBindingReceipt, RuntimeSurfacePlan,
};

use super::{GatewayRuntimeServiceHost, PrivateGatewayConfig};
use crate::components::ComponentPaths;

pub(super) async fn prepare_generation(
    host: &RuntimePluginSurfaceLifecycleHost,
    generation: &super::ServiceGeneration,
    tool_key: char,
    mcp_key: char,
) {
    host.prepare_tool(&generation.intent, &generation.tool, &key(tool_key))
        .await
        .unwrap();
    host.prepare_mcp(&generation.intent, &generation.mcp, &key(mcp_key))
        .await
        .unwrap();
}

pub(super) async fn service_receipt(
    store: &RuntimeBindingStore,
    intent: &PluginLifecycleIntent,
    plan: &RuntimeSurfacePlan,
) -> RuntimeServiceBindingReceipt {
    match store
        .get_generation(&intent.scope, &plan.surface(), intent.generation)
        .await
        .unwrap()
        .unwrap()
    {
        RuntimeBindingReceipt::Service(receipt) => receipt,
        receipt => panic!("expected Runtime Service receipt, got {receipt:?}"),
    }
}

pub(super) async fn assert_no_receipt(
    store: &RuntimeBindingStore,
    intent: &PluginLifecycleIntent,
    plan: &RuntimeSurfacePlan,
) {
    assert!(store
        .get_generation(&intent.scope, &plan.surface(), intent.generation)
        .await
        .unwrap()
        .is_none());
}

pub(super) async fn assert_tool_generation(
    gateway: SocketAddr,
    receipt: &RuntimeServiceBindingReceipt,
    generation: u64,
    plan: &RuntimeSurfacePlan,
) {
    let response = http_client()
        .get(gateway_endpoint(gateway, receipt, "/api"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest_rmcp::StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["generation"], generation);
    assert_eq!(body["unitId"], plan.spec().unit_id);
}

pub(super) async fn assert_mcp_generation(
    gateway: SocketAddr,
    receipt: &RuntimeServiceBindingReceipt,
    generation: u64,
    plan: &RuntimeSurfacePlan,
) {
    let endpoint = gateway_endpoint(gateway, receipt, "/mcp");
    let client = http_client();
    let initialize = client
        .post(&endpoint)
        .header("accept", "application/json, text/event-stream")
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "a3s-cli-test", "version": "1.0.0" }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(initialize.status(), reqwest_rmcp::StatusCode::OK);
    let session = initialize
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let initialized = client
        .post(&endpoint)
        .header("mcp-session-id", &session)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(initialized.status(), reqwest_rmcp::StatusCode::ACCEPTED);
    let listed: serde_json::Value = client
        .post(&endpoint)
        .header("mcp-session-id", &session)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["result"]["tools"][0]["name"], "generation");
    let called: serde_json::Value = client
        .post(&endpoint)
        .header("mcp-session-id", session)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "generation", "arguments": {} }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        called["result"]["content"][0]["text"],
        format!("{generation}:{}", plan.spec().unit_id)
    );
}

pub(super) async fn assert_route_hidden(
    gateway: SocketAddr,
    receipt: &RuntimeServiceBindingReceipt,
    path: &str,
) {
    let response = http_client()
        .get(gateway_endpoint(gateway, receipt, path))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest_rmcp::StatusCode::NOT_FOUND);
}

pub(super) fn assert_binding_removed(
    gateway: &GatewayRuntimeServiceHost,
    receipt: &RuntimeServiceBindingReceipt,
) {
    let identity = GatewayRuntimeServiceHost::receipt_identity(receipt).unwrap();
    assert!(gateway
        .gateway()
        .managed_service_status(&identity)
        .unwrap()
        .is_none());
}

fn gateway_endpoint(
    gateway: SocketAddr,
    receipt: &RuntimeServiceBindingReceipt,
    path: &str,
) -> String {
    let binding_id = receipt
        .endpoint_ref
        .as_str()
        .strip_prefix("gateway:managed-services/")
        .unwrap();
    format!("http://{gateway}/_a3s/runtime/{binding_id}{path}")
}

fn http_client() -> reqwest_rmcp::Client {
    reqwest_rmcp::Client::builder().no_proxy().build().unwrap()
}

pub(super) fn key(value: char) -> String {
    format!("sha256:{}", value.to_string().repeat(64))
}

pub(super) fn reserve_loopback_address() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    address
}

pub(super) async fn start_gateway(
    address: SocketAddr,
    paths: &ComponentPaths,
) -> Arc<GatewayRuntimeServiceHost> {
    GatewayRuntimeServiceHost::start(PrivateGatewayConfig { address }, paths)
        .await
        .unwrap()
}
