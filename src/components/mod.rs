//! Typed A3S product component catalog and lifecycle management.

mod catalog;
mod command;
mod discovery;
mod id;
mod lifecycle;
mod paths;
mod probe;
mod release_install;
mod state;

pub use command::{
    resolve_or_install, resolve_or_install_with, run_doctor, run_doctor_with, run_info,
    run_info_with, run_install, run_install_with, run_list, run_list_with, run_proxy,
    run_uninstall, run_uninstall_with, run_update, run_update_with, ComponentBatchFailure,
};
pub use id::ComponentId;
pub use paths::ComponentPaths;

fn progress(enabled: bool, message: impl std::fmt::Display) {
    if enabled {
        eprintln!("{message}");
    }
}

#[cfg(test)]
mod tests;
