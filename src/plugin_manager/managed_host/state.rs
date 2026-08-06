use std::time::{SystemTime, UNIX_EPOCH};

use a3s_use_core::{
    PlanScope, PluginHostObservationStatus, PluginHostUnavailableReason, PluginPackageId, UseError,
    UseResult,
};

use crate::components::code_cognitive_package_manager;
use crate::plugin_manager::PluginManager;

pub(super) async fn observation_status(
    manager: &PluginManager,
    scope: PlanScope,
    package_id: &PluginPackageId,
) -> UseResult<PluginHostObservationStatus> {
    let package_manager = code_cognitive_package_manager(&manager.component_paths, scope)?;
    Ok(
        match package_manager.observe_package(package_id.as_str()).await {
            Ok(state) => PluginHostObservationStatus::Available { state },
            Err(_) => PluginHostObservationStatus::Unavailable {
                reason: PluginHostUnavailableReason::StateUnstable,
            },
        },
    )
}

pub(super) fn now_ms() -> UseResult<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            UseError::new(
                "use.plugin.host_clock_invalid",
                "The managed Plugin Manager clock is before the Unix epoch.",
            )
        })?
        .as_millis();
    u64::try_from(millis).map_err(|_| {
        UseError::new(
            "use.plugin.host_clock_invalid",
            "The managed Plugin Manager clock cannot be represented in milliseconds.",
        )
    })
}
