//! Atomic A3S Use capability projection into one A3S Code Session.
//!
//! The resident Rust host keeps the complete Use snapshot and its cursor, but
//! never shares one acquired lease between Runs. The published provider asks
//! A3S Use for a fresh, non-clone snapshot lease at every Run admission.

use super::runtime_tasks::{DesiredRuntimeTask, RuntimeTaskInvoker, UseRuntimeTaskTool};
use super::{flow::UseFlowCatalogItem, flow_runtime::InstalledFlowRuntime};
use super::{managed_mcp, managed_mcp::McpRuntimeResolver};
#[cfg(test)]
use super::{CapabilityOrigin, RegistrySnapshot};
use super::{DesiredKnowledgeSurface, DesiredManagedMcp, DesiredSkill, DesiredUi};
use a3s_code_core::capability::{
    CapabilityContribution, CapabilityDescriptor, CapabilityId, CapabilityKind, CapabilitySet,
    CapabilitySource, CapabilityValue, CodeCatalogGeneration, RetainedUseGeneration,
    SessionCapabilityBatch, Sha256Digest, UseCapabilityGeneration, UseGenerationLeaseError,
    UseGenerationLeaseProvider, UsePackageGeneration,
};
use a3s_code_core::AgentSession;
#[cfg(test)]
use a3s_use::capability_registry::CAPABILITY_SNAPSHOT_CURSOR_SCHEMA;
use a3s_use::capability_registry::{
    CapabilityPackageGeneration, CapabilityRegistry, CapabilityRegistrySnapshot,
    CapabilitySnapshotCursor, CapabilitySnapshotLease,
};
use a3s_use_core::PluginSurfaceKind;
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
const MCP_SURFACE_REVISION_DOMAIN: &[u8] = b"a3s-cli-use-mcp-surface-v1\0";
const TOOL_SURFACE_REVISION_DOMAIN: &[u8] = b"a3s-cli-use-tool-surface-v1\0";

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

/// Build the complete next Code capability generation from one Use snapshot.
///
/// Built-in MCP and dynamic Knowledge query tools remain on their typed
/// compatibility paths. Exact extension MCP and immutable OKF readiness
/// surfaces participate through Core's fallible projection adapter, so
/// dependent Flow and UI descriptors can share the same commit.
pub(super) struct CapabilityBatchInputs<'a> {
    pub(super) mcp: &'a BTreeMap<String, DesiredManagedMcp>,
    pub(super) mcp_runtime: Option<&'a Arc<dyn McpRuntimeResolver>>,
    pub(super) skills: &'a BTreeMap<String, DesiredSkill>,
    pub(super) tool_tasks: &'a BTreeMap<String, DesiredRuntimeTask>,
    pub(super) runtime_tasks: Option<&'a Arc<dyn RuntimeTaskInvoker>>,
    pub(super) knowledge_surfaces: &'a BTreeMap<String, DesiredKnowledgeSurface>,
    pub(super) flows: &'a BTreeMap<String, UseFlowCatalogItem>,
    pub(super) flow_runtime: Option<&'a InstalledFlowRuntime>,
    pub(super) ui: &'a BTreeMap<String, DesiredUi>,
}

