//! One-shot A3S Use capability runtime preparation.

use std::sync::Arc;
use std::time::Duration;

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
}

impl Preparation {
    fn absent() -> Self {
        Self {
            evidence: None,
            warning: None,
        }
    }

    fn skipped(warning: impl Into<String>) -> Self {
        Self {
            evidence: None,
            warning: Some(warning.into()),
        }
    }

    fn ready(evidence: crate::use_registry::ScopedCapabilityRuntimeEvidence) -> Self {
        Self {
            evidence: Some(evidence),
            warning: None,
        }
    }
}

pub(super) async fn prepare(
    context: &InvocationContext,
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
    let cancellation = context.cancellation.child_token();
    let baseline_catalog = session.capability_catalog_stamp();
    let (registry, startup_warning) = crate::use_registry::start_scoped(
        executable,
        context.directory.clone(),
        knowledge_paths,
        cancellation,
        Arc::clone(&session),
    )
    .await;
    if context.cancellation.is_cancelled() {
        let _ = tokio::time::timeout(SHUTDOWN_BUDGET, registry.shutdown()).await;
        return Err(cancelled_error().into());
    }
    if policy == PreparationPolicy::InstalledOnly {
        if let Some(warning) = startup_warning.as_deref() {
            return skip_installed_runtime(
                &registry,
                session.as_ref(),
                &baseline_catalog,
                format!("installed A3S Use capability projection was skipped: {warning}"),
            )
            .await;
        }
    }
    let ready_budget = match policy {
        PreparationPolicy::InstalledOnly => INSTALLED_ONLY_READY_BUDGET,
        PreparationPolicy::Required => READY_BUDGET,
    };
    let frozen = registry
        .freeze_scoped_runtime(session.as_ref(), ready_budget, SHUTDOWN_BUDGET)
        .await;
    match frozen {
        Ok(evidence) => Ok(Preparation::ready(evidence)),
        Err(error) => {
            if context.cancellation.is_cancelled() {
                let _ = tokio::time::timeout(SHUTDOWN_BUDGET, registry.shutdown()).await;
                return Err(cancelled_error().into());
            }
            let mut message = format!("scoped capability runtime is not ready: {error:#}");
            if let Some(warning) = startup_warning {
                message.push_str("; ");
                message.push_str(&warning);
            }
            if policy == PreparationPolicy::InstalledOnly {
                return skip_installed_runtime(
                    &registry,
                    session.as_ref(),
                    &baseline_catalog,
                    format!("installed A3S Use capability projection was skipped: {message}"),
                )
                .await;
            }
            let _ = tokio::time::timeout(SHUTDOWN_BUDGET, registry.shutdown()).await;
            Err(runtime_error(message).into())
        }
    }
}

async fn skip_installed_runtime(
    registry: &crate::use_registry::UseRegistryHandle,
    session: &a3s_code_core::AgentSession,
    baseline_catalog: &a3s_code_core::capability::CapabilityCatalogStamp,
    warning: String,
) -> anyhow::Result<Preparation> {
    tokio::time::timeout(SHUTDOWN_BUDGET, registry.shutdown())
        .await
        .map_err(|_| {
            runtime_error("installed A3S Use could not be stopped safely before fallback execution")
        })?;
    if &session.capability_catalog_stamp() != baseline_catalog {
        return Err(runtime_error(
            "installed A3S Use changed the Session capability catalog before fallback",
        )
        .into());
    }
    Ok(Preparation::skipped(warning))
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
