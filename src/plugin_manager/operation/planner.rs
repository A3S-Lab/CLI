use a3s_use_core::{
    InstalledPluginPlanEvidence, PlanPackageRole, PlannedOperationImpact, PlannedStateEvidence,
    PluginCatalogRecord, PluginOperationAction, PluginOperationPlanDraft, PluginPlanningBundle,
    PluginSurfaceKind, VerifiedPluginCatalogRecord, PLUGIN_CATALOG_SCHEMA_V2,
    PLUGIN_CATALOG_SCHEMA_V3,
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
    installed: Option<&InstalledPluginPlanEvidence>,
    state_revision: u64,
    raw_plan: Value,
) -> PluginManagerResult<Value> {
    if raw_plan.get(OPERATION_PLAN_FIELD).is_some() {
        return Ok(raw_plan);
    }
    validate_state_revision(state_revision)?;
    match request.action {
        PluginLifecycleAction::Install => {
            attach_install_draft(request, installation, state_revision, raw_plan)
        }
        PluginLifecycleAction::Upgrade => {
            attach_upgrade_draft(request, installation, installed, state_revision, raw_plan)
        }
        PluginLifecycleAction::Uninstall => {
            attach_uninstall_draft(request, installation, installed, state_revision, raw_plan)
        }
    }
}

fn attach_install_draft(
    request: &PluginPlanRequest,
    installation: &PluginInstallationSnapshot,
    state_revision: u64,
    raw_plan: Value,
) -> PluginManagerResult<Value> {
    if !has_verified_catalog(&raw_plan) {
        return Ok(raw_plan);
    }
    let plan = single_component_plan(&raw_plan, request)?;
    ensure_component_mutates(plan)?;
    let catalog = verified_candidate_catalog(plan, request)?.ok_or_else(|| {
        planner_error("catalog-v2 candidate evidence disappeared during install planning")
    })?;
    ensure_safe_live_slice(&catalog.record)?;
    if installation
        .items
        .iter()
        .any(|item| item.package_id == catalog.record.package_id)
    {
        return Err(planner_error(
            "the requested package is already installed; resolve an upgrade instead",
        ));
    }
    let selected_surfaces = all_surfaces(&catalog);
    let transition = catalog
        .install_transition(PlanPackageRole::Root, &selected_surfaces)
        .map_err(|error| planner_error(error.to_string()))?;
    let draft = build_draft(
        request,
        PluginOperationAction::Install,
        catalog.record.package_id.clone(),
        vec![transition],
        PlannedOperationImpact {
            download_bytes: catalog.record.archive.length,
            installed_bytes_after: catalog.record.package.expanded_bytes,
            reclaimed_bytes: 0,
            drain_required: false,
            retained_data: false,
        },
        PlannedStateEvidence {
            state_revision,
            capability_generation: capability_generation(installation)?,
            receipt_digest: None,
        },
    )?;
    insert_draft(raw_plan, draft)
}

fn attach_upgrade_draft(
    request: &PluginPlanRequest,
    installation: &PluginInstallationSnapshot,
    installed: Option<&InstalledPluginPlanEvidence>,
    state_revision: u64,
    raw_plan: Value,
) -> PluginManagerResult<Value> {
    if !has_verified_catalog(&raw_plan) {
        return Ok(raw_plan);
    }
    let installed = installed.ok_or_else(|| {
        planner_error(
            "catalog-v2 upgrade candidate requires package-specific installed planning evidence",
        )
    })?;
    let plan = single_component_plan(&raw_plan, request)?;
    if plan.get("mutates").and_then(Value::as_bool) == Some(false) {
        return Ok(raw_plan);
    }
    ensure_component_mutates(plan)?;
    validate_installed_evidence(request, installation, installed)?;
    validate_component_current(plan, installed)?;
    ensure_safe_live_slice(&installed.verified_catalog.record)?;
    let candidate = verified_candidate_catalog(plan, request)?.ok_or_else(|| {
        planner_error("catalog-v2 candidate evidence disappeared during upgrade planning")
    })?;
    ensure_safe_live_slice(&candidate.record)?;
    let selected_surfaces = all_surfaces(&candidate);
    let transition = candidate
        .replace_transition(
            &installed.verified_catalog,
            PlanPackageRole::Root,
            &installed.selected_surfaces,
            &selected_surfaces,
        )
        .map_err(|error| planner_error(error.to_string()))?;
    let draft = build_draft(
        request,
        PluginOperationAction::Upgrade,
        candidate.record.package_id.clone(),
        vec![transition],
        PlannedOperationImpact {
            download_bytes: candidate.record.archive.length,
            installed_bytes_after: candidate.record.package.expanded_bytes,
            reclaimed_bytes: installed.verified_catalog.record.package.expanded_bytes,
            drain_required: false,
            retained_data: false,
        },
        PlannedStateEvidence {
            state_revision,
            capability_generation: installed.capability_generation,
            receipt_digest: Some(installed.receipt_digest.clone()),
        },
    )?;
    insert_draft(raw_plan, draft)
}

