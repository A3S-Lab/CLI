use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::plugin_manager::{
    PluginInstallationSnapshot, PluginLifecycleAction, PluginManager, PluginMarketplaceItem,
    PluginMarketplaceSnapshot, PluginPlanRequest,
};

use super::input::{
    self, InspectInput, ListInput, PackageScopeInput, PlanInput, SearchInput, SurfaceKind,
};
use super::PluginToolError;

pub(super) async fn execute(
    manager: &PluginManager,
    tool: &str,
    arguments: Option<Map<String, Value>>,
) -> Result<Value, PluginToolError> {
    match tool {
        "plugin_search" => search(manager, input::parse(arguments)?).await,
        "plugin_inspect" => inspect(manager, input::parse(arguments)?).await,
        "plugin_list_installed" => list_installed(manager, input::parse(arguments)?).await,
        "plugin_status" => status(manager, input::parse(arguments)?).await,
        "plugin_plan_install" => {
            plan(
                manager,
                PluginLifecycleAction::Install,
                input::parse(arguments)?,
            )
            .await
        }
        "plugin_plan_upgrade" => {
            plan(
                manager,
                PluginLifecycleAction::Upgrade,
                input::parse(arguments)?,
            )
            .await
        }
        "plugin_plan_uninstall" => plan_uninstall(manager, input::parse(arguments)?).await,
        _ => Err(PluginToolError::invalid(
            "tool is not exposed by the read-only Plugin Manager",
        )),
    }
}

async fn search(manager: &PluginManager, mut input: SearchInput) -> Result<Value, PluginToolError> {
    input.validate()?;
    let installed = manager.installation_snapshot().await;
    let marketplace = manager.marketplace(&installed.index()).await?;
    let matches = marketplace
        .items
        .iter()
        .filter(|item| search_match(item, &input))
        .collect::<Vec<_>>();
    let fingerprint = search_fingerprint(&input, &matches)?;
    let offset = cursor_offset(input.cursor.as_deref(), &fingerprint, matches.len())?;
    let end = offset.saturating_add(input.limit()).min(matches.len());
    let items = matches[offset..end]
        .iter()
        .map(|item| search_summary(item))
        .collect::<Vec<_>>();
    let next_cursor = (end < matches.len()).then(|| cursor(&fingerprint, end));
    let warnings = warnings(&marketplace, &installed);
    Ok(json!({
        "schemaVersion": 1,
        "catalogVerifiedAt": marketplace.verified_at,
        "query": {
            "text": input.query,
            "kind": input.kind.map(SurfaceKind::as_str),
            "channel": input.channel.map(|channel| channel.as_str()),
            "limit": input.limit(),
        },
        "items": items,
        "totalMatches": matches.len(),
        "nextCursor": next_cursor,
        "truncated": marketplace.truncated || end < matches.len(),
        "warnings": warnings,
    }))
}

async fn inspect(manager: &PluginManager, input: InspectInput) -> Result<Value, PluginToolError> {
    input.validate()?;
    let installed = manager.installation_snapshot().await;
    let marketplace = manager.marketplace(&installed.index()).await?;
    let matches = marketplace
        .items
        .iter()
        .filter(|item| {
            item.package_id == input.package_id
                && input
                    .version
                    .as_deref()
                    .is_none_or(|version| item.version == version)
                && input
                    .channel
                    .is_none_or(|channel| item.channel == channel.as_str())
        })
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Err(PluginToolError::new(
            "plugin.not_found",
            format!(
                "No verified release matched plugin '{}'; search the catalog and use an exact returned identity.",
                input.package_id
            ),
            false,
        ));
    }
    if matches.len() > 128 {
        return Err(PluginToolError::new(
            "plugin.inspect_ambiguous",
            "More than 128 verified releases matched; provide version and channel.",
            false,
        ));
    }
    Ok(json!({
        "schemaVersion": 1,
        "catalogVerifiedAt": marketplace.verified_at,
        "packageId": input.package_id,
        "matches": matches,
        "totalMatches": matches.len(),
        "sources": marketplace.registries,
        "warnings": warnings(&marketplace, &installed),
    }))
}

