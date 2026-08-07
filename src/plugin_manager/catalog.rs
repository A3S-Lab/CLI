use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Duration;

use a3s_use_core::{PluginSurfaceKind, VerifiedPluginCatalogRecord};
use a3s_use_extension::{
    search_cached_plugins, search_remote_plugins, PluginCatalogHost, PluginCatalogSearch,
    PluginCatalogSnapshot, ResolvedRemotePackage, TrustedRegistry, MAX_PLUGIN_CATALOG_PAGE_SIZE,
};
use tokio::time::timeout;

mod model;

pub use model::{
    PluginMarketplaceItem, PluginMarketplaceSnapshot, PluginMarketplaceSource,
    PluginMarketplaceSourceKind, PluginMarketplaceSourceMetadata,
};

use super::{
    PluginInstallationIndex, PluginManager, PluginManagerError, PluginManagerResult,
    MARKETPLACE_REFRESH_TIMEOUT_SECONDS, MAX_MARKETPLACE_ITEMS, MAX_MARKETPLACE_REGISTRIES,
};

const RELEASE_BUNDLE_SOURCE_NAME: &str = "A3S \u{53d1}\u{884c}\u{5305}";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CatalogAccess {
    Refresh,
    Cached,
}

pub(super) async fn marketplace(
    manager: &PluginManager,
    installed: &PluginInstallationIndex,
    access: CatalogAccess,
) -> PluginManagerResult<PluginMarketplaceSnapshot> {
    let records = manager
        .registry_store
        .list()
        .map_err(|error| PluginManagerError::Infrastructure(error.to_string()))?;
    if records.len() > MAX_MARKETPLACE_REGISTRIES {
        return Err(PluginManagerError::Infrastructure(format!(
            "plugin registry count exceeds the {MAX_MARKETPLACE_REGISTRIES}-source limit"
        )));
    }

    let mut registries = Vec::new();
    let mut items = Vec::new();
    let mut bundled_identities = HashSet::new();
    add_release_bundles(
        manager,
        installed,
        &mut registries,
        &mut items,
        &mut bundled_identities,
    )
    .await;

    for record in records {
        if !record.enabled {
            registries.push(PluginMarketplaceSource {
                name: record.name,
                url: record.url,
                source_kind: PluginMarketplaceSourceKind::Registry,
                configured: record.configured,
                enabled: false,
                verified: false,
                host_target: None,
                metadata: None,
                error: None,
            });
            continue;
        }
        if !record.configured {
            registries.push(PluginMarketplaceSource {
                name: record.name,
                url: record.url,
                source_kind: PluginMarketplaceSourceKind::Registry,
                configured: false,
                enabled: true,
                verified: false,
                host_target: None,
                metadata: None,
                error: None,
            });
            continue;
        }
        let trusted = match record.trusted_registry(&manager.component_paths.state_root) {
            Ok(trusted) => trusted,
            Err(error) => {
                registries.push(failed_registry_source(
                    record.name,
                    record.url,
                    error.to_string(),
                ));
                continue;
            }
        };
        let refresh_timeout = Duration::from_secs(MARKETPLACE_REFRESH_TIMEOUT_SECONDS);
        match timeout(
            refresh_timeout,
            browse_registry(&trusted, installed, access),
        )
        .await
        {
            Ok(Ok(catalog)) => {
                items.extend(catalog.items.into_iter().filter(|item| {
                    !bundled_identities.contains(&(
                        item.package_id.clone(),
                        item.version.clone(),
                        item.channel.clone(),
                    ))
                }));
                registries.push(catalog.source);
            }
            Ok(Err(error)) => registries.push(failed_registry_source(
                record.name,
                record.url,
                error.to_string(),
            )),
            Err(_) => registries.push(failed_registry_source(
                record.name,
                record.url,
                format!(
                    "registry verification timed out after {MARKETPLACE_REFRESH_TIMEOUT_SECONDS} seconds"
                ),
            )),
        }
    }

    registries.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.url.cmp(&right.url))
    });
    items.sort_by(|left, right| {
        left.component_id
            .cmp(&right.component_id)
            .then_with(|| left.channel.cmp(&right.channel))
            .then_with(|| left.registry_name.cmp(&right.registry_name))
    });
    let total_items = u64::try_from(items.len()).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "plugin Marketplace item count is invalid: {error}"
        ))
    })?;
    let truncated = items.len() > MAX_MARKETPLACE_ITEMS;
    items.truncate(MAX_MARKETPLACE_ITEMS);

    Ok(PluginMarketplaceSnapshot {
        schema_version: 1,
        verified_at: chrono::Utc::now().to_rfc3339(),
        registries,
        items,
        total_items,
        truncated,
    })
}

