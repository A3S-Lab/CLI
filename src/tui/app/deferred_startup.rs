//! First-frame gates for configured capabilities with external startup cost.

use super::*;
use a3s_code_core::mcp::McpServerConfig;
use a3s_code_core::sandbox::{
    BashSandbox, SandboxCommandRequest, SandboxExecutionOutput, SandboxOutput,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

const CONFIGURED_MCP_STOP_GRACE: Duration = Duration::from_millis(250);
const CONFIGURED_MCP_ABORT_SETTLE: Duration = Duration::from_millis(250);
const DEFERRED_SANDBOX_EXEC_WAIT: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct ConfiguredMcpTarget {
    generation: u64,
    session: Arc<AgentSession>,
}

/// Projects user-configured MCP servers into whichever TUI session is active.
///
/// Agent bootstrap receives a config with MCP removed, so no transport can
/// block terminal handoff. This runtime opens its one-way activation gate only
/// after the first frame, then uses the public live-session extension API. A
/// model or effort rebuild publishes a new target and receives the same
/// configured capability set without returning MCP to Agent bootstrap.
#[derive(Clone)]
pub(super) struct ConfiguredMcpRuntime {
    target: watch::Sender<ConfiguredMcpTarget>,
    projected: watch::Sender<u64>,
    next_generation: Arc<AtomicU64>,
    activation: CancellationToken,
    cancellation: CancellationToken,
    configured: bool,
}

impl ConfiguredMcpRuntime {
    pub(super) fn start(
        configs: Vec<McpServerConfig>,
        initial_session: Arc<AgentSession>,
    ) -> (Self, tokio::task::JoinHandle<()>) {
        let initial = ConfiguredMcpTarget {
            generation: 1,
            session: initial_session,
        };
        let (target, target_rx) = watch::channel(initial);
        let (projected, _) = watch::channel(0);
        let runtime = Self {
            target,
            projected,
            next_generation: Arc::new(AtomicU64::new(2)),
            activation: CancellationToken::new(),
            cancellation: CancellationToken::new(),
            configured: configs.iter().any(|config| config.enabled),
        };
        let worker = tokio::spawn(run_configured_mcp(
            Arc::new(configs),
            target_rx,
            runtime.projected.clone(),
            runtime.activation.clone(),
            runtime.cancellation.clone(),
        ));
        (runtime, worker)
    }

    pub(super) fn activation_command(&self) -> Option<Cmd<Msg>> {
        if !self.configured {
            return None;
        }
        let activation = self.activation.clone();
        Some(cmd::cmd(move || async move {
            activation.cancel();
            Msg::ConfiguredMcpStartupActivated
        }))
    }

    pub(super) fn activate(&self) {
        self.activation.cancel();
    }

    pub(super) fn replace_session(&self, session: Arc<AgentSession>) {
        let generation = self.next_generation.fetch_add(1, AtomicOrdering::AcqRel);
        self.target.send_replace(ConfiguredMcpTarget {
            generation,
            session,
        });
    }

    pub(super) async fn wait_for_initial_projection(&self) {
        if !self.configured {
            return;
        }
        let mut projected = self.projected.subscribe();
        while *projected.borrow_and_update() < 1 {
            if projected.changed().await.is_err() {
                return;
            }
        }
    }

    fn cancel(&self) {
        self.cancellation.cancel();
    }
}

async fn run_configured_mcp(
    configs: Arc<Vec<McpServerConfig>>,
    mut targets: watch::Receiver<ConfiguredMcpTarget>,
    projected: watch::Sender<u64>,
    activation: CancellationToken,
    cancellation: CancellationToken,
) {
    if !configs.iter().any(|config| config.enabled) {
        projected.send_replace(1);
        return;
    }
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => return,
        _ = activation.cancelled() => {}
    }

    'target: loop {
        let target = targets.borrow_and_update().clone();
        for config in configs.iter().filter(|config| config.enabled) {
            let result = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return,
                changed = targets.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    continue 'target;
                }
                result = target.session.add_mcp_server(config.clone()) => result,
            };
            if let Err(error) = result {
                tracing::warn!(
                    session_id = %target.session.session_id(),
                    server = %config.name,
                    error = %error,
                    "Configured MCP server failed during deferred TUI startup"
                );
            }
        }
        projected.send_replace(target.generation);
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return,
            changed = targets.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

