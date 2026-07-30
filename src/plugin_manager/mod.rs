//! Shared Plugin Manager application service.
//!
//! CLI, Web, and the management MCP adapter use this service instead of
//! independently assembling catalog state or lifecycle subprocess commands.
//! Registry trust and authorization remain owned by the umbrella A3S host;
//! package verification and mutation remain delegated to A3S Use.

mod capability;
mod catalog;
mod operation;
mod process;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tokio::sync::Mutex;

use crate::components::ComponentPaths;
use crate::registry::RegistryStore;

pub use capability::{PluginCapabilityEvidence, PluginCapabilityEvidenceStatus};
pub use catalog::{
    PluginMarketplaceItem, PluginMarketplaceSnapshot, PluginMarketplaceSource,
    PluginMarketplaceSourceKind, PluginMarketplaceSourceMetadata,
};
pub use process::{
    PluginApplyRequest, PluginLifecycleAction, PluginPackageToggleRequest, PluginPlanRequest,
};

pub type PluginInstallationIndex = BTreeMap<String, bool>;
pub type PluginManagerResult<T> = Result<T, PluginManagerError>;

const PLUGIN_OPERATION_TIMEOUT_SECONDS: u64 = 180;
const MARKETPLACE_REFRESH_TIMEOUT_SECONDS: u64 = 30;
const MAX_PLUGIN_COMMAND_OUTPUT: usize = 4 * 1024 * 1024;
const MAX_MARKETPLACE_REGISTRIES: usize = 64;
const MAX_MARKETPLACE_ITEMS: usize = 1_000;

#[derive(Debug, thiserror::Error)]
pub enum PluginManagerError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("{0}")]
    Timeout(String),
    #[error("{0}")]
    OperationFailed(String),
    #[error("{0}")]
    Upstream(String),
    #[error("{0}")]
    Infrastructure(String),
}

/// One process-local manager instance and its serialization boundaries.
///
/// The Tokio lock is shared by every adapter holding this manager. A separate
/// file lock serializes reviewed plans and lifecycle mutations with other host
/// processes. Marketplace reads take neither lock.
pub struct PluginManager {
    pub(super) component_paths: ComponentPaths,
    pub(super) registry_store: RegistryStore,
    process: process::A3sProcessAdapter,
    operation_store: operation::store::PluginOperationStore,
    operation_lock: Mutex<()>,
}

impl PluginManager {
    /// Construct the manager from the immutable host invocation context.
    pub fn from_host(config_path: &Path, workspace: &Path) -> PluginManagerResult<Self> {
        let component_paths = ComponentPaths::from_env_at(workspace)
            .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))?;
        let registry_root = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("registries");
        Ok(Self::new(
            config_path.to_path_buf(),
            workspace.to_path_buf(),
            component_paths,
            RegistryStore::new(registry_root),
        ))
    }

    pub fn new(
        config_path: PathBuf,
        workspace: PathBuf,
        component_paths: ComponentPaths,
        registry_store: RegistryStore,
    ) -> Self {
        let operation_store = operation::store(&component_paths.state_root);
        let process = process::A3sProcessAdapter::new(
            component_paths.current_exe.clone(),
            config_path,
            workspace,
        );
        Self {
            component_paths,
            registry_store,
            process,
            operation_store,
            operation_lock: Mutex::new(()),
        }
    }

    /// Browse every configured source and join it with the caller's immutable
    /// installed-state snapshot. Package archives are never requested.
    pub async fn marketplace(
        &self,
        installed: &PluginInstallationIndex,
    ) -> PluginManagerResult<PluginMarketplaceSnapshot> {
        catalog::marketplace(self, installed).await
    }

    /// Resolve the existing umbrella component dry-run through the one shared
    /// operation lock.
    pub async fn plan_operation(
        &self,
        request: &PluginPlanRequest,
    ) -> PluginManagerResult<serde_json::Value> {
        let _guard = self.operation_lock.lock().await;
        operation::plan(self, request).await
    }

    /// Apply an already reviewed umbrella component plan through the same
    /// serialized mutation path used by every adapter.
    pub async fn apply_operation(
        &self,
        request: &PluginApplyRequest,
    ) -> PluginManagerResult<serde_json::Value> {
        let _guard = self.operation_lock.lock().await;
        operation::apply(self, request).await
    }

    pub async fn set_package_enabled(
        &self,
        request: &PluginPackageToggleRequest,
    ) -> PluginManagerResult<serde_json::Value> {
        let _guard = self.operation_lock.lock().await;
        operation::set_enabled(self, request).await
    }
}
