//! Exact-generation managed Runtime Task tools projected into Code sessions.

use std::sync::Arc;

use a3s_code_core::use_runtime_tasks::{
    UsePlanScope, UsePlanScopeKind, UseProjectedLifecycleIdentity, UseRuntimeTaskDispatcher,
    UseRuntimeTaskError, UseRuntimeTaskExecutionV1, UseRuntimeTaskProjectionAdapter,
    UseRuntimeTaskProjectionV1, UseRuntimeTaskRequestV1, UseRuntimeTaskResult,
    USE_RUNTIME_TASK_RESULT_SCHEMA,
};
use a3s_runtime::ProviderId;
use a3s_use::plugin_runtime::{RuntimeTaskDispatchRequest, RuntimeTaskInvocation};
use a3s_use_core::{PlanScope, PlanScopeKind};
use anyhow::{bail, Context};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{
    CapabilityBinding, ExtensionLifecycleIdentity, ProjectedLifecycleIdentity,
    ProjectedPluginPlannerEvidence,
};

const MAX_TASK_TIMEOUT_MS: u64 = 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProjectedRuntimeTask {
    tool_name: String,
    surface_id: String,
    command: String,
    json_output: bool,
    timeout_ms: u64,
    scope: PlanScope,
    lifecycle_identity: ProjectedLifecycleIdentity,
    provider_id: String,
}

impl ProjectedRuntimeTask {
    pub(super) fn tool_name(&self) -> &str {
        &self.tool_name
    }

    pub(super) fn provider_id(&self) -> &str {
        &self.provider_id
    }
}

#[derive(Clone)]
pub(super) struct DesiredRuntimeTask {
    pub(super) capability_id: String,
    pub(super) route: String,
    pub(super) version: String,
    projection: ProjectedRuntimeTask,
    lifecycle_identity: ExtensionLifecycleIdentity,
    fingerprint: String,
}

impl DesiredRuntimeTask {
    pub(super) fn tool_name(&self) -> &str {
        &self.projection.tool_name
    }

    pub(super) fn surface_id(&self) -> &str {
        &self.projection.surface_id
    }

    pub(super) fn capability_id(&self) -> &str {
        &self.capability_id
    }

    pub(super) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeTaskOutcome {
    pub(crate) exit_code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) truncated: bool,
}

#[async_trait]
pub(crate) trait RuntimeTaskInvoker: Send + Sync {
    fn has_runtime_provider(&self, provider_id: &str) -> bool;

    async fn invoke_runtime_task(
        &self,
        request: RuntimeTaskDispatchRequest,
    ) -> anyhow::Result<RuntimeTaskOutcome>;
}

pub(super) fn validate_projected_runtime_tasks(binding: &CapabilityBinding) -> anyhow::Result<()> {
    if binding.tool_tasks.is_empty() {
        return Ok(());
    }
    if !binding.enabled {
        bail!(
            "A3S Use capability '{}' projects Runtime Tool Tasks while disabled",
            binding.id
        );
    }
    if !binding.surfaces.iter().any(|surface| surface == "tool") {
        bail!(
            "A3S Use capability '{}' projects Runtime Tool Tasks without declaring the surface",
            binding.id
        );
    }
    let planner = binding.planner_evidence.as_ref().with_context(|| {
        format!(
            "A3S Use capability '{}' projects Runtime Tool Tasks without exact package evidence",
            binding.id
        )
    })?;
    let mut names = std::collections::BTreeSet::new();
    let mut surfaces = std::collections::BTreeSet::new();
    for task in &binding.tool_tasks {
        validate_projected_runtime_task(&binding.id, binding.lifecycle_generation, planner, task)?;
        if !names.insert(task.tool_name.as_str()) {
            bail!(
                "A3S Use capability '{}' projects duplicate Runtime Tool name '{}'",
                binding.id,
                task.tool_name
            );
        }
        if !surfaces.insert(task.surface_id.as_str()) {
            bail!(
                "A3S Use capability '{}' projects duplicate Runtime Tool surface '{}'",
                binding.id,
                task.surface_id
            );
        }
    }
    Ok(())
}