pub(super) async fn stop_configured_mcp_runtime(
    runtime: &ConfiguredMcpRuntime,
    worker: &mut tokio::task::JoinHandle<()>,
) {
    runtime.cancel();
    if tokio::time::timeout(CONFIGURED_MCP_STOP_GRACE, &mut *worker)
        .await
        .is_ok()
    {
        return;
    }
    worker.abort();
    let _ = tokio::time::timeout(CONFIGURED_MCP_ABORT_SETTLE, &mut *worker).await;
}

#[derive(Clone)]
enum DeferredSandboxState {
    Preparing,
    Ready(Arc<dyn BashSandbox>),
    Unavailable(Arc<str>),
    Closed,
}

/// Stable session sandbox handle whose expensive backend is installed after
/// the first frame. Standard Bash never falls through to Core's host runner:
/// while preparation is pending or unavailable this proxy returns a bounded,
/// actionable failure. Explicit `require_escalated` calls retain the existing
/// reviewed host path because Core intentionally bypasses the sandbox handle.
pub(super) struct DeferredBashSandbox {
    state: watch::Sender<DeferredSandboxState>,
    transition: std::sync::Mutex<()>,
    closed: AtomicBool,
}

impl std::fmt::Debug for DeferredBashSandbox {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = match &*self.state.borrow() {
            DeferredSandboxState::Preparing => "preparing",
            DeferredSandboxState::Ready(_) => "ready",
            DeferredSandboxState::Unavailable(_) => "unavailable",
            DeferredSandboxState::Closed => "closed",
        };
        formatter
            .debug_struct("DeferredBashSandbox")
            .field("state", &state)
            .finish()
    }
}

impl DeferredBashSandbox {
    pub(super) fn new() -> Self {
        let (state, _) = watch::channel(DeferredSandboxState::Preparing);
        Self {
            state,
            transition: std::sync::Mutex::new(()),
            closed: AtomicBool::new(false),
        }
    }

    fn install(&self, sandbox: Arc<dyn BashSandbox>) -> Result<(), Arc<dyn BashSandbox>> {
        let _transition = self
            .transition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.closed.load(Ordering::Acquire) {
            return Err(sandbox);
        }
        self.state
            .send_replace(DeferredSandboxState::Ready(sandbox));
        Ok(())
    }

    fn mark_unavailable(&self, reason: impl Into<Arc<str>>) {
        let _transition = self
            .transition
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.closed.load(Ordering::Acquire) {
            self.state
                .send_replace(DeferredSandboxState::Unavailable(reason.into()));
        }
    }

    async fn backend(&self) -> anyhow::Result<Arc<dyn BashSandbox>> {
        let mut state = self.state.subscribe();
        let wait = async {
            loop {
                let snapshot = state.borrow_and_update().clone();
                match snapshot {
                    DeferredSandboxState::Preparing => {}
                    DeferredSandboxState::Ready(sandbox) => return Ok(sandbox),
                    DeferredSandboxState::Unavailable(reason) => {
                        anyhow::bail!("{reason}")
                    }
                    DeferredSandboxState::Closed => {
                        anyhow::bail!("local command sandbox is closed")
                    }
                }
                if state.changed().await.is_err() {
                    anyhow::bail!("local command sandbox state is unavailable")
                }
            }
        };
        tokio::time::timeout(DEFERRED_SANDBOX_EXEC_WAIT, wait)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "local command sandbox is still preparing; retry the command shortly"
                )
            })?
    }

    pub(super) async fn close(&self) {
        let ready = {
            let _transition = self
                .transition
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.closed.swap(true, Ordering::AcqRel) {
                return;
            }
            let ready = match &*self.state.borrow() {
                DeferredSandboxState::Ready(sandbox) => Some(Arc::clone(sandbox)),
                _ => None,
            };
            self.state.send_replace(DeferredSandboxState::Closed);
            ready
        };
        if let Some(sandbox) = ready {
            sandbox.shutdown().await;
        }
    }
}

