//! Typed A3S product component catalog and lifecycle management.

mod catalog;
mod cognitive_lifecycle;
mod command;
mod discovery;
mod id;
mod journal;
mod lifecycle;
mod lock;
mod paths;
mod plan;
mod probe;
mod release_install;
mod reviewed_cognitive;
mod state;

pub(crate) use cognitive_lifecycle::{
    code_cognitive_package_manager, code_cognitive_package_manager_with_authorization,
};
pub use command::{
    component_health_report, find_ready_executable_with, resolve_or_install,
    resolve_or_install_with, run_doctor, run_doctor_with, run_info, run_info_with, run_install,
    run_install_with, run_install_with_registries, run_list, run_list_with,
    run_list_with_registries, run_proxy, run_uninstall, run_uninstall_with, run_update,
    run_update_with, run_update_with_registries, run_upgrade_list_with,
    run_upgrade_list_with_registries, ComponentBatchFailure, ComponentHealthCheck,
    ComponentHealthReport, ComponentHealthStatus,
};
pub use id::ComponentId;
pub use paths::ComponentPaths;
pub use plan::ComponentPlanMismatch;
pub use probe::{webview_binary_supports_agent_island, webview_supports_agent_island_output};
pub(crate) use reviewed_cognitive::{
    apply_reviewed_cognitive_enablement, apply_reviewed_cognitive_package,
};

fn progress(enabled: bool, message: impl std::fmt::Display) {
    if enabled {
        eprintln!("{message}");
    }
}

#[cfg(test)]
mod tests;
