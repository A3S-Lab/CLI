use std::path::Path;
use std::sync::Arc;

use a3s_code_core::hitl::{ConfirmationPolicy, TimeoutAction};
use a3s_code_core::permissions::{
    InteractiveToolGuardrail, PermissionChecker, PermissionDecision, PermissionPolicy,
};
use a3s_code_core::{ManifestWorkspaceBackend, PlanningMode, SessionOptions, WorkspaceServices};

use crate::cli::args::{CodeMode, CodeToolPolicy};
use crate::host_command_guardrail::{bash_boundary_decision, HostCommandMode};

struct ExecPermissionChecker {
    interactive: InteractiveToolGuardrail,
    host_mode: HostCommandMode,
    sandbox_available: bool,
    tool_policy: CodeToolPolicy,
}

impl PermissionChecker for ExecPermissionChecker {
    fn expose_to_model(&self, tool_name: &str) -> bool {
        tool_allowed(self.tool_policy, tool_name)
            && !(self.host_mode == HostCommandMode::Plan && tool_name.eq_ignore_ascii_case("bash"))
            && self.interactive.expose_to_model(tool_name)
    }

    fn check(&self, tool_name: &str, args: &serde_json::Value) -> PermissionDecision {
        if targets_protected_workspace_metadata(tool_name, args) {
            if self.host_mode == HostCommandMode::Default {
                PermissionDecision::Ask
            } else {
                PermissionDecision::Deny
            }
        } else if !tool_allowed(self.tool_policy, tool_name) {
            PermissionDecision::Deny
        } else if tool_name.eq_ignore_ascii_case("bash") {
            bash_boundary_decision(
                &self.interactive,
                self.host_mode,
                self.sandbox_available,
                args,
            )
        } else {
            self.interactive.check(tool_name, args)
        }
    }
}

#[cfg(test)]
pub(super) fn session_options(
    mode: CodeMode,
    tool_policy: CodeToolPolicy,
    workspace: &Path,
    session_id: &str,
) -> SessionOptions {
    session_options_with_sandbox(mode, tool_policy, workspace, session_id, None)
}

pub(super) fn session_options_with_sandbox(
    mode: CodeMode,
    tool_policy: CodeToolPolicy,
    workspace: &Path,
    session_id: &str,
    sandbox: Option<Arc<dyn a3s_code_core::sandbox::BashSandbox>>,
) -> SessionOptions {
    let permission_policy = permission_policy(tool_policy);
    let sandbox_available = sandbox.is_some();
    let workspace_backend = ManifestWorkspaceBackend::new_with_access_policy(
        workspace,
        a3s_code_core::workspace::LocalWorkspaceAccessPolicy::CredentialBoundary,
    );
    let options = SessionOptions::new()
        .with_session_id(session_id)
        .with_workspace_backend(WorkspaceServices::local_with_manifest_backend(
            workspace_backend,
        ))
        .with_planning_mode(planning_mode(mode))
        .with_confirmation_policy(
            ConfirmationPolicy::enabled().with_timeout(30_000, TimeoutAction::Reject),
        )
        .with_permission_policy(permission_policy)
        .with_permission_checker(Arc::new(ExecPermissionChecker {
            interactive: InteractiveToolGuardrail::for_mode(mode_name(mode))
                .with_workspace(workspace),
            host_mode: host_mode(mode),
            sandbox_available,
            tool_policy,
        }));
    match sandbox {
        Some(sandbox) => options.with_sandbox_handle(sandbox),
        None => options,
    }
}

pub(super) fn validate_tool_policy(
    mode: CodeMode,
    tool_policy: CodeToolPolicy,
) -> anyhow::Result<()> {
    if tool_policy == CodeToolPolicy::WorkspaceWrite && mode != CodeMode::Auto {
        return Err(crate::cli::output::usage_error(
            "--tool-policy workspace-write requires --mode auto",
        ));
    }
    Ok(())
}