struct BrowsedRegistry {
    source: PluginMarketplaceSource,
    items: Vec<PluginMarketplaceItem>,
}

async fn browse_registry(
    trusted: &TrustedRegistry,
    installed: &PluginInstallationIndex,
    access: CatalogAccess,
) -> Result<BrowsedRegistry, a3s_use_core::UseError> {
    let host = PluginCatalogHost::current()?;
    let mut search = PluginCatalogSearch {
        query: String::new(),
        kind: None,
        channel: None,
        publisher: None,
        category: None,
        availability: None,
        cursor: None,
        limit: MAX_PLUGIN_CATALOG_PAGE_SIZE,
    };
    let mut page = match access {
        CatalogAccess::Refresh => search_remote_plugins(trusted, &host, &search).await?,
        CatalogAccess::Cached => search_cached_plugins(trusted, &host, &search).await?,
    };
    let snapshot = page.snapshot.clone();
    let snapshot_digest = snapshot.snapshot_digest.clone();
    let total_matches = page.total_matches;
    if total_matches > MAX_MARKETPLACE_ITEMS as u64 {
        return Err(a3s_use_core::UseError::new(
            "use.plugin.manager_catalog_too_large",
            format!(
                "The verified catalog has {total_matches} compatible releases; the Marketplace source limit is {MAX_MARKETPLACE_ITEMS}."
            ),
        ));
    }
    let mut complete = Vec::new();
    let mut identities = BTreeSet::new();
    loop {
        for plugin in page.plugins {
            let identity = catalog_identity(&plugin);
            if !identities.insert(identity) {
                return Err(a3s_use_core::UseError::new(
                    "use.plugin.manager_catalog_invalid",
                    "The verified catalog returned a duplicate plugin release.",
                ));
            }
            complete.push(plugin);
        }
        let Some(cursor) = page.next_cursor else {
            break;
        };
        search.cursor = Some(cursor);
        page = search_cached_plugins(trusted, &host, &search).await?;
        if page.snapshot.snapshot_digest != snapshot_digest || page.total_matches != total_matches {
            return Err(a3s_use_core::UseError::new(
                "use.plugin.manager_catalog_changed",
                "The verified catalog changed during Marketplace pagination.",
            ));
        }
    }
    if complete.len() as u64 != total_matches {
        return Err(a3s_use_core::UseError::new(
            "use.plugin.manager_catalog_invalid",
            "The verified catalog pagination result is incomplete.",
        ));
    }

    let mut items = complete
        .into_iter()
        .map(|plugin| catalog_item(plugin, installed))
        .collect::<Result<Vec<_>, _>>()?;
    if snapshot.catalog_records != snapshot.metadata.package_targets {
        return Err(a3s_use_core::UseError::new(
            "use.plugin.manager_catalog_incomplete",
            "Every Registry package target must carry a complete current catalog record.",
        ));
    }
    items = latest_registry_items(items)?;

    Ok(BrowsedRegistry {
        source: verified_registry_source(trusted, &snapshot),
        items,
    })
}

