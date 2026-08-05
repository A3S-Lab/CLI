use a3s_use_core::{
    InstalledPluginPlanEvidence, PlanPackageChangeKind, PlanPackageRole, PlannedOperationImpact,
    PlannedPackageTransition, PlannedStateEvidence, PluginCatalogRecord, PluginOperationAction,
    PluginOperationPlanDraft, PluginPackageLock, PluginPlanningBundle, PluginSurfaceKind,
    VerifiedPluginCatalogRecord, PLUGIN_CATALOG_SCHEMA_V2, PLUGIN_CATALOG_SCHEMA_V3,
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
    let package_lock = verified_candidate_lock(plan, request, &catalog)?;
    if installation
        .items
        .iter()
        .any(|item| item.package_id == catalog.record.package_id)
    {
        return Err(planner_error(
            "the requested package is already installed; resolve an upgrade instead",
        ));
    }
    let (transitions, impact) = match package_lock.as_ref() {
        Some(package_lock) => graph_install_delta(package_lock, installation)?,
        None => {
            ensure_safe_live_slice(&catalog.record)?;
            let selected_surfaces = all_surfaces(&catalog);
            let transition = catalog
                .install_transition(PlanPackageRole::Root, &selected_surfaces)
                .map_err(|error| planner_error(error.to_string()))?;
            (
                vec![transition],
                PlannedOperationImpact {
                    download_bytes: catalog.record.archive.length,
                    installed_bytes_after: catalog.record.package.expanded_bytes,
                    reclaimed_bytes: 0,
                    drain_required: false,
                    retained_data: false,
                    okf_changes: Vec::new(),
                },
            )
        }
    };
    let mut draft = build_draft(
        request,
        PluginOperationAction::Install,
        catalog.record.package_id.clone(),
        transitions,
        impact,
        PlannedStateEvidence {
            state_revision,
            capability_generation: capability_generation(installation)?,
            receipt_digest: None,
        },
    )?;
    if let Some(package_lock) = package_lock {
        draft.package_lock_digest = Some(
            package_lock
                .descriptor_digest()
                .map_err(|error| planner_error(error.to_string()))?,
        );
        draft
            .validate()
            .map_err(|error| planner_error(error.to_string()))?;
    }
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
            okf_changes: Vec::new(),
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
            okf_changes: Vec::new(),
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

    if catalog.record.planning.is_none() {
        if bundles.is_some_and(|bundles| bundles.contains_key(&request.component_id)) {
            return Err(planner_error(
                "a static catalog-v3 package must not acquire an executable planning bundle",
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

fn verified_candidate_lock(
    plan: &serde_json::Map<String, Value>,
    request: &PluginPlanRequest,
    catalog: &VerifiedPluginCatalogRecord,
) -> PluginManagerResult<Option<PluginPackageLock>> {
    let locks = plan.get("cognitivePackageLocks").and_then(Value::as_object);
    if catalog.record.schema == PLUGIN_CATALOG_SCHEMA_V2 {
        if locks.is_some_and(|locks| !locks.is_empty()) {
            return Err(planner_error(
                "catalog-v2 evidence must not acquire a cognitive-package lock",
            ));
        }
        return Ok(None);
    }
    let locks = locks.ok_or_else(|| {
        planner_error("catalog-v3 evidence omitted its complete cognitive-package lock")
    })?;
    if locks.len() != 1 {
        return Err(planner_error(
            "catalog-v3 evidence must carry exactly one cognitive-package lock",
        ));
    }
    let value = locks.get(&request.component_id).ok_or_else(|| {
        planner_error("the cognitive-package lock does not match the requested component")
    })?;
    let package_lock: PluginPackageLock = serde_json::from_value(value.clone())
        .map_err(|error| planner_error(format!("cognitive-package lock is invalid: {error}")))?;
    package_lock
        .validate()
        .map_err(|error| planner_error(error.to_string()))?;
    let root = package_lock
        .package(&package_lock.root_package_id)
        .ok_or_else(|| planner_error("the cognitive-package lock omitted its root"))?;
    if package_lock.root_package_id != catalog.record.package_id || root.catalog != *catalog {
        return Err(planner_error(
            "the cognitive-package lock root does not match the verified catalog",
        ));
    }
    Ok(Some(package_lock))
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

fn graph_install_delta(
    package_lock: &PluginPackageLock,
    installation: &PluginInstallationSnapshot,
) -> PluginManagerResult<(Vec<PlannedPackageTransition>, PlannedOperationImpact)> {
    let mut transitions = Vec::with_capacity(package_lock.packages.len());
    let mut download_bytes = 0_u64;
    let mut installed_bytes_after = 0_u64;
    for package in &package_lock.packages {
        ensure_safe_live_slice(&package.catalog.record)?;
        let selected_surfaces = all_surfaces(&package.catalog);
        let role = if package.package_id() == package_lock.root_package_id {
            PlanPackageRole::Root
        } else {
            PlanPackageRole::Dependency
        };
        installed_bytes_after = installed_bytes_after
            .checked_add(package.catalog.record.package.expanded_bytes)
            .ok_or_else(|| planner_error("cognitive-package installed size overflowed"))?;
        let transition = match installation
            .items
            .iter()
            .find(|item| item.package_id == package.package_id())
        {
            None => {
                download_bytes = download_bytes
                    .checked_add(package.catalog.record.archive.length)
                    .ok_or_else(|| planner_error("cognitive-package download size overflowed"))?;
                package
                    .catalog
                    .install_transition(role, &selected_surfaces)
                    .map_err(|error| planner_error(error.to_string()))?
            }
            Some(installed) if installed_package_matches_lock(installed, package) => {
                let state = package
                    .catalog
                    .selected_state(&selected_surfaces)
                    .map_err(|error| planner_error(error.to_string()))?;
                PlannedPackageTransition::resolved(
                    package.package_id(),
                    role,
                    PlanPackageChangeKind::Retain,
                    Some(state.clone()),
                    Some(state),
                    None,
                )
                .map_err(|error| planner_error(error.to_string()))?
            }
            Some(_) => {
                return Err(planner_error(format!(
                    "installed dependency '{}' differs from the reviewed cognitive-package lock and requires an explicit upgrade",
                    package.package_id()
                )))
            }
        };
        transitions.push(transition);
    }
    Ok((
        transitions,
        PlannedOperationImpact {
            download_bytes,
            installed_bytes_after,
            reclaimed_bytes: 0,
            drain_required: false,
            retained_data: false,
            okf_changes: Vec::new(),
        },
    ))
}

fn installed_package_matches_lock(
    installed: &crate::plugin_manager::capability::PluginInstalledPackage,
    package: &a3s_use_core::LockedPluginPackage,
) -> bool {
    let catalog = &package.catalog;
    let Some(evidence) = installed.planner_evidence.as_ref() else {
        return false;
    };
    installed.version == catalog.record.version
        && installed.enabled
        && installed.callable
        && installed.readiness == crate::plugin_manager::capability::PluginPackageReadiness::Ready
        && evidence.package_id == package.package_id()
        && evidence.package_sha256 == catalog.record.package.sha256.clone().unwrap_or_default()
        && evidence.manifest_sha256
            == catalog
                .record
                .package
                .manifest_sha256
                .clone()
                .unwrap_or_default()
        && evidence.catalog_record_digest == catalog.provenance.catalog_record_digest
        && evidence.desired_enabled
        && evidence.selected_surfaces == all_surfaces(catalog)
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
        PluginReleaseChannel, PluginSurfaceKind, PluginSurfaceRef, ResourcePermissionCeiling,
        SurfacePermissionCeiling, ToolWorkloadClass, VerifiedCatalogProvenance,
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
        record.surfaces[0].kind = PluginSurfaceKind::Tool;
        record.surfaces[0].workload = Some(ToolWorkloadClass::Task);
        record.permission_ceiling.surfaces = vec![SurfacePermissionCeiling {
            surface: PluginSurfaceRef {
                kind: PluginSurfaceKind::Tool,
                id: "guide".to_string(),
            },
            native_execution: true,
            child_process: false,
            filesystem: Vec::new(),
            network_egress: Vec::new(),
            private_service: false,
            secrets: Vec::new(),
            resources: Some(ResourcePermissionCeiling {
                cpu_millis: 1,
                memory_bytes: 1,
                pids: 1,
                ephemeral_storage_bytes: 1,
                task_timeout_ms: Some(1),
                max_stdout_bytes: Some(1),
                max_stderr_bytes: Some(1),
            }),
            ui_http: Vec::new(),
        }];
        record.permission_ceiling_digest = record.permission_ceiling.descriptor_digest().unwrap();
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
    fn catalog_v3_static_package_does_not_require_a_planning_bundle() {
        let mut record = skill_catalog().record;
        record.schema = PLUGIN_CATALOG_SCHEMA_V3.to_string();
        let catalog = VerifiedPluginCatalogRecord::new(
            record.clone(),
            VerifiedCatalogProvenance {
                catalog_record_digest: record.descriptor_digest().unwrap(),
                ..skill_catalog().provenance
            },
        )
        .unwrap();
        let resolved = ResolvedRemotePackage::from_verified_catalog(&catalog).unwrap();

        let output = attach_draft(
            &request(),
            &installation(),
            None,
            3,
            umbrella_plan(&catalog, &resolved),
        )
        .unwrap();

        let draft: PluginOperationPlanDraft =
            serde_json::from_value(output["pluginOperationPlan"].clone()).unwrap();
        assert_eq!(
            draft.package_lock_digest,
            Some(single_package_lock(&catalog).descriptor_digest().unwrap())
        );
    }

    #[test]
    fn catalog_v3_dependency_graph_is_bound_into_the_live_draft() {
        let base = schema_v3_skill_catalog("acme/base", Vec::new(), '1');
        let root = schema_v3_skill_catalog(
            "acme/guide",
            vec![a3s_use_core::PluginPackageDependency::new("acme/base", "^1.0.0").unwrap()],
            '2',
        );
        let package_lock = PluginPackageLock {
            schema: a3s_use_core::PLUGIN_PACKAGE_LOCK_SCHEMA.to_string(),
            root_package_id: "acme/guide".to_string(),
            host: a3s_use_core::PluginPackageLockHost::new("linux-x86_64", "0.3.0").unwrap(),
            packages: vec![
                a3s_use_core::LockedPluginPackage {
                    catalog: base.clone(),
                    dependencies: Vec::new(),
                },
                a3s_use_core::LockedPluginPackage {
                    catalog: root.clone(),
                    dependencies: vec![a3s_use_core::LockedPluginPackageDependency {
                        package_id: "acme/base".to_string(),
                        version_requirement: "^1.0.0".to_string(),
                        version: "1.0.0".to_string(),
                    }],
                },
            ],
        };
        package_lock.validate().unwrap();
        let resolved = ResolvedRemotePackage::from_verified_catalog(&root).unwrap();
        let mut raw = umbrella_plan(&root, &resolved);
        raw["plans"][0]["cognitivePackageLocks"] = serde_json::json!({
            "use/acme/guide": package_lock.clone()
        });

        let output = attach_draft(&request(), &installation(), None, 3, raw).unwrap();
        let draft: PluginOperationPlanDraft =
            serde_json::from_value(output["pluginOperationPlan"].clone()).unwrap();

        assert_eq!(
            draft.package_lock_digest,
            Some(package_lock.descriptor_digest().unwrap())
        );
        assert_eq!(draft.packages.len(), 2);
        assert_eq!(draft.packages[0].package_id, "acme/base");
        assert_eq!(draft.packages[0].role, PlanPackageRole::Dependency);
        assert_eq!(draft.packages[0].change, PlanPackageChangeKind::Add);
        assert_eq!(draft.packages[1].package_id, "acme/guide");
        assert_eq!(draft.packages[1].role, PlanPackageRole::Root);
        assert_eq!(draft.packages[1].change, PlanPackageChangeKind::Add);

        let capability = crate::plugin_manager::capability::PluginCapabilityEvidence {
            status: crate::plugin_manager::capability::PluginCapabilityEvidenceStatus::Verified,
            observed_at_ms: 1,
            generation: Some(9),
            revision: Some("f".repeat(64)),
            error: None,
        };
        let prepared = super::super::plan_artifact::prepare(
            super::super::plan_artifact::HostPlanContext {
                authorization: &crate::plugin_manager::PluginAuthorizationPolicy::default(),
                actor: a3s_use_core::PlanActor::User,
                scope: &crate::plugin_manager::default_plan_scope(),
                observed: super::super::plan_artifact::ObservedPlanState {
                    capability: &capability,
                    state_revision: 3,
                },
                identity: &super::super::store::PluginPlanIdentity {
                    operation_id: "install:acme-guide:graph-fixture".to_string(),
                    created_at_ms: 10,
                    expires_at_ms: 20,
                },
            },
            &request(),
            "a".repeat(64),
            output,
        )
        .unwrap();
        let envelope = prepared.plugin_operation_plan.unwrap();
        assert_eq!(envelope.package_lock.as_ref(), Some(&package_lock));
        assert_eq!(
            envelope.plan.package_lock_digest,
            Some(package_lock.descriptor_digest().unwrap())
        );

        let mut retained_installation = installation();
        retained_installation.items.push(PluginInstalledPackage {
            component_id: "use/acme/base".to_string(),
            package_id: "acme/base".to_string(),
            route: "base".to_string(),
            version: base.record.version.clone(),
            enabled: true,
            callable: true,
            readiness: PluginPackageReadiness::Ready,
            lifecycle_generation: Some(7),
            reconciliation: None,
            planner_evidence: Some(PluginPlannerEvidence {
                schema_version: 1,
                package_id: "acme/base".to_string(),
                package_sha256: base.record.package.sha256.clone().unwrap(),
                manifest_sha256: base.record.package.manifest_sha256.clone().unwrap(),
                receipt_digest: format!("sha256:{}", "4".repeat(64)),
                catalog_record_digest: base.provenance.catalog_record_digest.clone(),
                desired_enabled: true,
                selected_surfaces: all_surfaces(&base),
            }),
        });
        let mut retained_raw = umbrella_plan(&root, &resolved);
        retained_raw["plans"][0]["cognitivePackageLocks"] = serde_json::json!({
            "use/acme/guide": package_lock.clone()
        });
        let retained =
            attach_draft(&request(), &retained_installation, None, 3, retained_raw).unwrap();
        let retained: PluginOperationPlanDraft =
            serde_json::from_value(retained["pluginOperationPlan"].clone()).unwrap();
        assert_eq!(retained.packages[0].change, PlanPackageChangeKind::Retain);
        assert_eq!(retained.packages[1].change, PlanPackageChangeKind::Add);
        assert_eq!(retained.impact.download_bytes, root.record.archive.length);
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
        let mut plan = serde_json::json!({
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
        });
        if catalog.record.schema == PLUGIN_CATALOG_SCHEMA_V3
            && catalog.record.dependencies.is_empty()
        {
            plan["plans"][0]["cognitivePackageLocks"] = serde_json::json!({
                "use/acme/guide": single_package_lock(catalog)
            });
        }
        plan
    }

    fn single_package_lock(catalog: &VerifiedPluginCatalogRecord) -> PluginPackageLock {
        let package_lock = PluginPackageLock {
            schema: a3s_use_core::PLUGIN_PACKAGE_LOCK_SCHEMA.to_string(),
            root_package_id: catalog.record.package_id.clone(),
            host: a3s_use_core::PluginPackageLockHost::new("linux-x86_64", "0.3.0").unwrap(),
            packages: vec![a3s_use_core::LockedPluginPackage {
                catalog: catalog.clone(),
                dependencies: Vec::new(),
            }],
        };
        package_lock.validate().unwrap();
        package_lock
    }

    fn schema_v3_skill_catalog(
        package_id: &str,
        dependencies: Vec<a3s_use_core::PluginPackageDependency>,
        digest_character: char,
    ) -> VerifiedPluginCatalogRecord {
        let fixture = skill_catalog();
        let mut record = fixture.record;
        record.schema = PLUGIN_CATALOG_SCHEMA_V3.to_string();
        record.package_id = package_id.to_string();
        record.display_name = package_id.to_string();
        record.description = format!("Static fixture for {package_id}.");
        record.dependencies = dependencies;
        record.archive.target_name = format!(
            "extensions/{package_id}/1.0.0/stable/any/{}-1.0.0-any.tar.gz",
            package_id.replace('/', "-")
        );
        record.archive.sha256 = format!("sha256:{}", digest_character.to_string().repeat(64));
        record.package.sha256 = Some(format!(
            "sha256:{}",
            digest_character.to_ascii_uppercase().to_string().repeat(64)
        ));
        let provenance = VerifiedCatalogProvenance {
            catalog_record_digest: record.descriptor_digest().unwrap(),
            ..fixture.provenance
        };
        VerifiedPluginCatalogRecord::new(record, provenance).unwrap()
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
                lifecycle_generation: Some(7),
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
            dependencies: Vec::new(),
            target: "any".to_string(),
            surfaces: vec![CatalogSurface {
                kind: PluginSurfaceKind::Skill,
                id: "guide".to_string(),
                optional: false,
                workload: None,
                mcp_transport: None,
                mcp_tool_count: None,
                okf_bundle: None,
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
