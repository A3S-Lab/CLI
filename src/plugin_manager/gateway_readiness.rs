//! Private Gateway composition for persistent Runtime Service readiness.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(any(target_os = "linux", test))]
use a3s_gateway::config::{EntrypointConfig, GatewayConfig};
use a3s_gateway::managed_service::{ManagedServiceBindingIdentity, ManagedServiceBindingRequest};
use a3s_gateway::Gateway;
use a3s_runtime::contract::{RuntimeObservation, RuntimeServiceEndpoint};
use a3s_use::plugin_lifecycle::{
    PluginLifecycleIntent, PluginMcpServiceReadiness, PluginRuntimeServiceReadinessHost,
};
use a3s_use::plugin_runtime::{
    RuntimeEndpointRef, RuntimeServiceBindingReceipt, RuntimeSurfaceContract, RuntimeSurfacePlan,
};
use a3s_use_core::{PluginSurfaceKind, UseError, UseResult};
use a3s_use_extension::{PluginMcpLaunch, PluginMcpSurface, ToolSurface, ToolWorkload};
use async_trait::async_trait;
use tokio::time::Instant;

#[cfg(any(target_os = "linux", test))]
use super::{PluginManagerError, PluginManagerResult};
#[cfg(any(target_os = "linux", test))]
use crate::components::ComponentPaths;

use self::binding::{
    binding_error, managed_health, managed_target, validate_binding_context,
    validate_retirement_context,
};
use self::mcp::initialize_mcp;
#[cfg(test)]
use self::mcp::validate_gateway_endpoint;

mod binding;
mod mcp;

const GATEWAY_ENTRYPOINT: &str = "a3s-runtime";
#[cfg(any(target_os = "linux", test))]
const GATEWAY_STATE_PATH: &str = "use/gateway/managed-runtime-services.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PrivateGatewayConfig {
    pub(super) address: SocketAddr,
}

/// One process-owned private Gateway and the Use readiness adapter over it.
///
/// The Gateway owns its listener and durable route-state lock for exactly as
/// long as the Plugin Manager host is alive. Product exit paths call
/// [`Self::shutdown`] explicitly; `Drop` is a final asynchronous safety net.
pub(super) struct GatewayRuntimeServiceHost {
    gateway: Arc<Gateway>,
    address: SocketAddr,
}

impl std::fmt::Debug for GatewayRuntimeServiceHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GatewayRuntimeServiceHost")
            .field("address", &self.address)
            .field("state", &self.gateway.state())
            .finish_non_exhaustive()
    }
}

