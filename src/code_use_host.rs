//! Shared host policy for resolving the independently released A3S Use runtime.
//!
//! This module owns only component discovery and first-use installation policy.
//! Package graphs, lifecycle generations, and capability snapshots remain
//! authoritative in `a3s-use`.

use std::path::PathBuf;

use crate::cli::context::InvocationContext;

pub(crate) struct CodeUseResolution {
    pub(crate) executable: Option<PathBuf>,
    pub(crate) warning: Option<String>,
}

pub(crate) fn discover_ready(context: &InvocationContext) -> anyhow::Result<Option<PathBuf>> {
    a3s::components::find_ready_executable_with("use", &context.component_paths)
}

pub(crate) async fn resolve(
    context: &InvocationContext,
    install_if_missing: bool,
) -> CodeUseResolution {
    let allow_first_use_install = install_if_missing && context.network.allow_first_use_install;
    resolve_with(
        allow_first_use_install,
        context.network.offline,
        || discover_ready(context),
        || {
            a3s::components::resolve_or_install_with(
                "use",
                &context.component_paths,
                allow_first_use_install,
                false,
            )
        },
    )
    .await
}

pub(crate) async fn resolve_with<D, F, Fut>(
    allow_first_use_install: bool,
    offline: bool,
    discover: D,
    install: F,
) -> CodeUseResolution
where
    D: FnOnce() -> anyhow::Result<Option<PathBuf>>,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<PathBuf>>,
{
    match discover() {
        Ok(Some(executable)) => CodeUseResolution {
            executable: Some(executable),
            warning: None,
        },
        Ok(None) if allow_first_use_install => match install().await {
            Ok(executable) => CodeUseResolution {
                executable: Some(executable),
                warning: None,
            },
            Err(error) => CodeUseResolution {
                executable: None,
                warning: Some(format!(
                    "A3S Use first-use setup failed: {error}. Run /use repair, or use `a3s doctor use` and `a3s install use` for recovery"
                )),
            },
        },
        Ok(None) => CodeUseResolution {
            executable: None,
            warning: Some(if offline {
                "A3S Use is not ready and first-use setup is disabled in offline mode; run /use repair after going online, or use `a3s install use`"
                    .to_string()
            } else {
                "A3S Use is not ready and first-use setup is disabled by A3S_NO_AUTO_INSTALL; run /use repair, or use `a3s install use` for explicit setup"
                    .to_string()
            }),
        },
        Err(error) => CodeUseResolution {
            executable: None,
            warning: Some(format!(
                "A3S Use discovery failed: {error}. Run /use repair, or use `a3s doctor use` for recovery"
            )),
        },
    }
}
