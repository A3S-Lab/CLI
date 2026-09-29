use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct CodeArgs {
    /// Create an isolated Git worktree under `~/.a3s/worktrees`, then
    /// start the interactive TUI there. Optional NAME becomes the branch/path
    /// identity; omit for an auto-generated `launch-<id>`. Same isolation model
    /// as `/fork worktree`, for cold start (Cursor `agent --worktree` spirit).
    #[arg(long, value_name = "NAME", num_args = 0..=1, default_missing_value = "")]
    pub worktree: Option<String>,

    #[command(subcommand)]
    pub command: Option<CodeCommand>,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum CodeCommand {
    /// Run one non-interactive coding task.
    Exec(CodeExecArgs),
    /// Resume the newest or selected interactive session.
    Resume(CodeResumeArgs),
    /// Inspect or probe the native local command sandbox.
    Sandbox(CodeSandboxArgs),
    /// Inspect and manage trusted lifecycle hooks.
    Hooks(CodeHooksArgs),
    /// Inspect, export, or delete persisted sessions.
    Session(CodeSessionArgs),
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct CodeExecArgs {
    /// Prompt text. Quote multi-word prompts as one shell argument.
    #[arg(value_name = "PROMPT", conflicts_with = "prompt_file")]
    pub prompt: Option<String>,

    /// Read the prompt from a UTF-8 file.
    #[arg(long, value_name = "PATH", conflicts_with = "prompt")]
    pub prompt_file: Option<PathBuf>,

    /// Attach one or more PNG, JPEG, GIF, or WebP images. Repeat the flag or separate paths with commas.
    #[arg(
        short = 'i',
        long = "image",
        value_name = "PATH",
        value_delimiter = ','
    )]
    pub images: Vec<PathBuf>,

    /// Select planning or normal execution behavior.
    #[arg(long, value_enum, default_value_t = CodeMode::Default)]
    pub mode: CodeMode,

    /// Force-allow tool calls that would normally ask for confirmation
    /// (`--force` / `--yolo`). Critical rule denials, protected
    /// paths, catastrophic shell, and leaving the sandbox stay denied. Incompatible
    /// with `--mode plan`.
    #[arg(long = "force", visible_alias = "yolo", default_value_t = false)]
    pub force: bool,

    /// Restrict the tools exposed to non-interactive automation.
    #[arg(long, value_enum, default_value_t = CodeToolPolicy::Standard)]
    pub tool_policy: CodeToolPolicy,

    /// Control whether web_search and web_fetch are available for this run.
    #[arg(long, value_enum, default_value_t = CodeWebSearch::Auto)]
    pub web_search: CodeWebSearch,

    /// Override the configured model for this execution.
    #[arg(long, value_name = "PROVIDER/MODEL")]
    pub model: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum CodeMode {
    /// Read-only planning / exploration (`--mode plan` or `--mode ask`).
    #[value(alias = "ask")]
    Plan,
    #[default]
    Default,
    Auto,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum CodeToolPolicy {
    /// Preserve the ordinary Code Exec permission surface.
    #[default]
    Standard,
    /// Expose only bounded native workspace reads and search.
    ReadOnly,
    /// Add bounded workspace file edits without exposing process-capable tools.
    WorkspaceWrite,
    /// Keep governed local coding tools; network reads require explicit web-search enablement.
    LocalWorkspace,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum CodeWebSearch {
    /// Preserve the selected tool policy's existing behavior.
    #[default]
    Auto,
    /// Explicitly expose governed web_search and web_fetch reads.
    Enabled,
    /// Hide and deny web_search and web_fetch for the entire run.
    Disabled,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct CodeResumeArgs {
    #[arg(value_name = "SESSION_ID")]
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct CodeSandboxArgs {
    #[command(subcommand)]
    pub command: CodeSandboxCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum CodeSandboxCommand {
    /// Verify the native OS boundary without changing machine state.
    Status,
    /// Probe the native boundary and report whether any setup is required.
    Setup,
}

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct CodeHooksArgs {
    #[command(subcommand)]
    pub command: CodeHooksCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum CodeHooksCommand {
    /// List discovered, trusted, pending, and disabled hooks.
    List,
    /// Trust one exact hook definition, or every currently discovered definition.
    Trust(CodeHookTargetArgs),
    /// Disable one trusted hook without forgetting its trust.
    Disable(CodeHookIdArgs),
    /// Re-enable one disabled hook.
    Enable(CodeHookIdArgs),
}

#[derive(Clone, Debug, Args)]
pub(crate) struct CodeHookTargetArgs {
    #[arg(value_name = "ID|all")]
    pub id: String,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct CodeHookIdArgs {
    #[arg(value_name = "ID")]
    pub id: String,
}

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct CodeSessionArgs {
    #[command(subcommand)]
    pub command: CodeSessionCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum CodeSessionCommand {
    /// List sessions in the effective workspace.
    List,
    /// Show one session document.
    Show(SessionIdArgs),
    /// Export one session document.
    Export(SessionExportArgs),
    /// Delete one session document without touching workspace files.
    Delete(SessionDeleteArgs),
}

#[derive(Clone, Debug, Args)]
pub(crate) struct SessionIdArgs {
    #[arg(value_name = "SESSION_ID")]
    pub session_id: String,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct SessionExportArgs {
    #[arg(value_name = "SESSION_ID")]
    pub session_id: String,
    #[arg(long, value_name = "PATH")]
    pub output_file: Option<PathBuf>,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct SessionDeleteArgs {
    #[arg(value_name = "SESSION_ID")]
    pub session_id: String,
    #[arg(long)]
    pub yes: bool,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::args::{Cli, RootCommand};

    #[test]
    fn parses_exec_force_and_yolo_aliases() {
        for flag in ["--force", "--yolo"] {
            let cli =
                Cli::try_parse_from(["a3s", "code", "exec", flag, "ship the change"]).unwrap();
            let Some(RootCommand::Code(CodeArgs {
                command: Some(CodeCommand::Exec(args)),
                ..
            })) = cli.command
            else {
                panic!("expected the code exec route for {flag}");
            };
            assert!(args.force, "{flag} should set force");
            assert_eq!(args.mode, CodeMode::Default);
            assert_eq!(args.prompt.as_deref(), Some("ship the change"));
        }
    }

    #[test]
    fn parses_exec_automation_tool_policy() {
        let cli = Cli::try_parse_from([
            "a3s",
            "code",
            "exec",
            "--mode",
            "auto",
            "--tool-policy",
            "workspace-write",
            "update the selected code",
        ])
        .unwrap();

        let Some(RootCommand::Code(CodeArgs {
            command: Some(CodeCommand::Exec(args)),
            ..
        })) = cli.command
        else {
            panic!("expected the code exec route");
        };
        assert_eq!(args.mode, CodeMode::Auto);
        assert_eq!(args.tool_policy, CodeToolPolicy::WorkspaceWrite);
        assert_eq!(args.prompt.as_deref(), Some("update the selected code"));
    }

    #[test]
    fn parses_exec_mode_ask_as_plan_alias() {
        let cli = Cli::try_parse_from([
            "a3s",
            "code",
            "exec",
            "--mode",
            "ask",
            "explain the module without editing",
        ])
        .unwrap();

        let Some(RootCommand::Code(CodeArgs {
            command: Some(CodeCommand::Exec(args)),
            ..
        })) = cli.command
        else {
            panic!("expected the code exec route");
        };
        assert_eq!(args.mode, CodeMode::Plan);
        assert_eq!(
            args.prompt.as_deref(),
            Some("explain the module without editing")
        );
    }

    #[test]
    fn parses_code_worktree_flag_with_optional_name() {
        let bare = Cli::try_parse_from(["a3s", "code", "--worktree"]).unwrap();
        let Some(RootCommand::Code(CodeArgs {
            worktree: Some(name),
            command: None,
        })) = bare.command
        else {
            panic!("expected interactive code --worktree");
        };
        assert_eq!(name, "");

        let named = Cli::try_parse_from(["a3s", "code", "--worktree", "feature-x"]).unwrap();
        let Some(RootCommand::Code(CodeArgs {
            worktree: Some(name),
            command: None,
        })) = named.command
        else {
            panic!("expected named --worktree");
        };
        assert_eq!(name, "feature-x");
    }

    #[test]
    fn code_help_documents_worktree_flag() {
        use clap::CommandFactory;
        let mut cmd = Cli::command();
        let help = cmd
            .find_subcommand_mut("code")
            .expect("code subcommand")
            .render_long_help()
            .to_string();
        assert!(
            help.contains("--worktree"),
            "expected --worktree in `a3s code --help`, got:\n{help}"
        );
    }

    #[test]
    fn parses_explicit_exec_web_search_preference() {
        let cli = Cli::try_parse_from([
            "a3s",
            "code",
            "exec",
            "--web-search",
            "enabled",
            "research the current release",
        ])
        .unwrap();

        let Some(RootCommand::Code(CodeArgs {
            command: Some(CodeCommand::Exec(args)),
            ..
        })) = cli.command
        else {
            panic!("expected the code exec route");
        };
        assert_eq!(args.web_search, CodeWebSearch::Enabled);
        assert_eq!(args.prompt.as_deref(), Some("research the current release"));
    }

    #[test]
    fn parses_exec_local_workspace_tool_policy() {
        let cli = Cli::try_parse_from([
            "a3s",
            "code",
            "exec",
            "--mode",
            "auto",
            "--tool-policy",
            "local-workspace",
            "fix the selected task without network access",
        ])
        .unwrap();

        let Some(RootCommand::Code(CodeArgs {
            command: Some(CodeCommand::Exec(args)),
            ..
        })) = cli.command
        else {
            panic!("expected the code exec route");
        };
        assert_eq!(args.mode, CodeMode::Auto);
        assert_eq!(args.tool_policy, CodeToolPolicy::LocalWorkspace);
        assert_eq!(
            args.prompt.as_deref(),
            Some("fix the selected task without network access")
        );
    }

    #[test]
    fn parses_explicit_sandbox_lifecycle_commands() {
        for (name, expected) in [
            ("status", CodeSandboxCommand::Status),
            ("setup", CodeSandboxCommand::Setup),
        ] {
            let cli = Cli::try_parse_from(["a3s", "code", "sandbox", name]).unwrap();
            let Some(RootCommand::Code(CodeArgs {
                command: Some(CodeCommand::Sandbox(CodeSandboxArgs { command })),
                ..
            })) = cli.command
            else {
                panic!("expected the code sandbox route");
            };
            assert_eq!(
                std::mem::discriminant(&command),
                std::mem::discriminant(&expected)
            );
        }
    }

    #[test]
    fn parses_hook_trust_lifecycle_commands() {
        let list = Cli::try_parse_from(["a3s", "code", "hooks", "list"]).unwrap();
        assert!(matches!(
            list.command,
            Some(RootCommand::Code(CodeArgs {
                command: Some(CodeCommand::Hooks(CodeHooksArgs {
                    command: CodeHooksCommand::List,
                })),
                ..
            }))
        ));

        for (verb, expected_id) in [
            ("trust", "all"),
            ("disable", "project/pre-tool"),
            ("enable", "project/pre-tool"),
        ] {
            let cli = Cli::try_parse_from(["a3s", "code", "hooks", verb, expected_id]).unwrap();
            let Some(RootCommand::Code(CodeArgs {
                command: Some(CodeCommand::Hooks(CodeHooksArgs { command })),
                ..
            })) = cli.command
            else {
                panic!("expected the code hooks {verb} route");
            };
            let parsed_id = match command {
                CodeHooksCommand::Trust(args) => args.id,
                CodeHooksCommand::Disable(args) => args.id,
                CodeHooksCommand::Enable(args) => args.id,
                CodeHooksCommand::List => panic!("expected a mutating hook command"),
            };
            assert_eq!(parsed_id, expected_id);
        }
    }
}
