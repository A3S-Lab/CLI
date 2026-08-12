use std::path::Path;
use std::path::PathBuf;
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
    workspace: PathBuf,
    scheduled_policy: Option<crate::code_schedule::ScheduledExecutionPolicy>,
}

impl PermissionChecker for ExecPermissionChecker {
    fn expose_to_model(&self, tool_name: &str) -> bool {
        tool_allowed(self.tool_policy, tool_name)
            && !(self.host_mode == HostCommandMode::Plan && tool_name.eq_ignore_ascii_case("bash"))
            && self.interactive.expose_to_model(tool_name)
    }

    fn check(&self, tool_name: &str, args: &serde_json::Value) -> PermissionDecision {
        if self.tool_policy == CodeToolPolicy::ScheduledReport {
            let Some(policy) = self.scheduled_policy.as_ref() else {
                return PermissionDecision::Deny;
            };
            if tool_name.eq_ignore_ascii_case("git") {
                return if scheduled_git_is_read_only(args) {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Deny
                };
            }
            if matches!(
                tool_name.to_ascii_lowercase().as_str(),
                "write" | "edit" | "patch"
            ) {
                if !crate::code_schedule::is_scheduled_loop_artifact(
                    &self.workspace,
                    &policy.loop_id,
                    args,
                ) {
                    return PermissionDecision::Deny;
                }
                return PermissionDecision::Allow;
            }
            if !scheduled_read_is_allowed(
                &self.workspace,
                tool_name,
                args,
                &policy.denylist,
                &policy.protected_config_path,
            ) {
                return PermissionDecision::Deny;
            }
            return PermissionDecision::Allow;
        }
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

#[cfg(test)]
pub(super) fn session_options_with_sandbox(
    mode: CodeMode,
    tool_policy: CodeToolPolicy,
    workspace: &Path,
    session_id: &str,
    sandbox: Option<Arc<dyn a3s_code_core::sandbox::BashSandbox>>,
) -> SessionOptions {
    session_options_with_sandbox_and_schedule(
        mode,
        tool_policy,
        workspace,
        session_id,
        sandbox,
        None,
    )
}

pub(super) fn session_options_with_sandbox_and_schedule(
    mode: CodeMode,
    tool_policy: CodeToolPolicy,
    workspace: &Path,
    session_id: &str,
    sandbox: Option<Arc<dyn a3s_code_core::sandbox::BashSandbox>>,
    scheduled_policy: Option<crate::code_schedule::ScheduledExecutionPolicy>,
) -> SessionOptions {
    let permission_policy = permission_policy(tool_policy);
    let sandbox_available = sandbox.is_some();
    let workspace_backend = ManifestWorkspaceBackend::new_with_access_policy(
        workspace,
        a3s_code_core::workspace::LocalWorkspaceAccessPolicy::CredentialBoundary,
    );
    let max_tool_rounds = scheduled_policy
        .as_ref()
        .map(|policy| policy.max_tool_rounds);
    let mut options = SessionOptions::new()
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
            workspace: workspace.to_path_buf(),
            scheduled_policy,
        }));
    if let Some(max_tool_rounds) = max_tool_rounds {
        options = options.with_max_tool_rounds(max_tool_rounds);
    }
    match sandbox {
        Some(sandbox) => options.with_sandbox_handle(sandbox),
        None => options,
    }
}

