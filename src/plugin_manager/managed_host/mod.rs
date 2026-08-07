//! Fenced remote adapter over the same Plugin Manager used by CLI and Web.

mod fence;
mod reviewed_enablement;
mod state;

use std::sync::Arc;

use a3s_use_core::{
    PluginHostApplyRequest, PluginHostApplyResult, PluginHostCapabilities,
    PluginHostEnablementPlanRequest, PluginHostEnablementPlanResult, PluginHostManager,
    PluginHostObservationRequest, PluginHostObservationResult, PluginHostPlanRequest,
    PluginHostPlanResult, UseError, UseResult, PLUGIN_HOST_OBSERVATION_RESULT_SCHEMA,
};
use async_trait::async_trait;

pub use fence::PluginManagedScopeFenceStore;

use super::{operation, PluginManager, PluginManagerError};

/// Sole production-facing `PluginHostManager` adapter for the umbrella A3S
/// host. It delegates planning and apply to [`PluginManager`], retaining the
/// same operation store, lifecycle journal, package graph, grants, bindings,
/// and capability publication used by local CLI and Web adapters.
pub struct ManagedPluginHostManager {
    manager: Arc<PluginManager>,
    capabilities: PluginHostCapabilities,
    reviewed_enablement: reviewed_enablement::ReviewedEnablementStore,
    fences: PluginManagedScopeFenceStore,
}

impl ManagedPluginHostManager {
    pub fn new(
        manager: Arc<PluginManager>,
        host_id: impl Into<String>,
        manager_build_id: impl Into<String>,
    ) -> UseResult<Self> {
        let capabilities =
            PluginHostCapabilities::v4(host_id, env!("CARGO_PKG_VERSION"), manager_build_id)?;
        let fences =
            PluginManagedScopeFenceStore::from_state_root(&manager.component_paths.state_root);
        let reviewed_enablement = reviewed_enablement::ReviewedEnablementStore::from_state_root(
            &manager.component_paths.state_root,
        );
        Ok(Self {
            manager,
            capabilities,
            reviewed_enablement,
            fences,
        })
    }

    /// Trusted local enrollment/rotation boundary. Remote protocol methods
    /// only verify this store and never mutate it.
    pub fn fence_store(&self) -> &PluginManagedScopeFenceStore {
        &self.fences
    }
}

#[async_trait]
impl PluginHostManager for ManagedPluginHostManager {
    async fn capabilities(&self) -> UseResult<PluginHostCapabilities> {
        self.capabilities.validate()?;
        Ok(self.capabilities.clone())
    }

    async fn plan(&self, request: PluginHostPlanRequest) -> UseResult<PluginHostPlanResult> {
        request.validate_for_capabilities(&self.capabilities)?;
        let _fence = self.fences.lock_and_verify(&request.scope).await?;
        let _manager = self.manager.operation_lock.lock().await;
        let (result, _) = operation::plan_managed(&self.manager, &request, &self.capabilities)
            .await
            .map_err(manager_error)?;
        result.validate_for(&request, &self.capabilities)?;
        Ok(result)
    }

    async fn apply(&self, request: PluginHostApplyRequest) -> UseResult<PluginHostApplyResult> {
        request.validate_for_capabilities(&self.capabilities)?;
        let _fence = self.fences.lock_and_verify(&request.scope).await?;
        let _manager = self.manager.operation_lock.lock().await;
        if let Some(result) = reviewed_enablement::apply(
            &self.manager,
            &self.reviewed_enablement,
            &self.capabilities,
            &request,
        )
        .await?
        {
            return Ok(result);
        }
        let result = operation::apply_managed(&self.manager, &request, &self.capabilities)
            .await
            .map_err(manager_error)?;
        result.validate_for(&request, &self.capabilities)?;
        Ok(result)
    }

    async fn plan_enablement(
        &self,
        request: PluginHostEnablementPlanRequest,
    ) -> UseResult<PluginHostEnablementPlanResult> {
        request.validate_for_capabilities(&self.capabilities)?;
        let _fence = self.fences.lock_and_verify(&request.scope).await?;
        let _manager = self.manager.operation_lock.lock().await;
        let result = reviewed_enablement::plan(
            &self.manager,
            &self.reviewed_enablement,
            &self.capabilities,
            &request,
        )
        .await?;
        result.validate_for(&request, &self.capabilities)?;
        Ok(result)
    }

    async fn observe(
        &self,
        request: PluginHostObservationRequest,
    ) -> UseResult<PluginHostObservationResult> {
        request.validate_for_capabilities(&self.capabilities)?;
        let _fence = self.fences.lock_and_verify(&request.scope).await?;
        let _manager = self.manager.operation_lock.lock().await;
        let status = state::observation_status(
            &self.manager,
            request.scope.plan_scope(),
            &request.package_id,
        )
        .await?;
        let observed_at_ms = state::now_ms()?;
        let result = PluginHostObservationResult {
            schema: PLUGIN_HOST_OBSERVATION_RESULT_SCHEMA.to_string(),
            request_id: request.request_id.clone(),
            assignment_generation: request.assignment_generation,
            capabilities_digest: request.capabilities_digest.clone(),
            scope: request.scope.clone(),
            package_id: request.package_id.clone(),
            observed_at_ms,
            status,
        };
        result.validate_for(&request, &self.capabilities)?;
        Ok(result)
    }
}

fn manager_error(error: PluginManagerError) -> UseError {
    let (code, message) = match error {
        PluginManagerError::InvalidRequest(_) => (
            "use.plugin.host_request_invalid",
            "The managed plugin request does not match its durable reviewed operation.",
        ),
        PluginManagerError::Timeout(_) => (
            "use.plugin.host_timeout",
            "The managed Plugin Manager operation timed out.",
        ),
        PluginManagerError::OperationFailed(_) => (
            "use.plugin.host_operation_failed",
            "The managed Plugin Manager operation failed without publishing a substituted result.",
        ),
        PluginManagerError::Upstream(_) => (
            "use.plugin.host_upstream_failed",
            "The managed Plugin Manager rejected inconsistent upstream package evidence.",
        ),
        PluginManagerError::Infrastructure(_) => (
            "use.plugin.host_manager_unavailable",
            "The managed Plugin Manager durable state is unavailable.",
        ),
    };
    UseError::new(code, message)
}

#[cfg(test)]
mod tests;
