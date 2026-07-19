use super::*;
use a3s_code_core::permissions::{PermissionChecker, PermissionDecision};

struct TestSandbox;

#[async_trait::async_trait]
impl a3s_code_core::sandbox::BashSandbox for TestSandbox {
    async fn exec_command(
        &self,
        _command: &str,
        _guest_workspace: &str,
    ) -> anyhow::Result<a3s_code_core::sandbox::SandboxOutput> {
        Ok(a3s_code_core::sandbox::SandboxOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    async fn shutdown(&self) {}
}

fn checker(
    workspace: &Path,
    mode: Mode,
    sandboxed: bool,
) -> (TuiHitlPermissionChecker, TuiExecutionPolicy) {
    let gate = DeepResearchReportToolGate::default();
    gate.set_workspace(workspace);
    let sandbox =
        sandboxed.then(|| Arc::new(TestSandbox) as Arc<dyn a3s_code_core::sandbox::BashSandbox>);
    let execution = TuiExecutionPolicy::for_workspace(mode, workspace.to_path_buf(), sandbox);
    let checker = TuiHitlPermissionChecker::with_execution_policy(
        tui_permission_policy(),
        gate,
        execution.clone(),
    );
    (checker, execution)
}

#[test]
fn default_mode_is_quiet_inside_enforced_boundaries() {
    let workspace = tempfile::tempdir().unwrap();
    let (checker, _) = checker(workspace.path(), Mode::Default, true);

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
            "Skill",
            serde_json::json!({"skill_name": "workspace-maintenance"}),
        ),
        (
            "mcp__github__get_issue",
            serde_json::json!({"issue_number": 1}),
        ),
    ] {
        assert_eq!(
            checker.check(tool, &args),
            PermissionDecision::Allow,
            "Default should not enter HITL for bounded {tool}"
        );
    }

    assert_eq!(
        checker.check(
            "bash",
            &serde_json::json!({
                "command": "cargo test",
                "sandbox_permissions": "require_escalated",
                "justification": "Needs an approved host capability."
            })
        ),
        PermissionDecision::Ask
    );
    assert_eq!(
        checker.check(
            "git",
            &serde_json::json!({"command": "checkout", "ref": "feature"})
        ),
        PermissionDecision::Ask
    );
    for path in [
        ".git/config",
        ".a3s/permissions.acl",
        ".codex/config",
        ".claude/settings.json",
        ".mcp.json",
    ] {
        assert_eq!(
            checker.check(
                "write",
                &serde_json::json!({"file_path": path, "content": "changed"})
            ),
            PermissionDecision::Ask,
            "Default should require explicit approval for {path}"
        );
    }
    assert_eq!(
        checker.check("bash", &serde_json::json!({"command": "rm -rf /"})),
        PermissionDecision::Deny
    );
}

#[test]
fn missing_process_sandbox_prompts_in_default_and_denies_in_auto() {
    let workspace = tempfile::tempdir().unwrap();
    let (default_checker, _) = checker(workspace.path(), Mode::Default, false);
    let (auto_checker, _) = checker(workspace.path(), Mode::Auto, false);

    assert_eq!(
        default_checker.check("bash", &serde_json::json!({"command": "cargo test"})),
        PermissionDecision::Ask
    );
    assert_eq!(
        auto_checker.check("bash", &serde_json::json!({"command": "cargo test"})),
        PermissionDecision::Deny
    );
    assert_eq!(
        auto_checker.check(
            "write",
            &serde_json::json!({"file_path": "README.md", "content": "updated"})
        ),
        PermissionDecision::Allow
    );
}

#[test]
fn auto_mode_never_returns_ask() {
    let workspace = tempfile::tempdir().unwrap();
    let (checker, _) = checker(workspace.path(), Mode::Auto, true);

    for (tool, args, expected) in [
        (
            "write",
            serde_json::json!({"file_path": "README.md", "content": "updated"}),
            PermissionDecision::Allow,
        ),
        (
            "bash",
            serde_json::json!({"command": "cargo test"}),
            PermissionDecision::Allow,
        ),
        (
            "task",
            serde_json::json!({"prompt": "implement the change"}),
            PermissionDecision::Allow,
        ),
        (
            "mcp__github__create_issue",
            serde_json::json!({"title": "tracked work"}),
            PermissionDecision::Allow,
        ),
        (
            "bash",
            serde_json::json!({
                "command": "cargo test",
                "sandbox_permissions": "require_escalated",
                "justification": "Needs the host."
            }),
            PermissionDecision::Deny,
        ),
        (
            "write",
            serde_json::json!({
                "file_path": ".a3s/permissions.acl",
                "content": "changed"
            }),
            PermissionDecision::Deny,
        ),
        (
            "git",
            serde_json::json!({"command": "checkout", "ref": "feature"}),
            PermissionDecision::Deny,
        ),
        ("runtime", serde_json::json!({}), PermissionDecision::Deny),
    ] {
        assert_eq!(checker.check(tool, &args), expected, "{tool}");
        assert_ne!(
            checker.check(tool, &args),
            PermissionDecision::Ask,
            "{tool}"
        );
    }
    assert_eq!(
        checker.check("git", &serde_json::json!({"command": "status"})),
        PermissionDecision::Allow
    );
}

