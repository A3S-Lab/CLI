//! Shared host Bash policy for Code entry points.
//!
//! The Rust risk classifier proves a deliberately small read-only shell subset.
//! This adapter maps that assessment onto interactive execution modes without
//! treating a lexical guardrail as an operating-system isolation boundary.

use a3s_code_core::permissions::{InteractiveToolGuardrail, PermissionDecision};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostCommandMode {
    Default,
    Plan,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostBoundaryRequest {
    UseDefault,
    RequireEscalated,
    Invalid,
}

/// Decide whether one Bash invocation may use the configured execution boundary.
///
/// `sandbox_permissions` remains part of the Bash tool's compatibility schema,
/// and `require_escalated` always means an explicit host-boundary request:
/// Default asks and non-interactive modes deny it. A verified process sandbox
/// quietly admits ordinary workspace commands; without one, Default requires
/// approval for every non-denied host command and non-interactive modes deny.
pub(crate) fn bash_boundary_decision(
    _guardrail: &InteractiveToolGuardrail,
    mode: HostCommandMode,
    sandbox_available: bool,
    args: &serde_json::Value,
) -> PermissionDecision {
    if references_protected_path(args) {
        return PermissionDecision::Deny;
    }
    let command = match args.get("command").and_then(serde_json::Value::as_str) {
        Some(command) if !command.trim().is_empty() => command,
        _ => return PermissionDecision::Deny,
    };
    if InteractiveToolGuardrail::is_catastrophic_bash_command(command) {
        return PermissionDecision::Deny;
    }
    let request = host_boundary_request(args);
    if request == HostBoundaryRequest::Invalid {
        return PermissionDecision::Deny;
    }
    if request == HostBoundaryRequest::UseDefault && sandbox_available {
        return match mode {
            HostCommandMode::Plan => PermissionDecision::Deny,
            HostCommandMode::Default | HostCommandMode::Auto => PermissionDecision::Allow,
        };
    }
    match (request, mode) {
        (HostBoundaryRequest::Invalid, _) => PermissionDecision::Deny,
        (HostBoundaryRequest::RequireEscalated, HostCommandMode::Default) => {
            PermissionDecision::Ask
        }
        (HostBoundaryRequest::RequireEscalated, _) => PermissionDecision::Deny,
        (HostBoundaryRequest::UseDefault, HostCommandMode::Plan) => PermissionDecision::Deny,
        (HostBoundaryRequest::UseDefault, HostCommandMode::Auto) => PermissionDecision::Deny,
        (HostBoundaryRequest::UseDefault, HostCommandMode::Default) => PermissionDecision::Ask,
    }
}

#[cfg(test)]
fn host_bash_decision(
    guardrail: &InteractiveToolGuardrail,
    mode: HostCommandMode,
    args: &serde_json::Value,
) -> PermissionDecision {
    bash_boundary_decision(guardrail, mode, false, args)
}

fn references_protected_path(args: &serde_json::Value) -> bool {
    args.get("command")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|command| {
            command
                .split_whitespace()
                .map(clean_shell_token)
                .any(is_protected_path_token)
        })
}

fn is_protected_path_token(token: &str) -> bool {
    let normalized = token.replace('\\', "/");
    if is_normalized_protected_path_token(&normalized) {
        return true;
    }

    // Bash can hide a protected name with escapes (`.e\\nv`) or empty
    // quoting (`.e''nv`). Inspect a conservative de-obfuscated spelling as
    // well as the original token. False positives fail closed; this path is a
    // non-bypassable credential/control-plane floor, not the quiet allow-list.
    let deobfuscated = token
        .chars()
        .filter(|character| !matches!(character, '\\' | '\'' | '"' | '$' | '{' | '}'))
        .collect::<String>();
    deobfuscated != token && is_normalized_protected_path_token(&deobfuscated.replace('\\', "/"))
}

fn is_normalized_protected_path_token(normalized: &str) -> bool {
    let normalized = normalized.trim_start_matches("./");
    if normalized.is_empty() {
        return false;
    }
    if let Some((_, value)) = normalized.split_once('=') {
        if is_protected_path_token(value) {
            return true;
        }
    }
    if a3s_code_core::sandbox::is_protected_workspace_path(normalized) {
        return true;
    }

    normalized.split('/').any(|component| {
        let component = component.to_ascii_lowercase();
        component == ".env"
            || component.starts_with(".env.")
            || component.starts_with(".env-")
            || matches!(
                component.as_str(),
                ".ssh"
                    | ".gnupg"
                    | ".git-credentials"
                    | ".netrc"
                    | ".npmrc"
                    | ".pypirc"
                    | ".credentials.json"
                    | "credentials.toml"
                    | "id_rsa"
                    | "id_ed25519"
            )
    })
}

fn clean_shell_token(token: &str) -> &str {
    token.trim_matches(|character: char| {
        matches!(
            character,
            '\'' | '"' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ':'
        )
    })
}

