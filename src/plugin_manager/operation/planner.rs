use a3s_use_core::{
    PlanPackageRole, PlannedOperationImpact, PlannedStateEvidence, PluginCatalogRecord,
    PluginOperationAction, PluginOperationPlanDraft, PluginSurfaceKind,
    VerifiedPluginCatalogRecord, PLUGIN_CATALOG_SCHEMA_V2,
};
use a3s_use_extension::ResolvedRemotePackage;
use serde_json::Value;

use crate::plugin_manager::capability::PluginInstallationSnapshot;
use crate::plugin_manager::process::{PluginLifecycleAction, PluginPlanRequest};
use crate::plugin_manager::{PluginManagerError, PluginManagerResult};

const OPERATION_PLAN_FIELD: &str = "pluginOperationPlan";

pub(super) fn attach_draft(
    request: &PluginPlanRequest,
    installation: &PluginInstallationSnapshot,
    state_revision: u64,
    mut raw_plan: Value,
) -> PluginManagerResult<Value> {
    if request.action != PluginLifecycleAction::Install {
        return Ok(raw_plan);
    }
    if raw_plan.get(OPERATION_PLAN_FIELD).is_some() {
        return Ok(raw_plan);
    }
    let has_verified_catalog =
        raw_plan
            .get("plans")
            .and_then(Value::as_array)
            .is_some_and(|plans| {
                plans.iter().any(|plan| {
                    plan.get("verifiedPluginCatalogRecords")
                        .and_then(Value::as_object)
                        .is_some_and(|records| !records.is_empty())
                })
            });
    if !has_verified_catalog {
        return Ok(raw_plan);
    }
    let plan = single_component_plan(&raw_plan, request)?;
    let Some(catalog_value) = plan
        .get("verifiedPluginCatalogRecords")
        .and_then(|records| records.get(&request.component_id))
    else {
        return Ok(raw_plan);
    };
    let catalog: VerifiedPluginCatalogRecord = serde_json::from_value(catalog_value.clone())
        .map_err(|error| {
            planner_error(format!(
                "verified plugin catalog evidence is invalid: {error}"
            ))
        })?;
    catalog
        .validate()
        .map_err(|error| planner_error(error.to_string()))?;
    if catalog.record.schema != PLUGIN_CATALOG_SCHEMA_V2 {
        return Err(planner_error(
            "only catalog-v2 packages can produce a complete install draft",
        ));
    }
    let resolved_value = plan
        .get("resolvedRegistryPackages")
        .and_then(|packages| packages.get(&request.component_id))
        .ok_or_else(|| {
            planner_error("verified catalog evidence has no exact resolved registry target")
        })?;
    let resolved: ResolvedRemotePackage = serde_json::from_value(resolved_value.clone())
        .map_err(|error| planner_error(format!("resolved registry target is invalid: {error}")))?;
    let expected = ResolvedRemotePackage::from_verified_catalog(&catalog).map_err(|error| {
        planner_error(format!(
            "verified catalog cannot resolve its registry target: {error}"
        ))
    })?;
    if resolved != expected {
        return Err(planner_error(
            "verified catalog does not match the exact umbrella registry target",
        ));
    }
    ensure_safe_live_slice(&catalog.record)?;
    if !installation.available {
        return Err(planner_error(
            "A3S Use installation state is unavailable for complete planning",
        ));
    }
    if installation
        .items
        .iter()
        .any(|item| item.package_id == catalog.record.package_id)
    {
        return Err(planner_error(
            "the requested package is already installed; resolve an upgrade instead",
        ));
    }
    let capability_generation = installation
        .generation
        .filter(|generation| *generation > 0)
        .ok_or_else(|| planner_error("capability generation evidence is unavailable"))?;
    if state_revision == 0 {
        return Err(planner_error(
            "durable plugin planner state revision is unavailable",
        ));
    }
    let mut selected_surfaces = catalog
        .record
        .surfaces
        .iter()
        .map(|surface| surface.reference())
        .collect::<Vec<_>>();
    selected_surfaces.sort();
    let transition = catalog
        .install_transition(PlanPackageRole::Root, &selected_surfaces)
        .map_err(|error| planner_error(error.to_string()))?;
    let draft = PluginOperationPlanDraft::new(
        PluginOperationAction::Install,
        catalog.record.package_id.clone(),
        request.component_id.clone(),
        vec![transition],
        Vec::new(),
        Vec::new(),
        PlannedOperationImpact {
            download_bytes: catalog.record.archive.length,
            installed_bytes_after: catalog.record.package.expanded_bytes,
            reclaimed_bytes: 0,
            drain_required: false,
            retained_data: false,
        },
        PlannedStateEvidence {
            state_revision,
            capability_generation,
            receipt_digest: None,
        },
    )
    .map_err(|error| planner_error(error.to_string()))?;
    let object = raw_plan
        .as_object_mut()
        .ok_or_else(|| planner_error("umbrella component plan is not an object"))?;
    if object.contains_key(OPERATION_PLAN_FIELD) {
        return Err(planner_error(
            "umbrella component plan already contains a plugin draft",
        ));
    }
    object.insert(
        OPERATION_PLAN_FIELD.to_string(),
        serde_json::to_value(draft).map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "failed to encode plugin operation draft: {error}"
            ))
        })?,
    );
    Ok(raw_plan)
}