fn attach_uninstall_draft(
    request: &PluginPlanRequest,
    installation: &PluginInstallationSnapshot,
    installed: Option<&InstalledPluginPlanEvidence>,
    state_revision: u64,
    raw_plan: Value,
) -> PluginManagerResult<Value> {
    let Some(installed) = installed else {
        return Ok(raw_plan);
    };
    let plan = single_component_plan(&raw_plan, request)?;
    ensure_component_mutates(plan)?;
    validate_installed_evidence(request, installation, installed)?;
    validate_component_current(plan, installed)?;
    ensure_safe_live_slice(&installed.verified_catalog.record)?;
    let transition = installed
        .verified_catalog
        .remove_transition(PlanPackageRole::Root, &installed.selected_surfaces)
        .map_err(|error| planner_error(error.to_string()))?;
    let draft = build_draft(
        request,
        PluginOperationAction::Uninstall,
        installed.package_id.clone(),
        vec![transition],
        PlannedOperationImpact {
            download_bytes: 0,
            installed_bytes_after: 0,
            reclaimed_bytes: installed.verified_catalog.record.package.expanded_bytes,
            drain_required: false,
            retained_data: true,
        },
        PlannedStateEvidence {
            state_revision,
            capability_generation: installed.capability_generation,
            receipt_digest: Some(installed.receipt_digest.clone()),
        },
    )?;
    insert_draft(raw_plan, draft)
}

fn build_draft(
    request: &PluginPlanRequest,
    action: PluginOperationAction,
    package_id: String,
    transitions: Vec<a3s_use_core::PlannedPackageTransition>,
    impact: PlannedOperationImpact,
    state: PlannedStateEvidence,
) -> PluginManagerResult<PluginOperationPlanDraft> {
    PluginOperationPlanDraft::new(
        action,
        package_id,
        request.component_id.clone(),
        transitions,
        Vec::new(),
        Vec::new(),
        impact,
        state,
    )
    .map_err(|error| planner_error(error.to_string()))
}

