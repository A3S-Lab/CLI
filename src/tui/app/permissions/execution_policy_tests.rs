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
    let checker = TuiHitlPermissionChecker::with_grants_and_execution(
        tui_permission_policy(),
        gate,
        TuiPermissionGrants::default(),
        execution.clone(),
    );
    (checker, execution)
}

#[test]
fn default_mode_is_quiet_inside_the_enforced_workspace_boundary() {
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
fn default_mode_requires_hitl_when_the_process_sandbox_is_missing() {
    let workspace = tempfile::tempdir().unwrap();
    let (checker, _) = checker(workspace.path(), Mode::Default, false);

    assert_eq!(
        checker.check(
            "write",
            &serde_json::json!({"file_path": "README.md", "content": "updated"})
        ),
        PermissionDecision::Allow
    );
    assert_eq!(
        checker.check("bash", &serde_json::json!({"command": "cargo test"})),
        PermissionDecision::Ask
    );
}

#[test]
fn default_mode_can_remember_one_exact_protected_metadata_grant() {
    let workspace = tempfile::tempdir().unwrap();
    let gate = DeepResearchReportToolGate::default();
    gate.set_workspace(workspace.path());
    let invocation = serde_json::json!({
        "file_path": ".a3s/permissions.acl",
        "content": "permission { }"
    });
    let grants = TuiPermissionGrants::default();
    grants.allow_for_session(ExactPermissionGrant::from_invocation("write", &invocation));
    let checker = TuiHitlPermissionChecker::with_grants_and_execution(
        tui_permission_policy(),
        gate,
        grants,
        TuiExecutionPolicy::for_workspace(
            Mode::Default,
            workspace.path().to_path_buf(),
            Some(Arc::new(TestSandbox)),
        ),
    );

    assert_eq!(
        checker.check("write", &invocation),
        PermissionDecision::Allow
    );
    assert_eq!(
        checker.check(
            "write",
            &serde_json::json!({
                "file_path": ".a3s/config.acl",
                "content": "different target"
            })
        ),
        PermissionDecision::Ask,
        "the grant must retain the exact operation and protected resource"
    );
}

#[test]
fn auto_mode_resolves_non_denied_tools_without_hitl() {
    let workspace = tempfile::tempdir().unwrap();
    let (checker, _) = checker(workspace.path(), Mode::Auto, true);

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
    assert_eq!(
        checker.check(
            "bash",
            &serde_json::json!({
                "command": "cargo test",
                "sandbox_permissions": "require_escalated",
                "justification": "Needs the host."
            })
        ),
        PermissionDecision::Deny,
        "Auto must deny sandbox escape instead of entering HITL"
    );
    assert_eq!(
        checker.check(
            "write",
            &serde_json::json!({
                "file_path": ".a3s/permissions.acl",
                "content": "changed"
            })
        ),
        PermissionDecision::Deny,
        "Auto must fail closed for protected control metadata"
    );
    assert_eq!(
        checker.check(
            "git",
            &serde_json::json!({"command": "checkout", "ref": "feature"})
        ),
        PermissionDecision::Deny,
        "Auto must not write protected Git metadata outside the process sandbox"
    );
    assert_eq!(
        checker.check("git", &serde_json::json!({"command": "status"})),
        PermissionDecision::Allow,
        "read-only Git inspection remains quiet in Auto"
    );
    for tool in ["runtime", "agent_script", "unclassified_extension"] {
        assert_eq!(
            checker.check(tool, &serde_json::json!({})),
            PermissionDecision::Deny,
            "Auto must deny unbounded external tool {tool} without entering HITL"
        );
    }
}

#[test]
fn auto_mode_fails_closed_when_the_process_sandbox_is_missing() {
    let workspace = tempfile::tempdir().unwrap();
    let (checker, _) = checker(workspace.path(), Mode::Auto, false);

    assert_eq!(
        checker.check("bash", &serde_json::json!({"command": "cargo test"})),
        PermissionDecision::Deny
    );
    assert_eq!(
        checker.check(
            "write",
            &serde_json::json!({"file_path": "README.md", "content": "updated"})
        ),
        PermissionDecision::Allow
    );
}

#[test]
fn execution_mode_is_shared_across_checker_clones() {
    let workspace = tempfile::tempdir().unwrap();
    let (checker, execution) = checker(workspace.path(), Mode::Default, true);
    let clone = checker.clone();
    let args = serde_json::json!({
        "command": "cargo test",
        "sandbox_permissions": "require_escalated",
        "justification": "Needs the host."
    });

    assert_eq!(checker.check("bash", &args), PermissionDecision::Ask);
    execution.set_mode(Mode::Auto);
    assert_eq!(checker.check("bash", &args), PermissionDecision::Deny);
    assert_eq!(clone.check("bash", &args), PermissionDecision::Deny);
}

#[tokio::test]
async fn admitted_run_snapshots_do_not_follow_the_next_tui_mode() {
    use a3s_code_core::hitl::ConfirmationProvider;

    let workspace = tempfile::tempdir().unwrap();
    let execution = TuiExecutionPolicy::for_workspace(
        Mode::Auto,
        workspace.path().to_path_buf(),
        Some(Arc::new(TestSandbox)),
    );
    let checker = TuiHitlPermissionChecker::with_grants_and_execution(
        tui_permission_policy(),
        DeepResearchReportToolGate::default(),
        TuiPermissionGrants::default(),
        execution.clone(),
    );
    let provider = TuiModeConfirmationProvider::new(
        a3s_code_core::hitl::ConfirmationPolicy::enabled(),
        execution.clone(),
    );
    let run_checker = checker
        .snapshot_for_run()
        .expect("TUI checker must freeze the admitted run");
    let run_provider = provider
        .snapshot_for_run()
        .expect("TUI confirmation provider must freeze the admitted run");
    let escalation = serde_json::json!({
        "command": "cargo test",
        "sandbox_permissions": "require_escalated",
        "justification": "Needs the host."
    });

    execution.set_mode(Mode::Default);

    assert_eq!(checker.check("bash", &escalation), PermissionDecision::Ask);
    assert_eq!(
        run_checker.check("bash", &escalation),
        PermissionDecision::Deny,
        "an admitted Auto run must stay non-interactive after the TUI advances"
    );
    assert!(
        provider
            .confirmation_available_for("bash", &escalation)
            .await
    );
    assert!(
        !run_provider
            .confirmation_available_for("bash", &escalation)
            .await,
        "the same Auto run must not acquire a later Default HITL channel"
    );
}

#[tokio::test]
async fn run_confirmation_snapshot_keeps_session_settlement_reachable() {
    use a3s_code_core::hitl::ConfirmationProvider;

    let workspace = tempfile::tempdir().unwrap();
    let provider = TuiModeConfirmationProvider::new(
        a3s_code_core::hitl::ConfirmationPolicy::enabled(),
        TuiExecutionPolicy::for_workspace(
            Mode::Default,
            workspace.path().to_path_buf(),
            Some(Arc::new(TestSandbox)),
        ),
    );
    let run_provider = provider.snapshot_for_run().unwrap();
    let response = run_provider
        .request_confirmation(
            "snapshot-confirmation",
            "bash",
            &serde_json::json!({"command": "host command"}),
        )
        .await;

    assert_eq!(provider.pending_confirmations().await.len(), 1);
    assert!(provider
        .confirm("snapshot-confirmation", true, None)
        .await
        .unwrap());
    assert!(response.await.unwrap().approved);
}

#[test]
fn auto_confirmation_fallback_fails_closed_without_hitl() {
    let workspace = tempfile::tempdir().unwrap();
    let execution = TuiExecutionPolicy::for_workspace(
        Mode::Auto,
        workspace.path().to_path_buf(),
        Some(Arc::new(TestSandbox)),
    );

    assert_eq!(
        execution.auto_confirmation_decision(
            "bash",
            &serde_json::json!({"command": "cargo test"}),
            workspace.path(),
        ),
        Some(false),
        "an unexpected confirmation event is an escalation in Auto"
    );
    assert_eq!(
        execution.auto_confirmation_decision(
            "bash",
            &serde_json::json!({
                "command": "cargo test",
                "sandbox_permissions": "require_escalated",
                "justification": "Needs the host."
            }),
            workspace.path(),
        ),
        Some(false)
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

    let unavailable =
        TuiExecutionPolicy::for_workspace(Mode::Auto, workspace.path().to_path_buf(), None);
    assert_eq!(
        unavailable.auto_confirmation_decision(
            "bash",
            &serde_json::json!({"command": "cargo test"}),
            workspace.path(),
        ),
        Some(false)
    );
}

#[test]
fn plan_mode_is_read_only_even_with_session_grants() {
    let workspace = tempfile::tempdir().unwrap();
    let gate = DeepResearchReportToolGate::default();
    gate.set_workspace(workspace.path());
    let grants = TuiPermissionGrants::default();
    grants.allow_for_session(ExactPermissionGrant::from_invocation(
        "write",
        &serde_json::json!({"file_path": "README.md", "content": "old"}),
    ));
    let checker = TuiHitlPermissionChecker::with_grants_and_execution(
        tui_permission_policy(),
        gate,
        grants,
        TuiExecutionPolicy::for_workspace(
            Mode::Plan,
            workspace.path().to_path_buf(),
            Some(Arc::new(TestSandbox)),
        ),
    );

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
async fn auto_mode_rejects_tool_owned_confirmation_escalation() {
    use a3s_code_core::hitl::ConfirmationProvider;

    let workspace = tempfile::tempdir().unwrap();
    let execution = TuiExecutionPolicy::for_workspace(
        Mode::Default,
        workspace.path().to_path_buf(),
        Some(Arc::new(TestSandbox)),
    );
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
        provider
            .requires_confirmation("mcp__server__destructive")
            .await
    );
    assert!(
        !provider
            .confirmation_available_for("mcp__server__destructive", &serde_json::json!({}))
            .await,
        "Auto must fail closed before Core emits a confirmation event"
    );
    let response = provider
        .request_confirmation("tool-1", "mcp__server__destructive", &serde_json::json!({}))
        .await
        .await
        .expect("Auto rejection should resolve immediately");
    assert!(!response.approved);
    let response = provider
        .request_confirmation(
            "tool-2",
            "bash",
            &serde_json::json!({
                "command": "cargo test",
                "sandbox_permissions": "require_escalated",
                "justification": "Needs the host."
            }),
        )
        .await
        .await
        .expect("Auto escalation should resolve immediately");
    assert!(!response.approved);
    assert!(provider.pending_confirmations().await.is_empty());
}

#[tokio::test]
async fn tui_confirmation_timeouts_are_always_rejections() {
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
        assert_eq!(
            policy.timeout_action,
            TimeoutAction::Reject,
            "a terminal mode must never infer approval from silence"
        );
    }
}

#[tokio::test]
async fn terminal_confirmation_updates_and_expiry_cannot_manufacture_consent() {
    use a3s_code_core::hitl::{ConfirmationProvider, TimeoutAction};

    let workspace = tempfile::tempdir().unwrap();
    let execution =
        TuiExecutionPolicy::for_workspace(Mode::Default, workspace.path().to_path_buf(), None);
    let provider = TuiModeConfirmationProvider::new(
        a3s_code_core::hitl::ConfirmationPolicy::enabled(),
        execution,
    );
    provider
        .set_policy(
            a3s_code_core::hitl::ConfirmationPolicy::enabled()
                .with_timeout(654, TimeoutAction::AutoApprove),
        )
        .await;
    let policy = provider.policy().await;
    assert_eq!(policy.default_timeout_ms, 654);
    assert_eq!(policy.timeout_action, TimeoutAction::Reject);

    let response = provider
        .request_confirmation("tool-expire", "runtime", &serde_json::json!({}))
        .await;
    assert!(
        provider
            .expire("tool-expire", TimeoutAction::AutoApprove)
            .await
    );
    let response = response
        .await
        .expect("expiry must settle the exact request");
    assert!(!response.approved);
    assert!(response
        .reason
        .as_deref()
        .is_some_and(|reason| reason.contains("rejected")));
}

#[tokio::test]
async fn session_options_share_one_execution_policy_across_both_hitl_layers() {
    let workspace = tempfile::tempdir().unwrap();
    let execution = TuiExecutionPolicy::for_workspace(
        Mode::Default,
        workspace.path().to_path_buf(),
        Some(Arc::new(TestSandbox)),
    );
    let options = tui_session_options_with_gate_grants_and_execution(
        a3s_code_core::hitl::ConfirmationPolicy::enabled(),
        DeepResearchReportToolGate::default(),
        TuiPermissionGrants::default(),
        execution.clone(),
    );
    assert!(options.sandbox_handle.is_some());
    let checker = options
        .permission_checker
        .expect("TUI options should install a permission checker");
    let confirmation = options
        .confirmation_manager
        .expect("TUI options should install a confirmation provider");
    let args = serde_json::json!({"command": "cargo test"});

    assert_eq!(checker.check("bash", &args), PermissionDecision::Allow);
    assert!(confirmation.requires_confirmation("bash").await);

    execution.set_mode(Mode::Auto);
    assert_eq!(checker.check("bash", &args), PermissionDecision::Allow);
    assert!(confirmation.requires_confirmation("bash").await);
}
