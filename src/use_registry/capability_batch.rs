//! Atomic A3S Use Tool/Skill projection into one A3S Code Session.
//!
//! The resident Rust host keeps the complete Use snapshot and its cursor, but
//! never shares one acquired lease between Runs. The published provider asks
//! A3S Use for a fresh, non-clone snapshot lease at every Run admission.

use super::DesiredSkill;
#[cfg(test)]
use super::{CapabilityOrigin, RegistrySnapshot};
use a3s_code_core::capability::{
    CapabilityContribution, CapabilityDescriptor, CapabilityKind, CapabilitySet, CapabilitySource,
    CapabilityValue, CodeCatalogGeneration, RetainedUseGeneration, SessionCapabilityBatch,
    Sha256Digest, UseCapabilityGeneration, UseGenerationLeaseError, UseGenerationLeaseProvider,
    UsePackageGeneration,
};
use a3s_code_core::AgentSession;
#[cfg(test)]
use a3s_use::capability_registry::CAPABILITY_SNAPSHOT_CURSOR_SCHEMA;
use a3s_use::capability_registry::{
    CapabilityPackageGeneration, CapabilityRegistry, CapabilityRegistrySnapshot,
    CapabilitySnapshotCursor, CapabilitySnapshotLease,
};
use anyhow::{bail, Context};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
const CAPABILITY_REVISION_DOMAIN: &[u8] = b"a3s-cli-use-capability-revision-v1\0";
const HOST_SOURCE_REVISION_DOMAIN: &[u8] = b"a3s-cli-use-host-source-v1\0";
const SKILL_SURFACE_REVISION_DOMAIN: &[u8] = b"a3s-cli-use-skill-surface-v1\0";

/// Stable upstream identity used to distinguish an already-published batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CapabilitySnapshotIdentity {
    pub(super) generation: u64,
    pub(super) revision: String,
    pub(super) registry_revision: String,
}

#[derive(Clone)]
pub(super) struct CapabilitySnapshotAuthority {
    generation: UseCapabilityGeneration,
    backing: Arc<CapabilitySnapshotBacking>,
}

enum CapabilitySnapshotBacking {
    Native {
        registry: Arc<CapabilityRegistry>,
        // Retain the complete immutable snapshot beside its provider. The
        // provider intentionally retains no acquired generation lease.
        snapshot: Arc<CapabilityRegistrySnapshot>,
    },
    #[cfg(test)]
    Fixture { cursor: CapabilitySnapshotCursor },
}

impl CapabilitySnapshotAuthority {
    pub(super) fn native(
        registry: Arc<CapabilityRegistry>,
        snapshot: CapabilityRegistrySnapshot,
    ) -> anyhow::Result<Self> {
        snapshot.cursor().validate().map_err(|error| {
            anyhow::anyhow!(
                "A3S Use returned an invalid capability snapshot cursor: {}: {}",
                error.code,
                error.message
            )
        })?;
        if !snapshot.cursor().is_fully_leasable() {
            bail!(
                "A3S Use capability generation {} is not fully leasable; unavailable routes: {}",
                snapshot.cursor().generation,
                snapshot.cursor().unleasable_routes.join(", ")
            );
        }
        let generation = code_use_generation(snapshot.cursor())?;
        Ok(Self {
            generation,
            backing: Arc::new(CapabilitySnapshotBacking::Native {
                registry,
                snapshot: Arc::new(snapshot),
            }),
        })
    }

    #[cfg(test)]
    pub(super) fn fixture(snapshot: &RegistrySnapshot) -> anyhow::Result<Self> {
        let mut packages = snapshot
            .capabilities
            .iter()
            .filter(|binding| binding.origin == CapabilityOrigin::Extension)
            .filter_map(|binding| {
                let evidence = binding.planner_evidence.as_ref()?;
                let lifecycle_generation = binding.lifecycle_generation?;
                Some(CapabilityPackageGeneration {
                    package_id: evidence.package_id.clone(),
                    component_id: binding.id.clone(),
                    route: binding.route.clone(),
                    version: binding.version.clone(),
                    lifecycle_generation,
                    package_digest: canonical_digest_text(&evidence.package_sha256),
                    manifest_digest: canonical_digest_text(&evidence.manifest_sha256),
                })
            })
            .collect::<Vec<_>>();
        packages.sort();
        let registry_revision = digest_bytes(
            CAPABILITY_REVISION_DOMAIN,
            &serde_json::to_vec(snapshot).context("failed to fingerprint fixture Registry")?,
        )?;
        let cursor = CapabilitySnapshotCursor {
            schema: CAPABILITY_SNAPSHOT_CURSOR_SCHEMA.to_owned(),
            generation: snapshot.generation,
            revision: snapshot.revision.clone(),
            registry_revision: registry_revision.as_str().to_owned(),
            packages,
            unleasable_routes: Vec::new(),
        };
        cursor.validate().map_err(|error| {
            anyhow::anyhow!(
                "fixture capability cursor is invalid: {}: {}",
                error.code,
                error.message
            )
        })?;
        Ok(Self {
            generation: code_use_generation(&cursor)?,
            backing: Arc::new(CapabilitySnapshotBacking::Fixture { cursor }),
        })
    }

