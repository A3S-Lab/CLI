//! Session policy, DeepResearch tool gates, and terminal permission checks.

use super::*;

pub(super) fn with_recent_workspace_context(
    opts: SessionOptions,
    manifest: &Arc<LocalWorkspaceManifest>,
) -> SessionOptions {
    opts.with_context_provider(Arc::new(RecentWorkspaceFilesContextProvider::new(
        manifest.clone(),
    )))
}

pub(super) fn tui_session_options(
    confirmation: a3s_code_core::hitl::ConfirmationPolicy,
) -> SessionOptions {
    tui_session_options_with_gate_and_execution(
        confirmation,
        DeepResearchReportToolGate::default(),
        TuiExecutionPolicy::default(),
    )
}

#[cfg(test)]
pub(super) fn tui_session_options_with_gate(
    confirmation: a3s_code_core::hitl::ConfirmationPolicy,
    deep_research_report_tool_gate: DeepResearchReportToolGate,
) -> SessionOptions {
    tui_session_options_with_gate_and_execution(
        confirmation,
        deep_research_report_tool_gate,
        TuiExecutionPolicy::default(),
    )
}

pub(super) fn tui_session_options_with_gate_and_execution(
    confirmation: a3s_code_core::hitl::ConfirmationPolicy,
    deep_research_report_tool_gate: DeepResearchReportToolGate,
    execution_policy: TuiExecutionPolicy,
) -> SessionOptions {
    let permission_policy = tui_permission_policy();
    let confirmation_manager =
        TuiModeConfirmationProvider::new(confirmation, execution_policy.clone());
    SessionOptions::new()
        .with_auto_compact(false)
        .with_confirmation_manager(Arc::new(confirmation_manager))
        .with_permission_policy(permission_policy.clone())
        .with_permission_checker(Arc::new(TuiHitlPermissionChecker::with_execution(
            permission_policy,
            deep_research_report_tool_gate,
            execution_policy,
        )))
        .with_tool_timeout(TOOL_EXEC_TIMEOUT_MS)
        .with_duplicate_tool_call_threshold(TUI_DUPLICATE_TOOL_CALL_THRESHOLD)
}

/// Core serializable permission policy for the TUI.
///
/// The runtime checker below layers structured decisions for bash, git, and
/// batch on top of this policy. Keep this policy conservative and serializable
/// so persisted sessions still have a safe fallback.
pub(super) fn tui_permission_policy() -> a3s_code_core::permissions::PermissionPolicy {
    a3s_code_core::permissions::PermissionPolicy::new()
        .deny_all(&[
            "Read(/**)",
            "Read(**/../**)",
            "Grep(* /**)",
            "Grep(* **/../**)",
            "Glob(/**)",
            "Glob(**/../**)",
            "LS(/**)",
            "LS(**/../**)",
            "Write(/**)",
            "Edit(/**)",
            "Write(**/../**)",
            "Edit(**/../**)",
        ])
        .allow_all(&[
            "Read(*)",
            "Grep(*)",
            "Glob(*)",
            "LS(*)",
            "web_search(*)",
            "web_fetch(*)",
        ])
        .ask_all(&[
            "Write(*)",
            "Edit(*)",
            "Patch(*)",
            "Bash(*)",
            "Git(*)",
            "batch(*)",
            "program(*)",
            "task(*)",
            "parallel_task(*)",
            "dynamic_workflow(*)",
            "Skill(*)",
        ])
}

/// Active execution semantics shared by the TUI and every rebuilt Core
/// session. The value describes the running turn, not the mutable composer
/// mode, so a queued Auto turn cannot become interactive if the user changes
/// the footer mode before it starts.
#[derive(Clone)]
pub(super) struct TuiExecutionPolicy {
    mode: Arc<AtomicU8>,
}

impl Default for TuiExecutionPolicy {
    fn default() -> Self {
        Self::new(Mode::Default)
    }
}

impl TuiExecutionPolicy {
    const DEFAULT: u8 = 0;
    const PLAN: u8 = 1;
    const AUTO: u8 = 2;