fn planning_mode(mode: CodeMode) -> PlanningMode {
    match mode {
        CodeMode::Plan => PlanningMode::Enabled,
        CodeMode::Default => PlanningMode::Disabled,
        CodeMode::Auto => PlanningMode::Auto,
    }
}

fn mode_name(mode: CodeMode) -> &'static str {
    match mode {
        CodeMode::Plan => "plan",
        CodeMode::Default => "default",
        CodeMode::Auto => "auto",
    }
}

fn host_mode(mode: CodeMode) -> HostCommandMode {
    match mode {
        CodeMode::Default => HostCommandMode::Default,
        CodeMode::Plan => HostCommandMode::Plan,
        CodeMode::Auto => HostCommandMode::Auto,
    }
}

fn permission_policy(tool_policy: CodeToolPolicy) -> PermissionPolicy {
    if tool_policy == CodeToolPolicy::Standard {
        return PermissionPolicy::new()
            .deny_all(WORKSPACE_BOUNDARY_DENIES)
            .allow_all(STANDARD_READ_TOOLS)
            .ask_all(STANDARD_INTERACTIVE_TOOLS);
    }

    let mut closed = PermissionPolicy::new()
        .deny_all(WORKSPACE_BOUNDARY_DENIES)
        .allow_all(CLOSED_READ_TOOLS)
        .deny_all(&[
            "web_search(*)",
            "web_fetch(*)",
            "Bash(*)",
            "Git(*)",
            "batch(*)",
            "program(*)",
            "task(*)",
            "parallel_task(*)",
            "dynamic_workflow(*)",
            "Skill(*)",
            "runtime(*)",
            "download(*)",
            "use_knowledge_search(*)",
            "use_tool_*(*)",
            "mcp__*(*)",
        ]);
    closed.default_decision = PermissionDecision::Deny;
    match tool_policy {
        CodeToolPolicy::ReadOnly => closed.deny_all(&["Write(*)", "Edit(*)", "Patch(*)"]),
        // Patch path matching in legacy serialized policies is conservative, so
        // the persisted fallback asks. The live checker above remains the
        // authority and silently admits only a bounded, non-protected target.
        CodeToolPolicy::WorkspaceWrite => {
            closed.allow_all(&["Write(*)", "Edit(*)"]).ask("Patch(*)")
        }
        CodeToolPolicy::Standard => unreachable!(),
    }
}

const WORKSPACE_BOUNDARY_DENIES: &[&str] = &[
    "Read(/**)",
    "Read(**/../**)",
    "Search(** /**)",
    "Search(** **/../**)",
    "Grep(* /**)",
    "Grep(* **/../**)",
    "Bm25(* /**)",
    "Bm25(* **/../**)",
    "Glob(/**)",
    "Glob(**/../**)",
    "LS(/**)",
    "LS(**/../**)",
    "Write(/**)",
    "Edit(/**)",
    "Write(**/../**)",
    "Edit(**/../**)",
];

const STANDARD_READ_TOOLS: &[&str] = &[
    "Read(*)",
    "Search(*)",
    "Grep(*)",
    "Bm25(*)",
    "Glob(*)",
    "LS(*)",
    "web_search(*)",
    "web_fetch(*)",
    "code_symbols(*)",
    "code_navigation(*)",
    "code_diagnostics(*)",
    "search_skills(*)",
];

const CLOSED_READ_TOOLS: &[&str] = &[
    "Read(*)",
    "Search(*)",
    "Grep(*)",
    "Bm25(*)",
    "Glob(*)",
    "LS(*)",
    "generate_object(*)",
    "search_skills(*)",
];

const STANDARD_INTERACTIVE_TOOLS: &[&str] = &[
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
    "runtime(*)",
];

fn targets_protected_workspace_metadata(tool_name: &str, args: &serde_json::Value) -> bool {
    matches!(
        tool_name.to_ascii_lowercase().as_str(),
        "write" | "edit" | "patch"
    ) && args
        .get("file_path")
        .and_then(serde_json::Value::as_str)
        .is_some_and(a3s_code_core::sandbox::is_protected_workspace_path)
}

