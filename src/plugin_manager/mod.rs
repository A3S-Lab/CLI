//! Shared Plugin Manager application service.
//!
//! CLI and the management MCP adapter use this service instead of
//! independently assembling catalog state or lifecycle subprocess commands.
//! Registry trust and authorization remain owned by the umbrella A3S host;
//! package verification and mutation remain delegated to A3S Use.

mod capability;
mod catalog;
mod enablement_authorization;
mod gateway_readiness;
mod managed_host;
mod operation;
mod policy;
mod process;
mod runtime_composition;
mod runtime_host;
mod shared_service;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

use crate::components::ComponentPaths;
use crate::registry::RegistryStore;

pub use a3s_use::plugin_runtime::{
    RuntimeTaskDispatchRequest, RuntimeTaskExecution, RuntimeTaskInvocation,
};
pub use a3s_use_extension::ExtensionLifecycleIdentity;
pub use capability::{
    PluginCapabilityEvidence, PluginCapabilityEvidenceStatus, PluginInstallationSnapshot,
    PluginInstalledPackage, PluginPackageReadiness, PluginPlannerEvidence,
};
pub use catalog::{
    PluginMarketplaceItem, PluginMarketplaceSnapshot, PluginMarketplaceSource,
    PluginMarketplaceSourceKind, PluginMarketplaceSourceMetadata,
};
pub use managed_host::{ManagedPluginHostManager, PluginManagedScopeFenceStore};
pub use operation::reviewed_enablement::{
    PluginEnablementApplyRequest, PluginEnablementPlanRequest,
};
pub use policy::{
    PluginAuthorizationPolicy, PluginPolicyEvaluation, PluginPolicyHandoff, PluginPolicyViolation,
    PluginPolicyViolationCode, PLUGIN_POLICY_HANDOFF_DIGEST_ENV, PLUGIN_POLICY_HANDOFF_SOURCE_ENV,
    PLUGIN_POLICY_SCHEMA,
};
pub use process::{PluginApplyRequest, PluginLifecycleAction, PluginPlanRequest};
pub use runtime_host::PluginRuntimeHost;

pub type PluginInstallationIndex = BTreeMap<String, bool>;
pub type PluginManagerResult<T> = Result<T, PluginManagerError>;

fn default_plan_scope() -> a3s_use_core::PlanScope {
    a3s_use_core::PlanScope {
        kind: a3s_use_core::PlanScopeKind::User,
        id: a3s_use::cognitive_package::COGNITIVE_PACKAGE_DEFAULT_SCOPE.to_string(),
    }
}

/// Immutable host policy shared by every Plugin Manager adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginManagerPolicy {
    /// Restrict catalog access and delegated lifecycle work to local state.
    pub offline: bool,
    /// Evaluate complete immutable plans independently of plugin content.
    pub authorization: PluginAuthorizationPolicy,
}

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
    policy: PluginManagerPolicy,
    process: process::A3sProcessAdapter,
    operation_store: operation::store::PluginOperationStore,
    runtime_host: PluginRuntimeHost,
    operation_lock: Mutex<()>,
}

impl PluginManager {
    /// Construct the manager from the immutable host invocation context.
    pub fn from_host(config_path: &Path, workspace: &Path) -> PluginManagerResult<Self> {
        Self::from_host_with_policy(config_path, workspace, PluginManagerPolicy::default())
    }