fn validate_projected_runtime_task(
    binding_id: &str,
    lifecycle_generation: Option<u64>,
    planner: &ProjectedPluginPlannerEvidence,
    task: &ProjectedRuntimeTask,
) -> anyhow::Result<()> {
    let identity = task.lifecycle_identity.validated("Runtime Task")?;
    let expected_component_id = format!("use/{}", identity.package_id());
    if binding_id != expected_component_id
        || planner.package_id != identity.package_id()
        || planner.package_sha256 != identity.package_digest()
        || planner.manifest_sha256 != identity.manifest_digest()
        || lifecycle_generation != Some(identity.generation())
    {
        bail!(
            "A3S Use capability '{binding_id}' projects a Runtime Tool Task with mismatched package lifecycle identity"
        );
    }
    if !valid_tool_name(&task.tool_name)
        || !valid_segment(&task.surface_id)
        || task.command.is_empty()
        || task.command.len() > 256
        || task.command.trim() != task.command
        || task.command.chars().any(char::is_control)
        || task.timeout_ms == 0
        || task.timeout_ms > MAX_TASK_TIMEOUT_MS
        || !valid_scope_id(&task.scope.id)
        || ProviderId::parse(&task.provider_id).is_err()
    {
        bail!("A3S Use capability '{binding_id}' projects invalid Runtime Tool Task metadata");
    }
    Ok(())
}

fn valid_tool_name(value: &str) -> bool {
    value.starts_with("use_tool_")
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_scope_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/' | b'@')
        })
}

pub(super) fn desired_runtime_task(
    binding: &CapabilityBinding,
    projection: &ProjectedRuntimeTask,
) -> anyhow::Result<DesiredRuntimeTask> {
    let lifecycle_identity = projection.lifecycle_identity.validated("Runtime Task")?;
    let fingerprint = serde_json::to_string(&(binding.id.as_str(), projection))
        .context("failed to fingerprint an A3S Use Runtime Tool Task")?;
    Ok(DesiredRuntimeTask {
        capability_id: binding.id.clone(),
        route: binding.route.clone(),
        version: binding.version.clone(),
        projection: projection.clone(),
        lifecycle_identity,
        fingerprint,
    })
}

/// Host bridge: Core's portable Runtime Task dispatcher → CLI Plugin Manager
/// invoker (`RuntimeTaskDispatchRequest`).
pub(super) struct RuntimeTaskInvokerDispatcher {
    invoker: Arc<dyn RuntimeTaskInvoker>,
}

impl RuntimeTaskInvokerDispatcher {
    pub(super) fn new(invoker: Arc<dyn RuntimeTaskInvoker>) -> Self {
        Self { invoker }
    }
}

#[async_trait]
impl UseRuntimeTaskDispatcher for RuntimeTaskInvokerDispatcher {
    async fn invoke(
        &self,
        request: UseRuntimeTaskRequestV1,
    ) -> UseRuntimeTaskResult<UseRuntimeTaskExecutionV1> {
        request.validate()?;
        let identity = ExtensionLifecycleIdentity::new(
            &request.projection.lifecycle_identity.package_id,
            request.projection.lifecycle_identity.package_digest.clone(),
            request.projection.lifecycle_identity.manifest_digest.clone(),
            request.projection.lifecycle_identity.generation,
        )
        .map_err(|error| {
            UseRuntimeTaskError::Dispatch(format!("{}: {}", error.code, error.message))
        })?;
        let scope = PlanScope {
            kind: match request.projection.scope.kind {
                UsePlanScopeKind::User => PlanScopeKind::User,
                UsePlanScopeKind::Workspace => PlanScopeKind::Workspace,
            },
            id: request.projection.scope.id.clone(),
        };
        let invocation =
            RuntimeTaskInvocation::new(request.invocation_id.clone(), request.argv.clone())
                .map_err(|error| {
                    UseRuntimeTaskError::Dispatch(format!("{}: {}", error.code, error.message))
                })?;
        let dispatch = RuntimeTaskDispatchRequest::new(
            identity,
            scope,
            request.projection.surface_id.clone(),
            invocation,
            request.request_id.clone(),
            Some(request.deadline_at_ms),
        )
        .map_err(|error| {
            UseRuntimeTaskError::Dispatch(format!("{}: {}", error.code, error.message))
        })?;
        let outcome = self
            .invoker
            .invoke_runtime_task(dispatch)
            .await
            .map_err(|error| UseRuntimeTaskError::Dispatch(error.to_string()))?;
        Ok(UseRuntimeTaskExecutionV1 {
            schema: USE_RUNTIME_TASK_RESULT_SCHEMA.to_owned(),
            package_id: request.projection.lifecycle_identity.package_id.clone(),
            surface_id: request.projection.surface_id.clone(),
            lifecycle_generation: request.projection.lifecycle_identity.generation,
            provider_id: request.projection.provider_id.clone(),
            exit_code: outcome.exit_code,
            stdout: outcome.stdout,
            stderr: outcome.stderr,
            truncated: outcome.truncated,
        })
    }
}

