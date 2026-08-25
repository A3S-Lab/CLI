//! Bridge the shared Plugin Manager into the process-local Use watcher.

use a3s::plugin_manager::PluginManager;
use a3s_use::plugin_runtime::RuntimeTaskDispatchRequest;
use async_trait::async_trait;

use crate::use_registry::runtime_tasks::{RuntimeTaskInvoker, RuntimeTaskOutcome};
use crate::use_registry::McpRuntimeResolver;

#[async_trait]
impl RuntimeTaskInvoker for PluginManager {
    fn has_runtime_provider(&self, provider_id: &str) -> bool {
        PluginManager::has_runtime_provider(self, provider_id)
    }

    async fn invoke_runtime_task(
        &self,
        request: RuntimeTaskDispatchRequest,
    ) -> anyhow::Result<RuntimeTaskOutcome> {
        let execution = PluginManager::invoke_runtime_task(self, request)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(RuntimeTaskOutcome {
            exit_code: execution.exit_code,
            stdout: execution.stdout,
            stderr: execution.stderr,
            truncated: execution.truncated,
        })
    }
}

#[async_trait]
impl McpRuntimeResolver for PluginManager {
    async fn resolve_streamable_http(
        &self,
        provider_id: &str,
        endpoint_ref: &str,
        endpoint_path: &str,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<String> {
        if cancellation.is_cancelled() {
            anyhow::bail!("managed MCP Runtime resolution was cancelled");
        }
        self.resolve_runtime_mcp_endpoint(provider_id, endpoint_ref, endpoint_path)
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }
}