    pub(super) fn new(mode: Mode) -> Self {
        let policy = Self {
            mode: Arc::new(AtomicU8::new(Self::DEFAULT)),
        };
        policy.set_mode(mode);
        policy
    }

    pub(super) fn set_mode(&self, mode: Mode) {
        let encoded = match mode {
            Mode::Default => Self::DEFAULT,
            Mode::Plan => Self::PLAN,
            Mode::Auto => Self::AUTO,
        };
        self.mode.store(encoded, Ordering::SeqCst);
    }

    pub(super) fn mode(&self) -> Mode {
        match self.mode.load(Ordering::SeqCst) {
            Self::PLAN => Mode::Plan,
            Self::AUTO => Mode::Auto,
            _ => Mode::Default,
        }
    }

    /// Resolve a confirmation already emitted by Core without entering HITL.
    ///
    /// `None` keeps the interactive Default/Plan flow. Auto always returns a
    /// decision: non-denied calls are approved and hard-denied calls are
    /// rejected.
    pub(super) fn auto_confirmation_decision(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        workspace: &Path,
    ) -> Option<bool> {
        if self.mode() != Mode::Auto {
            return None;
        }
        let checker = a3s_code_core::permissions::InteractiveToolGuardrail::for_mode("auto")
            .with_workspace(workspace);
        Some(
            a3s_code_core::permissions::PermissionChecker::check(&checker, tool_name, args)
                != a3s_code_core::permissions::PermissionDecision::Deny,
        )
    }
}

/// Confirmation provider that preserves interactive HITL for Default/Plan
/// turns while disabling the escalation path for an immutable Auto turn.
///
/// Permission `Allow` normally bypasses HITL, but tool-owned metadata may
/// explicitly request a second confirmation check. Auto remains
/// non-interactive through that path; hard denials are resolved earlier by the
/// permission checker.
struct TuiModeConfirmationProvider {
    inner: Arc<a3s_code_core::hitl::ConfirmationManager>,
    execution_policy: TuiExecutionPolicy,
}

impl TuiModeConfirmationProvider {
    fn new(
        policy: a3s_code_core::hitl::ConfirmationPolicy,
        execution_policy: TuiExecutionPolicy,
    ) -> Self {
        let (event_tx, _) = tokio::sync::broadcast::channel(16);
        Self {
            inner: Arc::new(a3s_code_core::hitl::ConfirmationManager::new(
                policy, event_tx,
            )),
            execution_policy,
        }
    }

    fn auto_response() -> tokio::sync::oneshot::Receiver<a3s_code_core::hitl::ConfirmationResponse>
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = tx.send(a3s_code_core::hitl::ConfirmationResponse {
            approved: true,
            reason: None,
        });
        rx
    }
}

#[async_trait::async_trait]
impl a3s_code_core::hitl::ConfirmationProvider for TuiModeConfirmationProvider {
    async fn requires_confirmation(&self, tool_name: &str) -> bool {
        self.execution_policy.mode() != Mode::Auto
            && self.inner.requires_confirmation(tool_name).await
    }

    async fn request_confirmation(
        &self,
        tool_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> tokio::sync::oneshot::Receiver<a3s_code_core::hitl::ConfirmationResponse> {
        if self.execution_policy.mode() == Mode::Auto {
            Self::auto_response()
        } else {
            self.inner
                .request_confirmation(tool_id, tool_name, args)
                .await
        }
    }

    async fn confirm(
        &self,
        tool_id: &str,
        approved: bool,
        reason: Option<String>,
    ) -> Result<bool, String> {
        self.inner.confirm(tool_id, approved, reason).await
    }

    async fn policy(&self) -> a3s_code_core::hitl::ConfirmationPolicy {
        self.inner.policy().await
    }

    async fn set_policy(&self, policy: a3s_code_core::hitl::ConfirmationPolicy) {
        self.inner.set_policy(policy).await;
    }

    async fn check_timeouts(&self) -> usize {
        self.inner.check_timeouts().await
    }

    async fn cancel_all(&self) -> usize {
        self.inner.cancel_all().await
    }

    async fn pending_confirmations(&self) -> Vec<a3s_code_core::hitl::PendingConfirmationInfo> {
        self.inner.pending_confirmation_details().await
    }
}

fn plan_tool_is_read_only(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "read" | "grep" | "glob" | "ls" | "web_search" | "web_fetch"
    )
}

