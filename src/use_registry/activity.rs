use std::sync::Arc;

use a3s_use_extension::{ExtensionLifecycleIdentity, ExtensionPaths};
#[cfg(not(test))]
use a3s_use_extension::{ExtensionRegistry, ExtensionRouteLease};
#[cfg(not(test))]
use anyhow::Context;
use async_trait::async_trait;

pub(super) trait ActivityLeaseGuard: Send + Sync {}

#[async_trait]
pub(super) trait ActivityLeaseProvider: Send + Sync {
    async fn acquire(
        &self,
        identity: &ExtensionLifecycleIdentity,
    ) -> anyhow::Result<Option<Box<dyn ActivityLeaseGuard>>>;
}

#[cfg(not(test))]
struct RegistryActivityLeaseProvider {
    registry: ExtensionRegistry,
}

#[cfg(not(test))]
struct RegistryActivityLeaseGuard {
    _lease: ExtensionRouteLease,
}

#[cfg(not(test))]
impl ActivityLeaseGuard for RegistryActivityLeaseGuard {}

#[cfg(not(test))]
#[async_trait]
impl ActivityLeaseProvider for RegistryActivityLeaseProvider {
    async fn acquire(
        &self,
        identity: &ExtensionLifecycleIdentity,
    ) -> anyhow::Result<Option<Box<dyn ActivityLeaseGuard>>> {
        self.registry
            .acquire_published_lifecycle_generation(identity)
            .await
            .map(|lease| {
                lease.map(|lease| {
                    Box::new(RegistryActivityLeaseGuard { _lease: lease })
                        as Box<dyn ActivityLeaseGuard>
                })
            })
            .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))
            .with_context(|| {
                format!(
                    "failed to acquire Activity package lease '{}#{}'",
                    identity.package_id(),
                    identity.generation()
                )
            })
    }
}

#[cfg(not(test))]
pub(super) fn default_activity_lease_provider(
    paths: &ExtensionPaths,
) -> Arc<dyn ActivityLeaseProvider> {
    Arc::new(RegistryActivityLeaseProvider {
        registry: ExtensionRegistry::new(paths.clone()),
    })
}

#[cfg(test)]
struct UnitTestActivityLeaseProvider;

#[cfg(test)]
struct UnitTestActivityLeaseGuard;

#[cfg(test)]
impl ActivityLeaseGuard for UnitTestActivityLeaseGuard {}

#[cfg(test)]
#[async_trait]
impl ActivityLeaseProvider for UnitTestActivityLeaseProvider {
    async fn acquire(
        &self,
        _identity: &ExtensionLifecycleIdentity,
    ) -> anyhow::Result<Option<Box<dyn ActivityLeaseGuard>>> {
        Ok(Some(Box::new(UnitTestActivityLeaseGuard)))
    }
}

#[cfg(test)]
pub(super) fn default_activity_lease_provider(
    _paths: &ExtensionPaths,
) -> Arc<dyn ActivityLeaseProvider> {
    Arc::new(UnitTestActivityLeaseProvider)
}
