//! A3S Use-owned Registry source resolution for cognitive packages.
//!
//! Native A3S components are distributed through their declared release
//! channels. Cognitive packages use the single canonical Registry source
//! document owned by `a3s-use`. CLI, TUI, and manager MCP adapters all
//! receive this store, so catalog browsing, planning, and apply cannot select
//! different Registry authority.

use std::collections::{BTreeMap, BTreeSet};

use a3s_use::cognitive_package::{cognitive_package_host_target, COGNITIVE_PACKAGE_HOST_VERSION};
use a3s_use_core::{
    PluginPackageLock, PluginPackageLockHost, PluginPlanningBundle, VerifiedPluginCatalogRecord,
};
use a3s_use_extension::{
    prepare_cached_remote_package, prepare_remote_package, resolve_cached_remote_package_lock,
    resolve_remote_package_lock, ExtensionPaths, RegistrySourceSnapshot, RegistrySourceStore,
    ResolvedRemotePackage, TrustedRegistry,
};
use anyhow::{bail, Context};

mod reviewed_lock;

#[derive(Clone, Debug)]
pub struct ResolvedRegistryPackage {
    pub registry: TrustedRegistry,
    pub dependency_registries: Vec<TrustedRegistry>,
    pub registry_source_revision: String,
    pub package: ResolvedRemotePackage,
    pub verified_catalog: VerifiedPluginCatalogRecord,
    pub planning_bundle: Option<PluginPlanningBundle>,
}

#[derive(Clone, Debug)]
pub struct ResolvedRegistrySet {
    pub source_revision: String,
    pub default_registry: String,
    pub root: TrustedRegistry,
    pub dependencies: Vec<TrustedRegistry>,
}

impl ResolvedRegistrySet {
    pub fn root(&self) -> &TrustedRegistry {
        &self.root
    }

    pub fn dependencies(&self) -> &[TrustedRegistry] {
        &self.dependencies
    }

    pub fn registry(&self, name: &str) -> Option<&TrustedRegistry> {
        std::iter::once(&self.root)
            .chain(self.dependencies.iter())
            .find(|registry| registry.name() == name)
    }
}

pub(crate) fn catalog_root_sha256(value: &str) -> &str {
    value.strip_prefix("sha256:").unwrap_or(value)
}

#[derive(Clone, Debug)]
pub struct RegistryStore {
    paths: ExtensionPaths,
    sources: RegistrySourceStore,
    offline: bool,
}

impl RegistryStore {
    pub fn new(paths: ExtensionPaths, offline: bool) -> Self {
        Self {
            sources: RegistrySourceStore::new(paths.clone()),
            paths,
            offline,
        }
    }

    pub fn from_component_paths(paths: &crate::components::ComponentPaths, offline: bool) -> Self {
        Self::new(
            ExtensionPaths::new(paths.data_root.join("use"), paths.state_root.join("use")),
            offline,
        )
    }

    pub fn source_store(&self) -> &RegistrySourceStore {
        &self.sources
    }

    pub fn state_root(&self) -> &std::path::Path {
        self.paths.state_root()
    }

    #[cfg(test)]
    pub(crate) fn for_test(root: &std::path::Path) -> Self {
        Self::new(
            ExtensionPaths::new(root.join("data/use"), root.join("state/use")),
            false,
        )
    }

    #[cfg(test)]
    pub(crate) async fn add_test_source(
        &self,
        name: &str,
        url: &str,
        root_sha256: &str,
    ) -> anyhow::Result<RegistrySourceSnapshot> {
        let mutation = self
            .sources
            .add(a3s_use_extension::RegistrySourceInput::new(
                name,
                url,
                root_sha256,
                None,
                a3s_use_extension::VerifiedTargetCachePolicy::default(),
            ))
            .await
            .map_err(anyhow::Error::new)?;
        Ok(mutation.snapshot)
    }

    pub async fn snapshot(&self) -> anyhow::Result<RegistrySourceSnapshot> {
        self.sources.snapshot().await.map_err(anyhow::Error::new)
    }

    pub async fn resolve(&self, selected: Option<&str>) -> anyhow::Result<ResolvedRegistrySet> {
        let resolved = self
            .sources
            .resolve(selected)
            .await
            .map_err(anyhow::Error::new)?;
        let root = resolved.root().clone();
        let default_registry = root.name().to_string();
        Ok(ResolvedRegistrySet {
            source_revision: resolved.source_revision().to_string(),
            default_registry,
            root,
            dependencies: resolved.dependencies().to_vec(),
        })
    }