#[derive(Clone, Default)]
pub(super) struct DeepResearchReportToolGate {
    phase: Arc<AtomicU8>,
    network_disabled: Arc<std::sync::atomic::AtomicBool>,
    workspace: Arc<std::sync::RwLock<Option<PathBuf>>>,
    expected_slug: Arc<std::sync::RwLock<Option<String>>>,
}

impl DeepResearchReportToolGate {
    const INACTIVE: u8 = 0;
    const EVIDENCE: u8 = 1;
    const REPORT: u8 = 2;
    const SYNTHESIS: u8 = 3;

    pub(super) fn set_evidence_scope(&self, evidence_scope: DeepResearchEvidenceScope) {
        self.network_disabled
            .store(!evidence_scope.network_enabled(), Ordering::SeqCst);
        self.phase.store(Self::EVIDENCE, Ordering::SeqCst);
    }

    pub(super) fn set_workspace(&self, workspace: &Path) {
        if let Ok(mut stored) = self.workspace.write() {
            *stored = workspace.canonicalize().ok();
        }
    }

    pub(super) fn workspace(&self) -> Option<PathBuf> {
        self.workspace
            .read()
            .ok()
            .and_then(|workspace| workspace.clone())
    }

    pub(super) fn set_report_target(&self, workspace: &Path, query: &str) {
        self.set_workspace(workspace);
        if let Ok(mut stored) = self.expected_slug.write() {
            *stored = Some(deep_research_report_slug(query));
        }
    }

    pub(super) fn set_report_only(&self, enabled: bool) {
        self.phase.store(
            if enabled {
                Self::REPORT
            } else {
                Self::INACTIVE
            },
            Ordering::SeqCst,
        );
        if !enabled {
            self.network_disabled.store(false, Ordering::SeqCst);
            if let Ok(mut stored) = self.expected_slug.write() {
                *stored = None;
            }
        }
    }

    pub(super) fn set_synthesis_only(&self) {
        self.phase.store(Self::SYNTHESIS, Ordering::SeqCst);
    }

    pub(super) fn evidence_collection(&self) -> bool {
        self.phase.load(Ordering::SeqCst) == Self::EVIDENCE
    }

    pub(super) fn report_only(&self) -> bool {
        self.phase.load(Ordering::SeqCst) == Self::REPORT
    }

    pub(super) fn synthesis_only(&self) -> bool {
        self.phase.load(Ordering::SeqCst) == Self::SYNTHESIS
    }

    pub(super) fn finalization_only(&self) -> bool {
        matches!(
            self.phase.load(Ordering::SeqCst),
            Self::REPORT | Self::SYNTHESIS
        )
    }

    pub(super) fn network_disabled(&self) -> bool {
        self.network_disabled.load(Ordering::SeqCst)
    }

    pub(super) fn report_artifact_path_is_safe(&self, args: &serde_json::Value) -> bool {
        let Some(path) = args.get("file_path").and_then(serde_json::Value::as_str) else {
            return false;
        };
        let relative = Path::new(path);
        if relative.is_absolute() {
            return false;
        }
        let components = relative.components().collect::<Vec<_>>();
        if components.len() != 4
            || components[0].as_os_str() != std::ffi::OsStr::new(".a3s")
            || components[1].as_os_str() != std::ffi::OsStr::new("research")
            || !matches!(components[2], std::path::Component::Normal(_))
            || !matches!(
                components[3].as_os_str().to_str(),
                Some("report.md" | "index.html")
            )
        {
            return false;
        }
        let Some(expected_slug) = self.expected_slug.read().ok().and_then(|slug| slug.clone())
        else {
            return false;
        };
        if components[2].as_os_str() != std::ffi::OsStr::new(&expected_slug) {
            return false;
        }
        let Some(root) = self
            .workspace
            .read()
            .ok()
            .and_then(|workspace| workspace.clone())
        else {
            return false;
        };

        let mut current = root;
        for (index, component) in components.iter().enumerate() {
            current.push(component.as_os_str());
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        return false;
                    }
                    let is_target = index + 1 == components.len();
                    if (!is_target && !metadata.is_dir()) || (is_target && !metadata.is_file()) {
                        return false;
                    }
                    #[cfg(unix)]
                    if is_target {
                        use std::os::unix::fs::MetadataExt;
                        if metadata.nlink() > 1 {
                            return false;
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return false,
            }
        }
        true
    }
}

