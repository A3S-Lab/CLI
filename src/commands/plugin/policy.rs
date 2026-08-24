use std::path::{Path, PathBuf};

use a3s::plugin_manager::{
    PluginAuthorizationPolicy, PluginPolicyHandoff, PLUGIN_POLICY_HANDOFF_DIGEST_ENV,
    PLUGIN_POLICY_HANDOFF_SOURCE_ENV,
};

use crate::cli::context::InvocationContext;

/// Validated policy plus the exact source identity needed by child A3S
/// processes. Normal Code configuration and plugin authorization deliberately
/// remain separate values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostPluginAuthorization {
    policy: PluginAuthorizationPolicy,
    handoff: PluginPolicyHandoff,
}

impl HostPluginAuthorization {
    pub(crate) fn policy(&self) -> &PluginAuthorizationPolicy {
        &self.policy
    }

    pub(crate) fn handoff(&self) -> &PluginPolicyHandoff {
        &self.handoff
    }
}

pub(crate) async fn load_host_authorization_context(
    context: &InvocationContext,
) -> anyhow::Result<HostPluginAuthorization> {
    let user_config = context.user_config_path();
    let source = select_host_authorization_source(
        context.explicit_config.as_deref(),
        user_config.as_deref(),
    );
    load_from_source(source).await
}

/// Resolve a policy explicitly forwarded by a parent A3S process, falling back
/// to the normal operator-selected source for a direct invocation.
pub(crate) async fn load_forwarded_or_host_authorization(
    context: &InvocationContext,
) -> anyhow::Result<HostPluginAuthorization> {
    let Some(digest) = context.environment.utf8(PLUGIN_POLICY_HANDOFF_DIGEST_ENV)? else {
        return load_host_authorization_context(context).await;
    };
    let source = context
        .environment
        .nonempty_var_os(PLUGIN_POLICY_HANDOFF_SOURCE_ENV)
        .map(PathBuf::from);
    let handoff =
        PluginPolicyHandoff::from_locked_source(source, digest).map_err(super::manager_error)?;
    let policy = handoff
        .load_verified()
        .await
        .map_err(super::manager_error)?;
    Ok(HostPluginAuthorization { policy, handoff })
}

async fn load_from_source(source: Option<PathBuf>) -> anyhow::Result<HostPluginAuthorization> {
    let source = match source {
        Some(source) => Some(tokio::fs::canonicalize(&source).await.map_err(|error| {
            anyhow::anyhow!(
                "could not resolve host plugin policy source {}: {error}",
                source.display()
            )
        })?),
        None => None,
    };
    let policy = match source.as_deref() {
        Some(path) => PluginAuthorizationPolicy::from_acl_file(path)
            .await
            .map_err(super::manager_error),
        None => Ok(PluginAuthorizationPolicy::default()),
    }?;
    let handoff = PluginPolicyHandoff::new(&policy, source).map_err(super::manager_error)?;
    Ok(HostPluginAuthorization { policy, handoff })
}

fn select_host_authorization_source(
    explicit_config: Option<&Path>,
    user_config: Option<&Path>,
) -> Option<PathBuf> {
    explicit_config.map(Path::to_path_buf).or_else(|| {
        user_config
            .filter(|path| path.is_file())
            .map(Path::to_path_buf)
    })
}

#[cfg(test)]
mod tests {
    use super::select_host_authorization_source;

    #[test]
    fn explicit_config_wins_and_missing_user_config_does_not_invent_policy() {
        let temporary = tempfile::tempdir().unwrap();
        let explicit = temporary.path().join("explicit.acl");
        let user = temporary.path().join("user.acl");
        std::fs::write(&user, "plugins {}").unwrap();

        assert_eq!(
            select_host_authorization_source(Some(&explicit), Some(&user)),
            Some(explicit)
        );
        assert_eq!(
            select_host_authorization_source(None, Some(&user)),
            Some(user)
        );
        assert_eq!(
            select_host_authorization_source(
                None,
                Some(&temporary.path().join("workspace-only.acl")),
            ),
            None
        );
    }
}