    pub async fn resolve_package(
        &self,
        selected_registry: Option<&str>,
        package_id: &str,
        version: Option<&str>,
        channel: &str,
    ) -> anyhow::Result<ResolvedRegistryPackage> {
        let registries = self.resolve(selected_registry).await?;
        let prepared = self
            .prepare_package(registries.root(), package_id, version, channel, None)
            .await
            .map_err(|error| registry_error(registries.root(), error))?;
        let planning_bundle = prepared
            .load_planning_bundle()
            .await
            .map_err(|error| registry_error(registries.root(), error))?;
        Ok(ResolvedRegistryPackage {
            registry: registries.root().clone(),
            dependency_registries: registries.dependencies().to_vec(),
            registry_source_revision: registries.source_revision,
            package: prepared.resolved().clone(),
            verified_catalog: prepared.verified_catalog().clone(),
            planning_bundle,
        })
    }

    pub async fn resolve_cognitive_package_lock(
        &self,
        resolved: &ResolvedRegistryPackage,
    ) -> anyhow::Result<PluginPackageLock> {
        let registries = self.resolve(Some(resolved.registry.name())).await?;
        verify_source_revision(resolved, &registries)?;
        verify_registry_identity(&resolved.registry, registries.root())?;
        let catalog = &resolved.verified_catalog;
        let host = PluginPackageLockHost::new(
            cognitive_package_host_target().map_err(anyhow::Error::new)?,
            COGNITIVE_PACKAGE_HOST_VERSION,
        )
        .map_err(anyhow::Error::new)?;
        let lock = if self.offline {
            resolve_cached_remote_package_lock(
                registries.root(),
                registries.dependencies(),
                &catalog.record.package_id,
                Some(&catalog.record.version),
                catalog.record.channel,
                host,
            )
            .await
        } else {
            resolve_remote_package_lock(
                registries.root(),
                registries.dependencies(),
                &catalog.record.package_id,
                Some(&catalog.record.version),
                catalog.record.channel,
                host,
            )
            .await
        }
        .map_err(|error| registry_error(registries.root(), error))?;
        let selected = lock
            .package(&catalog.record.package_id)
            .context("cognitive-package lock omitted its requested root")?;
        if selected.catalog != *catalog {
            bail!("cognitive-package lock root does not match the reviewed Registry catalog");
        }
        Ok(lock)
    }

    pub async fn resolve_cognitive_package_planning_bundles(
        &self,
        resolved: &ResolvedRegistryPackage,
        lock: &PluginPackageLock,
    ) -> anyhow::Result<BTreeMap<String, PluginPlanningBundle>> {
        lock.validate().map_err(anyhow::Error::new)?;
        let registries = self.resolve(Some(resolved.registry.name())).await?;
        verify_source_revision(resolved, &registries)?;
        let by_name = std::iter::once(registries.root())
            .chain(registries.dependencies())
            .map(|registry| (registry.name(), registry))
            .collect::<BTreeMap<_, _>>();
        let mut bundles = BTreeMap::new();
        for package in &lock.packages {
            if package.catalog.record.planning.is_none() {
                continue;
            }
            let provenance = &package.catalog.provenance;
            let registry = by_name
                .get(provenance.registry_name.as_str())
                .with_context(|| {
                    format!(
                        "cognitive-package lock references disabled or missing Registry '{}'",
                        provenance.registry_name
                    )
                })?;
            verify_provenance(registry, provenance)?;
            let expected = ResolvedRemotePackage::from_verified_catalog(&package.catalog)
                .map_err(anyhow::Error::new)?;
            let plan_digest = expected.plan_digest().map_err(anyhow::Error::new)?;
            let prepared = self
                .prepare_package(
                    registry,
                    package.package_id(),
                    Some(package.version()),
                    package.catalog.record.channel.as_str(),
                    Some(&plan_digest),
                )
                .await
                .map_err(|error| registry_error(registry, error))?;
            if prepared.verified_catalog() != &package.catalog || prepared.resolved() != &expected {
                bail!(
                    "cognitive package '{}' changed after dependency review",
                    package.package_id()
                );
            }
            let bundle = prepared
                .load_planning_bundle()
                .await
                .map_err(|error| registry_error(registry, error))?
                .with_context(|| {
                    format!(
                        "executable cognitive package '{}' omitted its signed planning target",
                        package.package_id()
                    )
                })?;
            if bundles
                .insert(package.package_id().to_string(), bundle)
                .is_some()
            {
                bail!(
                    "cognitive package '{}' has duplicate planning evidence",
                    package.package_id()
                );
            }
        }
        Ok(bundles)
    }

