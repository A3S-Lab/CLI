//! Product-host composition for the standard A3S Use Plugin Manager MCP.
//!
//! Tool schemas, routing, planning, durable replay, and apply remain owned by
//! `PluginManagerService`. Code injects only its trusted host policy,
//! lifecycle providers, Registry paths, and confirmation boundary.

use std::sync::Arc;

use a3s_use::cognitive_package::CognitiveRegistryAccess;
use a3s_use::plugin_manager::{
    FailClosedPluginManagerConfirmationProvider, PluginManagerMcpServer, PluginManagerService,
};

/// Serve the exact frozen manager-v4 toolset over standard MCP stdio framing.
///
/// An MCP tool call never implies user confirmation. Until the product host
/// supplies exact durable confirmation evidence, `plugin_apply_plan` remains
/// present but fails closed through the injected provider.
pub async fn serve_stdio(
    service: PluginManagerService,
    access: CognitiveRegistryAccess,
) -> anyhow::Result<()> {
    let server = PluginManagerMcpServer::with_registry_access(
        service,
        access,
        Arc::new(FailClosedPluginManagerConfirmationProvider),
    )
    .map_err(anyhow::Error::new)?;
    server.serve_stdio().await.map_err(anyhow::Error::new)
}