pub(super) fn should_delay_deep_research_report_tool(
    deep_research_active: bool,
    gate: &DeepResearchReportToolGate,
) -> bool {
    deep_research_active && gate.report_only()
}

#[derive(Clone)]
pub(super) struct TuiHitlPermissionChecker {
    base: a3s_code_core::permissions::PermissionPolicy,
    deep_research_report_tool_gate: DeepResearchReportToolGate,
    execution_policy: TuiExecutionPolicy,
}

impl TuiHitlPermissionChecker {
    #[cfg(test)]
    pub(super) fn new(
        base: a3s_code_core::permissions::PermissionPolicy,
        deep_research_report_tool_gate: DeepResearchReportToolGate,
    ) -> Self {
        Self::with_execution(
            base,
            deep_research_report_tool_gate,
            TuiExecutionPolicy::default(),
        )
    }

    pub(super) fn with_execution(
        base: a3s_code_core::permissions::PermissionPolicy,
        deep_research_report_tool_gate: DeepResearchReportToolGate,
        execution_policy: TuiExecutionPolicy,
    ) -> Self {
        Self {
            base,
            deep_research_report_tool_gate,
            execution_policy,
        }
    }

    pub(super) fn check_tool(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> a3s_code_core::permissions::PermissionDecision {
        let base = self.base.check(tool_name, args);
        if matches!(base, a3s_code_core::permissions::PermissionDecision::Deny) {
            return base;
        }

        let evidence_collection = self.deep_research_report_tool_gate.evidence_collection();
        let report_only = self.deep_research_report_tool_gate.report_only();
        let tool = tool_name.to_ascii_lowercase();
        if self.deep_research_report_tool_gate.synthesis_only() {
            return a3s_code_core::permissions::PermissionDecision::Deny;
        }
        if report_only {
            return match tool.as_str() {
                "read" | "write" | "edit" => {
                    if self
                        .deep_research_report_tool_gate
                        .report_artifact_path_is_safe(args)
                    {
                        if tool == "read" {
                            base
                        } else {
                            a3s_code_core::permissions::PermissionDecision::Allow
                        }
                    } else {
                        a3s_code_core::permissions::PermissionDecision::Deny
                    }
                }
                _ => a3s_code_core::permissions::PermissionDecision::Deny,
            };
        }

        if self
            .deep_research_report_tool_gate
            .workspace()
            .is_some_and(|workspace| {
                let checker = a3s_code_core::permissions::InteractiveToolGuardrail::default()
                    .with_workspace(workspace);
                a3s_code_core::permissions::PermissionChecker::check(&checker, &tool, args)
                    == a3s_code_core::permissions::PermissionDecision::Deny
            })
        {
            return a3s_code_core::permissions::PermissionDecision::Deny;
        }
        if !evidence_collection {
            match self.execution_policy.mode() {
                Mode::Auto => {
                    return a3s_code_core::permissions::PermissionDecision::Allow;
                }
                Mode::Plan => {
                    return if plan_tool_is_read_only(&tool)
                        && matches!(base, a3s_code_core::permissions::PermissionDecision::Allow)
                    {
                        a3s_code_core::permissions::PermissionDecision::Allow
                    } else {
                        a3s_code_core::permissions::PermissionDecision::Deny
                    };
                }
                Mode::Default => {}
            }
        }
        let decision =
            a3s_code_core::permissions::InteractiveToolGuardrail::risk_decision(&tool, args);
        if !evidence_collection {
            return decision;
        }

        if self.deep_research_report_tool_gate.network_disabled()
            && matches!(tool.as_str(), "web_search" | "web_fetch")
        {
            return a3s_code_core::permissions::PermissionDecision::Deny;
        }
        if matches!(tool.as_str(), "bash" | "git") {
            return a3s_code_core::permissions::PermissionDecision::Deny;
        }
        if matches!(tool.as_str(), "write" | "edit") {
            return a3s_code_core::permissions::PermissionDecision::Deny;
        }
        if matches!(
            tool.as_str(),
            "parallel_task" | "dynamic_workflow" | "generate_object"
        ) && matches!(
            decision,
            a3s_code_core::permissions::PermissionDecision::Ask
        ) {
            return a3s_code_core::permissions::PermissionDecision::Allow;
        }

        // DeepResearch runs without interactive side effects. Anything that
        // normally needs confirmation is denied rather than silently approved.
        if matches!(
            decision,
            a3s_code_core::permissions::PermissionDecision::Ask
        ) {
            a3s_code_core::permissions::PermissionDecision::Deny
        } else {
            decision
        }
    }
}

impl a3s_code_core::permissions::PermissionChecker for TuiHitlPermissionChecker {
    fn expose_to_model(&self, tool_name: &str) -> bool {
        let tool = tool_name.to_ascii_lowercase();
        if self.deep_research_report_tool_gate.synthesis_only() {
            return false;
        }
        if self.deep_research_report_tool_gate.report_only() {
            return matches!(tool.as_str(), "read" | "write" | "edit");
        }
        if self.deep_research_report_tool_gate.evidence_collection() {
            return match tool.as_str() {
                "read" | "grep" | "glob" | "ls" => true,
                "web_search" | "web_fetch" => {
                    !self.deep_research_report_tool_gate.network_disabled()
                }
                _ => false,
            };
        }
        if self.execution_policy.mode() == Mode::Plan {
            return plan_tool_is_read_only(&tool);
        }
        true
    }

