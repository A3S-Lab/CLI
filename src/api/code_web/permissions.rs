use std::path::Path;

use a3s_code_core::hitl::{ConfirmationPolicy, TimeoutAction};
use a3s_code_core::permissions::{InteractiveToolGuardrail, PermissionPolicy};

const HITL_CONFIRM_TIMEOUT_MS: u64 = 60 * 60 * 1000;

const READ_ONLY_TOOLS: &[&str] = &[
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
    "generate_object(*)",
];

const INTERACTIVE_TOOLS: &[&str] = &[
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

pub(in crate::api::code_web) fn permission_policy_for_mode(_mode: &str) -> PermissionPolicy {
    // Keep a serializable fallback for persistence and delegated child runs.
    // The shared structured checker installed by `permission_checker_for_mode`
    // is authoritative for host-side execution.
    PermissionPolicy::new()
        .allow_all(READ_ONLY_TOOLS)
        .ask_all(INTERACTIVE_TOOLS)
}

pub(in crate::api::code_web) fn permission_checker_for_mode(
    mode: &str,
    workspace: &Path,
) -> InteractiveToolGuardrail {
    InteractiveToolGuardrail::for_mode(mode).with_workspace(workspace)
}

pub(in crate::api::code_web) fn confirmation_policy_for_mode(_mode: &str) -> ConfirmationPolicy {
    // Auto mode suppresses only bounded workspace prompts in the permission
    // checker. HITL stays enabled for shell, runtime, delegation, and unknown
    // integrations, while hard denials can never be bypassed.
    ConfirmationPolicy::enabled().with_timeout(HITL_CONFIRM_TIMEOUT_MS, TimeoutAction::Reject)
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_code_core::permissions::{
        PermissionChecker, PermissionDecision, ToolRiskAction, ToolRiskLevel,
    };
    use serde_json::json;

    fn checker(mode: &str) -> InteractiveToolGuardrail {
        permission_checker_for_mode(mode, Path::new("."))
    }

    #[test]
    fn default_mode_balances_low_risk_calls_and_side_effects() {
        let checker = checker("default");
        assert_eq!(
            checker.check("read", &json!({ "file_path": "src/main.rs" })),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("bash", &json!({ "command": "pwd" })),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("git", &json!({ "command": "status" })),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("write", &json!({ "file_path": "src/main.rs" })),
            PermissionDecision::Ask
        );
        assert!(confirmation_policy_for_mode("default").enabled);
    }

    #[test]
    fn plan_mode_keeps_reads_quiet_and_requires_explicit_escalation() {
        let checker = checker("plan");
        assert_eq!(
            checker.check("grep", &json!({ "pattern": "TODO" })),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("bash", &json!({ "command": "pwd" })),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("write", &json!({ "file_path": "src/main.rs" })),
            PermissionDecision::Ask
        );
        assert!(confirmation_policy_for_mode("plan").enabled);
    }

    #[test]
    fn auto_mode_streamlines_bounded_edits_but_keeps_safety_floor() {
        let checker = checker("auto");
        assert_eq!(
            checker.check("write", &json!({ "file_path": "src/main.rs" })),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("bash", &json!({ "command": "cargo test" })),
            PermissionDecision::Ask
        );
        assert_eq!(
            checker.check("runtime", &json!({ "tasks": ["external work"] })),
            PermissionDecision::Ask
        );
        assert_eq!(
            checker.check(
                "git",
                &json!({ "command": "checkout", "ref": "feature", "force": true })
            ),
            PermissionDecision::Ask
        );
        assert_eq!(
            checker.check("git", &json!({ "command": "unknown" })),
            PermissionDecision::Ask
        );
        assert_eq!(
            checker.check("bash", &json!({ "command": "cat *" })),
            PermissionDecision::Ask
        );
        for command in [
            "sort -o/tmp/a3s-hitl-bypass input.txt",
            "find . -fls output.txt",
            "sed w output.txt README.md",
            "sed e commands.txt",
        ] {
            assert_eq!(
                checker.check("bash", &json!({ "command": command })),
                PermissionDecision::Ask,
                "Web auto must retain HITL for shell side effects: {command}"
            );
        }
        for command in ["rg mkfs README.md", "cat docs/mkfs-guide.md"] {
            assert_eq!(
                checker.check("bash", &json!({ "command": command })),
                PermissionDecision::Allow,
                "read-only arguments must not be overblocked: {command}"
            );
        }
        assert_eq!(
            checker.check("bash", &json!({ "command": "mkfs /dev/disk9" })),
            PermissionDecision::Deny
        );
        assert_eq!(
            checker.check("bash", &json!({ "command": "rm -rf /" })),
            PermissionDecision::Deny
        );
        assert!(
            confirmation_policy_for_mode("auto").enabled,
            "auto mode must retain HITL for unbounded operations"
        );
    }

    #[test]
    fn web_auto_applies_selective_risk_routing() {
        let checker = checker("auto");
        for (tool, args, level, action, permission) in [
            (
                "read",
                json!({"file_path": "src/main.rs"}),
                ToolRiskLevel::Routine,
                ToolRiskAction::Allow,
                PermissionDecision::Allow,
            ),
            (
                "write",
                json!({"file_path": "src/main.rs"}),
                ToolRiskLevel::Bounded,
                ToolRiskAction::Allow,
                PermissionDecision::Allow,
            ),
            (
                "bash",
                json!({"command": "cargo test"}),
                ToolRiskLevel::High,
                ToolRiskAction::ReviewByLlm,
                PermissionDecision::Ask,
            ),
            (
                "bash",
                json!({"command": "rm -rf /"}),
                ToolRiskLevel::Critical,
                ToolRiskAction::RuleDeny,
                PermissionDecision::Deny,
            ),
        ] {
            assert_eq!(checker.assess(tool, &args).level, level);
            assert_eq!(checker.risk_action(tool, &args), action);
            assert_eq!(checker.check(tool, &args), permission);
        }

        assert!(
            confirmation_policy_for_mode("auto").enabled,
            "high risk must have an active HITL fallback"
        );
    }

    #[test]
    fn web_policy_persists_a_conservative_fallback_in_every_mode() {
        for mode in ["default", "plan", "auto"] {
            let policy = permission_policy_for_mode(mode);
            assert!(policy.enabled, "{mode} must not disable the policy");
            assert_eq!(
                policy.check("read", &json!({ "file_path": "src/main.rs" })),
                PermissionDecision::Allow
            );
            assert_eq!(
                policy.check("bash", &json!({ "command": "rm -rf /" })),
                PermissionDecision::Ask,
                "the serializable fallback must not silently allow side effects"
            );
        }
    }
}
