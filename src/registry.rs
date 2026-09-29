//! A3S Use Registry source configuration.
//!
//! `a3s registry` reads and mutates the canonical Registry source document
//! owned by `a3s-use-extension`. Component install does not place cognitive
//! packages from those sources.

use std::path::PathBuf;

use a3s_use_core::{InstallationId, InstallationKind};
use a3s_use_extension::{ExtensionPaths, RegistrySourceSnapshot, RegistrySourceStore};

/// Default managed-host installation for CLI Use projection (`user` /
/// `user/current`), matching capability snapshot scope.
pub fn default_user_installation() -> InstallationId {
    InstallationId::new(InstallationKind::User, "user/current")
        .expect("user/current is a valid Use installation id")
}

/// Installation-scoped Use extension paths for the default user host.
pub fn default_user_extension_paths(
    data_root: impl Into<PathBuf>,
    state_root: impl Into<PathBuf>,
) -> ExtensionPaths {
    ExtensionPaths::new(data_root, state_root, default_user_installation())
        .expect("default user/current Use paths are valid")
}

/// Absolute path to one installation-scoped extension receipt under CLI Use
/// roots (`…/use/installations/<kind>/<key>/extensions/<publisher>/<name>.json`).
pub fn extension_receipt_path(
    data_root: impl Into<PathBuf>,
    state_root: impl Into<PathBuf>,
    installation: InstallationId,
    package_id: &str,
) -> a3s_use_core::UseResult<PathBuf> {
    let paths = ExtensionPaths::new(data_root, state_root, installation)?;
    let mut path = paths.installation_state_root().join("extensions");
    for segment in package_id.split('/') {
        path.push(segment);
    }
    path.set_extension("json");
    Ok(path)
}

#[derive(Clone, Debug)]
pub struct RegistryStore {
    sources: RegistrySourceStore,
}

impl RegistryStore {
    pub fn new(paths: ExtensionPaths) -> Self {
        Self {
            sources: RegistrySourceStore::new(paths.use_paths().clone()),
        }
    }

    pub fn from_component_paths(paths: &crate::components::ComponentPaths) -> Self {
        Self::new(default_user_extension_paths(
            paths.data_root.join("use"),
            paths.state_root.join("use"),
        ))
    }

    pub fn source_store(&self) -> &RegistrySourceStore {
        &self.sources
    }

    pub async fn snapshot(&self) -> anyhow::Result<RegistrySourceSnapshot> {
        self.sources.snapshot().await.map_err(anyhow::Error::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_use_extension::{RegistrySourceInput, VerifiedTargetCachePolicy};

    #[tokio::test]
    async fn canonical_source_snapshot_is_the_only_registry_state() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = default_user_extension_paths(
            temporary.path().join("data/use"),
            temporary.path().join("state/use"),
        );
        let store = RegistryStore::new(paths);
        let mutation = store
            .source_store()
            .add(RegistrySourceInput::new(
                "fixture",
                "https://packages.example.test/",
                "a".repeat(64),
                None,
                VerifiedTargetCachePolicy::default(),
            ))
            .await
            .unwrap();
        let snapshot = store.snapshot().await.unwrap();
        assert_eq!(snapshot, mutation.snapshot);
        assert_eq!(snapshot.default_registry.as_deref(), Some("fixture"));
        assert_eq!(snapshot.sources[0].source_identity.len(), 64);
    }

    #[tokio::test]
    async fn source_revision_changes_when_registry_authority_changes() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = default_user_extension_paths(
            temporary.path().join("data/use"),
            temporary.path().join("state/use"),
        );
        let store = RegistryStore::new(paths);
        let added = store
            .source_store()
            .add(RegistrySourceInput::new(
                "fixture",
                "https://packages.example.test/",
                "b".repeat(64),
                None,
                VerifiedTargetCachePolicy::default(),
            ))
            .await
            .unwrap();
        let disabled = store
            .source_store()
            .disable("fixture", &added.snapshot.revision)
            .await
            .unwrap();
        assert_ne!(added.snapshot.revision, disabled.snapshot.revision);
    }
}