    pub fn from_host_with_policy(
        config_path: &Path,
        workspace: &Path,
        policy: PluginManagerPolicy,
    ) -> PluginManagerResult<Self> {
        let component_paths = ComponentPaths::from_env_at(workspace)
            .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))?;
        let registry_store = RegistryStore::from_component_paths(&component_paths, policy.offline);
        Ok(Self::new_with_policy(
            config_path.to_path_buf(),
            workspace.to_path_buf(),
            component_paths,
            registry_store,
            policy,
        ))
    }

    /// Construct the product host with an explicitly configured Runtime
    /// provider from the same trusted user or operator ACL that owns plugin
    /// authorization. Automatically discovered workspace configuration is not
    /// accepted as Runtime provider authority.
    pub async fn from_host_with_policy_and_runtime_config(
        config_path: &Path,
        workspace: &Path,
        runtime_config_path: Option<&Path>,
        policy: PluginManagerPolicy,
    ) -> PluginManagerResult<Self> {
        let component_paths = ComponentPaths::from_env_at(workspace)
            .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))?;
        let registry_store = RegistryStore::from_component_paths(&component_paths, policy.offline);
        let runtime_host =
            runtime_composition::compose(runtime_config_path, &component_paths).await?;
        Ok(Self::new_with_policy_and_runtime(
            config_path.to_path_buf(),
            workspace.to_path_buf(),
            component_paths,
            registry_store,
            policy,
            runtime_host,
        ))
    }

    pub fn new(
        config_path: PathBuf,
        workspace: PathBuf,
        component_paths: ComponentPaths,
        registry_store: RegistryStore,
    ) -> Self {
        Self::new_with_policy(
            config_path,
            workspace,
            component_paths,
            registry_store,
            PluginManagerPolicy::default(),
        )
    }

    pub fn new_with_policy(
        config_path: PathBuf,
        workspace: PathBuf,
        component_paths: ComponentPaths,
        registry_store: RegistryStore,
        policy: PluginManagerPolicy,
    ) -> Self {
        Self::new_with_policy_and_runtime(
            config_path,
            workspace,
            component_paths,
            registry_store,
            policy,
            PluginRuntimeHost::default(),
        )
    }

    pub fn new_with_policy_and_runtime(
        config_path: PathBuf,
        workspace: PathBuf,
        component_paths: ComponentPaths,
        registry_store: RegistryStore,
        policy: PluginManagerPolicy,
        runtime_host: PluginRuntimeHost,
    ) -> Self {
        let operation_store = operation::store(&component_paths.state_root);
        let process = process::A3sProcessAdapter::new(
            component_paths.current_exe.clone(),
            config_path,
            workspace,
            policy.offline,
        );
        Self {
            component_paths,
            registry_store,
            policy,
            process,
            operation_store,
            runtime_host,
            operation_lock: Mutex::new(()),
        }
    }

    /// Browse every configured source and join it with the caller's immutable
    /// installed-state snapshot. Package archives are never requested.
    pub async fn marketplace(
        &self,
        installed: &PluginInstallationIndex,
    ) -> PluginManagerResult<PluginMarketplaceSnapshot> {
        let access = if self.policy.offline {
            catalog::CatalogAccess::Cached
        } else {
            catalog::CatalogAccess::Refresh
        };
        catalog::marketplace(self, installed, access).await
    }

    /// Browse only the last verified on-disk metadata snapshot. This performs
    /// no registry network request and reports missing or expired caches per
    /// source.
    pub async fn marketplace_cached(
        &self,
        installed: &PluginInstallationIndex,
    ) -> PluginManagerResult<PluginMarketplaceSnapshot> {
        catalog::marketplace(self, installed, catalog::CatalogAccess::Cached).await
    }

    /// Observe installed A3S Use plugin receipts through the immutable
    /// capability snapshot. Unavailable Use state remains explicit in the
    /// returned snapshot instead of being confused with an empty installation.
    pub async fn installation_snapshot(&self) -> PluginInstallationSnapshot {
        capability::installation_snapshot(self).await
    }

    /// Evaluate one complete Use operation plan through the immutable policy
    /// shared by CLI, TUI, and management MCP adapters.
    pub fn evaluate_plan_authority(
        &self,
        plan: &a3s_use_core::PluginOperationPlan,
    ) -> PluginManagerResult<PluginPolicyEvaluation> {
        self.policy.authorization.evaluate_plan(plan)
    }

    /// Re-evaluate a stored plan at apply and reject policy or decision drift.
    pub fn verify_plan_authority(
        &self,
        plan: &a3s_use_core::PluginOperationPlan,
    ) -> PluginManagerResult<PluginPolicyEvaluation> {
        self.policy.authorization.verify_plan_authority(plan)
    }

    pub fn authorization_policy(&self) -> &PluginAuthorizationPolicy {
        &self.policy.authorization
    }

    /// Return the immutable policy selected at the host invocation boundary.
    pub fn policy(&self) -> &PluginManagerPolicy {
        &self.policy
    }

    /// Report whether this immutable host composition contains a named Runtime
    /// provider. Capability watchers use this read-only gate before exposing a
    /// reviewed managed Task to an Agent session.
    pub fn has_runtime_provider(&self, provider_id: &str) -> bool {
        self.runtime_host.has_provider(provider_id)
    }

    /// Invoke one exact published managed Tool Task generation.
    ///
    /// This path is shared by CLI, TUI, and agent adapters. It intentionally
    /// avoids the lifecycle operation mutex: the Use Registry lease admits or
    /// rejects the exact generation and keeps accepted work alive until
    /// Runtime output capture and cleanup have completed.
    pub async fn invoke_runtime_task(
        &self,
        request: RuntimeTaskDispatchRequest,
    ) -> PluginManagerResult<RuntimeTaskExecution> {
        self.runtime_host
            .invoke_runtime_task(&self.component_paths, request)
            .await
            .map_err(runtime_dispatch_error)
    }

    /// Gracefully stop host-owned Runtime transports before the process or
    /// TUI releases its Plugin Manager.
    pub async fn shutdown(&self) {
        self.runtime_host.shutdown().await;
    }

    /// Compose the Use-owned Plugin Manager application service over this
    /// host's exact Registry, lifecycle, Runtime, UI, and authorization
    /// boundaries. Presentation adapters must call this service instead of
    /// constructing their own catalog, plan, or mutation path.
    pub fn shared_service(
        &self,
    ) -> PluginManagerResult<a3s_use::plugin_manager::PluginManagerService> {
        shared_service::compose(self)
    }

    /// Resolve the existing umbrella component dry-run through the one shared
    /// operation lock.
    pub async fn plan_operation(
        &self,
        request: &PluginPlanRequest,
    ) -> PluginManagerResult<serde_json::Value> {
        self.plan_operation_for_actor(request, a3s_use_core::PlanActor::User)
            .await
    }

    /// Create a reviewed plan for a host-authenticated actor.
    ///
    /// Adapters must select the actor from their trusted invocation boundary;
    /// plugin input never chooses it.
    pub async fn plan_operation_for_actor(
        &self,
        request: &PluginPlanRequest,
        actor: a3s_use_core::PlanActor,
    ) -> PluginManagerResult<serde_json::Value> {
        let _guard = self.operation_lock.lock().await;
        operation::plan(self, request, actor).await
    }

    /// Apply an already reviewed umbrella component plan through the same
    /// serialized mutation path used by every adapter.
    pub async fn apply_operation(
        &self,
        request: &PluginApplyRequest,
    ) -> PluginManagerResult<serde_json::Value> {
        self.apply_operation_with_confirmation(request, false).await
    }

    /// Apply a reviewed plan after a trusted user-facing adapter collected
    /// exact confirmation for its operation ID and canonical digest.
    pub async fn apply_confirmed_operation(
        &self,
        request: &PluginApplyRequest,
    ) -> PluginManagerResult<serde_json::Value> {
        self.apply_operation_with_confirmation(request, true).await
    }

    async fn apply_operation_with_confirmation(
        &self,
        request: &PluginApplyRequest,
        confirmed: bool,
    ) -> PluginManagerResult<serde_json::Value> {
        let _guard = self.operation_lock.lock().await;
        let enablement = operation::reviewed_enablement::apply_if_present(
            self,
            &PluginEnablementApplyRequest {
                operation_id: request.operation_id.clone(),
                plan_digest: request.plan_digest.clone(),
            },
            confirmed,
        )
        .await?;
        if let Some(result) = enablement {
            return Ok(result);
        }
        operation::apply(self, request, confirmed).await
    }

    pub async fn plan_package_enablement(
        &self,
        request: &PluginEnablementPlanRequest,
    ) -> PluginManagerResult<serde_json::Value> {
        let _guard = self.operation_lock.lock().await;
        operation::reviewed_enablement::plan(self, request).await
    }

    pub async fn apply_package_enablement(
        &self,
        request: &PluginEnablementApplyRequest,
    ) -> PluginManagerResult<serde_json::Value> {
        self.apply_package_enablement_with_confirmation(request, false)
            .await
    }

    pub async fn apply_confirmed_package_enablement(
        &self,
        request: &PluginEnablementApplyRequest,
    ) -> PluginManagerResult<serde_json::Value> {
        self.apply_package_enablement_with_confirmation(request, true)
            .await
    }

    async fn apply_package_enablement_with_confirmation(
        &self,
        request: &PluginEnablementApplyRequest,
        confirmed: bool,
    ) -> PluginManagerResult<serde_json::Value> {
        let _guard = self.operation_lock.lock().await;
        operation::reviewed_enablement::apply(self, request, confirmed).await
    }
}

fn runtime_dispatch_error(error: a3s_use_core::UseError) -> PluginManagerError {
    let message = format!("{}: {}", error.code, error.message);
    match error.code.as_str() {
        "use.plugin.runtime.input_invalid" | "use.plugin.runtime.binding_path_invalid" => {
            PluginManagerError::InvalidRequest(message)
        }
        "use.plugin.runtime.deadline_exceeded" => PluginManagerError::Timeout(message),
        "use.plugin.runtime.provider_unavailable" => PluginManagerError::Upstream(message),
        "use.extension.io"
        | "use.extension.lifecycle_receipt_invalid"
        | "use.extension.receipt_invalid"
        | "use.extension.registry_invalid"
        | "use.plugin.runtime.binding_io"
        | "use.plugin.runtime.binding_receipt_invalid" => {
            PluginManagerError::Infrastructure(message)
        }
        _ => PluginManagerError::OperationFailed(message),
    }
}
