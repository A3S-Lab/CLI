//! Exact-generation A3S Flow projection from the A3S Use registry contract.

use super::{CapabilityBinding, CapabilityOrigin, CapabilityReadiness, ProjectedManagedAsset};
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const MAX_FLOW_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_FLOW_DESIGN_BYTES: usize = 4 * 1024 * 1024;
const MAX_FLOW_DESIGN_ITEMS: usize = 10_000;
const FLOW_DESIGN_SCHEMA: &str = "a3s.workflow.design.v1";
const INSTALLED_FLOW_REFERENCE_SCHEMA: &str = "a3s.use.installed-flow.v1";
const RESOLVED_FLOW_SCHEMA: &str = "a3s.use.resolved-flow.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum UseFlowEngine {
    A3sFlow,
}

impl UseFlowEngine {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::A3sFlow => "a3s-flow",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum UseFlowRuntime {
    NativeTs,
}

impl UseFlowRuntime {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NativeTs => "native-ts",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectedFlowSurface {
    pub(super) id: String,
    pub(super) engine: UseFlowEngine,
    pub(super) runtime: UseFlowRuntime,
    pub(super) source: ProjectedManagedAsset,
    pub(super) export_name: String,
    #[serde(default)]
    pub(super) requires_tools: Vec<String>,
    #[serde(default)]
    pub(super) requires_mcp: Vec<String>,
    #[serde(default)]
    pub(super) requires_okf: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UseFlowCatalogItem {
    pub(crate) key: String,
    pub(crate) package_id: String,
    pub(crate) route: String,
    pub(crate) version: String,
    pub(crate) lifecycle_generation: u64,
    pub(crate) id: String,
    pub(crate) engine: UseFlowEngine,
    pub(crate) runtime: UseFlowRuntime,
    #[serde(skip_serializing)]
    pub(crate) source_path: PathBuf,
    pub(crate) export_name: String,
    pub(crate) sha256: String,
    pub(crate) media_type: String,
    pub(crate) requires_tools: Vec<String>,
    pub(crate) requires_mcp: Vec<String>,
    pub(crate) requires_okf: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UseFlowCatalog {
    pub(crate) schema_version: u32,
    pub(crate) generation: u64,
    pub(crate) revision: String,
    pub(crate) items: Vec<UseFlowCatalogItem>,
}

/// Exact package-owned Flow identity persisted in a Code `flow.json` design.
///
/// The reference intentionally contains no filesystem path. Code resolves it
/// only against the digest-verified live Use catalog and copies the resolved
/// evidence into deployment metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InstalledFlowReference {
    pub(crate) schema: String,
    pub(crate) package_id: String,
    pub(crate) flow_id: String,
    pub(crate) version: String,
    pub(crate) lifecycle_generation: u64,
    pub(crate) source_sha256: String,
}

impl InstalledFlowReference {
    fn validate(&self) -> anyhow::Result<()> {
        if self.schema != INSTALLED_FLOW_REFERENCE_SCHEMA {
            bail!("installedFlow schema must be '{INSTALLED_FLOW_REFERENCE_SCHEMA}'");
        }
        if !valid_use_package_id(&self.package_id) {
            bail!("installedFlow packageId must use exact use/<publisher>/<name> syntax");
        }
        if !valid_segment(&self.flow_id) {
            bail!("installedFlow flowId is invalid");
        }
        let parsed_version = semver::Version::parse(&self.version)
            .context("installedFlow version must be canonical SemVer")?;
        if parsed_version.to_string() != self.version {
            bail!("installedFlow version must be canonical SemVer");
        }
        if self.lifecycle_generation == 0 {
            bail!("installedFlow lifecycleGeneration must be greater than zero");
        }
        if !is_lower_sha256(&self.source_sha256) {
            bail!("installedFlow sourceSha256 must be a lowercase SHA-256 digest");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedFlowDesign {
    pub(crate) value: serde_json::Value,
    pub(crate) installed_flow: Option<InstalledFlowReference>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FlowDesignEnvelope {
    version: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    installed_flow: Option<InstalledFlowReference>,
    nodes: Vec<serde_json::Value>,
    edges: Vec<serde_json::Value>,
    #[serde(flatten)]
    extensions: BTreeMap<String, serde_json::Value>,
}

/// Parse the stable Code design envelope while leaving designer-owned node
/// and extension payloads opaque. Known identity fields still use normal
/// Serde duplicate-field checks, so untrusted JSON cannot rely on
/// last-value-wins behavior for `installedFlow`.
pub(crate) fn parse_flow_design(input: &str) -> anyhow::Result<ParsedFlowDesign> {
    if input.is_empty() || input.len() > MAX_FLOW_DESIGN_BYTES {
        bail!("workflow design must contain between 1 and {MAX_FLOW_DESIGN_BYTES} UTF-8 bytes");
    }
    let envelope: FlowDesignEnvelope = serde_json::from_str(input).map_err(|error| {
        anyhow::anyhow!("workflow design does not match the typed schema: {error}")
    })?;
    if envelope.version != FLOW_DESIGN_SCHEMA {
        bail!("workflow design version must be '{FLOW_DESIGN_SCHEMA}'");
    }
    let name = envelope.name.trim();
    if name.is_empty() || name.len() > 256 {
        bail!("workflow design name must contain between 1 and 256 bytes");
    }
    if envelope.description.len() > 4096 {
        bail!("workflow design description exceeds 4096 bytes");
    }
    if envelope.nodes.len() > MAX_FLOW_DESIGN_ITEMS || envelope.edges.len() > MAX_FLOW_DESIGN_ITEMS
    {
        bail!("workflow design graph exceeds its item bound");
    }
    if envelope.extensions.len() > 256 {
        bail!("workflow design has too many top-level extension fields");
    }
    if let Some(reference) = &envelope.installed_flow {
        reference.validate()?;
    }
    let value = serde_json::from_str(input)
        .map_err(|error| anyhow::anyhow!("workflow design is not valid JSON: {error}"))?;
    Ok(ParsedFlowDesign {
        value,
        installed_flow: envelope.installed_flow,
    })
}

/// Exact, path-free binding returned after resolving a design against one
/// immutable live Use catalog snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolvedUseFlowIdentity {
    pub(crate) schema: String,
    pub(crate) catalog_generation: u64,
    pub(crate) catalog_revision: String,
    pub(crate) key: String,
    pub(crate) package_id: String,
    pub(crate) route: String,
    pub(crate) flow_id: String,
    pub(crate) version: String,
    pub(crate) lifecycle_generation: u64,
    pub(crate) engine: UseFlowEngine,
    pub(crate) runtime: UseFlowRuntime,
    pub(crate) export_name: String,
    pub(crate) source_sha256: String,
}

impl ResolvedUseFlowIdentity {
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "schema": self.schema,
            "catalogGeneration": self.catalog_generation,
            "catalogRevision": self.catalog_revision,
            "key": self.key,
            "packageId": self.package_id,
            "route": self.route,
            "flowId": self.flow_id,
            "version": self.version,
            "lifecycleGeneration": self.lifecycle_generation,
            "engine": self.engine.as_str(),
            "runtime": self.runtime.as_str(),
            "exportName": self.export_name,
            "sourceSha256": self.source_sha256,
        })
    }
}

impl UseFlowCatalog {
    pub(crate) fn is_available(&self) -> bool {
        self.schema_version == 1 && self.generation > 0 && is_lower_sha256(&self.revision)
    }