#[async_trait]
impl BashSandbox for DeferredBashSandbox {
    async fn exec_command(
        &self,
        command: &str,
        guest_workspace: &str,
    ) -> anyhow::Result<SandboxOutput> {
        self.backend()
            .await?
            .exec_command(command, guest_workspace)
            .await
    }

    async fn exec(&self, request: SandboxCommandRequest) -> anyhow::Result<SandboxExecutionOutput> {
        self.backend().await?.exec(request).await
    }

    async fn shutdown(&self) {
        self.close().await;
    }
}

fn sandbox_probe_warning(error: &anyhow::Error) -> String {
    format!(
        "The native local command sandbox failed its bounded OS capability probe: {error:#}. \
         Bash will remain denied in every mode. Repair the reported platform prerequisite \
         and restart `a3s code`"
    )
}

pub(super) async fn prepare_deferred_sandbox(
    workspace: &Path,
    sandbox: Arc<DeferredBashSandbox>,
    execution_policy: TuiExecutionPolicy,
) -> Option<String> {
    let (backend, warning) = match a3s_code_core::sandbox::native::NativeBashSandbox::new(workspace)
    {
        Ok(native) => match native.probe().await {
            Ok(()) => (Some(Arc::new(native) as Arc<dyn BashSandbox>), None),
            Err(error) => (None, Some(sandbox_probe_warning(&error))),
        },
        Err(error) => (None, Some(sandbox_probe_warning(&error))),
    };
    match backend {
        Some(backend) => match sandbox.install(backend) {
            Ok(()) => execution_policy.set_sandbox_available(true),
            Err(backend) => backend.shutdown().await,
        },
        None => {
            execution_policy.set_sandbox_available(false);
            sandbox.mark_unavailable(warning.clone().unwrap_or_else(|| {
                "The native local command sandbox is unavailable; Bash is denied".to_string()
            }));
        }
    }
    warning
}

pub(super) fn deferred_sandbox_setup_command(
    workspace: PathBuf,
    sandbox: Arc<DeferredBashSandbox>,
    execution_policy: TuiExecutionPolicy,
) -> Cmd<Msg> {
    cmd::cmd(move || async move {
        let warning = prepare_deferred_sandbox(&workspace, sandbox, execution_policy).await;
        Msg::SandboxStartupFinished { warning }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSandbox;

    #[async_trait]
    impl BashSandbox for TestSandbox {
        async fn exec_command(
            &self,
            command: &str,
            _guest_workspace: &str,
        ) -> anyhow::Result<SandboxOutput> {
            Ok(SandboxOutput {
                stdout: command.to_string(),
                stderr: String::new(),
                exit_code: 0,
            })
        }

        async fn shutdown(&self) {}
    }

    #[tokio::test]
    async fn deferred_sandbox_routes_only_after_verified_installation() {
        let sandbox = DeferredBashSandbox::new();
        assert!(sandbox.install(Arc::new(TestSandbox)).is_ok());

        let output = sandbox.exec_command("ready", "/workspace").await.unwrap();
        assert_eq!(output.stdout, "ready");
        sandbox.close().await;
        let error = match sandbox.exec_command("closed", "/workspace").await {
            Ok(_) => panic!("closed sandbox must reject execution"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("closed"), "{error}");
    }

    #[tokio::test]
    async fn unavailable_deferred_sandbox_never_falls_through_to_host() {
        let sandbox = DeferredBashSandbox::new();
        sandbox.mark_unavailable("verified sandbox unavailable");

        let error = match sandbox.exec_command("must-not-run", "/workspace").await {
            Ok(_) => panic!("unavailable sandbox must reject execution"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("verified sandbox unavailable"), "{error}");
    }
}
