use a3s_use_core::{
    PluginDesiredState, PluginHostObservationStatus, PluginHostPackageState,
    PluginHostUnavailableReason, PluginObservedState, PluginPackageId, UseError, UseResult,
};

use crate::plugin_manager::{PluginInstallationSnapshot, PluginInstalledPackage};

pub(super) fn observation_status(
    snapshot: &PluginInstallationSnapshot,
    package_id: &PluginPackageId,
) -> UseResult<PluginHostObservationStatus> {
    if !snapshot.available {
        return Ok(PluginHostObservationStatus::Unavailable {
            reason: PluginHostUnavailableReason::ManagerRecovering,
        });
    }
    let capability_generation = snapshot.generation.ok_or_else(state_unstable)?;
    let capability_revision =
        prefixed_digest(snapshot.revision.as_deref().ok_or_else(state_unstable)?)?;
    let Some(package) = snapshot
        .items
        .iter()
        .find(|item| item.package_id == package_id.as_str())
    else {
        let state = PluginHostPackageState {
            version: None,
            package_generation: None,
            package_digest: None,
            manifest_digest: None,
            receipt_digest: None,
            capability_generation,
            capability_revision,
            desired: PluginDesiredState::Absent,
            observed: PluginObservedState::Removed,
            selected_surfaces: Vec::new(),
        };
        state.validate()?;
        return Ok(PluginHostObservationStatus::Available { state });
    };
    match installed_state(package, capability_generation, capability_revision) {
        Ok(state) => Ok(PluginHostObservationStatus::Available { state }),
        Err(_) => Ok(PluginHostObservationStatus::Unavailable {
            reason: PluginHostUnavailableReason::StateUnstable,
        }),
    }
}

fn installed_state(
    package: &PluginInstalledPackage,
    capability_generation: u64,
    capability_revision: String,
) -> UseResult<PluginHostPackageState> {
    let evidence = package
        .planner_evidence
        .as_ref()
        .ok_or_else(state_unstable)?;
    let package_generation = package.lifecycle_generation.ok_or_else(state_unstable)?;
    let observed = package
        .reconciliation
        .as_ref()
        .and_then(|value| value.get("observed"))
        .and_then(serde_json::Value::as_str)
        .and_then(parse_observed)
        .ok_or_else(state_unstable)?;
    let desired = if package.enabled {
        PluginDesiredState::Enabled
    } else {
        PluginDesiredState::InstalledDisabled
    };
    let state = PluginHostPackageState {
        version: Some(package.version.clone()),
        package_generation: Some(package_generation),
        package_digest: Some(prefixed_digest(&evidence.package_sha256)?),
        manifest_digest: Some(prefixed_digest(&evidence.manifest_sha256)?),
        receipt_digest: Some(prefixed_digest(&evidence.receipt_digest)?),
        capability_generation,
        capability_revision,
        desired,
        observed,
        selected_surfaces: evidence.selected_surfaces.clone(),
    };
    state.validate()?;
    Ok(state)
}

fn parse_observed(value: &str) -> Option<PluginObservedState> {
    match value {
        "installed" => Some(PluginObservedState::Installed),
        "reconciling" => Some(PluginObservedState::Reconciling),
        "ready" => Some(PluginObservedState::Ready),
        "degraded" => Some(PluginObservedState::Degraded),
        "broken" => Some(PluginObservedState::Broken),
        "incompatible" => Some(PluginObservedState::Incompatible),
        "draining" => Some(PluginObservedState::Draining),
        "removed" => Some(PluginObservedState::Removed),
        _ => None,
    }
}

fn prefixed_digest(value: &str) -> UseResult<String> {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(state_unstable());
    }
    Ok(format!("sha256:{value}"))
}

fn state_unstable() -> UseError {
    UseError::new(
        "use.plugin.host_state_unstable",
        "The managed plugin package state is not backed by one complete stable capability snapshot.",
    )
}