    pub async fn require_configured_registry(&self) -> anyhow::Result<()> {
        self.resolve(None).await.map(drop)
    }

    pub async fn resolve_upgrade(
        &self,
        installed: &ResolvedRemotePackage,
    ) -> anyhow::Result<ResolvedRegistryPackage> {
        let registries = self.resolve(Some(&installed.registry_name)).await?;
        if registries.root().base_url().as_str() != installed.registry_url
            || registries.root().root_sha256() != installed.root_sha256
        {
            bail!(
                "installed package source Registry '{}' no longer matches its recorded URL and trust root",
                installed.registry_name
            );
        }
        let prepared = self
            .prepare_package(
                registries.root(),
                &installed.package_id,
                None,
                &installed.channel,
                None,
            )
            .await
            .map_err(|error| registry_error(registries.root(), error))?;
        let planning_bundle = prepared
            .load_planning_bundle()
            .await
            .map_err(|error| registry_error(registries.root(), error))?;
        Ok(ResolvedRegistryPackage {
            registry: registries.root().clone(),
            dependency_registries: registries.dependencies().to_vec(),
            registry_source_revision: registries.source_revision,
            package: prepared.resolved().clone(),
            verified_catalog: prepared.verified_catalog().clone(),
            planning_bundle,
        })
    }

    async fn prepare_package(
        &self,
        registry: &TrustedRegistry,
        package_id: &str,
        version: Option<&str>,
        channel: &str,
        expected_plan_digest: Option<&str>,
    ) -> Result<a3s_use_extension::PreparedRemotePackage, a3s_use_core::UseError> {
        if self.offline {
            prepare_cached_remote_package(
                registry,
                package_id,
                version,
                channel,
                expected_plan_digest,
            )
            .await
        } else {
            prepare_remote_package(registry, package_id, version, channel, expected_plan_digest)
                .await
        }
    }

    pub async fn verify_current_revision(&self, expected: &str) -> anyhow::Result<()> {
        let actual = self.snapshot().await?.revision;
        if actual != expected {
            bail!(
                "Registry source configuration changed after review: expected {}, found {}",
                expected,
                actual
            );
        }
        Ok(())
    }
}

fn verify_source_revision(
    resolved: &ResolvedRegistryPackage,
    current: &ResolvedRegistrySet,
) -> anyhow::Result<()> {
    if resolved.registry_source_revision != current.source_revision {
        bail!("Registry source configuration changed while resolving the cognitive-package plan");
    }
    Ok(())
}

fn verify_registry_identity(
    expected: &TrustedRegistry,
    actual: &TrustedRegistry,
) -> anyhow::Result<()> {
    if expected.name() != actual.name()
        || expected.base_url() != actual.base_url()
        || expected.root_sha256() != actual.root_sha256()
        || expected.datastore() != actual.datastore()
    {
        bail!(
            "Registry '{}' changed while resolving the cognitive-package plan",
            expected.name()
        );
    }
    Ok(())
}

fn verify_provenance(
    registry: &TrustedRegistry,
    provenance: &a3s_use_core::VerifiedCatalogProvenance,
) -> anyhow::Result<()> {
    if registry.name() != provenance.registry_name
        || registry.base_url().as_str() != provenance.registry_url
        || registry.root_sha256() != catalog_root_sha256(&provenance.root_sha256)
    {
        bail!(
            "Registry '{}' changed after cognitive-package dependency review",
            provenance.registry_name
        );
    }
    Ok(())
}

fn registry_error(registry: &TrustedRegistry, error: a3s_use_core::UseError) -> anyhow::Error {
    anyhow::anyhow!(
        "Registry '{}' at {} failed: {}",
        registry.name(),
        registry.base_url(),
        error
    )
}

pub(crate) fn lock_registry_names(lock: &PluginPackageLock) -> BTreeSet<String> {
    lock.packages
        .iter()
        .map(|package| package.catalog.provenance.registry_name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_use_extension::{RegistrySourceInput, VerifiedTargetCachePolicy};

    #[tokio::test]
    async fn canonical_source_snapshot_is_the_only_registry_state() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = ExtensionPaths::new(
            temporary.path().join("data/use"),
            temporary.path().join("state/use"),
        );
        let store = RegistryStore::new(paths, false);
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
        let paths = ExtensionPaths::new(
            temporary.path().join("data/use"),
            temporary.path().join("state/use"),
        );
        let store = RegistryStore::new(paths, false);
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
        store
            .verify_current_revision(&added.snapshot.revision)
            .await
            .unwrap_err();
    }
}