    fn check(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> a3s_code_core::permissions::PermissionDecision {
        self.check_tool(tool_name, args)
    }
}

pub(super) fn instant_from_epoch_ms(epoch_ms: u64) -> Instant {
    let now = Instant::now();
    if epoch_ms == 0 {
        return now;
    }
    let wall_now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(epoch_ms);
    let age_ms = wall_now_ms.saturating_sub(epoch_ms);
    now.checked_sub(Duration::from_millis(age_ms))
        .unwrap_or(now)
}

pub(super) fn touch_workspace_file_path_for_manifest(
    manifest: &LocalWorkspaceManifest,
    workspace: &str,
    path: &Path,
) {
    let root = Path::new(workspace);
    if let Ok(relative) = path.strip_prefix(root) {
        if let Some(path) = relative.to_str() {
            manifest.touch_file(path);
        }
    }
}

#[cfg(test)]
mod execution_policy_tests {
    use super::*;
    use a3s_code_core::hitl::ConfirmationProvider;
    use a3s_code_core::permissions::{PermissionChecker, PermissionDecision};

    fn checker(workspace: &Path, mode: Mode) -> (TuiHitlPermissionChecker, TuiExecutionPolicy) {
        let gate = DeepResearchReportToolGate::default();
        gate.set_workspace(workspace);
        let execution = TuiExecutionPolicy::new(mode);
        let checker = TuiHitlPermissionChecker::with_execution(
            tui_permission_policy(),
            gate,
            execution.clone(),
        );
        (checker, execution)
    }