    pub(crate) fn resolve_design(
        &self,
        design: &ParsedFlowDesign,
    ) -> anyhow::Result<ResolvedUseFlowIdentity> {
        let reference = design.installed_flow.as_ref().context(
            "workflow design has no installedFlow identity; bind an exact A3S Use Flow before deployment",
        )?;
        self.resolve(reference)
    }

    pub(crate) fn resolve(
        &self,
        reference: &InstalledFlowReference,
    ) -> anyhow::Result<ResolvedUseFlowIdentity> {
        reference.validate()?;
        if !self.is_available() {
            bail!("A3S Use Flow catalog identity is invalid");
        }
        let matches = self
            .items
            .iter()
            .filter(|item| item.package_id == reference.package_id && item.id == reference.flow_id)
            .collect::<Vec<_>>();
        let item = match matches.as_slice() {
            [] => bail!(
                "installedFlow '{}:{}' is not installed or ready in the current A3S Use catalog",
                reference.package_id,
                reference.flow_id
            ),
            [item] => *item,
            _ => bail!(
                "installedFlow '{}:{}' is ambiguous in the current A3S Use catalog",
                reference.package_id,
                reference.flow_id
            ),
        };
        if item.version != reference.version {
            bail!(
                "installedFlow version mismatch: design requires {}, current generation is {}",
                reference.version,
                item.version
            );
        }
        if item.lifecycle_generation != reference.lifecycle_generation {
            bail!(
                "installedFlow lifecycle generation mismatch: design requires {}, current generation is {}",
                reference.lifecycle_generation,
                item.lifecycle_generation
            );
        }
        if item.sha256 != reference.source_sha256 {
            bail!("installedFlow source digest does not match the current generation");
        }
        Ok(ResolvedUseFlowIdentity {
            schema: RESOLVED_FLOW_SCHEMA.to_string(),
            catalog_generation: self.generation,
            catalog_revision: self.revision.clone(),
            key: item.key.clone(),
            package_id: item.package_id.clone(),
            route: item.route.clone(),
            flow_id: item.id.clone(),
            version: item.version.clone(),
            lifecycle_generation: item.lifecycle_generation,
            engine: item.engine,
            runtime: item.runtime,
            export_name: item.export_name.clone(),
            source_sha256: item.sha256.clone(),
        })
    }
}

pub(super) fn validate_projected_flows(binding: &CapabilityBinding) -> anyhow::Result<()> {
    if binding.flows.is_empty() {
        return Ok(());
    }
    if !binding.surfaces.iter().any(|surface| surface == "flow") {
        bail!(
            "A3S Use capability '{}' projects A3S Flow without declaring the surface",
            binding.id
        );
    }
    if binding.origin != CapabilityOrigin::Extension
        || !binding.enabled
        || binding.readiness != CapabilityReadiness::Ready
    {
        bail!(
            "A3S Use capability '{}' projects A3S Flow without a ready enabled extension generation",
            binding.id
        );
    }
    if !binding.package_root.is_absolute() {
        bail!(
            "A3S Use capability '{}' has A3S Flow without an absolute package root",
            binding.id
        );
    }
    if binding
        .lifecycle_generation
        .is_none_or(|generation| generation == 0)
    {
        bail!(
            "A3S Use capability '{}' has A3S Flow without an exact lifecycle generation",
            binding.id
        );
    }

    let mut ids = BTreeSet::new();
    for flow in &binding.flows {
        if !ids.insert(flow.id.as_str()) || !valid_segment(&flow.id) {
            bail!(
                "A3S Use capability '{}' projects an invalid or duplicate A3S Flow ID '{}'",
                binding.id,
                flow.id
            );
        }
        if !flow.source.path.is_absolute()
            || flow
                .source
                .path
                .extension()
                .and_then(|value| value.to_str())
                != Some("ts")
            || flow.source.media_type != "text/typescript"
            || !is_lower_sha256(&flow.source.sha256)
        {
            bail!(
                "A3S Use capability '{}' projects invalid A3S Flow source evidence",
                binding.id
            );
        }
        if !valid_flow_export(&flow.export_name) {
            bail!(
                "A3S Use capability '{}' projects an invalid A3S Flow export '{}'",
                binding.id,
                flow.export_name
            );
        }
        validate_dependencies(&binding.id, &flow.id, "Tool", &flow.requires_tools)?;
        validate_dependencies(&binding.id, &flow.id, "MCP", &flow.requires_mcp)?;
        validate_dependencies(&binding.id, &flow.id, "OKF", &flow.requires_okf)?;
    }
    Ok(())
}

pub(super) async fn verify_managed_source(
    package_root: &Path,
    flow: &ProjectedFlowSurface,
) -> anyhow::Result<()> {
    let root = tokio::fs::canonicalize(package_root)
        .await
        .with_context(|| {
            format!(
                "failed to resolve A3S Flow package root {}",
                package_root.display()
            )
        })?;
    let metadata = tokio::fs::symlink_metadata(&flow.source.path)
        .await
        .with_context(|| {
            format!(
                "failed to inspect A3S Flow source {}",
                flow.source.path.display()
            )
        })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_FLOW_SOURCE_BYTES
    {
        bail!(
            "A3S Flow source '{}' is not a bounded regular package file",
            flow.source.path.display()
        );
    }
    let canonical = tokio::fs::canonicalize(&flow.source.path)
        .await
        .with_context(|| {
            format!(
                "failed to resolve A3S Flow source {}",
                flow.source.path.display()
            )
        })?;
    if !canonical.starts_with(&root) {
        bail!(
            "A3S Flow source '{}' escapes its managed package",
            flow.source.path.display()
        );
    }
    let bytes = tokio::fs::read(&canonical)
        .await
        .with_context(|| format!("failed to read A3S Flow source {}", canonical.display()))?;
    if format!("{:x}", Sha256::digest(&bytes)) != flow.source.sha256 {
        bail!(
            "A3S Flow source '{}' digest does not match the capability registry",
            canonical.display()
        );
    }
    std::str::from_utf8(&bytes).with_context(|| {
        format!(
            "A3S Flow source '{}' must be UTF-8 TypeScript",
            canonical.display()
        )
    })?;
    Ok(())
}

fn validate_dependencies(
    package_id: &str,
    flow_id: &str,
    kind: &str,
    dependencies: &[String],
) -> anyhow::Result<()> {
    let mut unique = BTreeSet::new();
    if let Some(dependency) = dependencies
        .iter()
        .find(|dependency| !valid_segment(dependency) || !unique.insert(dependency.as_str()))
    {
        bail!(
            "A3S Use capability '{package_id}' Flow '{flow_id}' has an invalid or duplicate {kind} dependency '{dependency}'"
        );
    }
    Ok(())
}

fn valid_flow_export(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && value.len() <= 128
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_use_package_id(value: &str) -> bool {
    let mut segments = value.split('/');
    matches!(segments.next(), Some("use"))
        && segments.next().is_some_and(valid_segment)
        && segments.next().is_some_and(valid_segment)
        && segments.next().is_none()
}

fn valid_segment(value: &str) -> bool {
    let mut characters = value.chars();
    !value.is_empty()
        && value.len() <= 63
        && matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}