#[test]
fn plan_mode_is_read_only() {
    let workspace = tempfile::tempdir().unwrap();
    let (checker, _) = checker(workspace.path(), Mode::Plan, true);

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
    assert_eq!(
        checker.check("bash", &serde_json::json!({"command": "pwd"})),
        PermissionDecision::Deny
    );
}

#[tokio::test]
async fn admitted_run_snapshots_do_not_follow_the_next_mode() {
    use a3s_code_core::hitl::ConfirmationProvider;

    let workspace = tempfile::tempdir().unwrap();
    let execution = TuiExecutionPolicy::for_workspace(
        Mode::Auto,
        workspace.path().to_path_buf(),
        Some(Arc::new(TestSandbox)),
    );
    let checker = TuiHitlPermissionChecker::with_execution_policy(
        tui_permission_policy(),
        DeepResearchReportToolGate::default(),
        execution.clone(),
    );
    let provider = TuiModeConfirmationProvider::new(
        a3s_code_core::hitl::ConfirmationPolicy::enabled(),
        execution.clone(),
    );
    let run_checker = checker.snapshot_for_run().unwrap();
    let run_provider = provider.snapshot_for_run().unwrap();
    let escalation = serde_json::json!({
        "command": "cargo test",
        "sandbox_permissions": "require_escalated",
        "justification": "Needs the host."
    });

    execution.set_mode(Mode::Default);

    assert_eq!(checker.check("bash", &escalation), PermissionDecision::Ask);
    assert_eq!(
        run_checker.check("bash", &escalation),
        PermissionDecision::Deny
    );
    assert!(
        provider
            .confirmation_available_for("bash", &escalation)
            .await
    );
    assert!(
        !run_provider
            .confirmation_available_for("bash", &escalation)
            .await
    );
}

#[tokio::test]
async fn auto_rejects_tool_owned_escalation_without_pending_hitl() {
    use a3s_code_core::hitl::ConfirmationProvider;

    let workspace = tempfile::tempdir().unwrap();
    let execution =
        TuiExecutionPolicy::for_workspace(Mode::Auto, workspace.path().to_path_buf(), None);
    let provider = TuiModeConfirmationProvider::new(
        a3s_code_core::hitl::ConfirmationPolicy::enabled(),
        execution,
    );

    assert!(
        !provider
            .confirmation_available_for("mcp__server__destructive", &serde_json::json!({}))
            .await
    );
    let response = provider
        .request_confirmation("tool-1", "mcp__server__destructive", &serde_json::json!({}))
        .await
        .await
        .unwrap();
    assert!(!response.approved);
    assert!(provider.pending_confirmations().await.is_empty());
}

#[tokio::test]
async fn terminal_confirmation_timeouts_are_always_rejections() {
    use a3s_code_core::hitl::{ConfirmationProvider, TimeoutAction};

    for mode in [Mode::Default, Mode::Plan, Mode::Auto] {
        let workspace = tempfile::tempdir().unwrap();
        let execution =
            TuiExecutionPolicy::for_workspace(mode, workspace.path().to_path_buf(), None);
        let provider = TuiModeConfirmationProvider::new(
            a3s_code_core::hitl::ConfirmationPolicy::enabled()
                .with_timeout(321, TimeoutAction::AutoApprove),
            execution,
        );
        let policy = provider.policy().await;
        assert_eq!(policy.default_timeout_ms, 321);
        assert_eq!(policy.timeout_action, TimeoutAction::Reject);
    }
}

#[tokio::test]
async fn session_options_share_execution_policy_across_both_layers() {
    let workspace = tempfile::tempdir().unwrap();
    let execution = TuiExecutionPolicy::for_workspace(
        Mode::Default,
        workspace.path().to_path_buf(),
        Some(Arc::new(TestSandbox)),
    );
    let options = tui_session_options_with_gate_and_execution(
        a3s_code_core::hitl::ConfirmationPolicy::enabled(),
        DeepResearchReportToolGate::default(),
        execution.clone(),
    );
    assert!(options.sandbox_handle.is_some());
    let checker = options.permission_checker.unwrap();
    let confirmation = options.confirmation_manager.unwrap();
    let args = serde_json::json!({"command": "cargo test"});

    assert_eq!(checker.check("bash", &args), PermissionDecision::Allow);
    assert!(confirmation.requires_confirmation("bash").await);

    execution.set_mode(Mode::Auto);
    assert_eq!(checker.check("bash", &args), PermissionDecision::Allow);
    assert!(!confirmation.confirmation_available_for("bash", &args).await);
}
