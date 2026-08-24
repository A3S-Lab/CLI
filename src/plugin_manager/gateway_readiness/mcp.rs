//! Bounded standard MCP initialize through the private Gateway endpoint.

use std::future::Future;
use std::net::IpAddr;

use a3s_use::plugin_runtime::RuntimeMcpInitializeEvidence;
use a3s_use_core::{UseError, UseResult};
use rmcp::model::{ClientInfo, ProtocolVersion};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use rmcp::ServiceExt;
use tokio::time::Instant;
use url::{Host, Url};

use super::epoch_millis;

pub(super) async fn initialize_mcp(
    endpoint: &str,
    expected_protocol: &str,
    observed_at_ms: u64,
    deadline: Option<Instant>,
) -> UseResult<RuntimeMcpInitializeEvidence> {
    validate_gateway_endpoint(endpoint)?;
    let expected: ProtocolVersion =
        serde_json::from_value(serde_json::Value::String(expected_protocol.to_string()))
            .map_err(|_| mcp_error("The reviewed MCP protocol version is invalid."))?;
    let client_info = ClientInfo {
        protocol_version: expected,
        ..ClientInfo::default()
    };
    let http = reqwest_rmcp::Client::builder()
        .no_proxy()
        .redirect(reqwest_rmcp::redirect::Policy::none())
        .build()
        .map_err(|_| mcp_error("The private MCP HTTP client could not be constructed."))?;
    let transport = StreamableHttpClientTransport::with_client(
        http,
        StreamableHttpClientTransportConfig::with_uri(endpoint.to_string()),
    );
    before_deadline(deadline, async move {
        let client = client_info
            .serve(transport)
            .await
            .map_err(|error| mcp_error(format!("MCP initialize failed: {error}")))?;
        let negotiated = client
            .peer_info()
            .ok_or_else(|| mcp_error("MCP initialize returned no negotiated server evidence."))?
            .protocol_version
            .to_string();
        if negotiated != expected_protocol {
            let _ = client.cancel().await;
            return Err(mcp_error(
                "MCP initialize negotiated a different protocol than the reviewed release.",
            ));
        }
        client
            .cancel()
            .await
            .map_err(|_| mcp_error("The MCP readiness session did not close cleanly."))?;
        let initialized_at_ms = epoch_millis("use.plugin.mcp_initialize_failed")?;
        if initialized_at_ms < observed_at_ms {
            return Err(mcp_error(
                "The host clock moved behind the Runtime observation during MCP initialize.",
            ));
        }
        RuntimeMcpInitializeEvidence::new(negotiated, initialized_at_ms)
    })
    .await
}

async fn before_deadline<T>(
    deadline: Option<Instant>,
    future: impl Future<Output = UseResult<T>>,
) -> UseResult<T> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, future)
            .await
            .map_err(|_| {
                mcp_error("MCP initialize exceeded the reviewed Runtime Service deadline.")
            })?,
        None => future.await,
    }
}

pub(super) fn validate_gateway_endpoint(endpoint: &str) -> UseResult<()> {
    let url = Url::parse(endpoint)
        .map_err(|_| mcp_error("The private Gateway returned an invalid MCP endpoint."))?;
    let address = match url.host() {
        Some(Host::Ipv4(address)) => Some(IpAddr::V4(address)),
        Some(Host::Ipv6(address)) => Some(IpAddr::V6(address)),
        Some(Host::Domain(_)) | None => None,
    }
    .filter(IpAddr::is_loopback);
    if url.scheme() != "http"
        || address.is_none()
        || url.port().is_none_or(|port| port == 0)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(mcp_error(
            "The private Gateway MCP endpoint must be a credential-free loopback HTTP URL.",
        ));
    }
    Ok(())
}

fn mcp_error(message: impl Into<String>) -> UseError {
    UseError::new("use.plugin.mcp_initialize_failed", message)
}