    pub(super) fn cursor(&self) -> &CapabilitySnapshotCursor {
        match self.backing.as_ref() {
            CapabilitySnapshotBacking::Native { snapshot, .. } => snapshot.cursor(),
            #[cfg(test)]
            CapabilitySnapshotBacking::Fixture { cursor } => cursor,
        }
    }

    pub(super) fn identity(&self) -> CapabilitySnapshotIdentity {
        let cursor = self.cursor();
        CapabilitySnapshotIdentity {
            generation: cursor.generation,
            revision: cursor.revision.clone(),
            registry_revision: cursor.registry_revision.clone(),
        }
    }
}

impl fmt::Debug for CapabilitySnapshotAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapabilitySnapshotAuthority")
            .field("identity", &self.identity())
            .field("packages", &self.cursor().packages.len())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl UseGenerationLeaseProvider for CapabilitySnapshotAuthority {
    fn use_generation(&self) -> &UseCapabilityGeneration {
        &self.generation
    }

    async fn acquire(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Box<dyn RetainedUseGeneration>, UseGenerationLeaseError> {
        if cancellation.is_cancelled() {
            return Err(UseGenerationLeaseError::new(
                "A3S Use snapshot lease acquisition was cancelled",
            ));
        }
        match self.backing.as_ref() {
            CapabilitySnapshotBacking::Native { registry, snapshot } => {
                let acquire = registry.acquire_snapshot_lease(snapshot.cursor());
                tokio::pin!(acquire);
                let lease = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        return Err(UseGenerationLeaseError::new(
                            "A3S Use snapshot lease acquisition was cancelled",
                        ));
                    }
                    result = &mut acquire => result.map_err(|error| {
                        UseGenerationLeaseError::new(format!("{}: {}", error.code, error.message))
                    })?,
                };
                let lease = lease.ok_or_else(|| {
                    UseGenerationLeaseError::new(
                        "A3S Use capability generation became stale, hidden, or unleasable",
                    )
                })?;
                if lease.cursor() != snapshot.cursor() {
                    return Err(UseGenerationLeaseError::new(
                        "A3S Use returned a snapshot lease for a different cursor",
                    ));
                }
                Ok(Box::new(NativeRetainedGeneration {
                    generation: self.generation.clone(),
                    _lease: lease,
                }))
            }
            #[cfg(test)]
            CapabilitySnapshotBacking::Fixture { .. } => Ok(Box::new(FixtureRetainedGeneration {
                generation: self.generation.clone(),
            })),
        }
    }
}

struct NativeRetainedGeneration {
    generation: UseCapabilityGeneration,
    _lease: CapabilitySnapshotLease,
}

impl RetainedUseGeneration for NativeRetainedGeneration {
    fn use_generation(&self) -> &UseCapabilityGeneration {
        &self.generation
    }
}

#[cfg(test)]
struct FixtureRetainedGeneration {
    generation: UseCapabilityGeneration,
}

#[cfg(test)]
impl RetainedUseGeneration for FixtureRetainedGeneration {
    fn use_generation(&self) -> &UseCapabilityGeneration {
        &self.generation
    }
}