pub(super) fn validate_tool_policy(
    mode: CodeMode,
    tool_policy: CodeToolPolicy,
) -> anyhow::Result<()> {
    if matches!(
        tool_policy,
        CodeToolPolicy::WorkspaceWrite | CodeToolPolicy::ScheduledReport
    ) && mode != CodeMode::Auto
    {
        return Err(crate::cli::output::usage_error(
            "write-capable closed tool policies require --mode auto",
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
        .allow_all(CLOSED_BASIC_READ_TOOLS)
        .deny_all(&[
            "web_search(*)",
            "web_fetch(*)",
            "Bash(*)",
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
        CodeToolPolicy::ReadOnly => closed
            .allow_all(CLOSED_LOCAL_HELPER_TOOLS)
            .deny_all(&["Write(*)", "Edit(*)", "Patch(*)"]),
        // Patch path matching in legacy serialized policies is conservative, so
        // the persisted fallback asks. The live checker above remains the
        // authority and silently admits only a bounded, non-protected target.
        CodeToolPolicy::WorkspaceWrite => closed
            .allow_all(CLOSED_LOCAL_HELPER_TOOLS)
            .deny("Git(*)")
            .allow_all(&["Write(*)", "Edit(*)"])
            .ask("Patch(*)"),
        CodeToolPolicy::ScheduledReport => closed
            .allow_all(&["Git(*)", "Write(*)", "Edit(*)"])
            .ask("Patch(*)"),
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

const CLOSED_BASIC_READ_TOOLS: &[&str] = &[
    "Read(*)",
    "Search(*)",
    "Grep(*)",
    "Bm25(*)",
    "Glob(*)",
    "LS(*)",
];

const CLOSED_LOCAL_HELPER_TOOLS: &[&str] = &["generate_object(*)", "search_skills(*)"];

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
    let basic_read = matches!(
        normalized.as_str(),
        "read" | "search" | "grep" | "bm25" | "glob" | "ls"
    );
    basic_read
        || (policy != CodeToolPolicy::ScheduledReport
            && matches!(normalized.as_str(), "generate_object" | "search_skills"))
        || (matches!(
            policy,
            CodeToolPolicy::WorkspaceWrite | CodeToolPolicy::ScheduledReport
        ) && matches!(normalized.as_str(), "write" | "edit" | "patch"))
        || (policy == CodeToolPolicy::ScheduledReport && normalized == "git")
}

fn scheduled_git_is_read_only(args: &serde_json::Value) -> bool {
    matches!(
        args.get("command").and_then(serde_json::Value::as_str),
        Some("status" | "log")
    ) && InteractiveToolGuardrail::risk_decision("git", args) == PermissionDecision::Allow
}

fn scheduled_read_is_allowed(
    workspace: &Path,
    tool_name: &str,
    args: &serde_json::Value,
    denylist: &[String],
    protected_config_path: &Path,
) -> bool {
    let tool = tool_name.to_ascii_lowercase();
    let (field, scoped_scan) = match tool.as_str() {
        "read" => ("file_path", false),
        "search" | "grep" | "bm25" | "glob" | "ls" => ("path", true),
        _ => return false,
    };
    let Some(path) = args.get(field).and_then(serde_json::Value::as_str) else {
        return false;
    };
    let Ok(workspace) = workspace.canonicalize() else {
        return false;
    };
    let requested = Path::new(path.trim());
    let target = if requested.is_absolute() {
        requested.canonicalize()
    } else {
        workspace.join(requested).canonicalize()
    };
    let Ok(target) = target else {
        return false;
    };
    let Ok(relative) = target.strip_prefix(&workspace) else {
        return false;
    };
    let Some(path) = normalized_relative_path(&relative.to_string_lossy()) else {
        return false;
    };
    if scoped_scan && (path.is_empty() || path == ".") && !denylist.is_empty() {
        return false;
    }
    const IMPLICIT_DENYLIST: &[&str] = &[
        ".git/**",
        ".a3s/config.acl",
        ".a3s/os-auth.json",
        ".codex/auth.json",
        ".claude/.credentials.json",
        ".claude.json",
        ".git-credentials",
        ".mcp.json",
        ".netrc",
        ".npmrc",
        ".pypirc",
    ];
    let denied = |candidate: &str| {
        denylist
            .iter()
            .map(String::as_str)
            .chain(IMPLICIT_DENYLIST.iter().copied())
            .any(|pattern| {
                if scoped_scan {
                    deny_pattern_overlaps_scope(pattern, candidate)
                } else {
                    deny_pattern_matches(pattern, candidate)
                }
            })
    };
    if denied(&path) || scheduled_sensitive_path(&path) {
        return false;
    }

    // The canonical target above is also the symlink boundary: both relative
    // and absolute tool paths must resolve inside the selected workspace.
    let protected_config = protected_config_path.canonicalize().ok();
    if protected_config.as_ref().is_some_and(|protected| {
        target.as_path() == protected.as_path() || (scoped_scan && protected.starts_with(&target))
    }) {
        return false;
    }
    !denied(&path) && !scheduled_sensitive_path(&path)
}

fn scheduled_sensitive_path(path: &str) -> bool {
    path.split('/').any(|component| {
        component
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(".env"))
    })
}

fn normalized_relative_path(value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with(['/', '\\']) {
        return None;
    }
    let value = value.replace('\\', "/");
    if value.is_empty() {
        return Some(String::new());
    }
    let mut parts = Vec::new();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            part if part.contains(':') || part.chars().any(char::is_control) => return None,
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

fn deny_pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.replace('\\', "/").to_ascii_lowercase();
    let path = path.to_ascii_lowercase();
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return path == prefix || path.starts_with(&format!("{prefix}/"));
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return path.starts_with(prefix);
    }
    path == pattern
}

fn deny_pattern_overlaps_scope(pattern: &str, scope: &str) -> bool {
    let pattern = pattern.replace('\\', "/").to_ascii_lowercase();
    let scope = scope.to_ascii_lowercase();
    let denied_root = pattern
        .strip_suffix("/**")
        .or_else(|| pattern.strip_suffix('*'))
        .unwrap_or(&pattern)
        .trim_end_matches('/');
    deny_pattern_matches(&pattern, &scope)
        || denied_root == scope
        || denied_root.starts_with(&format!("{scope}/"))
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

    #[tokio::test]
    async fn scheduled_report_profile_is_loop_scoped_and_denylist_aware() {
        let workspace = tempfile::tempdir().unwrap();
        crate::tui::loop_engineering::init_loop(workspace.path().to_str().unwrap(), "daily-triage")
            .unwrap();
        std::fs::create_dir_all(workspace.path().join("src")).unwrap();
        std::fs::create_dir_all(workspace.path().join("secrets")).unwrap();
        std::fs::create_dir_all(workspace.path().join("settings")).unwrap();
        std::fs::create_dir_all(workspace.path().join(".a3s/loops/other/reports")).unwrap();
        std::fs::write(workspace.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(workspace.path().join("secrets/token.txt"), "secret\n").unwrap();
        std::fs::write(workspace.path().join(".ENV"), "secret\n").unwrap();
        std::fs::write(
            workspace.path().join("settings/agent-config.acl"),
            "secret\n",
        )
        .unwrap();
        std::fs::write(
            workspace.path().join(".a3s/loops/daily-triage/STATE.md"),
            "ready\n",
        )
        .unwrap();

        let options = session_options_with_sandbox_and_schedule(
            CodeMode::Auto,
            CodeToolPolicy::ScheduledReport,
            workspace.path(),
            "scheduled-test",
            None,
            Some(crate::code_schedule::ScheduledExecutionPolicy {
                loop_id: "daily-triage".to_string(),
                denylist: vec![".env*".to_string(), "secrets/**".to_string()],
                max_tool_rounds: 3,
                protected_config_path: workspace.path().join("settings/agent-config.acl"),
            }),
        );
        assert_eq!(options.max_tool_rounds, Some(3));
        let checker = options.permission_checker.as_ref().unwrap();

        for tool in [
            "read", "search", "grep", "bm25", "glob", "ls", "git", "write",
        ] {
            assert!(
                checker.expose_to_model(tool),
                "scheduled profile hid {tool}"
            );
        }
        for tool in [
            "bash",
            "task",
            "runtime",
            "web_fetch",
            "generate_object",
            "search_skills",
            "mcp__untrusted__read_host",
        ] {
            assert!(
                !checker.expose_to_model(tool),
                "scheduled profile exposed {tool}"
            );
            assert_eq!(checker.check(tool, &json!({})), PermissionDecision::Deny);
        }

        assert_eq!(
            checker.check("read", &json!({"file_path": "src/main.rs"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check(
                "read",
                &json!({"file_path": workspace.path().join("src/main.rs")}),
            ),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check(
                "search",
                &json!({
                    "query": "triage",
                    "path": workspace.path().join(".a3s/loops/daily-triage/skills"),
                }),
            ),
            PermissionDecision::Allow
        );
        for denied in [
            json!({"file_path": "secrets/token.txt"}),
            json!({"file_path": workspace.path().join("secrets/token.txt")}),
            json!({"file_path": ".ENV"}),
            json!({"file_path": "/etc/passwd"}),
            json!({"file_path": "settings/agent-config.acl"}),
            json!({"file_path": workspace.path().join("settings/agent-config.acl")}),
            json!({}),
        ] {
            assert_eq!(checker.check("read", &denied), PermissionDecision::Deny);
        }
        assert_eq!(
            checker.check("search", &json!({"query": "fn", "path": "src"})),
            PermissionDecision::Allow
        );
        for denied in [
            json!({"query": "secret", "path": "secrets"}),
            json!({"query": "secret", "path": "."}),
            json!({"query": "secret", "path": "settings"}),
            json!({"query": "secret"}),
        ] {
            assert_eq!(checker.check("search", &denied), PermissionDecision::Deny);
        }

        for allowed in [
            ".a3s/loops/daily-triage/STATE.md",
            ".a3s/loops/daily-triage/reports/latest.md",
        ] {
            assert_eq!(
                checker.check("write", &json!({"file_path": allowed, "content": "ok\n"})),
                PermissionDecision::Allow,
                "scheduled profile rejected {allowed}"
            );
        }
        for allowed in [
            workspace.path().join(".a3s/loops/daily-triage/STATE.md"),
            workspace
                .path()
                .join(".a3s/loops/daily-triage/reports/absolute.md"),
        ] {
            assert_eq!(
                checker.check("write", &json!({"file_path": allowed, "content": "ok\n"})),
                PermissionDecision::Allow,
                "scheduled profile rejected an absolute loop artifact"
            );
        }
        for denied in [
            "src/generated.rs",
            ".a3s/loops/daily-triage/loop.toml",
            ".a3s/loops/other/reports/latest.md",
        ] {
            assert_eq!(
                checker.check("write", &json!({"file_path": denied, "content": "bad\n"})),
                PermissionDecision::Deny,
                "scheduled profile admitted {denied}"
            );
        }
        for denied in [
            workspace.path().join("src/generated.rs"),
            workspace.path().join(".a3s/loops/daily-triage/loop.toml"),
            workspace.path().join("../outside.md"),
        ] {
            assert_eq!(
                checker.check("write", &json!({"file_path": denied, "content": "bad\n"})),
                PermissionDecision::Deny,
                "scheduled profile admitted an absolute non-artifact path"
            );
        }

        assert_eq!(
            checker.check("git", &json!({"command": "status"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("git", &json!({"command": "log"})),
            PermissionDecision::Allow
        );
        assert_eq!(
            checker.check("git", &json!({"command": "diff"})),
            PermissionDecision::Deny
        );

        let persisted = options.permission_policy.as_ref().unwrap();
        assert_eq!(
            persisted.check("generate_object", &json!({})),
            PermissionDecision::Deny
        );
        assert_eq!(
            persisted.check("search_skills", &json!({})),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn workspace_write_requires_auto_mode() {
        assert!(validate_tool_policy(CodeMode::Default, CodeToolPolicy::WorkspaceWrite).is_err());
        assert!(validate_tool_policy(CodeMode::Plan, CodeToolPolicy::WorkspaceWrite).is_err());
        assert!(validate_tool_policy(CodeMode::Auto, CodeToolPolicy::WorkspaceWrite).is_ok());
        assert!(validate_tool_policy(CodeMode::Default, CodeToolPolicy::ScheduledReport).is_err());
        assert!(validate_tool_policy(CodeMode::Plan, CodeToolPolicy::ScheduledReport).is_err());
        assert!(validate_tool_policy(CodeMode::Auto, CodeToolPolicy::ScheduledReport).is_ok());
    }
}
