use std::path::Path;
use std::sync::Arc;

use a3s_code_core::hitl::{ConfirmationPolicy, TimeoutAction};
use a3s_code_core::permissions::{
    InteractiveToolGuardrail, PermissionChecker, PermissionDecision, PermissionPolicy,
};
#[cfg(test)]
use a3s_code_core::ManifestWorkspaceBackend;
use a3s_code_core::{PlanningMode, SessionOptions, WorkspaceServices};

use crate::cli::args::{CodeMode, CodeToolPolicy, CodeWebSearch};
use crate::host_command_guardrail::{bash_boundary_decision, HostCommandMode};

mod local_workspace;

pub(super) struct ExecSessionPolicy {
    mode: CodeMode,
    force: bool,
    tool_policy: CodeToolPolicy,
    web_search: CodeWebSearch,
}

impl ExecSessionPolicy {
    #[cfg(test)]
    pub(super) fn new(
        mode: CodeMode,
        tool_policy: CodeToolPolicy,
        web_search: CodeWebSearch,
    ) -> Self {
        Self::with_force(mode, false, tool_policy, web_search)
    }

    pub(super) fn with_force(
        mode: CodeMode,
        force: bool,
        tool_policy: CodeToolPolicy,
        web_search: CodeWebSearch,
    ) -> Self {
        Self {
            mode,
            force,
            tool_policy,
            web_search,
        }
    }
}

struct ExecPermissionChecker {
    interactive: InteractiveToolGuardrail,
    host_mode: HostCommandMode,
    sandbox_available: bool,
    tool_policy: CodeToolPolicy,
    web_search: CodeWebSearch,
}

