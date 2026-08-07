use a3s_use_core::{
    CatalogAvailability, CatalogPackage, CatalogSurface, PluginPermissionCeiling,
    VerifiedCatalogProvenance,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginMarketplaceSourceKind {
    Registry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMarketplaceSourceMetadata {
    pub package_targets: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_records: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub targets_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_at_unix_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMarketplaceSource {
    pub name: String,
    pub url: String,
    pub source_kind: PluginMarketplaceSourceKind,
    pub configured: bool,
    pub enabled: bool,
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<PluginMarketplaceSourceMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMarketplaceItem {
    pub component_id: String,
    pub package_id: String,
    pub display_name: String,
    pub registry_name: String,
    pub registry_url: String,
    pub source_kind: PluginMarketplaceSourceKind,
    pub version: String,
    pub channel: String,
    pub target: String,
    pub archive_name: String,
    pub length: u64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_plan_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_use: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<CatalogSurface>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surface_kinds: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_ceiling: Option<PluginPermissionCeiling>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_ceiling_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<CatalogPackage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability: Option<CatalogAvailability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<VerifiedCatalogProvenance>,
    pub installed: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMarketplaceSnapshot {
    pub schema_version: u32,
    pub verified_at: String,
    pub registries: Vec<PluginMarketplaceSource>,
    pub items: Vec<PluginMarketplaceItem>,
    pub total_items: u64,
    pub truncated: bool,
}
