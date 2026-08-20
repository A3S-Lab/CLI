pub(super) const PERSISTED_CODE_READ_TOOLS: &[&str] = &[
    "code_symbols(*)",
    "code_navigation(*)",
    "code_diagnostics(*)",
];

pub(super) const PERSISTED_GOVERNED_TOOLS: &[&str] = &[
    "Bash(*)",
    "Git(*)",
    "batch(*)",
    "program(*)",
    "task(*)",
    "parallel_task(*)",
    "dynamic_workflow(*)",
    "Skill(*)",
];

pub(super) fn tool_allowed(tool_name: &str) -> bool {
    matches!(
        tool_name.to_ascii_lowercase().as_str(),
        "read"
            | "search"
            | "grep"
            | "bm25"
            | "glob"
            | "ls"
            | "code_symbols"
            | "code_navigation"
            | "code_diagnostics"
            | "generate_object"
            | "search_skills"
            | "write"
            | "edit"
            | "patch"
            | "bash"
            | "git"
            | "batch"
            | "program"
            | "task"
            | "parallel_task"
            | "dynamic_workflow"
            | "skill"
    )
}

pub(super) fn is_orchestration_tool(tool_name: &str) -> bool {
    matches!(
        tool_name.to_ascii_lowercase().as_str(),
        "batch" | "program" | "task" | "parallel_task" | "dynamic_workflow" | "skill"
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use a3s_code_core::permissions::PermissionDecision;
    use a3s_code_core::sandbox::{BashSandbox, SandboxOutput};
    use serde_json::json;

    use super::super::{session_options, session_options_with_sandbox};
    use crate::cli::args::{CodeMode, CodeToolPolicy};

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
    async fn local_workspace_profile_keeps_governed_coding_tools_offline() {
        let workspace = tempfile::tempdir().unwrap();
        let options = session_options_with_sandbox(
            CodeMode::Auto,
            CodeToolPolicy::LocalWorkspace,
            workspace.path(),
            "local-workspace-test",
            Some(Arc::new(TestSandbox)),
        );
        assert!(options.sandbox_handle.is_some());
        let checker = options.permission_checker.as_ref().unwrap();

        for tool in [
            "read",
            "search",
            "grep",
            "bm25",
            "glob",
            "ls",
            "code_symbols",
            "code_navigation",
            "code_diagnostics",
            "generate_object",
            "search_skills",
            "write",
            "edit",
            "patch",
            "bash",
            "git",
            "batch",
            "program",
            "task",
            "parallel_task",
            "dynamic_workflow",
            "Skill",
        ] {
            assert!(checker.expose_to_model(tool), "local profile hid {tool}");
        }
        for tool in [
            "web_search",
            "web_fetch",
            "download",
            "runtime",
            "use_knowledge_search",
            "use_tool_untrusted",
            "mcp__untrusted__write_host",
            "unknown_dynamic_tool",
        ] {
            assert!(
                !checker.expose_to_model(tool),
                "local profile exposed {tool}"
            );
            assert_eq!(
                checker.check(tool, &json!({})),
                PermissionDecision::Deny,
                "local profile admitted {tool}"
            );
        }

        assert_eq!(
            checker.check("read", &json!({"file_path": "src/main.rs"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("code_diagnostics", &json!({"path": "src"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("write", &json!({"file_path": "src/generated.rs"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("patch", &json!({"file_path": "src/main.rs"})),
            PermissionDecision::Allow
        );
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
                    "justification": "needs host network"
                }),
            ),
            PermissionDecision::Deny
        );
        assert_eq!(
            checker.check("git", &json!({"command": "checkout", "ref": "feature"})),
            PermissionDecision::Allow
        );
        for tool in [
            "batch",
            "program",
            "task",
            "parallel_task",
            "dynamic_workflow",
            "Skill",
        ] {
            assert_eq!(
                checker.check(tool, &json!({})),
                PermissionDecision::Allow,
                "local profile blocked governed orchestration tool {tool}"
            );
        }
        for protected in [".git/config", ".a3s/config.acl", ".vscode/settings.json"] {
            assert_eq!(
                checker.check("write", &json!({"file_path": protected})),
                PermissionDecision::Deny,
                "local profile admitted protected metadata {protected}"
            );
        }
        assert_eq!(
            checker.check("write", &json!({"file_path": "../outside.rs"})),
            PermissionDecision::Deny
        );

        let persisted = options.permission_policy.as_ref().unwrap();
        assert_eq!(persisted.default_decision, PermissionDecision::Deny);
        assert_eq!(
            persisted.check("read", &json!({"file_path": "src/main.rs"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            persisted.check("code_symbols", &json!({"path": "src"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            persisted.check("write", &json!({"file_path": "src/generated.rs"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            persisted.check("bash", &json!({"command": "cargo test"})),
            PermissionDecision::Ask
        );
        assert_eq!(persisted.check("task", &json!({})), PermissionDecision::Ask);
        for tool in [
            "web_search",
            "web_fetch",
            "download",
            "runtime",
            "use_knowledge_search",
            "use_tool_untrusted",
            "mcp__untrusted__write_host",
            "unknown_dynamic_tool",
        ] {
            assert_eq!(
                persisted.check(tool, &json!({})),
                PermissionDecision::Deny,
                "persisted local profile admitted {tool}"
            );
        }
    }

    #[tokio::test]
    async fn local_workspace_hides_bash_without_a_verified_sandbox() {
        let workspace = tempfile::tempdir().unwrap();
        let options = session_options(
            CodeMode::Auto,
            CodeToolPolicy::LocalWorkspace,
            workspace.path(),
            "local-workspace-no-sandbox-test",
        );
        let checker = options.permission_checker.as_ref().unwrap();
        assert!(!checker.expose_to_model("bash"));
        assert_eq!(
            checker.check("bash", &json!({"command": "cargo test"})),
            PermissionDecision::Deny
        );
    }
}