fn insert_draft(
    mut raw_plan: Value,
    draft: PluginOperationPlanDraft,
) -> PluginManagerResult<Value> {
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

fn has_verified_catalog(raw_plan: &Value) -> bool {
    raw_plan
        .get("plans")
        .and_then(Value::as_array)
        .is_some_and(|plans| {
            plans.iter().any(|plan| {
                plan.get("verifiedPluginCatalogRecords")
                    .and_then(Value::as_object)
                    .is_some_and(|records| !records.is_empty())
            })
        })
}

fn verified_candidate_catalog(
    plan: &serde_json::Map<String, Value>,
    request: &PluginPlanRequest,
) -> PluginManagerResult<Option<VerifiedPluginCatalogRecord>> {
    let Some(records) = plan
        .get("verifiedPluginCatalogRecords")
        .and_then(Value::as_object)
    else {
        return Ok(None);
    };
    if records.is_empty() {
        return Ok(None);
    }
    let catalog_value = records.get(&request.component_id).ok_or_else(|| {
        planner_error("verified catalog records do not contain the requested component")
    })?;
    let catalog: VerifiedPluginCatalogRecord = serde_json::from_value(catalog_value.clone())
        .map_err(|error| {
            planner_error(format!(
                "verified plugin catalog evidence is invalid: {error}"
            ))
        })?;
    catalog
        .validate()
        .map_err(|error| planner_error(error.to_string()))?;
    if !matches!(
        catalog.record.schema.as_str(),
        PLUGIN_CATALOG_SCHEMA_V2 | PLUGIN_CATALOG_SCHEMA_V3
    ) {
        return Err(planner_error(
            "only catalog-v2 or catalog-v3 packages can produce a complete lifecycle draft",
        ));
    }
    let package_id = request
        .component_id
        .strip_prefix("use/")
        .unwrap_or_default();
    if catalog.record.package_id != package_id {
        return Err(planner_error(
            "verified catalog package identity does not match the plugin request",
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
    validate_planning_bundle(plan, request, &catalog)?;
    Ok(Some(catalog))
}

fn validate_planning_bundle(
    plan: &serde_json::Map<String, Value>,
    request: &PluginPlanRequest,
    catalog: &VerifiedPluginCatalogRecord,
) -> PluginManagerResult<()> {
    let bundles = plan
        .get("verifiedPluginPlanningBundles")
        .and_then(Value::as_object);
    if catalog.record.schema == PLUGIN_CATALOG_SCHEMA_V2 {
        if bundles.is_some_and(|bundles| bundles.contains_key(&request.component_id)) {
            return Err(planner_error(
                "catalog-v2 evidence must not acquire an executable planning bundle",
            ));
        }
        return Ok(());
    }

    let bundle_value = bundles
        .and_then(|bundles| bundles.get(&request.component_id))
        .ok_or_else(|| {
            planner_error("catalog-v3 evidence omitted its verified executable planning bundle")
        })?;
    let bundle: PluginPlanningBundle = serde_json::from_value(bundle_value.clone())
        .map_err(|error| planner_error(format!("plugin planning bundle is invalid: {error}")))?;
    bundle
        .validate_catalog_binding(catalog)
        .map_err(|error| planner_error(error.to_string()))
}

fn validate_installed_evidence(
    request: &PluginPlanRequest,
    installation: &PluginInstallationSnapshot,
    evidence: &InstalledPluginPlanEvidence,
) -> PluginManagerResult<()> {
    evidence
        .validate()
        .map_err(|error| planner_error(error.to_string()))?;
    let expected_package_id = request
        .component_id
        .strip_prefix("use/")
        .unwrap_or_default();
    if evidence.component_id != request.component_id
        || evidence.package_id != expected_package_id
        || !installation.available
        || installation.generation != Some(evidence.capability_generation)
        || installation.revision.as_deref() != Some(evidence.capability_revision.as_str())
    {
        return Err(planner_error(
            "installed planning evidence does not match the requested capability snapshot",
        ));
    }
    let item = installation
        .items
        .iter()
        .find(|item| item.component_id == request.component_id)
        .ok_or_else(|| {
            planner_error("installed planning evidence has no matching capability package")
        })?;
    let summary = item
        .planner_evidence
        .as_ref()
        .ok_or_else(|| planner_error("installed package omitted compact planner evidence"))?;
    let catalog = &evidence.verified_catalog;
    if item.package_id != evidence.package_id
        || item.version != evidence.version
        || item.enabled != evidence.desired_enabled
        || summary.package_id != evidence.package_id
        || catalog.record.package.sha256.as_deref() != Some(summary.package_sha256.as_str())
        || catalog.record.package.manifest_sha256.as_deref()
            != Some(summary.manifest_sha256.as_str())
        || summary.receipt_digest != evidence.receipt_digest
        || summary.catalog_record_digest != catalog.provenance.catalog_record_digest
        || summary.desired_enabled != evidence.desired_enabled
        || summary.selected_surfaces != evidence.selected_surfaces
    {
        return Err(planner_error(
            "package-specific installed evidence drifted from the compact capability evidence",
        ));
    }
    Ok(())
}

fn ensure_component_mutates(plan: &serde_json::Map<String, Value>) -> PluginManagerResult<()> {
    if plan.get("mutates").and_then(Value::as_bool) != Some(true) {
        return Err(planner_error(
            "umbrella component plan does not contain an exact mutation",
        ));
    }
    Ok(())
}

fn validate_component_current(
    plan: &serde_json::Map<String, Value>,
    evidence: &InstalledPluginPlanEvidence,
) -> PluginManagerResult<()> {
    let current = plan
        .get("current")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            planner_error("umbrella component plan omitted installed current-state evidence")
        })?;
    if current.get("version").and_then(Value::as_str) != Some(evidence.version.as_str()) {
        return Err(planner_error(
            "umbrella component current state drifted from installed planning evidence",
        ));
    }
    if let Some(receipt) = current.get("receipt").and_then(Value::as_object) {
        if receipt.get("componentId").and_then(Value::as_str)
            != Some(evidence.component_id.as_str())
            || receipt.get("version").and_then(Value::as_str) != Some(evidence.version.as_str())
        {
            return Err(planner_error(
                "umbrella component receipt drifted from installed planning evidence",
            ));
        }
    }
    Ok(())
}

fn capability_generation(installation: &PluginInstallationSnapshot) -> PluginManagerResult<u64> {
    if !installation.available || installation.revision.is_none() {
        return Err(planner_error(
            "A3S Use installation state is unavailable for complete planning",
        ));
    }
    installation
        .generation
        .filter(|generation| *generation > 0)
        .ok_or_else(|| planner_error("capability generation evidence is unavailable"))
}

fn validate_state_revision(state_revision: u64) -> PluginManagerResult<()> {
    if state_revision == 0 {
        return Err(planner_error(
            "durable plugin planner state revision is unavailable",
        ));
    }
    Ok(())
}

fn all_surfaces(catalog: &VerifiedPluginCatalogRecord) -> Vec<a3s_use_core::PluginSurfaceRef> {
    let mut surfaces = catalog
        .record
        .surfaces
        .iter()
        .map(|surface| surface.reference())
        .collect::<Vec<_>>();
    surfaces.sort();
    surfaces
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
        || plan.get("action").and_then(Value::as_str) != Some(request.action.as_str())
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
        "complete plugin lifecycle draft is unavailable: {}",
        message.into()
    ))
}

