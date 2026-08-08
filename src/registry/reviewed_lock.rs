use std::collections::BTreeMap;

use a3s_use_core::PluginPackageLock;
use a3s_use_extension::TrustedRegistry;
use anyhow::{bail, Context};

use super::{catalog_root_sha256, lock_registry_names, RegistryStore};

pub(crate) struct TrustedPackageLockRegistries {
    pub root: TrustedRegistry,
    pub dependencies: Vec<TrustedRegistry>,
}

impl RegistryStore {
    /// Reconstruct only enabled Registry identities frozen in one reviewed
    /// package lock. Added, removed, disabled, or replaced sources therefore
    /// fail before archive download or lifecycle mutation.
    pub(crate) async fn trusted_registries_for_lock(
        &self,
        lock: &PluginPackageLock,
    ) -> anyhow::Result<TrustedPackageLockRegistries> {
        lock.validate().map_err(anyhow::Error::new)?;
        let root_node = lock
            .package(&lock.root_package_id)
            .context("reviewed cognitive-package lock omitted its root package")?;
        let root_name = root_node.catalog.provenance.registry_name.as_str();
        let resolved = self.resolve(Some(root_name)).await?;
        let required = lock_registry_names(lock);
        let configured = std::iter::once(resolved.root())
            .chain(resolved.dependencies())
            .map(|registry| (registry.name(), registry))
            .collect::<BTreeMap<_, _>>();
        let mut root = None;
        let mut dependencies = Vec::new();
        for name in required {
            let registry = configured
                .get(name.as_str())
                .with_context(|| format!("reviewed Registry '{name}' is no longer enabled"))?;
            let identities = lock
                .packages
                .iter()
                .filter(|package| package.catalog.provenance.registry_name == name)
                .map(|package| {
                    (
                        package.catalog.provenance.registry_url.as_str(),
                        package.catalog.provenance.root_sha256.as_str(),
                    )
                })
                .collect::<std::collections::BTreeSet<_>>();
            if identities.len() != 1 {
                bail!(
                    "reviewed cognitive-package lock gives Registry '{name}' conflicting trust identities"
                );
            }
            let (url, root_sha256) = identities
                .into_iter()
                .next()
                .context("reviewed Registry identity unexpectedly disappeared")?;
            if registry.base_url().as_str() != url
                || registry.root_sha256() != catalog_root_sha256(root_sha256)
            {
                bail!("reviewed Registry '{name}' no longer matches its locked URL and trust root");
            }
            if name == root_name {
                root = Some((*registry).clone());
            } else {
                dependencies.push((*registry).clone());
            }
        }
        dependencies.sort_by(|left, right| left.name().cmp(right.name()));
        Ok(TrustedPackageLockRegistries {
            root: root.context("reviewed cognitive-package root Registry is unavailable")?,
            dependencies,
        })
    }
}
