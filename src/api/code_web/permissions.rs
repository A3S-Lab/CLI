use a3s_code_core::hitl::{ConfirmationPolicy, TimeoutAction};
use a3s_code_core::permissions::{PermissionDecision, PermissionPolicy};

const HITL_CONFIRM_TIMEOUT_MS: u64 = 60 * 60 * 1000;

const READ_ONLY_TOOLS: &[&str] = &[
    "Read(*)",
    "Grep(*)",
    "Glob(*)",
    "LS(*)",
    "web_search(*)",
    "web_fetch(*)",
];

const MUTATING_TOOLS: &[&str] = &[
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

pub(in crate::api::code_web) fn permission_policy_for_mode(mode: &str) -> PermissionPolicy {
    match mode {
        "auto" => {
            let mut policy = PermissionPolicy::new();
            policy.enabled = false;
            policy
        }
        "plan" => {
            let mut policy = PermissionPolicy::new()
                .allow_all(READ_ONLY_TOOLS)
                .deny_all(MUTATING_TOOLS);
            policy.default_decision = PermissionDecision::Deny;
            policy
        }
        _ => PermissionPolicy::new()
            .allow_all(READ_ONLY_TOOLS)
            .allow_all(&[
                "Write(.a3s/research/**)",
                "Write(**/.a3s/research/**)",
                "Edit(.a3s/research/**)",
                "Edit(**/.a3s/research/**)",
            ])
            .ask_all(MUTATING_TOOLS),
    }
}

pub(in crate::api::code_web) fn confirmation_policy_for_mode(mode: &str) -> ConfirmationPolicy {
    if mode == "auto" {
        ConfirmationPolicy::default()
    } else {
        ConfirmationPolicy::enabled().with_timeout(HITL_CONFIRM_TIMEOUT_MS, TimeoutAction::Reject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_mode_reads_freely_and_asks_before_writes() {
        let policy = permission_policy_for_mode("default");
        assert_eq!(
            policy.check("read", &json!({ "file_path": "src/main.rs" })),
            PermissionDecision::Allow
        );
        assert_eq!(
            policy.check("write", &json!({ "file_path": "src/main.rs" })),
            PermissionDecision::Ask
        );
        assert!(confirmation_policy_for_mode("default").enabled);
    }

    #[test]
    fn plan_mode_denies_mutations_instead_of_waiting_for_confirmation() {
        let policy = permission_policy_for_mode("plan");
        assert_eq!(
            policy.check("grep", &json!({ "pattern": "TODO" })),
            PermissionDecision::Allow
        );
        assert_eq!(
            policy.check("bash", &json!({ "command": "cargo fmt" })),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn auto_mode_disables_both_permission_prompts_and_hitl_waits() {
        let policy = permission_policy_for_mode("auto");
        assert_eq!(
            policy.check("write", &json!({ "file_path": "src/main.rs" })),
            PermissionDecision::Allow
        );
        assert!(!confirmation_policy_for_mode("auto").enabled);
    }
}