#[cfg(test)]
mod tests {
    use a3s_use_core::{
        CatalogArchive, CatalogAvailability, CatalogPackage, CatalogPlanningTarget, CatalogSurface,
        PluginCatalogRecord, PluginOperationPlanDraft, PluginPermissionCeiling,
        PluginReleaseChannel, PluginSurfaceKind, VerifiedCatalogProvenance,
        VerifiedPluginCatalogRecord, PLUGIN_CATALOG_SCHEMA_V2, PLUGIN_CATALOG_SCHEMA_V3,
        PLUGIN_PERMISSION_SCHEMA,
    };

    use crate::plugin_manager::capability::{
        PluginInstalledPackage, PluginPackageReadiness, PluginPlannerEvidence,
    };

    use super::*;

    #[test]
    fn plan_ready_skill_install_emits_a_complete_live_draft() {
        let catalog = skill_catalog();
        let resolved = ResolvedRemotePackage::from_verified_catalog(&catalog).unwrap();
        let raw = umbrella_plan(&catalog, &resolved);

        let output = attach_draft(&request(), &installation(), None, 3, raw).unwrap();
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
            None,
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
    fn catalog_v3_requires_its_verified_planning_bundle() {
        let mut record = skill_catalog().record;
        record.schema = PLUGIN_CATALOG_SCHEMA_V3.to_string();
        record.planning = Some(CatalogPlanningTarget {
            target_name: "extensions/acme/guide/1.0.0/stable/any/planning-v1.json".to_string(),
            length: 123,
            sha256: format!("sha256:{}", "9".repeat(64)),
        });
        let catalog = VerifiedPluginCatalogRecord::new(
            record.clone(),
            VerifiedCatalogProvenance {
                catalog_record_digest: record.descriptor_digest().unwrap(),
                ..skill_catalog().provenance
            },
        )
        .unwrap();
        let resolved = ResolvedRemotePackage::from_verified_catalog(&catalog).unwrap();

        let error = attach_draft(
            &request(),
            &installation(),
            None,
            3,
            umbrella_plan(&catalog, &resolved),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("omitted its verified executable planning bundle"));
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

        let output = attach_draft(&request(), &installation(), None, 3, raw.clone()).unwrap();

        assert_eq!(output, raw);
        assert!(output.get("pluginOperationPlan").is_none());
    }

    #[test]
    fn plan_ready_skill_upgrade_joins_exact_installed_evidence() {
        let installed_catalog = skill_catalog();
        let candidate = upgraded_catalog();
        let resolved = ResolvedRemotePackage::from_verified_catalog(&candidate).unwrap();
        let evidence = installed_evidence(installed_catalog);
        let request = lifecycle_request(PluginLifecycleAction::Upgrade);

        let output = attach_draft(
            &request,
            &installation_with(&evidence),
            Some(&evidence),
            7,
            umbrella_plan_for(&candidate, &resolved, "upgrade"),
        )
        .unwrap();
        let draft: PluginOperationPlanDraft =
            serde_json::from_value(output["pluginOperationPlan"].clone()).unwrap();

        assert_eq!(draft.action, PluginOperationAction::Upgrade);
        assert_eq!(
            draft.packages[0].before.as_ref().unwrap().release.version,
            "1.0.0"
        );
        assert_eq!(
            draft.packages[0].after.as_ref().unwrap().release.version,
            "2.0.0"
        );
        assert_eq!(draft.state.receipt_digest, Some(evidence.receipt_digest));
        assert_eq!(draft.impact.reclaimed_bytes, 456);
    }

    #[test]
    fn catalog_v2_upgrade_cannot_fall_back_without_installed_evidence() {
        let candidate = upgraded_catalog();
        let resolved = ResolvedRemotePackage::from_verified_catalog(&candidate).unwrap();
        let request = lifecycle_request(PluginLifecycleAction::Upgrade);

        let error = attach_draft(
            &request,
            &installation(),
            None,
            7,
            umbrella_plan_for(&candidate, &resolved, "upgrade"),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("requires package-specific installed planning evidence"));
    }

    #[test]
    fn plan_ready_skill_uninstall_emits_exact_remove_draft() {
        let evidence = installed_evidence(skill_catalog());
        let request = lifecycle_request(PluginLifecycleAction::Uninstall);
        let raw = serde_json::json!({
            "dryRun": true,
            "planDigest": "a".repeat(64),
            "plans": [{
                "component": "use/acme/guide",
                "action": "uninstall",
                "mutates": true,
                "current": {
                    "version": "1.0.0"
                }
            }]
        });

        let output = attach_draft(
            &request,
            &installation_with(&evidence),
            Some(&evidence),
            8,
            raw,
        )
        .unwrap();
        let draft: PluginOperationPlanDraft =
            serde_json::from_value(output["pluginOperationPlan"].clone()).unwrap();

        assert_eq!(draft.action, PluginOperationAction::Uninstall);
        assert!(draft.packages[0].after.is_none());
        assert!(draft.packages[0].before.is_some());
        assert!(draft.impact.retained_data);
        assert_eq!(draft.impact.reclaimed_bytes, 456);
    }

    #[test]
    fn installed_evidence_must_match_the_compact_capability_snapshot() {
        let mut evidence = installed_evidence(skill_catalog());
        let installation = installation_with(&evidence);
        evidence.capability_revision = "0".repeat(64);
        let request = lifecycle_request(PluginLifecycleAction::Uninstall);
        let raw = serde_json::json!({
            "dryRun": true,
            "planDigest": "a".repeat(64),
            "plans": [{
                "component": "use/acme/guide",
                "action": "uninstall",
                "mutates": true,
                "current": {
                    "version": "1.0.0"
                }
            }]
        });

        let error = attach_draft(&request, &installation, Some(&evidence), 8, raw).unwrap_err();

        assert!(error
            .to_string()
            .contains("does not match the requested capability snapshot"));
    }

    fn request() -> PluginPlanRequest {
        PluginPlanRequest {
            action: PluginLifecycleAction::Install,
            component_id: "use/acme/guide".to_string(),
            version: Some("1.0.0".to_string()),
            channel: Some("stable".to_string()),
        }
    }

    fn lifecycle_request(action: PluginLifecycleAction) -> PluginPlanRequest {
        PluginPlanRequest {
            action,
            component_id: "use/acme/guide".to_string(),
            version: None,
            channel: None,
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
        umbrella_plan_for(catalog, resolved, "install")
    }

    fn umbrella_plan_for(
        catalog: &VerifiedPluginCatalogRecord,
        resolved: &ResolvedRemotePackage,
        action: &str,
    ) -> Value {
        serde_json::json!({
            "dryRun": true,
            "planDigest": "a".repeat(64),
            "plans": [{
                "component": "use/acme/guide",
                "action": action,
                "mutates": true,
                "current": {
                    "version": "1.0.0"
                },
                "resolvedRegistryPackages": {
                    "use/acme/guide": resolved
                },
                "verifiedPluginCatalogRecords": {
                    "use/acme/guide": catalog
                }
            }]
        })
    }

    fn installed_evidence(catalog: VerifiedPluginCatalogRecord) -> InstalledPluginPlanEvidence {
        InstalledPluginPlanEvidence {
            schema: a3s_use_core::INSTALLED_PLUGIN_PLAN_EVIDENCE_SCHEMA.to_string(),
            component_id: "use/acme/guide".to_string(),
            package_id: "acme/guide".to_string(),
            version: catalog.record.version.clone(),
            capability_generation: 9,
            capability_revision: "f".repeat(64),
            receipt_digest: format!("sha256:{}", "e".repeat(64)),
            desired_enabled: true,
            selected_surfaces: all_surfaces(&catalog),
            verified_catalog: catalog,
        }
    }

    fn installation_with(evidence: &InstalledPluginPlanEvidence) -> PluginInstallationSnapshot {
        let catalog = &evidence.verified_catalog;
        PluginInstallationSnapshot {
            schema_version: 1,
            available: true,
            observed_at_ms: 1,
            generation: Some(evidence.capability_generation),
            revision: Some(evidence.capability_revision.clone()),
            items: vec![PluginInstalledPackage {
                component_id: evidence.component_id.clone(),
                package_id: evidence.package_id.clone(),
                route: "guide".to_string(),
                version: evidence.version.clone(),
                enabled: evidence.desired_enabled,
                callable: true,
                readiness: PluginPackageReadiness::Ready,
                reconciliation: None,
                planner_evidence: Some(PluginPlannerEvidence {
                    schema_version: 1,
                    package_id: evidence.package_id.clone(),
                    package_sha256: catalog.record.package.sha256.clone().unwrap(),
                    manifest_sha256: catalog.record.package.manifest_sha256.clone().unwrap(),
                    receipt_digest: evidence.receipt_digest.clone(),
                    catalog_record_digest: catalog.provenance.catalog_record_digest.clone(),
                    desired_enabled: evidence.desired_enabled,
                    selected_surfaces: evidence.selected_surfaces.clone(),
                }),
            }],
            error: None,
        }
    }

    fn upgraded_catalog() -> VerifiedPluginCatalogRecord {
        let installed = skill_catalog();
        let mut record = installed.record;
        record.version = "2.0.0".to_string();
        record.archive.target_name =
            "extensions/acme/guide/2.0.0/stable/any/guide-2.0.0-any.tar.gz".to_string();
        record.archive.sha256 = format!("sha256:{}", "1".repeat(64));
        record.package.sha256 = Some(format!("sha256:{}", "2".repeat(64)));
        record.package.manifest_sha256 = Some(format!("sha256:{}", "3".repeat(64)));
        VerifiedPluginCatalogRecord::new(
            record.clone(),
            VerifiedCatalogProvenance {
                catalog_record_digest: record.descriptor_digest().unwrap(),
                ..installed.provenance
            },
        )
        .unwrap()
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
            planning: None,
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
