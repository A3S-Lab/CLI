use a3s_acl::Block;
use tokio::io::AsyncReadExt;

use super::parse_value::{
    access, boolean, decision, http_method_list, machine_id_list, port_list,
    reject_unknown_attributes, required_access, required_string, segment_list, surface_list,
    unsigned, unsigned_u32, valid_dns_name, valid_portable_scope_path,
};
use super::{
    policy_error, PluginAuthorizationPolicy, PluginPolicyNetworkCeiling,
    PluginPolicyPermissionCeiling, PluginPolicyWorkspaceCeiling, PolicyFilesystemAccess,
    MAX_POLICY_BYTES, MAX_POLICY_CAPTURE_BYTES, MAX_POLICY_CPU_MILLIS, MAX_POLICY_PIDS,
    MAX_POLICY_PLAN_ITEMS, MAX_POLICY_RESOURCE_BYTES, MAX_POLICY_RULES, MAX_POLICY_TASK_TIMEOUT_MS,
    PLUGIN_POLICY_SCHEMA,
};
use crate::plugin_manager::PluginManagerResult;

const POLICY_ATTRIBUTES: &[&str] = &[
    "schema",
    "agent_install",
    "agent_upgrade",
    "agent_uninstall",
    "trusted_registries",
    "trusted_publishers",
    "allowed_surfaces",
    "max_download_bytes",
    "max_installed_bytes",
    "max_packages",
    "max_surfaces",
    "allow_user_scope",
    "workspace_ids",
    "max_workspaces",
];

const PERMISSION_ATTRIBUTES: &[&str] = &[
    "plugin_data",
    "temporary",
    "native_execution",
    "child_process",
    "private_service",
    "secrets",
    "ui_http",
    "ui_methods",
    "max_ui_path_prefixes",
    "max_cpu_millis",
    "max_memory_bytes",
    "max_pids",
    "max_ephemeral_storage_bytes",
    "max_task_timeout_ms",
    "max_stdout_bytes",
    "max_stderr_bytes",
];

impl PluginAuthorizationPolicy {
    /// Parse the optional top-level `plugins` block from an A3S ACL document.
    ///
    /// Other top-level blocks belong to the normal A3S configuration and are
    /// intentionally ignored. The selected `plugins` block is strict.
    pub fn from_acl(source: &str) -> PluginManagerResult<Self> {
        if source.len() > MAX_POLICY_BYTES {
            return Err(policy_error(format!(
                "ACL input must not exceed {MAX_POLICY_BYTES} bytes"
            )));
        }
        if source.trim().is_empty() {
            return Ok(Self::default());
        }
        let document = a3s_acl::parse_acl(source)
            .map_err(|error| policy_error(format!("ACL parsing failed: {error}")))?;
        let blocks = document
            .blocks
            .iter()
            .filter(|block| block.name == "plugins")
            .collect::<Vec<_>>();
        match blocks.as_slice() {
            [] => Ok(Self::default()),
            [block] => parse_policy(block),
            _ => Err(policy_error(
                "the A3S ACL document contains more than one `plugins` block",
            )),
        }
    }