fn tool_allowed(policy: CodeToolPolicy, tool_name: &str) -> bool {
    if policy == CodeToolPolicy::Standard {
        return true;
    }
    let normalized = tool_name.to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "read" | "search" | "grep" | "bm25" | "glob" | "ls" | "generate_object" | "search_skills"
    ) || (policy == CodeToolPolicy::WorkspaceWrite
        && matches!(normalized.as_str(), "write" | "edit" | "patch"))
}

#[cfg(test)]
mod tests {
    use a3s_code_core::permissions::PermissionDecision;
    use a3s_code_core::sandbox::{BashSandbox, SandboxOutput};
    use a3s_code_core::PlanningMode;
    use serde_json::json;

    use super::*;

    struct TestSandbox;

    #[async_trait::async_trait]
    impl BashSandbox for TestSandbox {
        async fn exec_command(
            &self,
            _command: &str,
            _guest_workspace: &str,
        ) -> anyhow::Result<SandboxOutput> {
            Ok(SandboxOutput {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: 0,
            })
        }

        async fn shutdown(&self) {}
    }

    #[tokio::test]
    async fn verified_sandbox_is_attached_and_governs_standard_exec_bash() {
        let workspace = tempfile::tempdir().unwrap();
        for (mode, escalation) in [
            (CodeMode::Default, PermissionDecision::Ask),
            (CodeMode::Auto, PermissionDecision::Deny),
        ] {
            let options = session_options_with_sandbox(
                mode,
                CodeToolPolicy::Standard,
                workspace.path(),
                "sandboxed-exec-test",
                Some(Arc::new(TestSandbox)),
            );
            assert!(options.sandbox_handle.is_some());
            let checker = options.permission_checker.as_ref().unwrap();
            assert_eq!(
                checker.check("bash", &json!({"command": "cargo test"})),
                PermissionDecision::Allow
            );
            assert_eq!(
                checker.check(
                    "bash",
                    &json!({
                        "command": "cargo test",
                        "sandbox_permissions": "require_escalated",
                        "justification": "needs a host capability"
                    }),
                ),
                escalation
            );
            assert_eq!(
                checker.check("bash", &json!({"command": "rm -rf /"})),
                PermissionDecision::Deny
            );
        }
    }

