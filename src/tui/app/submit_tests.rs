//! Focused contracts for host-direct `!` shell submission.

use super::app_submit::{direct_shell_tool_args, next_direct_shell_call_id};
use super::TOOL_EXEC_TIMEOUT_MS;

#[test]
fn direct_shell_uses_the_exact_core_bash_contract() {
    let args = direct_shell_tool_args("pwd");

    assert_eq!(
        args,
        serde_json::json!({
            "command": "pwd",
            "timeout": TOOL_EXEC_TIMEOUT_MS,
        })
    );
}

#[test]
fn direct_shell_call_ids_are_unique_and_host_scoped() {
    let first = next_direct_shell_call_id();
    let second = next_direct_shell_call_id();
    let prefix = format!("host-bash-{}-", std::process::id());

    assert_ne!(first, second);
    assert!(first.starts_with(&prefix), "{first}");
    assert!(second.starts_with(&prefix), "{second}");
}
