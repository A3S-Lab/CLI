//! Exact-generation A3S Flow projection from the A3S Use registry contract.

use super::{CapabilityBinding, CapabilityOrigin, CapabilityReadiness, ProjectedManagedAsset};
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const MAX_FLOW_SOURCE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum UseFlowEngine {
    A3sFlow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum UseFlowRuntime {
    NativeTs,
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

fn valid_segment(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}