impl GatewayRuntimeServiceHost {
    #[cfg(any(target_os = "linux", test))]
    pub(super) async fn start(
        private: PrivateGatewayConfig,
        paths: &ComponentPaths,
    ) -> PluginManagerResult<Arc<Self>> {
        validate_private_address(private.address)?;
        let state_file = paths.state_root.join(GATEWAY_STATE_PATH);
        if !state_file.is_absolute() {
            return Err(PluginManagerError::Infrastructure(
                "private Gateway state requires an absolute component state root".to_string(),
            ));
        }

        let mut config = GatewayConfig::default();
        config.entrypoints.clear();
        config.entrypoints.insert(
            GATEWAY_ENTRYPOINT.to_string(),
            EntrypointConfig::new(private.address.to_string()),
        );
        config.observability.metrics_enabled = false;
        config.observability.access_log_enabled = false;
        config.observability.tracing_enabled = false;

        let gateway = Arc::new(
            Gateway::with_managed_service_state(config, state_file).map_err(|error| {
                PluginManagerError::Infrastructure(format!(
                    "could not construct the private Runtime Gateway: {error}"
                ))
            })?,
        );
        gateway.start().await.map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "could not start the private Runtime Gateway: {error}"
            ))
        })?;
        Ok(Arc::new(Self {
            gateway,
            address: private.address,
        }))
    }

    pub(super) async fn shutdown(&self) {
        self.gateway.shutdown().await;
    }

    pub(super) fn resolve_mcp_endpoint(
        &self,
        endpoint_ref: &RuntimeEndpointRef,
        endpoint_path: &str,
    ) -> UseResult<String> {
        if self.gateway.is_shutdown() || !self.gateway.is_running() {
            return Err(binding_error(
                "The private Runtime Gateway is not available for MCP projection.",
            ));
        }
        if !valid_service_path(endpoint_path) {
            return Err(binding_error(
                "The projected MCP endpoint path is outside the reviewed HTTP contract.",
            ));
        }
        let binding_id = endpoint_ref
            .as_str()
            .strip_prefix("gateway:managed-services/")
            .ok_or_else(|| {
                binding_error(
                    "The projected MCP endpoint is not owned by the managed Runtime Gateway.",
                )
            })?;
        let endpoint = format!(
            "http://{}/_a3s/runtime/{binding_id}{endpoint_path}",
            self.address
        );
        self::mcp::validate_gateway_endpoint(&endpoint)?;
        Ok(endpoint)
    }

    #[cfg(test)]
    pub(super) fn gateway(&self) -> &Gateway {
        self.gateway.as_ref()
    }

    // Keep every reviewed identity axis explicit at this trust boundary. A
    // bundled product-specific request would duplicate Use's frozen contract.
    #[allow(clippy::too_many_arguments)]
    async fn bind_route(
        &self,
        intent: &PluginLifecycleIntent,
        expected_kind: PluginSurfaceKind,
        surface_id: &str,
        plan: &RuntimeSurfacePlan,
        observation: &RuntimeObservation,
        runtime_endpoint: &RuntimeServiceEndpoint,
        idempotency_key: &str,
        deadline_at_ms: Option<u64>,
        service_path: &str,
    ) -> UseResult<a3s_gateway::managed_service::ManagedServiceBinding> {
        validate_binding_context(
            intent,
            expected_kind,
            surface_id,
            plan,
            observation,
            runtime_endpoint,
        )?;
        let deadline = deadline_from_epoch_ms(deadline_at_ms, "use.plugin.gateway_bind_failed")?;
        let target = managed_target(
            &plan.surface(),
            plan.context().scope(),
            &observation.unit_id,
            observation.generation,
        )?;
        let health = managed_health(plan, runtime_endpoint)?;
        let request = ManagedServiceBindingRequest::new(
            idempotency_key,
            GATEWAY_ENTRYPOINT,
            target,
            runtime_endpoint.socket_addr(),
            service_path,
            health,
        )
        .map_err(|error| gateway_error("use.plugin.gateway_bind_failed", "bind", error))?;
        self.gateway
            .bind_managed_service(request, deadline)
            .await
            .map_err(|error| gateway_error("use.plugin.gateway_bind_failed", "bind", error))
    }

    fn receipt_identity(
        receipt: &RuntimeServiceBindingReceipt,
    ) -> UseResult<ManagedServiceBindingIdentity> {
        let target = managed_target(
            &receipt.surface,
            &receipt.scope,
            &receipt.unit_id,
            receipt.generation,
        )?;
        ManagedServiceBindingIdentity::new(receipt.endpoint_ref.as_str(), target).map_err(|error| {
            gateway_error(
                "use.plugin.gateway_binding_invalid",
                "reconstruct binding identity for",
                error,
            )
        })
    }
}

impl Drop for GatewayRuntimeServiceHost {
    fn drop(&mut self) {
        if self.gateway.is_shutdown() || !self.gateway.is_running() {
            return;
        }
        let gateway = self.gateway.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                gateway.shutdown().await;
            });
        }
    }
}

#[async_trait]
impl PluginRuntimeServiceReadinessHost for GatewayRuntimeServiceHost {
    async fn bind_tool_service(
        &self,
        intent: &PluginLifecycleIntent,
        surface: &ToolSurface,
        plan: &RuntimeSurfacePlan,
        observation: &RuntimeObservation,
        runtime_endpoint: &RuntimeServiceEndpoint,
        idempotency_key: &str,
        deadline_at_ms: Option<u64>,
    ) -> UseResult<RuntimeEndpointRef> {
        let ToolWorkload::Service(service) = &surface.workload else {
            return Err(binding_error(
                "A private Gateway route can bind only a persistent Tool Service.",
            ));
        };
        let RuntimeSurfaceContract::ToolService {
            port_name,
            base_path,
            ..
        } = plan.contract()
        else {
            return Err(binding_error(
                "The reviewed Runtime contract is not a Tool Service.",
            ));
        };
        if port_name != &runtime_endpoint.port_name || base_path != &service.base_path {
            return Err(binding_error(
                "The Tool Service route does not match its reviewed Runtime contract.",
            ));
        }
        let binding = self
            .bind_route(
                intent,
                PluginSurfaceKind::Tool,
                &surface.id,
                plan,
                observation,
                runtime_endpoint,
                idempotency_key,
                deadline_at_ms,
                base_path,
            )
            .await?;
        RuntimeEndpointRef::parse(binding.endpoint_ref().to_string())
    }