fn single_component_plan<'a>(
    raw_plan: &'a Value,
    request: &PluginPlanRequest,
) -> PluginManagerResult<&'a serde_json::Map<String, Value>> {
    let plans = raw_plan
        .get("plans")
        .and_then(Value::as_array)
        .ok_or_else(|| planner_error("umbrella component plan has no plans array"))?;
    if plans.len() != 1 {
        return Err(planner_error(
            "plugin lifecycle planning requires exactly one component plan",
        ));
    }
    let plan = plans[0]
        .as_object()
        .ok_or_else(|| planner_error("umbrella component plan item is not an object"))?;
    if plan.get("component").and_then(Value::as_str) != Some(request.component_id.as_str())
        || plan.get("action").and_then(Value::as_str) != Some("install")
    {
        return Err(planner_error(
            "umbrella component plan does not match the plugin request",
        ));
    }
    Ok(plan)
}

fn ensure_safe_live_slice(record: &PluginCatalogRecord) -> PluginManagerResult<()> {
    if record.surfaces.iter().any(|surface| {
        matches!(
            surface.kind,
            PluginSurfaceKind::Tool | PluginSurfaceKind::Mcp
        )
    }) {
        return Err(planner_error(
            "executable plugin surfaces require an explicit live Runtime provider assignment",
        ));
    }
    if !record.permission_ceiling.surfaces.is_empty() {
        return Err(planner_error(
            "permission-bearing plugin surfaces require durable grant-saga integration",
        ));
    }
    Ok(())
}

fn planner_error(message: impl Into<String>) -> PluginManagerError {
    PluginManagerError::Upstream(format!(
        "complete plugin install draft is unavailable: {}",
        message.into()
    ))
}

#[cfg(test)]
mod tests {
    use a3s_use_core::{
        CatalogArchive, CatalogAvailability, CatalogPackage, CatalogSurface, PluginCatalogRecord,
        PluginOperationPlanDraft, PluginPermissionCeiling, PluginReleaseChannel, PluginSurfaceKind,
        VerifiedCatalogProvenance, VerifiedPluginCatalogRecord, PLUGIN_CATALOG_SCHEMA_V2,
        PLUGIN_PERMISSION_SCHEMA,
    };

    use super::*;

    #[test]
    fn plan_ready_skill_install_emits_a_complete_live_draft() {
        let catalog = skill_catalog();
        let resolved = ResolvedRemotePackage::from_verified_catalog(&catalog).unwrap();
        let raw = umbrella_plan(&catalog, &resolved);

        let output = attach_draft(&request(), &installation(), 3, raw).unwrap();
        let draft: PluginOperationPlanDraft =
            serde_json::from_value(output["pluginOperationPlan"].clone()).unwrap();

        assert_eq!(draft.package_id, "acme/guide");
        assert_eq!(draft.state.state_revision, 3);
        assert_eq!(draft.state.capability_generation, 9);
        assert_eq!(draft.packages.len(), 1);
        assert_eq!(draft.impact.download_bytes, 123);
        assert_eq!(draft.impact.installed_bytes_after, 456);
        assert!(draft.providers.is_empty());
    }

    #[test]
    fn live_draft_rejects_catalog_and_target_drift() {
        let catalog = skill_catalog();
        let mut resolved = ResolvedRemotePackage::from_verified_catalog(&catalog).unwrap();
        resolved.length += 1;

        let error = attach_draft(
            &request(),
            &installation(),
            3,
            umbrella_plan(&catalog, &resolved),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("does not match the exact umbrella registry target"));
    }