    /// Read one host-owned ACL file through a fixed input bound.
    pub async fn from_acl_file(path: &std::path::Path) -> PluginManagerResult<Self> {
        let file = tokio::fs::File::open(path).await.map_err(|error| {
            crate::plugin_manager::PluginManagerError::Infrastructure(format!(
                "could not open plugin policy source {}: {error}",
                path.display()
            ))
        })?;
        let mut bytes = Vec::new();
        file.take((MAX_POLICY_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| {
                crate::plugin_manager::PluginManagerError::Infrastructure(format!(
                    "could not read plugin policy source {}: {error}",
                    path.display()
                ))
            })?;
        if bytes.len() > MAX_POLICY_BYTES {
            return Err(policy_error(format!(
                "ACL input must not exceed {MAX_POLICY_BYTES} bytes"
            )));
        }
        let source = std::str::from_utf8(&bytes).map_err(|_| {
            policy_error(format!("ACL input {} must be valid UTF-8", path.display()))
        })?;
        Self::from_acl(source)
    }
}

fn parse_policy(block: &Block) -> PluginManagerResult<PluginAuthorizationPolicy> {
    if !block.labels.is_empty() {
        return Err(policy_error("the `plugins` block does not accept labels"));
    }
    reject_unknown_attributes(block, POLICY_ATTRIBUTES, "plugins")?;
    let schema = required_string(block, "schema")?;
    if schema != PLUGIN_POLICY_SCHEMA {
        return Err(policy_error(format!(
            "unsupported plugin policy schema `{schema}`"
        )));
    }
    let permissions = block
        .blocks
        .iter()
        .filter(|nested| nested.name == "permissions")
        .collect::<Vec<_>>();
    if block
        .blocks
        .iter()
        .any(|nested| nested.name != "permissions")
    {
        return Err(policy_error(
            "the `plugins` block accepts only one nested `permissions` block",
        ));
    }
    let permissions = match permissions.as_slice() {
        [] => PluginPolicyPermissionCeiling::default(),
        [permissions] => parse_permissions(permissions)?,
        _ => {
            return Err(policy_error(
                "the `plugins` block contains more than one `permissions` block",
            ))
        }
    };

    Ok(PluginAuthorizationPolicy {
        schema: schema.to_string(),
        agent_install: decision(block, "agent_install")?,
        agent_upgrade: decision(block, "agent_upgrade")?,
        agent_uninstall: decision(block, "agent_uninstall")?,
        trusted_registries: segment_list(block, "trusted_registries")?,
        trusted_publishers: segment_list(block, "trusted_publishers")?,
        allowed_surfaces: surface_list(block, "allowed_surfaces")?,
        max_download_bytes: unsigned(block, "max_download_bytes", MAX_POLICY_RESOURCE_BYTES)?,
        max_installed_bytes: unsigned(block, "max_installed_bytes", MAX_POLICY_RESOURCE_BYTES)?,
        max_packages: unsigned_u32(block, "max_packages", MAX_POLICY_PLAN_ITEMS)?,
        max_surfaces: unsigned_u32(block, "max_surfaces", MAX_POLICY_PLAN_ITEMS)?,
        allow_user_scope: boolean(block, "allow_user_scope")?,
        workspace_ids: machine_id_list(block, "workspace_ids")?,
        max_workspaces: unsigned_u32(block, "max_workspaces", MAX_POLICY_PLAN_ITEMS)?,
        permissions,
    })
}

fn parse_permissions(block: &Block) -> PluginManagerResult<PluginPolicyPermissionCeiling> {
    if !block.labels.is_empty() {
        return Err(policy_error(
            "the `permissions` policy block does not accept labels",
        ));
    }
    reject_unknown_attributes(block, PERMISSION_ATTRIBUTES, "permissions")?;
    let mut network = Vec::new();
    let mut workspace = Vec::new();
    for nested in &block.blocks {
        match nested.name.as_str() {
            "network" => network.push(parse_network(nested)?),
            "workspace" => workspace.push(parse_workspace(nested)?),
            name => {
                return Err(policy_error(format!(
                    "unsupported permissions policy block `{name}`"
                )))
            }
        }
    }
    network.sort();
    workspace.sort();
    if network.windows(2).any(|pair| pair[0].host == pair[1].host) {
        return Err(policy_error(
            "the permissions policy contains a duplicate network host",
        ));
    }
    if workspace
        .windows(2)
        .any(|pair| pair[0].path == pair[1].path)
    {
        return Err(policy_error(
            "the permissions policy contains a duplicate workspace path",
        ));
    }
    if network.len() > MAX_POLICY_RULES || workspace.len() > MAX_POLICY_RULES {
        return Err(policy_error(format!(
            "permission rule count exceeds {MAX_POLICY_RULES}"
        )));
    }

    let secrets = boolean(block, "secrets")?;
    if secrets {
        return Err(policy_error(
            "unattended secret grants are unsupported; `secrets` must be false",
        ));
    }
    let ui_http = boolean(block, "ui_http")?;
    let ui_methods = http_method_list(block, "ui_methods")?;
    let max_ui_path_prefixes = unsigned_u32(block, "max_ui_path_prefixes", MAX_POLICY_PLAN_ITEMS)?;
    if ui_http != (!ui_methods.is_empty() && max_ui_path_prefixes > 0) {
        return Err(policy_error(
            "`ui_http` requires non-empty `ui_methods` and a positive `max_ui_path_prefixes`; disabled UI HTTP requires both to be empty or zero",
        ));
    }

    Ok(PluginPolicyPermissionCeiling {
        plugin_data: access(block, "plugin_data")?,
        temporary: access(block, "temporary")?,
        native_execution: boolean(block, "native_execution")?,
        child_process: boolean(block, "child_process")?,
        private_service: boolean(block, "private_service")?,
        secrets,
        ui_http,
        ui_methods,
        max_ui_path_prefixes,
        max_cpu_millis: unsigned(block, "max_cpu_millis", MAX_POLICY_CPU_MILLIS)?,
        max_memory_bytes: unsigned(block, "max_memory_bytes", MAX_POLICY_RESOURCE_BYTES)?,
        max_pids: unsigned_u32(block, "max_pids", MAX_POLICY_PIDS)?,
        max_ephemeral_storage_bytes: unsigned(
            block,
            "max_ephemeral_storage_bytes",
            MAX_POLICY_RESOURCE_BYTES,
        )?,
        max_task_timeout_ms: unsigned(block, "max_task_timeout_ms", MAX_POLICY_TASK_TIMEOUT_MS)?,
        max_stdout_bytes: unsigned(block, "max_stdout_bytes", MAX_POLICY_CAPTURE_BYTES)?,
        max_stderr_bytes: unsigned(block, "max_stderr_bytes", MAX_POLICY_CAPTURE_BYTES)?,
        network,
        workspace,
    })
}

fn parse_network(block: &Block) -> PluginManagerResult<PluginPolicyNetworkCeiling> {
    if block.labels.len() != 1 || !block.blocks.is_empty() {
        return Err(policy_error(
            "a `network` rule requires one host label and no nested blocks",
        ));
    }
    reject_unknown_attributes(block, &["ports"], "network")?;
    let host = block.labels[0].clone();
    if !valid_dns_name(&host) {
        return Err(policy_error(format!(
            "network host `{host}` is not an exact canonical DNS name or IP address"
        )));
    }
    let ports = port_list(block, "ports")?;
    if ports.is_empty() {
        return Err(policy_error(format!(
            "network host `{host}` requires at least one port"
        )));
    }
    Ok(PluginPolicyNetworkCeiling { host, ports })
}

fn parse_workspace(block: &Block) -> PluginManagerResult<PluginPolicyWorkspaceCeiling> {
    if block.labels.len() != 1 || !block.blocks.is_empty() {
        return Err(policy_error(
            "a `workspace` permission rule requires one path label and no nested blocks",
        ));
    }
    reject_unknown_attributes(block, &["access"], "workspace")?;
    let path = block.labels[0].clone();
    if !valid_portable_scope_path(&path) {
        return Err(policy_error(format!(
            "workspace policy path `{path}` is not portable and scope-relative"
        )));
    }
    let access = required_access(block, "access")?;
    if access == PolicyFilesystemAccess::None {
        return Err(policy_error(
            "workspace permission rules must grant `read` or `read-write`",
        ));
    }
    Ok(PluginPolicyWorkspaceCeiling { path, access })
}