async fn list_installed(
    manager: &PluginManager,
    input: ListInput,
) -> Result<Value, PluginToolError> {
    input.validate()?;
    let snapshot = manager.installation_snapshot().await;
    let fingerprint = installed_fingerprint(&snapshot)?;
    let offset = cursor_offset(input.cursor.as_deref(), &fingerprint, snapshot.items.len())?;
    let end = offset
        .saturating_add(input.limit())
        .min(snapshot.items.len());
    let next_cursor = (end < snapshot.items.len()).then(|| cursor(&fingerprint, end));
    Ok(json!({
        "schemaVersion": 1,
        "scope": input::scope_value(input.scope_kind, &input.scope_id),
        "available": snapshot.available,
        "observedAtMs": snapshot.observed_at_ms,
        "generation": snapshot.generation,
        "revision": snapshot.revision,
        "items": &snapshot.items[offset..end],
        "totalItems": snapshot.items.len(),
        "nextCursor": next_cursor,
        "error": snapshot.error,
    }))
}

async fn status(
    manager: &PluginManager,
    input: PackageScopeInput,
) -> Result<Value, PluginToolError> {
    input.validate()?;
    let snapshot = manager.installation_snapshot().await;
    if !snapshot.available {
        return Ok(json!({
            "schemaVersion": 1,
            "scope": input::scope_value(input.scope_kind, &input.scope_id),
            "available": false,
            "observedAtMs": snapshot.observed_at_ms,
            "item": null,
            "error": snapshot.error,
        }));
    }
    let item = snapshot
        .items
        .iter()
        .find(|item| item.package_id == input.package_id)
        .ok_or_else(|| {
            PluginToolError::new(
                "plugin.not_installed",
                format!("Plugin '{}' is not installed.", input.package_id),
                false,
            )
        })?;
    Ok(json!({
        "schemaVersion": 1,
        "scope": input::scope_value(input.scope_kind, &input.scope_id),
        "available": true,
        "observedAtMs": snapshot.observed_at_ms,
        "generation": snapshot.generation,
        "revision": snapshot.revision,
        "item": item,
    }))
}

async fn plan(
    manager: &PluginManager,
    action: PluginLifecycleAction,
    input: PlanInput,
) -> Result<Value, PluginToolError> {
    input.validate()?;
    if action == PluginLifecycleAction::Upgrade
        && (input.version_requirement.is_some() || input.channel.is_some())
    {
        return Err(PluginToolError::new(
            "plugin.upgrade_constraint_unsupported",
            "This host release upgrades to the resolver-selected compatible release; omit versionRequirement and channel.",
            false,
        ));
    }
    let version = if action == PluginLifecycleAction::Install {
        input.exact_version()?
    } else {
        None
    };
    let channel = if action == PluginLifecycleAction::Install {
        input.channel.map(|channel| channel.as_str().to_string())
    } else {
        None
    };
    let plan = manager
        .plan_operation(&PluginPlanRequest {
            action,
            component_id: format!("use/{}", input.package_id),
            version,
            channel,
        })
        .await?;
    Ok(json!({
        "schemaVersion": 1,
        "scope": input::scope_value(input.scope_kind, &input.scope_id),
        "plan": plan,
    }))
}

async fn plan_uninstall(
    manager: &PluginManager,
    input: PackageScopeInput,
) -> Result<Value, PluginToolError> {
    input.validate()?;
    let plan = manager
        .plan_operation(&PluginPlanRequest {
            action: PluginLifecycleAction::Uninstall,
            component_id: format!("use/{}", input.package_id),
            version: None,
            channel: None,
        })
        .await?;
    Ok(json!({
        "schemaVersion": 1,
        "scope": input::scope_value(input.scope_kind, &input.scope_id),
        "plan": plan,
    }))
}

fn search_match(item: &PluginMarketplaceItem, input: &SearchInput) -> bool {
    let query = input.query.to_lowercase();
    let text_match = [
        Some(item.package_id.as_str()),
        Some(item.component_id.as_str()),
        Some(item.display_name.as_str()),
        item.description.as_deref(),
        item.publisher.as_deref(),
    ]
    .into_iter()
    .flatten()
    .chain(item.keywords.iter().map(String::as_str))
    .chain(item.categories.iter().map(String::as_str))
    .any(|value| value.to_lowercase().contains(&query));
    text_match
        && input.kind.is_none_or(|kind| {
            item.surface_kinds
                .iter()
                .any(|candidate| candidate == kind.as_str())
        })
        && input
            .channel
            .is_none_or(|channel| item.channel == channel.as_str())
}

