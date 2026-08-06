use std::path::{Path, PathBuf};

use a3s::plugin_manager::PluginAuthorizationPolicy;

use crate::cli::context::InvocationContext;

/// Load authorization only from an operator-selected file or the user-level
/// A3S config. Automatically discovered workspace config may restrict normal
/// Code behavior but cannot pre-authorize plugin mutation.
pub(crate) async fn load_host_authorization(
    context: &InvocationContext,
) -> anyhow::Result<PluginAuthorizationPolicy> {
    let user_config = context.user_config_path();
    let source = select_host_authorization_source(
        context.explicit_config.as_deref(),
        user_config.as_deref(),
    );
    match source {
        Some(path) => PluginAuthorizationPolicy::from_acl_file(&path)
            .await
            .map_err(super::manager_error),
        None => Ok(PluginAuthorizationPolicy::default()),
    }
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