impl PermissionChecker for ExecPermissionChecker {
    fn expose_to_model(&self, tool_name: &str) -> bool {
        execution_tool_allowed(self.tool_policy, self.web_search, tool_name)
            && !(self.tool_policy == CodeToolPolicy::LocalWorkspace
                && tool_name.eq_ignore_ascii_case("bash")
                && !self.sandbox_available)
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
        } else if !execution_tool_allowed(self.tool_policy, self.web_search, tool_name) {
            PermissionDecision::Deny
        } else if tool_name.eq_ignore_ascii_case("bash") {
            bash_boundary_decision(
                &self.interactive,
                self.host_mode,
                self.sandbox_available,
                args,
            )
        } else if self.tool_policy == CodeToolPolicy::LocalWorkspace
            && local_workspace::is_orchestration_tool(tool_name)
        {
            // These wrappers dispatch nested calls through the run's inherited
            // permission checker. Admitting the wrapper preserves local agentic
            // work while every nested network, Runtime, MCP, and unknown tool
            // remains denied by this same closed policy.
            PermissionDecision::Allow
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
fn session_options_with_web_search(
    mode: CodeMode,
    tool_policy: CodeToolPolicy,
    web_search: CodeWebSearch,
    workspace: &Path,
    session_id: &str,
) -> SessionOptions {
    let workspace_backend = ManifestWorkspaceBackend::new_with_access_policy(
        workspace,
        a3s_code_core::workspace::LocalWorkspaceAccessPolicy::CredentialBoundary,
    );
    session_options_with_workspace_services(
        ExecSessionPolicy::new(mode, tool_policy, web_search),
        workspace,
        session_id,
        None,
        WorkspaceServices::local_with_manifest_backend(workspace_backend),
    )
}

#[cfg(test)]
pub(super) fn session_options_with_sandbox(
    mode: CodeMode,
    tool_policy: CodeToolPolicy,
    workspace: &Path,
    session_id: &str,
    sandbox: Option<Arc<dyn a3s_code_core::sandbox::BashSandbox>>,
) -> SessionOptions {
    let workspace_backend = ManifestWorkspaceBackend::new_with_access_policy(
        workspace,
        a3s_code_core::workspace::LocalWorkspaceAccessPolicy::CredentialBoundary,
    );
    session_options_with_workspace_services(
        ExecSessionPolicy::new(mode, tool_policy, CodeWebSearch::Auto),
        workspace,
        session_id,
        sandbox,
        WorkspaceServices::local_with_manifest_backend(workspace_backend),
    )
}

pub(super) fn session_options_with_workspace_services(
    policy: ExecSessionPolicy,
    workspace: &Path,
    session_id: &str,
    sandbox: Option<Arc<dyn a3s_code_core::sandbox::BashSandbox>>,
    workspace_services: Arc<WorkspaceServices>,
) -> SessionOptions {
    let ExecSessionPolicy {
        mode,
        force,
        tool_policy,
        web_search,
    } = policy;
    let permission_policy = permission_policy(tool_policy, web_search);
    let sandbox_available = sandbox.is_some();
    let effective_mode = if force {
        // Force keeps Auto planning so the run may execute, while the
        // permission checker uses Force approval semantics.
        CodeMode::Auto
    } else {
        mode
    };
    let mut options = SessionOptions::new()
        .with_session_id(session_id)
        // `code exec` is a one-shot automation boundary. The model may still
        // perform bounded tool rounds inside the turn, but a short final
        // answer must not be reclassified into synthetic "continue working"
        // user turns that corrupt strict protocols such as PR review JSON.
        .with_continuation(false)
        .with_workspace_backend(workspace_services)
        .with_planning_mode(planning_mode(effective_mode))
        .with_confirmation_policy(
            ConfirmationPolicy::enabled().with_timeout(30_000, TimeoutAction::Reject),
        )
        .with_permission_policy(permission_policy)
        .with_permission_checker(Arc::new(ExecPermissionChecker {
            interactive: InteractiveToolGuardrail::for_mode(guardrail_mode_name(mode, force))
                .with_workspace(workspace),
            host_mode: host_mode_for(mode, force),
            sandbox_available,
            tool_policy,
            web_search,
        }));
    if matches!(mode, CodeMode::Plan) {
        options = options.with_prompt_slots(
            a3s_code_core::SystemPromptSlots::default().with_style(a3s_code_core::AgentStyle::Plan),
        );
    }
    // Match TUI session wiring: discover project/user skill roots so
    // `search_skills` / `skill` work under `code exec`, not only interactively.
    // Also materialize always-available built-ins (`$okf`) the same way TUI does.
    let workspace_key = workspace.to_string_lossy();
    let configured_skill_dir = std::env::var_os("A3S_SKILL_DIR")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| crate::user_paths::user_home_dir().map(|home| home.join(".a3s/skills")))
        .unwrap_or_else(|| std::path::PathBuf::from(".a3s/skills"));
    let mut skill_dirs = crate::agent_skills::agent_skill_dirs_with_configured(
        workspace_key.as_ref(),
        &configured_skill_dir,
    );
    if let Some(builtin) = crate::agent_skills::ensure_builtin_skills_dir() {
        skill_dirs.push(builtin);
    }
    if !skill_dirs.is_empty() {
        options = options.with_skill_dirs(skill_dirs);
    }
    match sandbox {
        Some(sandbox) => options.with_sandbox_handle(sandbox),
        None => options,
    }
}

#[cfg(test)]
pub(super) fn validate_tool_policy(
    mode: CodeMode,
    tool_policy: CodeToolPolicy,
) -> anyhow::Result<()> {
    validate_exec_policy(mode, false, tool_policy)
}

pub(super) fn validate_exec_policy(
    mode: CodeMode,
    force: bool,
    tool_policy: CodeToolPolicy,
) -> anyhow::Result<()> {
    if force && mode == CodeMode::Plan {
        return Err(crate::cli::output::usage_error(
            "--force/--yolo is incompatible with --mode plan",
        ));
    }
    if matches!(
        tool_policy,
        CodeToolPolicy::WorkspaceWrite | CodeToolPolicy::LocalWorkspace
    ) && mode != CodeMode::Auto
        && !force
    {
        return Err(crate::cli::output::usage_error(
            "write-capable closed tool policies require --mode auto or --force/--yolo",
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

fn guardrail_mode_name(mode: CodeMode, force: bool) -> &'static str {
    if force {
        "force"
    } else {
        mode_name(mode)
    }
}

fn host_mode(mode: CodeMode) -> HostCommandMode {
    match mode {
        CodeMode::Default => HostCommandMode::Default,
        CodeMode::Plan => HostCommandMode::Plan,
        CodeMode::Auto => HostCommandMode::Auto,
    }
}

fn host_mode_for(mode: CodeMode, force: bool) -> HostCommandMode {
    if force {
        HostCommandMode::Force
    } else {
        host_mode(mode)
    }
}

fn permission_policy(tool_policy: CodeToolPolicy, web_search: CodeWebSearch) -> PermissionPolicy {
    if tool_policy == CodeToolPolicy::Standard {
        let policy = PermissionPolicy::new()
            .deny_all(WORKSPACE_BOUNDARY_DENIES)
            .allow_all(STANDARD_READ_TOOLS)
            .ask_all(STANDARD_INTERACTIVE_TOOLS);
        return apply_web_search_policy(policy, tool_policy, web_search);
    }

    let mut closed = PermissionPolicy::new()
        .deny_all(WORKSPACE_BOUNDARY_DENIES)
        .allow_all(CLOSED_BASIC_READ_TOOLS)
        .deny_all(CLOSED_EXTERNAL_TOOLS);
    closed.default_decision = PermissionDecision::Deny;
    let policy = match tool_policy {
        CodeToolPolicy::ReadOnly => closed
            .allow_all(CLOSED_LOCAL_HELPER_TOOLS)
            .deny_all(CLOSED_PROCESS_TOOLS)
            .deny("Git(*)")
            .deny_all(&["Write(*)", "Edit(*)", "Patch(*)"]),
        // Patch path matching in legacy serialized policies is conservative, so
        // the persisted fallback asks. The live checker above remains the
        // authority and silently admits only a bounded, non-protected target.
        CodeToolPolicy::WorkspaceWrite => closed
            .allow_all(CLOSED_LOCAL_HELPER_TOOLS)
            .deny_all(CLOSED_PROCESS_TOOLS)
            .deny("Git(*)")
            .allow_all(&["Write(*)", "Edit(*)"])
            .ask("Patch(*)"),
        // The serializable fallback keeps process and delegation tools at Ask.
        // The live checker admits them only under the inherited closed policy,
        // and Bash additionally requires the verified native sandbox handle.
        CodeToolPolicy::LocalWorkspace => closed
            .allow_all(CLOSED_LOCAL_HELPER_TOOLS)
            .allow_all(local_workspace::PERSISTED_CODE_READ_TOOLS)
            .allow_all(&["Write(*)", "Edit(*)"])
            .ask_all(local_workspace::PERSISTED_GOVERNED_TOOLS)
            .ask("Patch(*)"),
        CodeToolPolicy::Standard => unreachable!(),
    };
    apply_web_search_policy(policy, tool_policy, web_search)
}

fn apply_web_search_policy(
    policy: PermissionPolicy,
    tool_policy: CodeToolPolicy,
    preference: CodeWebSearch,
) -> PermissionPolicy {
    match preference {
        CodeWebSearch::Enabled => policy.allow_all(WEB_READ_TOOLS),
        CodeWebSearch::Disabled => policy.deny_all(WEB_READ_TOOLS),
        CodeWebSearch::Auto if tool_policy == CodeToolPolicy::Standard => policy,
        CodeWebSearch::Auto => policy.deny_all(WEB_READ_TOOLS),
    }
}

const WORKSPACE_BOUNDARY_DENIES: &[&str] = &[
    // Absolute paths are not denied here: ExecPermissionChecker's interactive
    // guardrail is workspace-aware and admits in-workspace absolutes while
    // still denying host escapes. Keep lexical `..` escapes fail-closed.
    "Read(**/../**)",
    "Search(** **/../**)",
    "Grep(* **/../**)",
    "Bm25(* **/../**)",
    "Glob(**/../**)",
    "LS(**/../**)",
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

const WEB_READ_TOOLS: &[&str] = &["web_search(*)", "web_fetch(*)"];

const CLOSED_BASIC_READ_TOOLS: &[&str] = &[
    "Read(*)",
    "Search(*)",
    "Grep(*)",
    "Bm25(*)",
    "Glob(*)",
    "LS(*)",
];

const CLOSED_LOCAL_HELPER_TOOLS: &[&str] = &["generate_object(*)", "search_skills(*)"];

const CLOSED_EXTERNAL_TOOLS: &[&str] = &[
    "runtime(*)",
    "download(*)",
    "use_knowledge_search(*)",
    "use_tool_*(*)",
    "mcp__*(*)",
];

fn execution_tool_allowed(
    policy: CodeToolPolicy,
    web_search: CodeWebSearch,
    tool_name: &str,
) -> bool {
    if matches!(
        tool_name.to_ascii_lowercase().as_str(),
        "web_search" | "web_fetch"
    ) {
        return match web_search {
            CodeWebSearch::Enabled => true,
            CodeWebSearch::Disabled => false,
            CodeWebSearch::Auto => tool_allowed(policy, tool_name),
        };
    }
    tool_allowed(policy, tool_name)
}

const CLOSED_PROCESS_TOOLS: &[&str] = &[
    "Bash(*)",
    "batch(*)",
    "program(*)",
    "task(*)",
    "parallel_task(*)",
    "dynamic_workflow(*)",
    "Skill(*)",
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
    if policy == CodeToolPolicy::LocalWorkspace {
        return local_workspace::tool_allowed(&normalized);
    }
    let basic_read = matches!(
        normalized.as_str(),
        "read" | "search" | "grep" | "bm25" | "glob" | "ls"
    );
    basic_read
        || matches!(normalized.as_str(), "generate_object" | "search_skills")
        || (policy == CodeToolPolicy::WorkspaceWrite
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
    async fn explicit_web_search_preference_is_independent_of_the_workspace_tool_policy() {
        let workspace = tempfile::tempdir().unwrap();
        for policy in [
            CodeToolPolicy::Standard,
            CodeToolPolicy::ReadOnly,
            CodeToolPolicy::WorkspaceWrite,
            CodeToolPolicy::LocalWorkspace,
        ] {
            let mode = if matches!(
                policy,
                CodeToolPolicy::WorkspaceWrite | CodeToolPolicy::LocalWorkspace
            ) {
                CodeMode::Auto
            } else {
                CodeMode::Default
            };
            let enabled = session_options_with_web_search(
                mode,
                policy,
                CodeWebSearch::Enabled,
                workspace.path(),
                "web-enabled",
            );
            let enabled_checker = enabled.permission_checker.as_ref().unwrap();
            assert!(enabled_checker.expose_to_model("web_search"));
            assert_eq!(
                enabled_checker.check("web_search", &json!({"query": "a3s"})),
                PermissionDecision::Allow
            );

            let disabled = session_options_with_web_search(
                mode,
                policy,
                CodeWebSearch::Disabled,
                workspace.path(),
                "web-disabled",
            );
            let disabled_checker = disabled.permission_checker.as_ref().unwrap();
            assert!(!disabled_checker.expose_to_model("web_search"));
            assert_eq!(
                disabled_checker.check("web_search", &json!({"query": "a3s"})),
                PermissionDecision::Deny
            );
        }
    }

    #[tokio::test]
    async fn exec_session_options_discover_workspace_a3s_skills() {
        let workspace = tempfile::tempdir().unwrap();
        let skill_root = workspace.path().join(".a3s/skills/wb-probe");
        std::fs::create_dir_all(&skill_root).unwrap();
        std::fs::write(
            skill_root.join("SKILL.md"),
            "---\nname: wb-probe-skill\ndescription: probe\n---\n# Probe\n",
        )
        .unwrap();

        let options = session_options_with_sandbox(
            CodeMode::Plan,
            CodeToolPolicy::ReadOnly,
            workspace.path(),
            "skill-dirs-exec-test",
            None,
        );
        let expected = workspace.path().join(".a3s/skills");
        assert!(
            options
                .skill_dirs
                .iter()
                .any(|dir| dir == &expected || dir.ends_with(".a3s/skills")),
            "expected workspace .a3s/skills in {:?}",
            options.skill_dirs
        );
        assert!(
            options
                .skill_dirs
                .iter()
                .any(|dir| dir.ends_with(".a3s/cli/skills")),
            "expected built-in $okf root (.a3s/cli/skills) in {:?}",
            options.skill_dirs
        );
        let okf_skill = options
            .skill_dirs
            .iter()
            .find(|dir| dir.ends_with(".a3s/cli/skills"))
            .unwrap()
            .join("okf/SKILL.md");
        assert!(
            okf_skill.is_file(),
            "built-in okf skill must be materialized at {}",
            okf_skill.display()
        );
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
    async fn plan_mode_installs_agent_style_plan_on_prompt_slots() {
        let workspace = tempfile::tempdir().unwrap();
        let options = session_options_with_workspace_services(
            ExecSessionPolicy::new(
                CodeMode::Plan,
                CodeToolPolicy::Standard,
                CodeWebSearch::Auto,
            ),
            workspace.path(),
            "plan-style-exec-test",
            None,
            WorkspaceServices::local_with_manifest_backend(
                ManifestWorkspaceBackend::new_with_access_policy(
                    workspace.path(),
                    a3s_code_core::workspace::LocalWorkspaceAccessPolicy::CredentialBoundary,
                ),
            ),
        );
        assert_eq!(options.planning_mode, PlanningMode::Enabled);
        let slots = options
            .prompt_slots
            .as_ref()
            .expect("Plan mode must set Core prompt slots");
        assert_eq!(slots.style, Some(a3s_code_core::AgentStyle::Plan));
    }

    #[tokio::test]
    async fn force_mode_allows_high_risk_review_candidates_but_keeps_hard_denies() {
        let workspace = tempfile::tempdir().unwrap();
        let options = session_options_with_workspace_services(
            ExecSessionPolicy::with_force(
                CodeMode::Default,
                true,
                CodeToolPolicy::Standard,
                CodeWebSearch::Auto,
            ),
            workspace.path(),
            "force-exec-test",
            Some(Arc::new(TestSandbox)),
            WorkspaceServices::local_with_manifest_backend(
                ManifestWorkspaceBackend::new_with_access_policy(
                    workspace.path(),
                    a3s_code_core::workspace::LocalWorkspaceAccessPolicy::CredentialBoundary,
                ),
            ),
        );
        let checker = options.permission_checker.as_ref().unwrap();
        assert_eq!(options.planning_mode, PlanningMode::Auto);
        assert_eq!(
            checker.check("write", &json!({"file_path": "answer.txt"})),
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
                    "justification": "needs a host capability"
                }),
            ),
            PermissionDecision::Deny,
            "force must not leave the sandbox boundary"
        );
        assert_eq!(
            checker.check("bash", &json!({"command": "rm -rf /"})),
            PermissionDecision::Deny
        );
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
                match mode {
                    // Plan denies bounded workspace mutations; Default asks.
                    CodeMode::Plan => PermissionDecision::Deny,
                    _ => PermissionDecision::Ask,
                }
            );
            assert_eq!(
                checker.check("bash", &json!({"command": "pwd"})),
                PermissionDecision::Deny
            );
        }
    }

    #[tokio::test]
    async fn plan_and_read_only_admit_in_workspace_absolute_and_files_reads() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("README.md"), "ok\n").unwrap();
        let inside = workspace.path().join("README.md");

        for (mode, policy) in [
            (CodeMode::Plan, CodeToolPolicy::Standard),
            (CodeMode::Auto, CodeToolPolicy::ReadOnly),
        ] {
            let options = session_options(mode, policy, workspace.path(), "abs-read-test");
            let checker = options.permission_checker.as_ref().unwrap();

            assert_eq!(
                checker.check("read", &json!({"file_path": &inside})),
                PermissionDecision::Allow,
                "{mode:?}/{policy:?} must allow absolute in-workspace reads"
            );
            assert_eq!(
                checker.check("read", &json!({"files": [{"path": "README.md"}]})),
                PermissionDecision::Allow,
                "{mode:?}/{policy:?} must allow relative files[] reads"
            );
            assert_eq!(
                checker.check(
                    "read",
                    &json!({"files": [{"path": inside.to_string_lossy()}]}),
                ),
                PermissionDecision::Allow,
                "{mode:?}/{policy:?} must allow absolute files[] reads"
            );
            assert_eq!(
                checker.check("read", &json!({"file_path": "/etc/passwd"})),
                PermissionDecision::Deny,
                "{mode:?}/{policy:?} must still deny host absolute reads"
            );
        }
    }