    #[test]
    fn auto_mode_resolves_non_denied_tools_without_hitl() {
        let workspace = tempfile::tempdir().unwrap();
        let (checker, _) = checker(workspace.path(), Mode::Auto);

        for (tool, args) in [
            (
                "write",
                serde_json::json!({"file_path": "README.md", "content": "updated"}),
            ),
            ("bash", serde_json::json!({"command": "cargo test"})),
            (
                "task",
                serde_json::json!({"prompt": "inspect and implement the change"}),
            ),
            (
                "mcp__github__create_issue",
                serde_json::json!({"title": "tracked work"}),
            ),
        ] {
            assert_eq!(
                checker.check(tool, &args),
                PermissionDecision::Allow,
                "Auto must not enter HITL for {tool}"
            );
        }

        assert_eq!(
            checker.check("bash", &serde_json::json!({"command": "rm -rf /"})),
            PermissionDecision::Deny,
            "hard guardrails remain authoritative in Auto"
        );
    }

    #[test]
    fn execution_mode_is_shared_across_checker_clones() {
        let workspace = tempfile::tempdir().unwrap();
        let (checker, execution) = checker(workspace.path(), Mode::Default);
        let clone = checker.clone();
        let args = serde_json::json!({"command": "cargo test"});

        assert_eq!(checker.check("bash", &args), PermissionDecision::Ask);
        execution.set_mode(Mode::Auto);
        assert_eq!(checker.check("bash", &args), PermissionDecision::Allow);
        assert_eq!(clone.check("bash", &args), PermissionDecision::Allow);
    }

