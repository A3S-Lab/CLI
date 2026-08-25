//! One-shot A3S Use capability runtime preparation.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use a3s::plugin_manager::{PluginManager, PluginManagerPolicy};
use serde_json::json;

use crate::cli::context::InvocationContext;
use crate::cli::output::{CliError, ExitClass};

const READY_BUDGET: Duration = Duration::from_secs(30);
const INSTALLED_ONLY_READY_BUDGET: Duration = Duration::from_secs(3);
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PreparationPolicy {
    InstalledOnly,
    Required,
}

pub(super) struct Preparation {
    pub(super) evidence: Option<crate::use_registry::ScopedCapabilityRuntimeEvidence>,
    pub(super) warning: Option<String>,
    runtime_tasks: ScopedRuntimeTaskHost,
}

impl Preparation {
    fn absent() -> Self {
        Self {
            evidence: None,
            warning: None,
            runtime_tasks: ScopedRuntimeTaskHost::default(),
        }
    }

    fn skipped(warning: impl Into<String>) -> Self {
        Self {
            evidence: None,
            warning: Some(warning.into()),
            runtime_tasks: ScopedRuntimeTaskHost::default(),
        }
    }

    fn ready(
        evidence: crate::use_registry::ScopedCapabilityRuntimeEvidence,
        warning: Option<String>,
        runtime_tasks: ScopedRuntimeTaskHost,
    ) -> Self {
        Self {
            evidence: Some(evidence),
            warning,
            runtime_tasks,
        }
    }

    pub(super) async fn shutdown(&mut self) {
        self.runtime_tasks.shutdown().await;
    }
}

#[derive(Default)]
struct ScopedRuntimeTaskHost {
    manager: Option<Arc<PluginManager>>,
}

impl ScopedRuntimeTaskHost {
    async fn compose(context: &InvocationContext, config_path: &Path) -> (Self, Option<String>) {
        let authorization = match crate::commands::plugin::load_host_authorization_context(context)
            .await
        {
            Ok(authorization) => authorization,
            Err(error) => {
                return (
                    Self::default(),
                    Some(format!(
                        "managed Runtime Tasks are unavailable because the host plugin authorization policy could not be loaded: {error:#}"
                    )),
                )
            }
        };
        match PluginManager::from_host_with_policy_and_runtime_config(
            config_path,
            &context.directory,
            authorization.handoff().source(),
            PluginManagerPolicy {
                offline: context.network.offline,
                authorization: authorization.policy().clone(),
            },
        )
        .await
        {
            Ok(manager) => (
                Self {
                    manager: Some(Arc::new(manager)),
                },
                None,
            ),
            Err(error) => (
                Self::default(),
                Some(format!(
                    "managed Runtime Tasks are unavailable because the Code Plugin Manager host could not be initialized: {error}"
                )),
            ),
        }
    }

    fn invoker(&self) -> Option<Arc<dyn crate::use_registry::RuntimeTaskInvoker>> {
        self.manager
            .as_ref()
            .map(|manager| Arc::clone(manager) as Arc<dyn crate::use_registry::RuntimeTaskInvoker>)
    }

    async fn shutdown(&mut self) {
        if let Some(manager) = self.manager.take() {
            manager.shutdown().await;
        }
    }
}