    async fn bind_mcp_service(
        &self,
        intent: &PluginLifecycleIntent,
        surface: &PluginMcpSurface,
        plan: &RuntimeSurfacePlan,
        observation: &RuntimeObservation,
        runtime_endpoint: &RuntimeServiceEndpoint,
        idempotency_key: &str,
        deadline_at_ms: Option<u64>,
    ) -> UseResult<PluginMcpServiceReadiness> {
        if !matches!(surface.launch, PluginMcpLaunch::StreamableHttp { .. }) {
            return Err(binding_error(
                "A private Gateway route can bind only Streamable HTTP MCP.",
            ));
        }
        let RuntimeSurfaceContract::McpService {
            port_name,
            endpoint_path,
            protocol_version,
            ..
        } = plan.contract()
        else {
            return Err(binding_error(
                "The reviewed Runtime contract is not an MCP Service.",
            ));
        };
        if port_name != &runtime_endpoint.port_name {
            return Err(binding_error(
                "The MCP endpoint does not match its reviewed Runtime port.",
            ));
        }
        let binding = self
            .bind_route(
                intent,
                PluginSurfaceKind::Mcp,
                &surface.id,
                plan,
                observation,
                runtime_endpoint,
                idempotency_key,
                deadline_at_ms,
                endpoint_path,
            )
            .await?;
        let deadline = deadline_from_epoch_ms(deadline_at_ms, "use.plugin.gateway_bind_failed")?;
        let initialize = initialize_mcp(
            binding.endpoint(),
            protocol_version,
            observation.observed_at_ms,
            deadline,
        )
        .await?;
        Ok(PluginMcpServiceReadiness::new(
            RuntimeEndpointRef::parse(binding.endpoint_ref().to_string())?,
            initialize,
        ))
    }

    async fn drain_service(
        &self,
        intent: &PluginLifecycleIntent,
        receipt: &RuntimeServiceBindingReceipt,
        idempotency_key: &str,
        deadline_at_ms: Option<u64>,
    ) -> UseResult<()> {
        validate_retirement_context(intent, receipt)?;
        let identity = Self::receipt_identity(receipt)?;
        let deadline = deadline_from_epoch_ms(deadline_at_ms, "use.plugin.gateway_drain_failed")?;
        self.gateway
            .drain_managed_service(&identity, idempotency_key, deadline)
            .await
            .map_err(|error| gateway_error("use.plugin.gateway_drain_failed", "drain", error))
    }

    async fn remove_service(
        &self,
        intent: &PluginLifecycleIntent,
        receipt: &RuntimeServiceBindingReceipt,
        idempotency_key: &str,
        deadline_at_ms: Option<u64>,
    ) -> UseResult<()> {
        validate_retirement_context(intent, receipt)?;
        let identity = Self::receipt_identity(receipt)?;
        let deadline = deadline_from_epoch_ms(deadline_at_ms, "use.plugin.gateway_remove_failed")?;
        self.gateway
            .remove_managed_service(&identity, idempotency_key, deadline)
            .await
            .map_err(|error| gateway_error("use.plugin.gateway_remove_failed", "remove", error))
    }
}

#[cfg(any(target_os = "linux", test))]
fn validate_private_address(address: SocketAddr) -> PluginManagerResult<()> {
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(PluginManagerError::InvalidRequest(
            "`plugin_runtime.gateway.address` must be a positive numeric loopback TCP socket"
                .to_string(),
        ));
    }
    Ok(())
}

fn valid_service_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 2048
        && !value.contains("//")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'?' | b'#' | b'\\'))
        && !value
            .split('/')
            .any(|segment| matches!(segment, "." | ".."))
}

fn deadline_from_epoch_ms(
    deadline_at_ms: Option<u64>,
    error_code: &'static str,
) -> UseResult<Option<Instant>> {
    let Some(deadline_at_ms) = deadline_at_ms else {
        return Ok(None);
    };
    let now_ms = epoch_millis(error_code)?;
    let remaining = deadline_at_ms.checked_sub(now_ms).ok_or_else(|| {
        UseError::new(
            error_code,
            "The Runtime Service lifecycle deadline has expired.",
        )
    })?;
    Instant::now()
        .checked_add(Duration::from_millis(remaining))
        .map(Some)
        .ok_or_else(|| {
            UseError::new(
                error_code,
                "The Runtime Service lifecycle deadline is outside the supported clock range.",
            )
        })
}

fn epoch_millis(error_code: &'static str) -> UseResult<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| UseError::new(error_code, "The host wall clock precedes the Unix epoch."))?
        .as_millis();
    u64::try_from(millis).map_err(|_| {
        UseError::new(
            error_code,
            "The host wall clock is outside the supported timestamp range.",
        )
    })
}

fn gateway_error(code: &'static str, action: &str, error: impl std::fmt::Display) -> UseError {
    UseError::new(
        code,
        format!("The private Gateway could not {action} the exact Runtime Service: {error}"),
    )
}

#[cfg(test)]
mod tests;
