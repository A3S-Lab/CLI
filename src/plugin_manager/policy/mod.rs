//! Host-owned authorization policy for immutable plugin operation plans.
//!
//! Package metadata declares a signed permission ceiling. This module applies
//! the separately trusted host policy that decides whether an exact plan may
//! run unattended, requires confirmation, or must be denied.

mod evaluate;
mod parse;
mod parse_value;

#[cfg(test)]
pub(crate) mod tests;

use a3s_use_core::{
    FilesystemAccess, HttpMethod, PlanActor, PlanAuthority, PlanPolicyDecision, PluginSurfaceKind,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{PluginManagerError, PluginManagerResult};

pub const PLUGIN_POLICY_SCHEMA: &str = "a3s.plugin-policy.v1";

const MAX_POLICY_RULES: usize = 256;
const MAX_POLICY_BYTES: usize = 256 * 1024;
const MAX_POLICY_RESOURCE_BYTES: u64 = 16 * 1024 * 1024 * 1024 * 1024;
const MAX_POLICY_CAPTURE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_POLICY_TASK_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_POLICY_CPU_MILLIS: u64 = 1_000_000;
const MAX_POLICY_PIDS: u32 = 1_000_000;
const MAX_POLICY_PLAN_ITEMS: u32 = 512;

/// Validated host policy used to authorize an immutable plugin operation plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginAuthorizationPolicy {
    schema: String,
    agent_install: PlanPolicyDecision,
    agent_upgrade: PlanPolicyDecision,
    agent_uninstall: PlanPolicyDecision,
    trusted_registries: Vec<String>,
    trusted_publishers: Vec<String>,
    allowed_surfaces: Vec<PluginSurfaceKind>,
    max_download_bytes: u64,
    max_installed_bytes: u64,
    max_packages: u32,
    max_surfaces: u32,
    allow_user_scope: bool,
    workspace_ids: Vec<String>,
    max_workspaces: u32,
    permissions: PluginPolicyPermissionCeiling,
}

impl Default for PluginAuthorizationPolicy {
    fn default() -> Self {
        Self {
            schema: PLUGIN_POLICY_SCHEMA.to_string(),
            agent_install: PlanPolicyDecision::Ask,
            agent_upgrade: PlanPolicyDecision::Ask,
            agent_uninstall: PlanPolicyDecision::Ask,
            trusted_registries: Vec::new(),
            trusted_publishers: Vec::new(),
            allowed_surfaces: Vec::new(),
            max_download_bytes: 0,
            max_installed_bytes: 0,
            max_packages: 0,
            max_surfaces: 0,
            allow_user_scope: false,
            workspace_ids: Vec::new(),
            max_workspaces: 0,
            permissions: PluginPolicyPermissionCeiling::default(),
        }
    }
}

impl PluginAuthorizationPolicy {
    /// Digest the normalized policy, independent of ACL attribute ordering.
    pub fn descriptor_digest(&self) -> PluginManagerResult<String> {
        let bytes = serde_json::to_vec(self).map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "failed to encode normalized plugin policy: {error}"
            ))
        })?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    fn configured_decision(
        &self,
        actor: PlanActor,
        action: a3s_use_core::PluginOperationAction,
    ) -> PlanPolicyDecision {
        if actor == PlanActor::User {
            return PlanPolicyDecision::Ask;
        }
        match action {
            a3s_use_core::PluginOperationAction::Install
            | a3s_use_core::PluginOperationAction::Enable => self.agent_install,
            a3s_use_core::PluginOperationAction::Upgrade => self.agent_upgrade,
            a3s_use_core::PluginOperationAction::Uninstall
            | a3s_use_core::PluginOperationAction::Disable => self.agent_uninstall,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginPolicyPermissionCeiling {
    plugin_data: PolicyFilesystemAccess,
    temporary: PolicyFilesystemAccess,
    native_execution: bool,
    child_process: bool,
    private_service: bool,
    secrets: bool,
    ui_http: bool,
    ui_methods: Vec<HttpMethod>,
    max_ui_path_prefixes: u32,
    max_cpu_millis: u64,
    max_memory_bytes: u64,
    max_pids: u32,
    max_ephemeral_storage_bytes: u64,
    max_task_timeout_ms: u64,
    max_stdout_bytes: u64,
    max_stderr_bytes: u64,
    network: Vec<PluginPolicyNetworkCeiling>,
    workspace: Vec<PluginPolicyWorkspaceCeiling>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
enum PolicyFilesystemAccess {
    #[default]
    None,
    Read,
    ReadWrite,
}

impl PolicyFilesystemAccess {
    fn allows(self, requested: FilesystemAccess) -> bool {
        match requested {
            FilesystemAccess::Read => self >= Self::Read,
            FilesystemAccess::ReadWrite => self == Self::ReadWrite,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginPolicyNetworkCeiling {
    host: String,
    ports: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginPolicyWorkspaceCeiling {
    path: String,
    access: PolicyFilesystemAccess,
}

/// Stable reason that prevented an exact plan from using unattended `allow`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginPolicyViolationCode {
    ChildProcessNotAllowed,
    DownloadSizeExceeded,
    FilesystemNotAllowed,
    InstalledSizeExceeded,
    NativeExecutionNotAllowed,
    NativeUnconfined,
    NetworkEgressNotAllowed,
    PackageCountExceeded,
    PrivateServiceNotAllowed,
    ResourceLimitExceeded,
    SecretsNotAllowed,
    SurfaceCountExceeded,
    SurfaceKindNotAllowed,
    UiHttpNotAllowed,
    UnsupportedSource,
    UntrustedPublisher,
    UntrustedRegistry,
    UserScopeNotAllowed,
    WorkspaceCountExceeded,
    WorkspaceNotAllowed,
}

/// One deterministic policy mismatch and the plan subject that caused it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPolicyViolation {
    pub code: PluginPolicyViolationCode,
    pub subject: String,
}

/// Policy result that can be copied into `PluginOperationPlan.authority`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPolicyEvaluation {
    pub actor: PlanActor,
    pub configured_decision: PlanPolicyDecision,
    pub decision: PlanPolicyDecision,
    pub policy_digest: String,
    pub confirmation_required: bool,
    pub violations: Vec<PluginPolicyViolation>,
}

impl PluginPolicyEvaluation {
    pub fn authority(&self) -> PlanAuthority {
        PlanAuthority {
            actor: self.actor,
            decision: self.decision,
            policy_digest: self.policy_digest.clone(),
            confirmation_required: self.confirmation_required,
        }
    }
}

fn policy_error(message: impl Into<String>) -> PluginManagerError {
    PluginManagerError::InvalidRequest(format!("invalid plugin policy: {}", message.into()))
}