pub(super) async fn prepare(
    context: &InvocationContext,
    config_path: &Path,
    session: Arc<a3s_code_core::AgentSession>,
    policy: PreparationPolicy,
) -> anyhow::Result<Preparation> {
    if context.cancellation.is_cancelled() {
        return Err(cancelled_error().into());
    }
    let executable = match policy {
        PreparationPolicy::InstalledOnly => match crate::code_use_host::discover_ready(context) {
            Ok(Some(executable)) => executable,
            Ok(None) => return Ok(Preparation::absent()),
            Err(error) => {
                return Ok(Preparation::skipped(format!(
                    "installed A3S Use discovery was skipped: {error:#}"
                )))
            }
        },
        PreparationPolicy::Required => {
            let resolution = crate::code_use_host::resolve(context, true).await;
            let Some(executable) = resolution.executable else {
                if context.cancellation.is_cancelled() {
                    return Err(cancelled_error().into());
                }
                let reason = resolution.warning.unwrap_or_else(|| {
                    "A3S Use is unavailable and did not provide recovery guidance".to_string()
                });
                return Err(runtime_error(reason).into());
            };
            executable
        }
    };
    let knowledge_paths = a3s_use_extension::ExtensionPaths::new(
        context.component_paths.data_root.join("use"),
        context.component_paths.state_root.join("use"),
    );
    let (mut runtime_tasks, runtime_host_warning) =
        ScopedRuntimeTaskHost::compose(context, config_path).await;
    if context.cancellation.is_cancelled() {
        runtime_tasks.shutdown().await;
        return Err(cancelled_error().into());
    }
    let cancellation = context.cancellation.child_token();
    let baseline_catalog = session.capability_catalog_stamp();
    let baseline_tools = session.tool_names().into_iter().collect::<BTreeSet<_>>();
    let (registry, startup_warning) = crate::use_registry::start_scoped(
        executable,
        context.directory.clone(),
        knowledge_paths,
        cancellation,
        Arc::clone(&session),
        runtime_tasks.invoker(),
    )
    .await;
    let startup_warning = join_warnings([runtime_host_warning, startup_warning]);
    if context.cancellation.is_cancelled() {
        let _ = tokio::time::timeout(SHUTDOWN_BUDGET, registry.shutdown()).await;
        runtime_tasks.shutdown().await;
        return Err(cancelled_error().into());
    }
    let ready_budget = match policy {
        PreparationPolicy::InstalledOnly => INSTALLED_ONLY_READY_BUDGET,
        PreparationPolicy::Required => READY_BUDGET,
    };
    let frozen = registry
        .freeze_scoped_runtime(session.as_ref(), ready_budget, SHUTDOWN_BUDGET)
        .await;
    match frozen {
        Ok(evidence) => Ok(Preparation::ready(evidence, startup_warning, runtime_tasks)),
        Err(error) => {
            if context.cancellation.is_cancelled() {
                let _ = tokio::time::timeout(SHUTDOWN_BUDGET, registry.shutdown()).await;
                runtime_tasks.shutdown().await;
                return Err(cancelled_error().into());
            }
            let mut message = format!("scoped capability runtime is not ready: {error:#}");
            if let Some(warning) = startup_warning {
                message.push_str("; ");
                message.push_str(&warning);
            }
            if policy == PreparationPolicy::InstalledOnly {
                let skipped = skip_installed_runtime(
                    &registry,
                    session.as_ref(),
                    &baseline_catalog,
                    &baseline_tools,
                    format!("installed A3S Use capability projection was skipped: {message}"),
                )
                .await;
                runtime_tasks.shutdown().await;
                return skipped;
            }
            let _ = tokio::time::timeout(SHUTDOWN_BUDGET, registry.shutdown()).await;
            runtime_tasks.shutdown().await;
            Err(runtime_error(message).into())
        }
    }
}

async fn skip_installed_runtime(
    registry: &crate::use_registry::UseRegistryHandle,
    session: &a3s_code_core::AgentSession,
    baseline_catalog: &a3s_code_core::capability::CapabilityCatalogStamp,
    baseline_tools: &BTreeSet<String>,
    warning: String,
) -> anyhow::Result<Preparation> {
    tokio::time::timeout(SHUTDOWN_BUDGET, registry.shutdown())
        .await
        .map_err(|_| {
            runtime_error("installed A3S Use could not be stopped safely before fallback execution")
        })?;
    let current_tools = session.tool_names().into_iter().collect::<BTreeSet<_>>();
    if &session.capability_catalog_stamp() != baseline_catalog || &current_tools != baseline_tools {
        return Err(runtime_error(
            "installed A3S Use changed the Session capability or dynamic-tool catalog before fallback",
        )
        .into());
    }
    Ok(Preparation::skipped(warning))
}

fn join_warnings(warnings: [Option<String>; 2]) -> Option<String> {
    let warnings = warnings.into_iter().flatten().collect::<Vec<_>>();
    (!warnings.is_empty()).then(|| warnings.join("; "))
}

pub(super) fn cancelled_error() -> CliError {
    CliError::new(
        "operation.cancelled",
        "scoped capability preparation cancelled",
        ExitClass::Cancelled,
    )
}

fn runtime_error(message: impl Into<String>) -> CliError {
    CliError::new(
        "capability-runtime.unavailable",
        message,
        ExitClass::Failure,
    )
    .with_suggestion(
        "Run `a3s doctor use`, install or repair A3S Use, then retry the scoped execution.",
    )
    .with_details(json!({
        "schema": crate::use_registry::SCOPED_CAPABILITY_RUNTIME_SCHEMA,
        "mode": "scoped-v1",
        "ready": false,
    }))
}