    #[tokio::test]
    async fn automation_profiles_are_closed_over_process_capable_tools() {
        let workspace = tempfile::tempdir().unwrap();
        for policy in [CodeToolPolicy::ReadOnly, CodeToolPolicy::WorkspaceWrite] {
            let options = session_options(CodeMode::Auto, policy, workspace.path(), "exec-test");
            assert_eq!(options.continuation_enabled, Some(false));
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
    fn write_capable_closed_policies_require_auto_mode() {
        assert!(validate_tool_policy(CodeMode::Default, CodeToolPolicy::WorkspaceWrite).is_err());
        assert!(validate_tool_policy(CodeMode::Plan, CodeToolPolicy::WorkspaceWrite).is_err());
        assert!(validate_tool_policy(CodeMode::Auto, CodeToolPolicy::WorkspaceWrite).is_ok());
        assert!(
            validate_exec_policy(CodeMode::Default, true, CodeToolPolicy::WorkspaceWrite).is_ok()
        );
        assert!(
            validate_exec_policy(CodeMode::Plan, true, CodeToolPolicy::WorkspaceWrite).is_err()
        );
        assert!(validate_tool_policy(CodeMode::Default, CodeToolPolicy::LocalWorkspace).is_err());
        assert!(validate_tool_policy(CodeMode::Plan, CodeToolPolicy::LocalWorkspace).is_err());
        assert!(validate_tool_policy(CodeMode::Auto, CodeToolPolicy::LocalWorkspace).is_ok());
        assert!(
            validate_exec_policy(CodeMode::Default, true, CodeToolPolicy::LocalWorkspace).is_ok()
        );
        assert!(validate_exec_policy(CodeMode::Plan, true, CodeToolPolicy::Standard).is_err());
    }
}