fn host_boundary_request(args: &serde_json::Value) -> HostBoundaryRequest {
    match args.get("sandbox_permissions") {
        None => HostBoundaryRequest::UseDefault,
        Some(serde_json::Value::String(value)) if value == "use_default" => {
            HostBoundaryRequest::UseDefault
        }
        Some(serde_json::Value::String(value)) if value == "require_escalated" => {
            HostBoundaryRequest::RequireEscalated
        }
        Some(_) => HostBoundaryRequest::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn guardrail() -> InteractiveToolGuardrail {
        InteractiveToolGuardrail::default().with_workspace(".")
    }

    #[test]
    fn missing_sandbox_requires_approval_for_all_non_denied_default_commands() {
        let guardrail = guardrail();
        for command in ["pwd", "ls -la", "git --no-pager diff -- README.md"] {
            assert_eq!(
                host_bash_decision(
                    &guardrail,
                    HostCommandMode::Default,
                    &json!({"command": command}),
                ),
                PermissionDecision::Ask,
                "host execution must require approval without a sandbox: {command}"
            );
        }

        for command in [
            "cargo test",
            "printf result > output.txt",
            "cat *",
            "echo $(date)",
            "git -C .. status",
            "git diff",
            "git diff --check",
            "git remote -v",
            "rg --hidden TOKEN .",
            "cat docs\\guide.md",
        ] {
            assert_eq!(
                host_bash_decision(
                    &guardrail,
                    HostCommandMode::Default,
                    &json!({"command": command}),
                ),
                PermissionDecision::Ask,
                "unproven host command must retain HITL: {command}"
            );
        }
    }

    #[test]
    fn non_interactive_modes_fail_closed_without_a_sandbox() {
        let guardrail = guardrail();
        for mode in [HostCommandMode::Plan, HostCommandMode::Auto] {
            assert_eq!(
                host_bash_decision(&guardrail, mode, &json!({"command": "cargo test"}),),
                PermissionDecision::Deny
            );
        }
        assert_eq!(
            host_bash_decision(
                &guardrail,
                HostCommandMode::Plan,
                &json!({"command": "pwd"}),
            ),
            PermissionDecision::Deny
        );
        assert_eq!(
            host_bash_decision(
                &guardrail,
                HostCommandMode::Auto,
                &json!({"command": "pwd"}),
            ),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn verified_sandbox_quietly_admits_default_and_auto_commands() {
        let guardrail = guardrail();
        for mode in [HostCommandMode::Default, HostCommandMode::Auto] {
            for command in ["pwd", "cargo test", "printf result > output.txt"] {
                assert_eq!(
                    bash_boundary_decision(&guardrail, mode, true, &json!({"command": command}),),
                    PermissionDecision::Allow,
                    "verified sandbox should govern routine work: {command}"
                );
            }
        }
        assert_eq!(
            bash_boundary_decision(
                &guardrail,
                HostCommandMode::Default,
                true,
                &json!({
                    "command": "cargo test",
                    "sandbox_permissions": "require_escalated",
                    "justification": "requires a host capability"
                }),
            ),
            PermissionDecision::Ask
        );
        assert_eq!(
            bash_boundary_decision(
                &guardrail,
                HostCommandMode::Auto,
                true,
                &json!({
                    "command": "cargo test",
                    "sandbox_permissions": "require_escalated",
                    "justification": "requires a host capability"
                }),
            ),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn credential_and_control_paths_cannot_bypass_workspace_file_policy() {
        let guardrail = guardrail();
        for command in [
            "cat .env",
            "head services/api/.env.production",
            "cat .git/config",
            "cat .a3s/config.acl",
            "rg --file=.env TOKEN .",
            "cat ~/.ssh/id_ed25519",
            "cat .e\\nv",
            "cat .e''nv",
            "cat $'.env'",
        ] {
            assert_eq!(
                host_bash_decision(
                    &guardrail,
                    HostCommandMode::Default,
                    &json!({"command": command}),
                ),
                PermissionDecision::Deny,
                "credential and control paths must not be readable through Bash: {command}"
            );
        }
    }

    #[test]
    fn explicit_escalation_invalid_metadata_and_catastrophic_commands_fail_closed() {
        let guardrail = guardrail();
        assert_eq!(
            host_bash_decision(
                &guardrail,
                HostCommandMode::Default,
                &json!({
                    "command": "pwd",
                    "sandbox_permissions": "require_escalated",
                    "justification": "needs a host capability"
                }),
            ),
            PermissionDecision::Ask
        );
        for mode in [HostCommandMode::Plan, HostCommandMode::Auto] {
            assert_eq!(
                host_bash_decision(
                    &guardrail,
                    mode,
                    &json!({
                        "command": "pwd",
                        "sandbox_permissions": "require_escalated"
                    }),
                ),
                PermissionDecision::Deny
            );
        }
        assert_eq!(
            host_bash_decision(
                &guardrail,
                HostCommandMode::Default,
                &json!({"command": "pwd", "sandbox_permissions": 7}),
            ),
            PermissionDecision::Deny
        );
        assert_eq!(
            host_bash_decision(
                &guardrail,
                HostCommandMode::Default,
                &json!({"command": "rm -rf /"}),
            ),
            PermissionDecision::Deny
        );
    }

    #[cfg(unix)]
    #[test]
    fn existing_workspace_symlinks_are_never_silently_followed() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), workspace.path().join("escape")).unwrap();
        let guardrail = InteractiveToolGuardrail::default().with_workspace(workspace.path());

        for mode in [HostCommandMode::Default, HostCommandMode::Auto] {
            assert_eq!(
                host_bash_decision(
                    &guardrail,
                    mode,
                    &json!({"command": "cat escape/secret.txt"}),
                ),
                PermissionDecision::Deny
            );
        }
    }
}