    #[test]
    fn executable_catalog_requires_live_provider_selection() {
        let mut record = skill_catalog().record;
        record.surfaces[0].kind = PluginSurfaceKind::Tool;

        let error = ensure_safe_live_slice(&record).unwrap_err();

        assert!(error
            .to_string()
            .contains("explicit live Runtime provider assignment"));
    }

    #[test]
    fn legacy_catalog_plan_remains_compatible_without_claiming_a_draft() {
        let raw = serde_json::json!({
            "dryRun": true,
            "planDigest": "a".repeat(64),
            "plans": [{
                "component": "use/acme/guide",
                "action": "install",
                "resolvedRegistryPackages": {}
            }]
        });

        let output = attach_draft(&request(), &installation(), 3, raw.clone()).unwrap();

        assert_eq!(output, raw);
        assert!(output.get("pluginOperationPlan").is_none());
    }

    fn request() -> PluginPlanRequest {
        PluginPlanRequest {
            action: PluginLifecycleAction::Install,
            component_id: "use/acme/guide".to_string(),
            version: Some("1.0.0".to_string()),
            channel: Some("stable".to_string()),
        }
    }

    fn installation() -> PluginInstallationSnapshot {
        PluginInstallationSnapshot {
            schema_version: 1,
            available: true,
            observed_at_ms: 1,
            generation: Some(9),
            revision: Some("f".repeat(64)),
            items: Vec::new(),
            error: None,
        }
    }

    fn umbrella_plan(
        catalog: &VerifiedPluginCatalogRecord,
        resolved: &ResolvedRemotePackage,
    ) -> Value {
        serde_json::json!({
            "dryRun": true,
            "planDigest": "a".repeat(64),
            "plans": [{
                "component": "use/acme/guide",
                "action": "install",
                "resolvedRegistryPackages": {
                    "use/acme/guide": resolved
                },
                "verifiedPluginCatalogRecords": {
                    "use/acme/guide": catalog
                }
            }]
        })
    }

    fn skill_catalog() -> VerifiedPluginCatalogRecord {
        let permissions = PluginPermissionCeiling {
            schema: PLUGIN_PERMISSION_SCHEMA.to_string(),
            surfaces: Vec::new(),
        };
        let record = PluginCatalogRecord {
            schema: PLUGIN_CATALOG_SCHEMA_V2.to_string(),
            package_id: "acme/guide".to_string(),
            display_name: "Guide".to_string(),
            description: "Workspace guidance.".to_string(),
            publisher: "acme".to_string(),
            keywords: vec!["guide".to_string()],
            categories: vec!["productivity".to_string()],
            version: "1.0.0".to_string(),
            channel: PluginReleaseChannel::Stable,
            requires_use: ">=0.2.1, <0.4.0".to_string(),
            target: "any".to_string(),
            surfaces: vec![CatalogSurface {
                kind: PluginSurfaceKind::Skill,
                id: "guide".to_string(),
                optional: false,
                workload: None,
                mcp_transport: None,
                mcp_tool_count: None,
                requires: Vec::new(),
            }],
            permission_ceiling_digest: permissions.descriptor_digest().unwrap(),
            permission_ceiling: permissions,
            archive: CatalogArchive {
                target_name: "extensions/acme/guide/1.0.0/stable/any/guide-1.0.0-any.tar.gz"
                    .to_string(),
                length: 123,
                sha256: format!("sha256:{}", "a".repeat(64)),
            },
            package: CatalogPackage {
                expanded_bytes: 456,
                file_count: 2,
                sha256: Some(format!("sha256:{}", "b".repeat(64))),
                manifest_sha256: Some(format!("sha256:{}", "c".repeat(64))),
            },
            license: "Apache-2.0".to_string(),
            repository: "https://github.com/acme/guide".to_string(),
            availability: CatalogAvailability::Available,
        };
        VerifiedPluginCatalogRecord::new(
            record.clone(),
            VerifiedCatalogProvenance {
                registry_name: "fixture".to_string(),
                registry_url: "http://127.0.0.1:43111/".to_string(),
                root_sha256: format!("sha256:{}", "d".repeat(64)),
                root_version: 1,
                timestamp_version: 2,
                snapshot_version: 2,
                targets_version: 2,
                catalog_record_digest: record.descriptor_digest().unwrap(),
            },
        )
        .unwrap()
    }
}