pub(super) fn code_runtime_task_projection(
    task: &DesiredRuntimeTask,
) -> anyhow::Result<UseRuntimeTaskProjectionV1> {
    let projection = UseRuntimeTaskProjectionV1 {
        tool_name: task.projection.tool_name.clone(),
        surface_id: task.projection.surface_id.clone(),
        command: task.projection.command.clone(),
        json_output: task.projection.json_output,
        timeout_ms: task.projection.timeout_ms,
        scope: UsePlanScope {
            kind: match task.projection.scope.kind {
                PlanScopeKind::User => UsePlanScopeKind::User,
                PlanScopeKind::Workspace => UsePlanScopeKind::Workspace,
            },
            id: task.projection.scope.id.clone(),
        },
        lifecycle_identity: UseProjectedLifecycleIdentity {
            package_id: task.lifecycle_identity.package_id().to_string(),
            package_digest: task.lifecycle_identity.package_digest().to_string(),
            manifest_digest: task.lifecycle_identity.manifest_digest().to_string(),
            generation: task.lifecycle_identity.generation(),
        },
        provider_id: task.projection.provider_id.clone(),
    };
    projection
        .validate()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(projection)
}

pub(super) fn projection_adapter(
    task: DesiredRuntimeTask,
    snapshot_digest: impl Into<String>,
    invoker: Arc<dyn RuntimeTaskInvoker>,
) -> anyhow::Result<UseRuntimeTaskProjectionAdapter> {
    let projection = code_runtime_task_projection(&task)?;
    let dispatcher: Arc<dyn UseRuntimeTaskDispatcher> =
        Arc::new(RuntimeTaskInvokerDispatcher::new(invoker));
    UseRuntimeTaskProjectionAdapter::new(snapshot_digest, projection, dispatcher)
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use a3s_code_core::capability::CapabilityProjectionAdapter;
    use a3s_code_core::use_runtime_tasks::USE_RUNTIME_TASK_REQUEST_SCHEMA;
    use a3s_use_core::PlanScopeKind;
    use tokio_util::sync::CancellationToken;

    use super::*;

    struct RecordingInvoker {
        requests: Mutex<Vec<RuntimeTaskDispatchRequest>>,
        outcome: RuntimeTaskOutcome,
    }

    impl RecordingInvoker {
        fn new(stdout: impl Into<String>) -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                outcome: RuntimeTaskOutcome {
                    exit_code: 0,
                    stdout: stdout.into(),
                    stderr: "fixture warning".to_string(),
                    truncated: false,
                },
            }
        }
    }

    #[async_trait]
    impl RuntimeTaskInvoker for RecordingInvoker {
        fn has_runtime_provider(&self, provider_id: &str) -> bool {
            provider_id == "test-runtime"
        }

        async fn invoke_runtime_task(
            &self,
            request: RuntimeTaskDispatchRequest,
        ) -> anyhow::Result<RuntimeTaskOutcome> {
            self.requests
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(request);
            Ok(self.outcome.clone())
        }
    }

    fn fixture_task(json_output: bool) -> DesiredRuntimeTask {
        let projection = ProjectedRuntimeTask {
            tool_name: "use_tool_report_convert_0123456789abcdef".to_string(),
            surface_id: "convert".to_string(),
            command: "acme-convert".to_string(),
            json_output,
            timeout_ms: 30_000,
            scope: PlanScope {
                kind: PlanScopeKind::Workspace,
                id: "workspace:fixture".to_string(),
            },
            lifecycle_identity: ProjectedLifecycleIdentity {
                package_id: "acme/report".to_string(),
                package_digest: format!("sha256:{}", "a".repeat(64)),
                manifest_digest: format!("sha256:{}", "b".repeat(64)),
                generation: 7,
            },
            provider_id: "test-runtime".to_string(),
        };
        DesiredRuntimeTask {
            route: "fixture".to_string(),
            version: "1.0.0".to_string(),
            capability_id: "use/acme/report".to_string(),
            lifecycle_identity: projection
                .lifecycle_identity
                .validated("Runtime Task")
                .unwrap(),
            fingerprint: "fixture-runtime-task".to_string(),
            projection,
        }
    }

    fn snapshot_digest() -> String {
        format!("sha256:{}", "c".repeat(64))
    }

    #[tokio::test]
    async fn core_adapter_prepares_from_desired_runtime_task() {
        let invoker = Arc::new(RecordingInvoker::new(r#"{"answer":42}"#));
        let adapter = projection_adapter(
            fixture_task(true),
            snapshot_digest(),
            invoker as Arc<dyn RuntimeTaskInvoker>,
        )
        .unwrap();
        assert_eq!(adapter.snapshot_digest(), snapshot_digest());
        assert_eq!(
            adapter.projection().tool_name,
            "use_tool_report_convert_0123456789abcdef"
        );
        let _ = Box::new(adapter)
            .prepare(CancellationToken::new())
            .await
            .expect("Core Runtime Task adapter must prepare");
    }

    #[tokio::test]
    async fn dispatcher_forwards_exact_identity_scope_surface_and_argv() {
        let invoker = Arc::new(RecordingInvoker::new(r#"{"answer":42}"#));
        let dispatcher = RuntimeTaskInvokerDispatcher::new(Arc::clone(&invoker) as _);
        let projection = code_runtime_task_projection(&fixture_task(true)).unwrap();
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let deadline_at_ms = before + 30_000;
        let execution = dispatcher
            .invoke(UseRuntimeTaskRequestV1 {
                schema: USE_RUNTIME_TASK_REQUEST_SCHEMA.to_owned(),
                projection: projection.clone(),
                invocation_id: "code-use-fixture-invocation".to_owned(),
                request_id: "code-use-fixture-request".to_owned(),
                argv: vec!["input.txt".to_owned(), "--format=json".to_owned()],
                deadline_at_ms,
            })
            .await
            .unwrap();

        assert_eq!(execution.package_id, "acme/report");
        assert_eq!(execution.surface_id, "convert");
        assert_eq!(execution.lifecycle_generation, 7);
        assert_eq!(execution.provider_id, "test-runtime");
        assert_eq!(execution.exit_code, 0);
        assert_eq!(execution.stdout, r#"{"answer":42}"#);
        assert_eq!(execution.stderr, "fixture warning");
        assert!(!execution.truncated);

        let requests = invoker
            .requests
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.identity().package_id(), "acme/report");
        assert_eq!(
            request.identity().package_digest(),
            format!("sha256:{}", "a".repeat(64))
        );
        assert_eq!(
            request.identity().manifest_digest(),
            format!("sha256:{}", "b".repeat(64))
        );
        assert_eq!(request.identity().generation(), 7);
        assert_eq!(request.scope().kind, PlanScopeKind::Workspace);
        assert_eq!(request.scope().id, "workspace:fixture");
        assert_eq!(request.surface_id(), "convert");
        assert_eq!(
            request.invocation().args(),
            &["input.txt".to_string(), "--format=json".to_string()]
        );
        assert_eq!(
            request.invocation().invocation_id(),
            "code-use-fixture-invocation"
        );
        assert_eq!(request.request_id(), "code-use-fixture-request");
        assert_eq!(request.deadline_at_ms(), Some(deadline_at_ms));
    }
}