pub(super) async fn capability_batch(
    session: &AgentSession,
    authority: &CapabilitySnapshotAuthority,
    inputs: CapabilityBatchInputs<'_>,
    cancellation: CancellationToken,
) -> anyhow::Result<SessionCapabilityBatch> {
    if cancellation.is_cancelled() {
        bail!("A3S Use capability batch construction was cancelled");
    }
    let CapabilityBatchInputs {
        mcp,
        mcp_runtime,
        skills,
        tool_tasks,
        runtime_tasks,
        knowledge_surfaces,
        flows,
        flow_runtime,
        ui,
    } = inputs;
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

    #[derive(Default)]
    struct GroupedCapabilities<'a> {
        mcp: Vec<(&'a String, &'a DesiredManagedMcp)>,
        skills: Vec<(&'a String, &'a DesiredSkill)>,
        tool_tasks: Vec<(&'a String, &'a DesiredRuntimeTask)>,
        knowledge_surfaces: Vec<(&'a String, &'a DesiredKnowledgeSurface)>,
        flows: Vec<(&'a String, &'a UseFlowCatalogItem)>,
        ui: Vec<(&'a String, &'a DesiredUi)>,
    }

    let source_key = |component_id: &str| {
        packages_by_component
            .get(component_id)
            .map(|package| SourceKey::Package(package.package_id.clone()))
            .unwrap_or_else(|| SourceKey::Host(component_id.to_owned()))
    };
    let mut grouped = BTreeMap::<SourceKey, GroupedCapabilities<'_>>::new();
    for (name, server) in mcp {
        let package = packages_by_component
            .get(server.capability_id())
            .with_context(|| {
                format!(
                    "A3S Use MCP '{}' has no exact package cursor for '{}'",
                    name,
                    server.capability_id()
                )
            })?;
        validate_mcp_package_generation(server, package)?;
        grouped
            .entry(SourceKey::Package(package.package_id.clone()))
            .or_default()
            .mcp
            .push((name, server));
    }
    for (name, skill) in skills {
        grouped
            .entry(source_key(&skill.package_id))
            .or_default()
            .skills
            .push((name, skill));
    }
    for (name, task) in tool_tasks {
        let package = packages_by_component
            .get(task.capability_id())
            .with_context(|| {
                format!(
                    "A3S Use Runtime Tool Task '{}' has no exact package cursor for '{}'",
                    name,
                    task.capability_id()
                )
            })?;
        grouped
            .entry(SourceKey::Package(package.package_id.clone()))
            .or_default()
            .tool_tasks
            .push((name, task));
    }
    for (name, surface) in knowledge_surfaces {
        let package = packages_by_component
            .get(surface.component_id.as_str())
            .with_context(|| {
                format!(
                    "A3S Use OKF surface '{}' has no exact package cursor for '{}'",
                    name, surface.component_id
                )
            })?;
        validate_knowledge_package_generation(surface, package)?;
        grouped
            .entry(SourceKey::Package(package.package_id.clone()))
            .or_default()
            .knowledge_surfaces
            .push((name, surface));
    }
    for (name, contribution) in ui {
        grouped
            .entry(source_key(&contribution.package_id))
            .or_default()
            .ui
            .push((name, contribution));
    }
    for (name, flow) in flows {
        let package = packages_by_component
            .get(flow.package_id.as_str())
            .with_context(|| {
                format!(
                    "A3S Use Flow '{}' has no exact package cursor for '{}'",
                    name, flow.package_id
                )
            })?;
        grouped
            .entry(SourceKey::Package(package.package_id.clone()))
            .or_default()
            .flows
            .push((name, flow));
    }

    let mut contributions = Vec::with_capacity(grouped.len());
    let runtime_tasks = match (tool_tasks.is_empty(), runtime_tasks) {
        (true, _) => None,
        (false, Some(runtime_tasks)) => Some(runtime_tasks),
        (false, None) => {
            bail!(
                "A3S Use Runtime Tool Tasks are projected without a Plugin Manager Runtime composition"
            )
        }
    };
    let flow_runtime = match (flows.is_empty(), flow_runtime) {
        (true, _) => None,
        (false, Some(runtime)) => Some(runtime),
        (false, None) => {
            bail!("A3S Use Flows are projected without a workspace-local A3S Flow runtime")
        }
    };
    let mut values = Vec::with_capacity(
        mcp.len()
            .saturating_add(skills.len())
            .saturating_add(tool_tasks.len())
            .saturating_add(knowledge_surfaces.len())
            .saturating_add(flows.len())
            .saturating_add(ui.len()),
    );
    let mut mcp_adapters = Vec::with_capacity(mcp.len());
    let mut flow_adapters = Vec::with_capacity(flows.len());
    for (key, grouped_capabilities) in grouped {
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
                host_source_digest(
                    component_id,
                    &grouped_capabilities.skills,
                    &grouped_capabilities.tool_tasks,
                    &grouped_capabilities.ui,
                )?,
            )?,
        };

        let mut descriptors = Vec::with_capacity(
            grouped_capabilities
                .mcp
                .len()
                .saturating_add(grouped_capabilities.skills.len())
                .saturating_add(grouped_capabilities.tool_tasks.len())
                .saturating_add(grouped_capabilities.knowledge_surfaces.len())
                .saturating_add(grouped_capabilities.flows.len())
                .saturating_add(grouped_capabilities.ui.len()),
        );
        for (public_name, server) in grouped_capabilities.mcp {
            let descriptor = CapabilityDescriptor::new(
                &source,
                CapabilityKind::Mcp,
                server.surface_id().to_string(),
                public_name.clone(),
                digest_bytes(MCP_SURFACE_REVISION_DOMAIN, server.fingerprint().as_bytes())?,
                [],
            )?;
            mcp_adapters.push((
                descriptor.id().clone(),
                managed_mcp::projection_adapter(server, mcp_runtime, cancellation.clone()).await?,
            ));
            descriptors.push(descriptor);
        }
        for (name, skill) in grouped_capabilities.skills {
            let descriptor = CapabilityDescriptor::new(
                &source,
                CapabilityKind::Skill,
                skill.surface_id.clone(),
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
        for (name, task) in grouped_capabilities.tool_tasks {
            let descriptor = CapabilityDescriptor::new(
                &source,
                CapabilityKind::Tool,
                task.surface_id().to_string(),
                name.clone(),
                digest_bytes(TOOL_SURFACE_REVISION_DOMAIN, task.fingerprint().as_bytes())?,
                [],
            )?;
            let invoker = runtime_tasks.context(
                "A3S Use Runtime Tool Tasks are projected without a Plugin Manager Runtime composition",
            )?;
            values.push((
                descriptor.id().clone(),
                CapabilityValue::Tool(Arc::new(UseRuntimeTaskTool::new(
                    task.clone(),
                    Arc::clone(invoker),
                ))),
            ));
            descriptors.push(descriptor);
        }
        for (public_name, surface) in grouped_capabilities.knowledge_surfaces {
            let descriptor = CapabilityDescriptor::new(
                &source,
                CapabilityKind::KnowledgeSurface,
                surface.surface_id.clone(),
                public_name.clone(),
                surface.binding.surface_digest().clone(),
                [],
            )?;
            values.push((
                descriptor.id().clone(),
                CapabilityValue::KnowledgeSurface(Arc::clone(&surface.binding)),
            ));
            descriptors.push(descriptor);
        }
        for (public_name, flow) in grouped_capabilities.flows {
            let dependencies = flow
                .requires_tools
                .iter()
                .map(|dependency| {
                    CapabilityId::new(&source, CapabilityKind::Tool, dependency.clone())
                        .map_err(Into::into)
                })
                .chain(flow.requires_mcp.iter().map(|dependency| {
                    CapabilityId::new(&source, CapabilityKind::Mcp, dependency.clone())
                        .map_err(Into::into)
                }))
                .chain(flow.requires_okf.iter().map(|dependency| {
                    CapabilityId::new(
                        &source,
                        CapabilityKind::KnowledgeSurface,
                        dependency.clone(),
                    )
                    .map_err(Into::into)
                }))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let descriptor = CapabilityDescriptor::new(
                &source,
                CapabilityKind::Flow,
                flow.id.clone(),
                public_name.clone(),
                Sha256Digest::new(flow.atomic_fingerprint())?,
                dependencies,
            )?;
            let runtime = flow_runtime.context(
                "A3S Use Flows are projected without a workspace-local A3S Flow runtime",
            )?;
            flow_adapters.push((
                descriptor.id().clone(),
                runtime.projection_adapter(flow.clone()),
            ));
            descriptors.push(descriptor);
        }
        for (public_name, contribution) in grouped_capabilities.ui {
            let dependencies = contribution
                .dependencies
                .iter()
                .map(|dependency| {
                    Ok(CapabilityId::new(
                        &source,
                        code_dependency_kind(dependency.kind)?,
                        dependency.id.clone(),
                    )?)
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            let descriptor = CapabilityDescriptor::new(
                &source,
                CapabilityKind::Ui,
                contribution.surface_id.clone(),
                public_name.clone(),
                contribution.binding.surface_digest().clone(),
                dependencies,
            )?;
            values.push((
                descriptor.id().clone(),
                CapabilityValue::Ui(Arc::clone(&contribution.binding)),
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
    for (id, adapter) in mcp_adapters {
        batch.stage(id, adapter)?;
    }
    for (id, adapter) in flow_adapters {
        batch.stage(id, adapter)?;
    }
    Ok(batch)
}

fn validate_mcp_package_generation(
    server: &DesiredManagedMcp,
    package: &CapabilityPackageGeneration,
) -> anyhow::Result<()> {
    let identity = &server.lifecycle_identity;
    if identity.package_id() != package.package_id
        || identity.generation() != package.lifecycle_generation
        || identity.package_digest() != canonical_digest_text(&package.package_digest)
        || identity.manifest_digest() != canonical_digest_text(&package.manifest_digest)
    {
        bail!(
            "A3S Use MCP '{}:{}' does not match its exact package cursor",
            server.capability_id(),
            server.surface_id()
        );
    }
    Ok(())
}

fn validate_knowledge_package_generation(
    surface: &DesiredKnowledgeSurface,
    package: &CapabilityPackageGeneration,
) -> anyhow::Result<()> {
    if surface.package_id != package.package_id
        || surface.lifecycle_generation != package.lifecycle_generation
        || surface.package_digest != package.package_digest
        || surface.manifest_digest != package.manifest_digest
    {
        bail!(
            "A3S Use OKF surface '{}:{}' does not match its exact package cursor",
            surface.component_id,
            surface.surface_id
        );
    }
    Ok(())
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
    tool_tasks: &[(&String, &DesiredRuntimeTask)],
    ui: &[(&String, &DesiredUi)],
) -> anyhow::Result<Sha256Digest> {
    let mut hasher = Sha256::new();
    hasher.update(HOST_SOURCE_REVISION_DOMAIN);
    hash_field(&mut hasher, component_id.as_bytes());
    for (name, skill) in skills {
        hash_field(&mut hasher, b"skill");
        hash_field(&mut hasher, skill.surface_id.as_bytes());
        hash_field(&mut hasher, name.as_bytes());
        hash_field(&mut hasher, skill.fingerprint.as_bytes());
    }
    for (name, task) in tool_tasks {
        hash_field(&mut hasher, b"tool");
        hash_field(&mut hasher, task.surface_id().as_bytes());
        hash_field(&mut hasher, name.as_bytes());
        hash_field(&mut hasher, task.fingerprint().as_bytes());
    }
    for (name, contribution) in ui {
        hash_field(&mut hasher, b"ui");
        hash_field(&mut hasher, contribution.surface_id.as_bytes());
        hash_field(&mut hasher, name.as_bytes());
        hash_field(&mut hasher, contribution.fingerprint.as_bytes());
    }
    Sha256Digest::new(format!("sha256:{:x}", hasher.finalize())).map_err(Into::into)
}

fn code_dependency_kind(kind: PluginSurfaceKind) -> anyhow::Result<CapabilityKind> {
    match kind {
        PluginSurfaceKind::Tool => Ok(CapabilityKind::Tool),
        PluginSurfaceKind::Skill => Ok(CapabilityKind::Skill),
        PluginSurfaceKind::Mcp => Ok(CapabilityKind::Mcp),
        PluginSurfaceKind::Flow => Ok(CapabilityKind::Flow),
        PluginSurfaceKind::Okf | PluginSurfaceKind::Ui => {
            bail!("A3S Use UI projects unsupported {kind:?} dependency evidence")
        }
    }
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