    #[test]
    fn plan_mode_is_read_only() {
        let workspace = tempfile::tempdir().unwrap();
        let (checker, _) = checker(workspace.path(), Mode::Plan);

        assert_eq!(
            checker.check("read", &serde_json::json!({"file_path": "README.md"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check(
                "write",
                &serde_json::json!({"file_path": "README.md", "content": "new"})
            ),
            PermissionDecision::Deny
        );
        assert!(!checker.expose_to_model("write"));
    }

    #[test]
    fn auto_confirmation_fallback_never_routes_to_hitl() {
        let workspace = tempfile::tempdir().unwrap();
        let execution = TuiExecutionPolicy::new(Mode::Auto);

        assert_eq!(
            execution.auto_confirmation_decision(
                "bash",
                &serde_json::json!({"command": "cargo test"}),
                workspace.path(),
            ),
            Some(true)
        );
        assert_eq!(
            execution.auto_confirmation_decision(
                "bash",
                &serde_json::json!({"command": "rm -rf /"}),
                workspace.path(),
            ),
            Some(false),
            "hard denials must be rejected automatically instead of opening HITL"
        );

        execution.set_mode(Mode::Default);
        assert_eq!(
            execution.auto_confirmation_decision(
                "bash",
                &serde_json::json!({"command": "cargo test"}),
                workspace.path(),
            ),
            None
        );
    }

    #[tokio::test]
    async fn auto_mode_bypasses_tool_owned_confirmation_escalation() {
        let execution = TuiExecutionPolicy::new(Mode::Default);
        let provider = TuiModeConfirmationProvider::new(
            a3s_code_core::hitl::ConfirmationPolicy::enabled(),
            execution.clone(),
        );

        assert!(
            provider
                .requires_confirmation("mcp__server__destructive")
                .await
        );

        execution.set_mode(Mode::Auto);
        assert!(
            !provider
                .requires_confirmation("mcp__server__destructive")
                .await
        );
        let response = provider
            .request_confirmation("tool-1", "mcp__server__destructive", &serde_json::json!({}))
            .await
            .await
            .expect("Auto confirmation should resolve immediately");
        assert!(response.approved);
        assert!(provider.pending_confirmations().await.is_empty());
    }

    #[tokio::test]
    async fn session_options_share_one_execution_policy_across_both_hitl_layers() {
        let execution = TuiExecutionPolicy::new(Mode::Default);
        let options = tui_session_options_with_gate_and_execution(
            a3s_code_core::hitl::ConfirmationPolicy::enabled(),
            DeepResearchReportToolGate::default(),
            execution.clone(),
        );
        let checker = options
            .permission_checker
            .expect("TUI options should install a permission checker");
        let confirmation = options
            .confirmation_manager
            .expect("TUI options should install a confirmation provider");
        let args = serde_json::json!({"command": "cargo test"});

        assert_eq!(checker.check("bash", &args), PermissionDecision::Ask);
        assert!(confirmation.requires_confirmation("bash").await);

        execution.set_mode(Mode::Auto);
        assert_eq!(checker.check("bash", &args), PermissionDecision::Allow);
        assert!(!confirmation.requires_confirmation("bash").await);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RuntimeEvidenceMode {
    Any,
    ParallelReportView,
}

#[derive(Clone)]
pub(super) struct RuntimeExpectation {
    label: String,
    policy: RuntimePolicy,
    evidence_mode: RuntimeEvidenceMode,
    runtime_tool: bool,
    parallel_work: bool,
    remote_view: bool,
    warned_missing: bool,
}

impl RuntimeExpectation {
    pub(super) fn required(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            policy: RuntimePolicy::Required,
            evidence_mode: RuntimeEvidenceMode::Any,
            runtime_tool: false,
            parallel_work: false,
            remote_view: false,
            warned_missing: false,
        }
    }

    pub(super) fn required_report_view(label: impl Into<String>) -> Self {
        Self {
            evidence_mode: RuntimeEvidenceMode::ParallelReportView,
            ..Self::required(label)
        }
    }

    pub(super) fn record_tool(&mut self, name: &str) {
        match name {
            "runtime" => self.runtime_tool = true,
            "dynamic_workflow" | "parallel_task" | "task" => self.parallel_work = true,
            _ => {}
        }
    }

    pub(super) fn record_parallel_work(&mut self) {
        self.parallel_work = true;
    }

    pub(super) fn record_remote_view(&mut self) {
        self.remote_view = true;
    }

    pub(super) fn has_parallel_evidence(&self) -> bool {
        self.runtime_tool || self.parallel_work
    }

    pub(super) fn is_satisfied(&self) -> bool {
        match self.evidence_mode {
            RuntimeEvidenceMode::Any => self.has_parallel_evidence() || self.remote_view,
            RuntimeEvidenceMode::ParallelReportView => {
                self.has_parallel_evidence() && self.remote_view
            }
        }
    }

    pub(super) fn missing_expectation(&self) -> String {
        match self.evidence_mode {
            RuntimeEvidenceMode::Any => {
                "expected `dynamic_workflow`, `runtime`, `parallel_task`, or an OS shaped `.view`/`viewUrl` response"
                    .to_string()
            }
            RuntimeEvidenceMode::ParallelReportView => match (self.has_parallel_evidence(), self.remote_view) {
                (false, false) => {
                    "expected `dynamic_workflow`/OS Runtime/`parallel_task` fan-out plus an OS shaped `.view`/`viewUrl` report response".to_string()
                }
                (false, true) => {
                    "expected `dynamic_workflow`/OS Runtime/`parallel_task` fan-out before the report view".to_string()
                }
                (true, false) => {
                    "expected an OS shaped `.view`/`viewUrl` response for the report".to_string()
                }
                (true, true) => unreachable!("satisfied expectations are filtered before warning"),
            },
        }
    }

    pub(super) fn missing_warning(&mut self) -> Option<String> {
        if self.policy != RuntimePolicy::Required || self.is_satisfied() || self.warned_missing {
            return None;
        }
        self.warned_missing = true;
        Some(format!(
            "  Runtime evidence missing for {} - {} before the final answer",
            self.label,
            self.missing_expectation()
        ))
    }

    pub(super) fn corrective_prompt(&self) -> Option<String> {
        if self.policy != RuntimePolicy::Required || self.is_satisfied() {
            return None;
        }
        Some(format!(
            "The previous turn ended without the required OS Runtime evidence for {}: {}. \
             Continue the same task, explicitly use `dynamic_workflow` first; inside it use \
             the signed-in `runtime` tool or a host-side `parallel_task` step as required, \
             create or surface the shaped OS `.view`/`viewUrl` report response when required, \
             and only then give the final answer. If the OS capability is unavailable, explain exactly \
             which OS endpoint or response field is missing and provide local report artifact paths.",
            self.label,
            self.missing_expectation()
        ))
    }
}