pub(super) fn catalog_item(
    plugin: VerifiedPluginCatalogRecord,
    installed: &PluginInstallationIndex,
) -> Result<PluginMarketplaceItem, a3s_use_core::UseError> {
    let resolved = ResolvedRemotePackage::from_verified_catalog(&plugin)?;
    let plan_digest = resolved.plan_digest()?;
    let record = &plugin.record;
    let component_id = format!("use/{}", record.package_id);
    let enabled = installed.get(&component_id).copied();
    let mut surface_kinds = record
        .surfaces
        .iter()
        .map(|surface| surface_kind_name(surface.kind).to_string())
        .collect::<Vec<_>>();
    surface_kinds.sort();
    surface_kinds.dedup();
    Ok(PluginMarketplaceItem {
        component_id,
        package_id: record.package_id.clone(),
        display_name: record.display_name.clone(),
        registry_name: plugin.provenance.registry_name.clone(),
        registry_url: plugin.provenance.registry_url.clone(),
        source_kind: PluginMarketplaceSourceKind::Registry,
        version: record.version.clone(),
        channel: record.channel.as_str().to_owned(),
        target: record.target.clone(),
        archive_name: resolved.archive_name,
        length: record.archive.length,
        sha256: resolved.sha256,
        signed_plan_digest: Some(plan_digest),
        integrity_digest: Some(plugin.provenance.catalog_record_digest.clone()),
        catalog_schema: Some(record.schema.clone()),
        description: Some(record.description.clone()),
        publisher: Some(record.publisher.clone()),
        keywords: record.keywords.clone(),
        categories: record.categories.clone(),
        requires_use: Some(record.requires_use.clone()),
        surfaces: record.surfaces.clone(),
        surface_kinds,
        permission_ceiling: Some(record.permission_ceiling.clone()),
        permission_ceiling_digest: Some(record.permission_ceiling_digest.clone()),
        package: Some(record.package.clone()),
        license: Some(record.license.clone()),
        repository: Some(record.repository.clone()),
        availability: Some(record.availability.clone()),
        provenance: Some(plugin.provenance.clone()),
        installed: enabled.is_some(),
        enabled: enabled.unwrap_or(false),
    })
}

fn latest_registry_items(
    items: Vec<PluginMarketplaceItem>,
) -> Result<Vec<PluginMarketplaceItem>, a3s_use_core::UseError> {
    let mut latest = BTreeMap::<(String, String, String), PluginMarketplaceItem>::new();
    for item in items {
        let key = (
            item.registry_name.clone(),
            item.package_id.clone(),
            item.channel.clone(),
        );
        let candidate = semver::Version::parse(&item.version).map_err(|error| {
            a3s_use_core::UseError::new(
                "use.plugin.manager_catalog_invalid",
                format!("The verified catalog contains an invalid version: {error}"),
            )
        })?;
        let replace = latest.get(&key).is_none_or(|current| {
            semver::Version::parse(&current.version)
                .map(|version| candidate > version)
                .unwrap_or(true)
        });
        if replace {
            latest.insert(key, item);
        }
    }
    Ok(latest.into_values().collect())
}

