use std::path::Path;
use std::sync::Arc;

use a3s_code_core::hitl::{ConfirmationPolicy, TimeoutAction};
use a3s_code_core::permissions::{InteractiveToolGuardrail, PermissionPolicy};
use a3s_code_core::{PlanningMode, SessionOptions};

use crate::cli::args::CodeMode;

pub(super) fn session_options(
    mode: CodeMode,
    workspace: &Path,
    session_id: &str,
) -> SessionOptions {
    let permission_policy = permission_policy();
    SessionOptions::new()
        .with_session_id(session_id)
        .with_planning_mode(planning_mode(mode))
        .with_confirmation_policy(
            ConfirmationPolicy::enabled().with_timeout(30_000, TimeoutAction::Reject),
        )
        .with_permission_policy(permission_policy)
        .with_permission_checker(Arc::new(
            InteractiveToolGuardrail::for_mode(mode_name(mode)).with_workspace(workspace),
        ))
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

fn permission_policy() -> PermissionPolicy {
    PermissionPolicy::new()
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
            "code_symbols(*)",
            "code_navigation(*)",
            "code_diagnostics(*)",
            "search_skills(*)",
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
            "runtime(*)",
        ])
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
        let options = session_options(CodeMode::Auto, workspace.path(), "exec-test");
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
            checker.check("bash", &json!({"command": "cargo test"})),
            PermissionDecision::Ask
        );
        assert_eq!(
            checker.check("bash", &json!({"command": "rm -rf /"})),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn default_and_plan_modes_never_silently_approve_writes() {
        let workspace = tempfile::tempdir().unwrap();
        for (mode, planning) in [
            (CodeMode::Default, PlanningMode::Disabled),
            (CodeMode::Plan, PlanningMode::Enabled),
        ] {
            let options = session_options(mode, workspace.path(), "exec-test");
            let checker = options
                .permission_checker
                .as_ref()
                .expect("exec must install a permission checker");

            assert_eq!(options.planning_mode, planning);
            assert_eq!(
                checker.check("write", &json!({"file_path": "answer.txt"})),
                PermissionDecision::Ask
            );
        }
    }
}