/// Build the complete next Code Tool/Skill generation from one Use snapshot.
///
/// Runtime Tool Tasks, Knowledge tools, and MCP wrappers deliberately remain
/// on their typed compatibility paths until Core supports their asynchronous
/// lifecycle categories as one atomic batch.
pub(super) fn skill_batch(
    session: &AgentSession,
    authority: &CapabilitySnapshotAuthority,
    skills: &BTreeMap<String, DesiredSkill>,
) -> anyhow::Result<SessionCapabilityBatch> {
    let code_generation = session
        .capability_catalog_stamp()
        .generation()
        .checked_next()
        .context("A3S Code capability catalog generation is exhausted")?;
    let packages_by_component = authority
        .cursor()
        .packages
        .iter()
        .map(|package| (package.component_id.as_str(), package))
        .collect::<BTreeMap<_, _>>();

    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    enum SourceKey {
        Package(String),
        Host(String),
    }

    let mut grouped = BTreeMap::<SourceKey, Vec<(&String, &DesiredSkill)>>::new();
    for (name, skill) in skills {
        let key = packages_by_component
            .get(skill.package_id.as_str())
            .map(|package| SourceKey::Package(package.package_id.clone()))
            .unwrap_or_else(|| SourceKey::Host(skill.package_id.clone()));
        grouped.entry(key).or_default().push((name, skill));
    }

    let mut contributions = Vec::with_capacity(grouped.len());
    let mut values = Vec::with_capacity(skills.len());
    for (key, grouped_skills) in grouped {
        let source = match &key {
            SourceKey::Package(package_id) => {
                let package = authority
                    .cursor()
                    .packages
                    .iter()
                    .find(|package| &package.package_id == package_id)
                    .context("A3S Use package cursor disappeared while building projection")?;
                CapabilitySource::use_package(
                    authority.generation.clone(),
                    code_package_generation(package)?,
                )?
            }
            SourceKey::Host(component_id) => CapabilitySource::host(
                format!("a3s-use/{component_id}"),
                host_source_digest(component_id, &grouped_skills)?,
            )?,
        };

        let mut descriptors = Vec::with_capacity(grouped_skills.len());
        for (name, skill) in grouped_skills {
            let descriptor = CapabilityDescriptor::new(
                &source,
                CapabilityKind::Skill,
                name.clone(),
                name.clone(),
                digest_bytes(SKILL_SURFACE_REVISION_DOMAIN, skill.fingerprint.as_bytes())?,
                [],
            )?;
            values.push((
                descriptor.id().clone(),
                CapabilityValue::Skill(Arc::clone(&skill.skill)),
            ));
            descriptors.push(descriptor);
        }
        contributions.push(CapabilityContribution::new(source, descriptors)?);
    }

    let target = CapabilitySet::from_use_projection(
        CodeCatalogGeneration::new(code_generation.get()),
        authority.generation.clone(),
        contributions,
    )?;
    let provider: Arc<dyn UseGenerationLeaseProvider> = Arc::new(authority.clone());
    let mut batch = SessionCapabilityBatch::from_use_projection(target, provider)?;
    for (id, value) in values {
        batch.stage_value(id, value)?;
    }
    Ok(batch)
}

fn code_use_generation(
    cursor: &CapabilitySnapshotCursor,
) -> anyhow::Result<UseCapabilityGeneration> {
    Ok(UseCapabilityGeneration::new(
        cursor.generation,
        Sha256Digest::new(canonical_digest_text(&cursor.revision))?,
        Sha256Digest::new(cursor.registry_revision.clone())?,
    ))
}

fn code_package_generation(
    package: &CapabilityPackageGeneration,
) -> anyhow::Result<UsePackageGeneration> {
    Ok(UsePackageGeneration::new(
        package.package_id.clone(),
        package.component_id.clone(),
        package.route.clone(),
        package.version.clone(),
        package.lifecycle_generation,
        Sha256Digest::new(package.package_digest.clone())?,
        Sha256Digest::new(package.manifest_digest.clone())?,
    )?)
}

fn host_source_digest(
    component_id: &str,
    skills: &[(&String, &DesiredSkill)],
) -> anyhow::Result<Sha256Digest> {
    let mut hasher = Sha256::new();
    hasher.update(HOST_SOURCE_REVISION_DOMAIN);
    hash_field(&mut hasher, component_id.as_bytes());
    for (name, skill) in skills {
        hash_field(&mut hasher, name.as_bytes());
        hash_field(&mut hasher, skill.fingerprint.as_bytes());
    }
    Sha256Digest::new(format!("sha256:{:x}", hasher.finalize())).map_err(Into::into)
}

