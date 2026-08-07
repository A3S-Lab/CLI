use std::collections::BTreeMap;
use std::sync::Arc;

use a3s_use_core::OkfCapabilityProjection;
use a3s_use_extension::{ExtensionLifecycleIdentity, ExtensionPaths};
#[cfg(not(test))]
use a3s_use_extension::{ExtensionRegistry, ExtensionRouteLease};
use anyhow::bail;
#[cfg(not(test))]
use anyhow::Context;
use async_trait::async_trait;

#[async_trait]
pub(super) trait KnowledgeLeaseProvider: Send + Sync {
    async fn acquire(
        &self,
        projections: &[OkfCapabilityProjection],
    ) -> anyhow::Result<Box<dyn KnowledgeLeaseGuard>>;
}

pub(super) trait KnowledgeLeaseGuard: Send + Sync {}

#[cfg(not(test))]
struct RegistryKnowledgeLeaseProvider {
    registry: ExtensionRegistry,
}

#[cfg(not(test))]
impl RegistryKnowledgeLeaseProvider {
    fn new(paths: &ExtensionPaths) -> Self {
        Self {
            registry: ExtensionRegistry::new(paths.clone()),
        }
    }
}

#[cfg(not(test))]
struct RegistryKnowledgeLeaseGuard {
    _leases: Vec<ExtensionRouteLease>,
}

#[cfg(not(test))]
impl KnowledgeLeaseGuard for RegistryKnowledgeLeaseGuard {}

#[cfg(not(test))]
#[async_trait]
impl KnowledgeLeaseProvider for RegistryKnowledgeLeaseProvider {
    async fn acquire(
        &self,
        projections: &[OkfCapabilityProjection],
    ) -> anyhow::Result<Box<dyn KnowledgeLeaseGuard>> {
        let identities = knowledge_lifecycle_identities(projections)?;
        let mut leases = Vec::with_capacity(identities.len());
        for identity in identities {
            let lease = self
                .registry
                .acquire_published_lifecycle_generation(&identity)
                .await
                .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?
                .with_context(|| {
                    format!(
                        "managed OKF Knowledge package '{}#{}' is no longer the exact published generation",
                        identity.package_id(),
                        identity.generation()
                    )
                })?;
            leases.push(lease);
        }
        Ok(Box::new(RegistryKnowledgeLeaseGuard { _leases: leases }))
    }
}

#[cfg(not(test))]
pub(super) fn default_knowledge_lease_provider(
    paths: &ExtensionPaths,
) -> Arc<dyn KnowledgeLeaseProvider> {
    Arc::new(RegistryKnowledgeLeaseProvider::new(paths))
}

#[cfg(test)]
struct UnitTestKnowledgeLeaseProvider;

#[cfg(test)]
struct UnitTestKnowledgeLeaseGuard;

#[cfg(test)]
impl KnowledgeLeaseGuard for UnitTestKnowledgeLeaseGuard {}

#[cfg(test)]
#[async_trait]
impl KnowledgeLeaseProvider for UnitTestKnowledgeLeaseProvider {
    async fn acquire(
        &self,
        projections: &[OkfCapabilityProjection],
    ) -> anyhow::Result<Box<dyn KnowledgeLeaseGuard>> {
        knowledge_lifecycle_identities(projections)?;
        Ok(Box::new(UnitTestKnowledgeLeaseGuard))
    }
}

#[cfg(test)]
pub(super) fn default_knowledge_lease_provider(
    _paths: &ExtensionPaths,
) -> Arc<dyn KnowledgeLeaseProvider> {
    Arc::new(UnitTestKnowledgeLeaseProvider)
}

pub(super) fn knowledge_lifecycle_identities(
    projections: &[OkfCapabilityProjection],
) -> anyhow::Result<Vec<ExtensionLifecycleIdentity>> {
    if projections.is_empty() {
        bail!("managed OKF Knowledge lease acquisition requires at least one projection");
    }
    let mut packages = BTreeMap::<(String, u64), (String, String)>::new();
    for projection in projections {
        projection.validate().map_err(|error| {
            anyhow::anyhow!(
                "invalid managed OKF Knowledge projection: {}: {}",
                error.code,
                error.message
            )
        })?;
        let key = (projection.surface.package_id.clone(), projection.generation);
        let evidence = (
            projection.package_digest.clone(),
            projection.manifest_digest.clone(),
        );
        if packages
            .insert(key.clone(), evidence.clone())
            .is_some_and(|existing| existing != evidence)
        {
            bail!(
                "managed OKF Knowledge projections disagree on exact package generation '{}#{}'",
                key.0,
                key.1
            );
        }
    }
    packages
        .into_iter()
        .map(
            |((package_id, generation), (package_digest, manifest_digest))| {
                ExtensionLifecycleIdentity::new(
                    package_id,
                    package_digest,
                    manifest_digest,
                    generation,
                )
                .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))
            },
        )
        .collect()
}