    #[tokio::test]
    async fn auto_mode_allows_bounded_edits_but_preserves_the_safety_floor() {
        let workspace = tempfile::tempdir().unwrap();
        let options = session_options(
            CodeMode::Auto,
            CodeToolPolicy::Standard,
            workspace.path(),
            "exec-test",
        );
        let checker = options
            .permission_checker
            .as_ref()
            .expect("exec must install a permission checker");

        assert_eq!(options.planning_mode, PlanningMode::Auto);
        assert!(
            options
                .confirmation_policy
                .as_ref()
                .expect("exec must install a confirmation manager policy")
                .enabled
        );
        assert_eq!(
            checker.check("write", &json!({"file_path": "answer.txt"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("bash", &json!({"command": "pwd"})),
            PermissionDecision::Deny
        );
        assert_eq!(
            checker.check("bash", &json!({"command": "cargo test"})),
            PermissionDecision::Deny
        );
        assert_eq!(
            checker.check("bash", &json!({"command": "rm -rf /"})),
            PermissionDecision::Deny
        );
    }

    #[tokio::test]
    async fn default_and_plan_modes_preserve_their_interactive_boundaries() {
        let workspace = tempfile::tempdir().unwrap();
        for (mode, planning) in [
            (CodeMode::Default, PlanningMode::Disabled),
            (CodeMode::Plan, PlanningMode::Enabled),
        ] {
            let options = session_options(
                mode,
                CodeToolPolicy::Standard,
                workspace.path(),
                "exec-test",
            );
            let checker = options
                .permission_checker
                .as_ref()
                .expect("exec must install a permission checker");

            assert_eq!(options.planning_mode, planning);
            assert_eq!(
                checker.check("write", &json!({"file_path": "answer.txt"})),
                PermissionDecision::Ask
            );
            let expected_bash = match mode {
                CodeMode::Default => PermissionDecision::Ask,
                CodeMode::Plan => PermissionDecision::Deny,
                CodeMode::Auto => unreachable!(),
            };
            assert_eq!(
                checker.check("bash", &json!({"command": "pwd"})),
                expected_bash
            );
        }
    }

    #[tokio::test]
    async fn automation_profiles_are_closed_over_process_capable_tools() {
        let workspace = tempfile::tempdir().unwrap();
        for policy in [CodeToolPolicy::ReadOnly, CodeToolPolicy::WorkspaceWrite] {
            let options = session_options(CodeMode::Auto, policy, workspace.path(), "exec-test");
            let checker = options.permission_checker.as_ref().unwrap();

            assert!(checker.expose_to_model("read"));
            assert!(!checker.expose_to_model("web_fetch"));
            assert!(!checker.expose_to_model("code_diagnostics"));
            for tool in [
                "bash",
                "git",
                "task",
                "program",
                "runtime",
                "Skill",
                "mcp__untrusted__write_host",
            ] {
                assert!(!checker.expose_to_model(tool), "{policy:?} exposed {tool}");
                assert_eq!(
                    checker.check(tool, &json!({})),
                    PermissionDecision::Deny,
                    "{policy:?} admitted {tool}"
                );
            }
            let expected_write = if policy == CodeToolPolicy::WorkspaceWrite {
                PermissionDecision::Allow
            } else {
                PermissionDecision::Deny
            };
            assert_eq!(
                checker.check("write", &json!({"file_path": "answer.txt"})),
                expected_write
            );
            assert_eq!(
                checker.check("write", &json!({"file_path": "../answer.txt"})),
                PermissionDecision::Deny
            );
            assert_eq!(
                checker.check("patch", &json!({"file_path": "/etc/passwd"})),
                PermissionDecision::Deny
            );
            for protected in [".git/config", ".a3s/config.acl", ".vscode/settings.json"] {
                assert_eq!(
                    checker.check("write", &json!({"file_path": protected})),
                    PermissionDecision::Deny,
                    "{policy:?} admitted protected metadata {protected}"
                );
            }
        }
    }

    #[tokio::test]
    async fn persisted_automation_policy_is_closed_by_default() {
        let workspace = tempfile::tempdir().unwrap();
        for policy in [CodeToolPolicy::ReadOnly, CodeToolPolicy::WorkspaceWrite] {
            let options = session_options(CodeMode::Auto, policy, workspace.path(), "exec-test");
            let persisted = options.permission_policy.as_ref().unwrap();

            assert_eq!(persisted.default_decision, PermissionDecision::Deny);
            assert_eq!(
                persisted.check("read", &json!({"file_path": "src/main.rs"})),
                PermissionDecision::Allow
            );
            assert_eq!(
                persisted.check("web_fetch", &json!({"url": "https://example.com"})),
                PermissionDecision::Deny
            );
            assert_eq!(
                persisted.check("unknown_dynamic_tool", &json!({})),
                PermissionDecision::Deny
            );
            let expected_write = if policy == CodeToolPolicy::WorkspaceWrite {
                PermissionDecision::Allow
            } else {
                PermissionDecision::Deny
            };
            assert_eq!(
                persisted.check("write", &json!({"file_path": "answer.txt"})),
                expected_write
            );
            let expected_patch = if policy == CodeToolPolicy::WorkspaceWrite {
                PermissionDecision::Ask
            } else {
                PermissionDecision::Deny
            };
            assert_eq!(
                persisted.check("patch", &json!({"file_path": "answer.txt"})),
                expected_patch
            );
        }
    }

    #[test]
    fn workspace_write_requires_auto_mode() {
        assert!(validate_tool_policy(CodeMode::Default, CodeToolPolicy::WorkspaceWrite).is_err());
        assert!(validate_tool_policy(CodeMode::Plan, CodeToolPolicy::WorkspaceWrite).is_err());
        assert!(validate_tool_policy(CodeMode::Auto, CodeToolPolicy::WorkspaceWrite).is_ok());
    }
}