fn digest_bytes(domain: &[u8], value: &[u8]) -> anyhow::Result<Sha256Digest> {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hash_field(&mut hasher, value);
    Sha256Digest::new(format!("sha256:{:x}", hasher.finalize())).map_err(Into::into)
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn canonical_digest_text(value: &str) -> String {
    if value.starts_with("sha256:") {
        value.to_owned()
    } else {
        format!("sha256:{value}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn public_runtime_owners_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CapabilitySnapshotAuthority>();
        assert_send_sync::<NativeRetainedGeneration>();
    }

    #[test]
    fn native_authority_acquires_independent_real_snapshot_leases() {
        std::thread::Builder::new()
            .name("cli-native-use-lease".to_owned())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("native lease test runtime")
                    .block_on(native_authority_lease_scenario());
            })
            .expect("native lease test thread")
            .join()
            .expect("native lease test thread panicked");
    }

    async fn native_authority_lease_scenario() {
        let temporary = tempfile::tempdir().unwrap();
        let extension_registry =
            a3s_use_extension::ExtensionRegistry::new(a3s_use_extension::ExtensionPaths::new(
                temporary.path().join("data"),
                temporary.path().join("state"),
            ));
        let source = temporary.path().join("package");
        tokio::fs::create_dir_all(source.join("skills/guide"))
            .await
            .unwrap();
        tokio::fs::write(source.join("README.md"), b"# Guide\n")
            .await
            .unwrap();
        tokio::fs::write(
            source.join("skills/guide/SKILL.md"),
            b"---\nname: guide\ndescription: Test guide.\n---\n\n# Guide\n",
        )
        .await
        .unwrap();
        tokio::fs::write(
            source.join("a3s-use-extension.acl"),
            br#"extension "acme/guide" {
  schema_version = 3
  version        = "1.0.0"
  route          = "guide"
  requires_use   = ">=0.3.0, <0.4.0"
  actions        = ["read"]

  repository {
    url      = "https://github.com/acme/guide"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }

  skill "guide" {
    path          = "skills/guide/SKILL.md"
    requires_tool = []
    requires_mcp  = []
    optional      = false
  }
}
"#,
        )
        .await
        .unwrap();
        let package = a3s_use_extension::ExtensionLifecyclePackage::prepare_local(
            "acme/guide",
            &source,
            true,
        )
        .await
        .unwrap();
        let identity = a3s_use_extension::ExtensionLifecycleIdentity::new(
            package.package_id(),
            package.package_digest(),
            package.manifest_digest(),
            31,
        )
        .unwrap();
        extension_registry
            .commit_lifecycle_package(&identity, &package)
            .await
            .unwrap();
        extension_registry
            .publish_lifecycle_package(&identity)
            .await
            .unwrap();

        let registry = Arc::new(CapabilityRegistry::new(extension_registry.clone()));
        let snapshot = registry.snapshot().await.unwrap();
        assert_eq!(snapshot.cursor().packages.len(), 1);
        let authority = CapabilitySnapshotAuthority::native(Arc::clone(&registry), snapshot)
            .expect("published Use snapshot must be fully leasable");
        let first = authority
            .acquire(CancellationToken::new())
            .await
            .expect("first Run must acquire its own real Use lease");
        let second = authority
            .acquire(CancellationToken::new())
            .await
            .expect("second Run must acquire another real Use lease");
        assert_eq!(first.use_generation(), authority.use_generation());
        assert_eq!(second.use_generation(), authority.use_generation());

        extension_registry
            .hide_lifecycle_package_with_evidence(&identity)
            .await
            .unwrap();
        let current = registry.snapshot().await.unwrap();
        assert_ne!(current.cursor(), authority.cursor());
        let current_authority =
            CapabilitySnapshotAuthority::native(Arc::clone(&registry), current).unwrap();
        current_authority
            .acquire(CancellationToken::new())
            .await
            .expect("the replacement snapshot must remain leasable");
        let stale = match authority.acquire(CancellationToken::new()).await {
            Ok(_) => panic!("a hidden snapshot cursor unexpectedly admitted a new Run"),
            Err(error) => error,
        };
        assert!(stale
            .message()
            .contains("became stale, hidden, or unleasable"));

        let blocked = extension_registry
            .drain_lifecycle_package(&identity, Duration::from_millis(10))
            .await
            .expect_err("both admitted Runs still retain the hidden generation");
        assert_eq!(blocked.code, "use.extension.drain_timeout");
        drop(first);
        let still_blocked = extension_registry
            .drain_lifecycle_package(&identity, Duration::from_millis(10))
            .await
            .expect_err("the second Run owns an independent snapshot lease");
        assert_eq!(still_blocked.code, "use.extension.drain_timeout");
        drop(second);
        extension_registry
            .drain_lifecycle_package(&identity, Duration::from_secs(1))
            .await
            .expect("the generation must drain after the final Run lease drops");
    }
}