async fn add_release_bundles(
    manager: &PluginManager,
    installed: &PluginInstallationIndex,
    registries: &mut Vec<PluginMarketplaceSource>,
    items: &mut Vec<PluginMarketplaceItem>,
    identities: &mut HashSet<(String, String, String)>,
) {
    match crate::components::list_release_bundles_with(&manager.component_paths).await {
        Ok(packages) if !packages.is_empty() => {
            if packages.len() > MAX_MARKETPLACE_ITEMS {
                registries.push(PluginMarketplaceSource {
                    name: RELEASE_BUNDLE_SOURCE_NAME.to_string(),
                    url: "a3s-use://release-bundles".to_string(),
                    source_kind: PluginMarketplaceSourceKind::ReleaseBundle,
                    configured: true,
                    enabled: true,
                    verified: false,
                    host_target: None,
                    metadata: None,
                    error: Some(format!(
                        "release bundle count exceeds the {MAX_MARKETPLACE_ITEMS}-item source limit"
                    )),
                });
                return;
            }
            let package_count = packages.len() as u64;
            for package in packages {
                let enabled = installed.get(&package.component_id).copied();
                identities.insert((
                    package.package_id.clone(),
                    package.version.clone(),
                    "stable".to_string(),
                ));
                items.push(PluginMarketplaceItem {
                    component_id: package.component_id,
                    display_name: package_display_name(&package.package_id),
                    package_id: package.package_id.clone(),
                    registry_name: RELEASE_BUNDLE_SOURCE_NAME.to_string(),
                    registry_url: "a3s-use://release-bundles".to_string(),
                    source_kind: PluginMarketplaceSourceKind::ReleaseBundle,
                    version: package.version,
                    channel: "stable".to_string(),
                    target: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
                    archive_name: format!("release-bundle/{}", package.package_id),
                    length: package.byte_count,
                    sha256: package.package_sha256.clone(),
                    signed_plan_digest: None,
                    integrity_digest: Some(package.package_sha256),
                    catalog_schema: None,
                    description: None,
                    publisher: None,
                    keywords: Vec::new(),
                    categories: Vec::new(),
                    requires_use: None,
                    surfaces: Vec::new(),
                    surface_kinds: package.surfaces,
                    permission_ceiling: None,
                    permission_ceiling_digest: None,
                    package: None,
                    license: None,
                    repository: None,
                    availability: None,
                    provenance: None,
                    installed: enabled.is_some(),
                    enabled: enabled.unwrap_or(false),
                });
            }
            registries.push(PluginMarketplaceSource {
                name: RELEASE_BUNDLE_SOURCE_NAME.to_string(),
                url: "a3s-use://release-bundles".to_string(),
                source_kind: PluginMarketplaceSourceKind::ReleaseBundle,
                configured: true,
                enabled: true,
                verified: true,
                host_target: Some(format!(
                    "{}-{}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )),
                metadata: Some(PluginMarketplaceSourceMetadata {
                    package_targets: package_count,
                    catalog_records: None,
                    root_version: None,
                    timestamp_version: None,
                    snapshot_version: None,
                    targets_version: None,
                    verified_at_unix_seconds: None,
                    age_seconds: None,
                    snapshot_digest: None,
                }),
                error: None,
            });
        }
        Ok(_) => {}
        Err(error) => registries.push(PluginMarketplaceSource {
            name: RELEASE_BUNDLE_SOURCE_NAME.to_string(),
            url: "a3s-use://release-bundles".to_string(),
            source_kind: PluginMarketplaceSourceKind::ReleaseBundle,
            configured: true,
            enabled: true,
            verified: false,
            host_target: None,
            metadata: None,
            error: Some(concise_error(&error.to_string())),
        }),
    }
}

fn verified_registry_source(
    trusted: &TrustedRegistry,
    snapshot: &PluginCatalogSnapshot,
) -> PluginMarketplaceSource {
    PluginMarketplaceSource {
        name: trusted.name().to_owned(),
        url: trusted.base_url().to_string(),
        source_kind: PluginMarketplaceSourceKind::Registry,
        configured: true,
        enabled: true,
        verified: true,
        host_target: Some(snapshot.host_target.clone()),
        metadata: Some(PluginMarketplaceSourceMetadata {
            package_targets: snapshot.metadata.package_targets,
            catalog_records: Some(snapshot.catalog_records),
            root_version: Some(snapshot.metadata.root_version),
            timestamp_version: Some(snapshot.metadata.timestamp_version),
            snapshot_version: Some(snapshot.metadata.snapshot_version),
            targets_version: Some(snapshot.metadata.targets_version),
            verified_at_unix_seconds: Some(snapshot.verified_at_unix_seconds),
            age_seconds: Some(snapshot.age_seconds),
            snapshot_digest: Some(snapshot.snapshot_digest.clone()),
        }),
        error: None,
    }
}

fn failed_registry_source(name: String, url: String, error: String) -> PluginMarketplaceSource {
    PluginMarketplaceSource {
        name,
        url,
        source_kind: PluginMarketplaceSourceKind::Registry,
        configured: true,
        enabled: true,
        verified: false,
        host_target: None,
        metadata: None,
        error: Some(concise_error(&error)),
    }
}

fn catalog_identity(plugin: &VerifiedPluginCatalogRecord) -> (String, String, String, String) {
    (
        plugin.record.package_id.clone(),
        plugin.record.version.clone(),
        plugin.record.channel.as_str().to_owned(),
        plugin.record.target.clone(),
    )
}

const fn surface_kind_name(kind: PluginSurfaceKind) -> &'static str {
    match kind {
        PluginSurfaceKind::Flow => "flow",
        PluginSurfaceKind::Mcp => "mcp",
        PluginSurfaceKind::Okf => "okf",
        PluginSurfaceKind::Skill => "skill",
        PluginSurfaceKind::Tool => "tool",
        PluginSurfaceKind::Ui => "ui",
    }
}

pub(super) fn package_display_name(package_id: &str) -> String {
    if package_id == "a3s/science" {
        return "\u{79d1}\u{7814}".to_string();
    }
    package_id
        .rsplit('/')
        .next()
        .unwrap_or(package_id)
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + characters.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn concise_error(value: &str) -> String {
    let value = value.trim().replace(['\n', '\r'], " ");
    let mut concise = value.chars().take(500).collect::<String>();
    if value.chars().count() > 500 {
        concise.push('…');
    }
    concise
}
