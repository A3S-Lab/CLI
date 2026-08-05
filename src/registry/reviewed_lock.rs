use std::collections::BTreeMap;
use std::path::Path;

use a3s_use_core::PluginPackageLock;
use a3s_use_extension::TrustedRegistry;
use anyhow::{bail, Context};

use super::RegistryStore;

pub(crate) struct TrustedPackageLockRegistries {
    pub root: TrustedRegistry,
    pub dependencies: Vec<TrustedRegistry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LockRegistryIdentity {
    url: String,
    root_sha256: String,
}

impl RegistryStore {
    /// Reconstruct only the configured Registry identities frozen in one
    /// reviewed cognitive-package lock.
    pub(crate) fn trusted_registries_for_lock(
        &self,
        state_root: &Path,
        lock: &PluginPackageLock,
    ) -> anyhow::Result<TrustedPackageLockRegistries> {
        lock.validate().map_err(anyhow::Error::new)?;
        let root_node = lock
            .package(&lock.root_package_id)
            .context("reviewed cognitive-package lock omitted its root package")?;
        let root_registry_name = root_node.catalog.provenance.registry_name.as_str();
        let mut identities = BTreeMap::new();
        for package in &lock.packages {
            let provenance = &package.catalog.provenance;
            let identity = LockRegistryIdentity {
                url: provenance.registry_url.clone(),
                root_sha256: normalize_sha256(&provenance.root_sha256).to_string(),
            };
            if identities
                .insert(provenance.registry_name.clone(), identity.clone())
                .is_some_and(|existing| existing != identity)
            {
                bail!(
                    "reviewed cognitive-package lock gives Registry '{}' conflicting trust identities",
                    provenance.registry_name
                );
            }
        }

        let mut root = None;
        let mut dependencies = Vec::new();
        for (name, identity) in identities {
            let record = self
                .get(&name)?
                .with_context(|| format!("reviewed Registry '{name}' is no longer configured"))?;
            if record.url != identity.url
                || normalize_sha256(&record.trust_root) != identity.root_sha256
            {
                bail!("reviewed Registry '{name}' no longer matches its locked URL and trust root");
            }
            let registry = record.trusted_registry(state_root)?;
            if name == root_registry_name {
                root = Some(registry);
            } else {
                dependencies.push(registry);
            }
        }
        let root = root.context("reviewed cognitive-package root Registry is unavailable")?;
        Ok(TrustedPackageLockRegistries { root, dependencies })
    }
}

fn normalize_sha256(value: &str) -> &str {
    value.strip_prefix("sha256:").unwrap_or(value)
}