fn search_summary(item: &PluginMarketplaceItem) -> Value {
    json!({
        "packageId": item.package_id,
        "displayName": item.display_name,
        "description": item.description,
        "version": item.version,
        "channel": item.channel,
        "surfaceKinds": item.surface_kinds,
        "downloadBytes": item.length,
        "archiveSha256": item.sha256,
        "permissionCeilingDigest": item.permission_ceiling_digest,
        "source": {
            "name": item.registry_name,
            "url": item.registry_url,
            "kind": item.source_kind,
        },
        "provenance": item.provenance,
        "availability": item.availability,
        "installed": item.installed,
        "enabled": item.enabled,
    })
}

fn warnings(
    marketplace: &PluginMarketplaceSnapshot,
    installed: &PluginInstallationSnapshot,
) -> Vec<String> {
    let mut warnings = marketplace
        .registries
        .iter()
        .filter_map(|source| {
            source
                .error
                .as_ref()
                .map(|error| format!("{}: {error}", source.name))
        })
        .collect::<Vec<_>>();
    if let Some(error) = &installed.error {
        warnings.push(format!(
            "installed plugin state is unavailable; catalog flags may be incomplete: {error}"
        ));
    }
    warnings
}

fn search_fingerprint(
    input: &SearchInput,
    matches: &[&PluginMarketplaceItem],
) -> Result<String, PluginToolError> {
    let identities = matches
        .iter()
        .map(|item| {
            (
                item.component_id.as_str(),
                item.version.as_str(),
                item.channel.as_str(),
                item.registry_name.as_str(),
                item.sha256.as_str(),
            )
        })
        .collect::<Vec<_>>();
    digest(&(
        input.query.as_str(),
        input.kind.map(SurfaceKind::as_str),
        input.channel.map(|channel| channel.as_str()),
        identities,
    ))
}

fn installed_fingerprint(snapshot: &PluginInstallationSnapshot) -> Result<String, PluginToolError> {
    digest(&(
        snapshot.available,
        snapshot.generation,
        snapshot.revision.as_deref(),
        &snapshot.items,
    ))
}

fn digest<T: Serialize>(value: &T) -> Result<String, PluginToolError> {
    let encoded = serde_json::to_vec(value).map_err(|error| {
        PluginToolError::new(
            "plugin.cursor_invalid",
            format!("Could not bind the result cursor: {error}"),
            false,
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn cursor(fingerprint: &str, offset: usize) -> String {
    format!("v1:{fingerprint}:{offset}")
}

fn cursor_offset(
    cursor: Option<&str>,
    fingerprint: &str,
    item_count: usize,
) -> Result<usize, PluginToolError> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let mut segments = cursor.split(':');
    let version = segments.next();
    let cursor_fingerprint = segments.next();
    let offset = segments.next();
    if version != Some("v1") || cursor_fingerprint != Some(fingerprint) || segments.next().is_some()
    {
        return Err(PluginToolError::new(
            "plugin.cursor_stale",
            "Cursor does not match the current verified result set; restart from the first page.",
            false,
        ));
    }
    let offset = offset
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|offset| *offset <= item_count)
        .ok_or_else(|| PluginToolError::invalid("cursor offset is invalid"))?;
    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_is_bound_to_one_result_snapshot() {
        let fingerprint = "a".repeat(64);
        let cursor = cursor(&fingerprint, 7);
        assert_eq!(cursor_offset(Some(&cursor), &fingerprint, 10).unwrap(), 7);
        assert_eq!(
            cursor_offset(Some(&cursor), &"b".repeat(64), 10)
                .unwrap_err()
                .code,
            "plugin.cursor_stale"
        );
        assert!(cursor_offset(Some("v1:bad:999"), "bad", 10).is_err());
    }
}
