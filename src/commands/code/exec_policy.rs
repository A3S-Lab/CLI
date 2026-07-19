use std::path::Path;
use std::sync::Arc;

use a3s_code_core::hitl::{ConfirmationPolicy, TimeoutAction};
use a3s_code_core::sandbox::BashSandbox;
use a3s_code_core::{PlanningMode, SessionOptions};

use crate::cli::args::CodeMode;

pub(super) fn session_options(
    mode: CodeMode,
    workspace: &Path,
    session_id: &str,
    sandbox: Option<Arc<dyn BashSandbox>>,
) -> SessionOptions {
    crate::tui::governed_code_session_options(
        mode_name(mode),
        workspace,
        sandbox,
        ConfirmationPolicy::enabled().with_timeout(30_000, TimeoutAction::Reject),
    )
    .with_session_id(session_id)
    .with_planning_mode(planning_mode(mode))
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

#[cfg(test)]
mod tests {
    use a3s_code_core::permissions::PermissionDecision;
    use a3s_code_core::PlanningMode;
    use serde_json::json;

    use super::*;

    #[test]
    fn auto_mode_allows_bounded_edits_but_preserves_the_safety_floor() {
        let workspace = tempfile::tempdir().unwrap();
        let options = session_options(
            CodeMode::Auto,
            workspace.path(),
            "exec-test",
            Some(Arc::new(TestSandbox)),
        );
        let checker = options
            .permission_checker
            .as_ref()
            .expect("exec must install a permission checker");

        assert_eq!(options.planning_mode, PlanningMode::Auto);
        assert!(
            options.confirmation_manager.is_some(),
            "exec must install the shared confirmation manager"
        );
        assert_eq!(
            checker.check("write", &json!({"file_path": "answer.txt"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("bash", &json!({"command": "cargo test"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("bash", &json!({"command": "rm -rf /"})),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn default_allows_bounded_writes_while_plan_denies_them() {
        let workspace = tempfile::tempdir().unwrap();
        for (mode, planning, expected) in [
            (
                CodeMode::Default,
                PlanningMode::Disabled,
                PermissionDecision::Allow,
            ),
            (
                CodeMode::Plan,
                PlanningMode::Enabled,
                PermissionDecision::Deny,
            ),
        ] {
            let options = session_options(mode, workspace.path(), "exec-test", None);
            let checker = options
                .permission_checker
                .as_ref()
                .expect("exec must install a permission checker");

            assert_eq!(options.planning_mode, planning);
            assert_eq!(
                checker.check("write", &json!({"file_path": "answer.txt"})),
                expected
            );
        }
    }

    #[tokio::test]
    async fn auto_without_a_process_sandbox_denies_bash_without_hitl() {
        let workspace = tempfile::tempdir().unwrap();
        let options = session_options(CodeMode::Auto, workspace.path(), "exec-test", None);
        let checker = options.permission_checker.unwrap();
        let confirmation = options.confirmation_manager.unwrap();
        let args = json!({"command": "cargo test"});

        assert_eq!(checker.check("bash", &args), PermissionDecision::Deny);
        assert!(!confirmation.confirmation_available_for("bash", &args).await);
    }

    struct TestSandbox;

    #[async_trait::async_trait]
    impl BashSandbox for TestSandbox {
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
}
