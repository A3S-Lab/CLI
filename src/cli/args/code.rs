use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

use super::{PassthroughArgs, TopArgs};

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct CodeArgs {
    #[command(subcommand)]
    pub command: Option<CodeCommand>,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum CodeCommand {
    /// Run one non-interactive coding task.
    Exec(CodeExecArgs),
    /// Resume the newest or selected interactive session.
    Resume(CodeResumeArgs),
    /// Gather evidence and create Markdown and HTML reports.
    #[command(alias = "deepresearch", alias = "deep-research")]
    Research(CodeResearchArgs),
    /// Run the immutable, headless Agent protocol service declared by a release manifest.
    Harness(CodeHarnessArgs),
    /// Inspect or explicitly prepare the local command sandbox.
    Sandbox(CodeSandboxArgs),
    /// Inspect and manage trusted lifecycle hooks.
    Hooks(CodeHooksArgs),
    /// Manage durable local schedules for engineered loops.
    Schedule(CodeScheduleArgs),
    /// Review or apply the immutable workspace result of a remote Cloud execution.
    Remote(CodeRemoteArgs),
    /// Inspect, export, or delete persisted sessions.
    Session(CodeSessionArgs),
    /// Manage Agent assets.
    Agent(AgentArgs),
    /// Manage MCP assets.
    Mcp(McpArgs),
    /// Manage Skill assets.
    Skill(SkillArgs),
    /// Manage Flow assets.
    Flow(FlowArgs),
    /// Manage OKF knowledge-package assets.
    Okf(OkfArgs),
    /// Manage the workspace knowledge base.
    Kb(KbArgs),
    /// Search or inspect durable context history.
    #[command(alias = "ctx")]
    Context(ContextArgs),
    /// Inspect long-term memory.
    #[command(alias = "mem")]
    Memory(MemoryArgs),

    /// Deprecated alias for top-level authentication.
    #[command(name = "login", hide = true)]
    LegacyLogin(LegacyLoginArgs),
    /// Deprecated alias for top-level authentication.
    #[command(name = "logout", hide = true)]
    LegacyLogout,
    /// Deprecated alias for top-level authentication.
    #[command(name = "auth", hide = true)]
    LegacyAuth(PassthroughArgs),
    /// Deprecated alias for top-level configuration.
    #[command(name = "config", hide = true)]
    LegacyConfig(PassthroughArgs),
    /// Deprecated alias for `a3s config paths`.
    #[command(name = "dirs", hide = true)]
    LegacyDirs,
    /// Deprecated alias for `a3s model list`.
    #[command(name = "models", hide = true)]
    LegacyModels,
    /// Deprecated alias for top-level model commands.
    #[command(name = "model", hide = true)]
    LegacyModel(PassthroughArgs),
    /// Deprecated alias for `a3s top`.
    #[command(name = "top", hide = true)]
    LegacyTop(TopArgs),
    /// Deprecated alias for `a3s self update`.
    #[command(name = "update", hide = true)]
    LegacyUpdate,
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

    /// Restrict the tools exposed to non-interactive automation.
    #[arg(long, value_enum, default_value_t = CodeToolPolicy::Standard)]
    pub tool_policy: CodeToolPolicy,

    /// Override the configured model for this execution.
    #[arg(long, value_name = "PROVIDER/MODEL")]
    pub model: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum CodeMode {
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
    /// Allow read-only Git inspection and writes only to engineered-loop reports.
    #[value(hide = true)]
    ScheduledReport,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct CodeResumeArgs {
    #[arg(value_name = "SESSION_ID")]
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct CodeResearchArgs {
    /// Research question. Multiple shell words are joined for compatibility.
    #[arg(value_name = "QUERY", required = true)]
    pub query: Vec<String>,

    /// Restrict evidence collection to the workspace and other local sources.
    #[arg(long, conflicts_with = "web")]
    pub local_only: bool,

    /// Explicitly allow web and workspace evidence, overriding query wording.
    #[arg(long, conflicts_with = "local_only")]
    pub web: bool,

    /// Directory that should receive generated report artifacts.
    #[arg(long, value_name = "PATH")]
    pub report_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct CodeHarnessArgs {
    /// Admitted Agent release manifest. Its health port and paths are authoritative.
    #[arg(long, value_name = "PATH")]
    pub manifest: PathBuf,

    /// Interface on which the release service listens.
    #[arg(long, value_name = "IP", default_value = "0.0.0.0")]
    pub listen: std::net::IpAddr,
}

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct CodeSandboxArgs {
    #[command(subcommand)]
    pub command: CodeSandboxCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum CodeSandboxCommand {
    /// Verify the managed runtime and native OS boundary without changing machine state.
    Status,
    /// Perform the explicit one-time Windows machine setup and verify the result.
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
pub(crate) struct CodeScheduleArgs {
    #[command(subcommand)]
    pub command: CodeScheduleCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum CodeScheduleCommand {
    /// List workspace loop schedules and their last run.
    List,
    /// Enable recurring execution for one audited L1 loop.
    Enable(CodeScheduleEnableArgs),
    /// Disable recurring execution without deleting history.
    Disable(CodeScheduleLoopArgs),
    /// Queue one immediate run through the background worker.
    Run(CodeScheduleLoopArgs),
    /// Start the workspace schedule worker.
    Start,
    /// Ask the worker to stop after its current run.
    Stop,
    /// Inspect the singleton worker and enabled schedules.
    Status,
    /// Read and acknowledge pending completion notifications.
    Notifications,
    /// Internal detached worker entry point.
    #[command(hide = true)]
    Worker,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct CodeScheduleEnableArgs {
    /// Existing engineered-loop id under `.a3s/loops`.
    #[arg(value_name = "LOOP_ID")]
    pub loop_id: String,

    /// Override the loop cadence (for example 15m, 2h, or 1d).
    #[arg(long, value_name = "CADENCE")]
    pub every: Option<String>,

    /// Pin one provider-qualified model for unattended runs.
    #[arg(long, value_name = "PROVIDER/MODEL")]
    pub model: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct CodeScheduleLoopArgs {
    /// Existing scheduled loop id.
    #[arg(value_name = "LOOP_ID")]
    pub loop_id: String,
}

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct CodeRemoteArgs {
    #[command(subcommand)]
    pub command: CodeRemoteCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum CodeRemoteCommand {
    /// Print the exact Git-compatible diff captured for a terminal execution.
    Diff(CodeRemoteExecutionArgs),
    /// Apply the captured diff to the effective local Git workspace.
    Apply(CodeRemoteExecutionArgs),
}

#[derive(Clone, Debug, Args)]
pub(crate) struct CodeRemoteExecutionArgs {
    /// Cloud Agent execution ID.
    #[arg(value_name = "EXECUTION_ID", value_parser = parse_non_nil_uuid)]
    pub execution_id: String,

    /// Cloud organization that owns the execution.
    #[arg(long, value_name = "ORGANIZATION_ID", value_parser = parse_non_nil_uuid)]
    pub organization: String,
}

fn parse_non_nil_uuid(value: &str) -> Result<String, String> {
    let value = uuid::Uuid::parse_str(value).map_err(|_| "expected a UUID".to_string())?;
    if value.is_nil() {
        return Err("UUID must not be nil".into());
    }
    Ok(value.to_string())
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

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum AgentCommand {
    List(AssetListArgs),
    Clone(AssetCloneArgs),
    Review(AssetPathArgs),
    Activity(AssetQueryArgs),
    Publish(AgentPublishArgs),
    Run(AgentActionArgs),
    Deploy(AssetPathArgs),
    Open(AgentActionArgs),
    Logs(AgentActionArgs),
    Status(AgentActionArgs),
}

#[derive(Clone, Debug, Args)]
pub(crate) struct AgentPublishArgs {
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,
    #[arg(long, value_enum)]
    pub kind: AgentKind,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct AgentActionArgs {
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,
    #[arg(long, value_enum)]
    pub kind: Option<AgentKind>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum AgentKind {
    Agentic,
    Application,
    Tool,
}

macro_rules! asset_family {
    ($args:ident, $command:ident, [$($verb:ident),+ $(,)?]) => {
        #[derive(Clone, Debug, Args)]
        #[command(subcommand_required = true, arg_required_else_help = true)]
        pub(crate) struct $args {
            #[command(subcommand)]
            pub command: $command,
        }

        #[derive(Clone, Debug, Subcommand)]
        pub(crate) enum $command {
            List(AssetListArgs),
            Clone(AssetCloneArgs),
            Review(AssetPathArgs),
            Activity(AssetQueryArgs),
            $($verb(AssetPathArgs)),+
        }
    };
}

asset_family!(
    McpArgs,
    McpCommand,
    [Publish, Run, Test, Deploy, Open, Logs, Status]
);
asset_family!(SkillArgs, SkillCommand, [Publish, Deploy, Open, Status]);
asset_family!(
    FlowArgs,
    FlowCommand,
    [Publish, Run, Deploy, Open, Logs, Status]
);
asset_family!(OkfArgs, OkfCommand, [Publish, Deploy, Status]);

#[derive(Clone, Debug, Args)]
pub(crate) struct AssetListArgs {
    #[arg(long, value_enum)]
    pub location: AssetLocation,
    #[arg(value_name = "QUERY")]
    pub query: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum AssetLocation {
    Local,
    Os,
    All,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct AssetCloneArgs {
    #[arg(value_name = "GIT_URL")]
    pub git_url: String,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct AssetPathArgs {
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct AssetQueryArgs {
    #[arg(value_name = "QUERY")]
    pub query: Option<String>,
}

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct KbArgs {
    #[command(subcommand)]
    pub command: KbCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum KbCommand {
    Stats,
    Add(KbTextArgs),
    Import(KbImportArgs),
    Search(KbTextArgs),
    Path,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct KbTextArgs {
    #[arg(value_name = "TEXT")]
    pub text: String,
}

#[derive(Clone, Debug, Args)]
pub(crate) struct KbImportArgs {
    #[arg(value_name = "FILE_OR_DIRECTORY")]
    pub path: PathBuf,
}

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct ContextArgs {
    #[command(subcommand)]
    pub command: ContextCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum ContextCommand {
    Search(ContextQueryArgs),
    Show(ContextShowArgs),
}

#[derive(Clone, Debug, Args)]
pub(crate) struct ContextQueryArgs {
    #[arg(value_name = "QUERY")]
    pub query: String,
}

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct ContextShowArgs {
    #[command(subcommand)]
    pub command: ContextShowCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum ContextShowCommand {
    Event(ContextEventArgs),
    Session(SessionIdArgs),
}

#[derive(Clone, Debug, Args)]
pub(crate) struct ContextEventArgs {
    #[arg(value_name = "EVENT_ID")]
    pub event_id: String,
    #[arg(long, default_value_t = 3)]
    pub window: usize,
}

#[derive(Clone, Debug, Args)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub(crate) struct MemoryArgs {
    #[command(subcommand)]
    pub command: MemoryCommand,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum MemoryCommand {
    List(MemoryListArgs),
    Stats,
    Path,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct MemoryListArgs {
    #[arg(value_name = "QUERY")]
    pub query: Option<String>,
}

#[derive(Clone, Debug, Default, Args)]
pub(crate) struct LegacyLoginArgs {
    /// Captures unsafe legacy positional credentials for redacted rejection.
    #[arg(value_name = "LEGACY_TOKEN", hide = true)]
    pub values: Vec<OsString>,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::args::{Cli, RootCommand};

    const ORGANIZATION_ID: &str = "019c0000-0000-7000-8000-000000000001";
    const EXECUTION_ID: &str = "019c0000-0000-7000-8000-000000000002";

    #[test]
    fn parses_remote_diff_with_tenant_and_execution_ids() {
        let cli = Cli::try_parse_from([
            "a3s",
            "code",
            "remote",
            "diff",
            EXECUTION_ID,
            "--organization",
            ORGANIZATION_ID,
        ])
        .unwrap();

        let Some(RootCommand::Code(CodeArgs {
            command:
                Some(CodeCommand::Remote(CodeRemoteArgs {
                    command: CodeRemoteCommand::Diff(args),
                })),
        })) = cli.command
        else {
            panic!("expected the code remote diff route");
        };
        assert_eq!(args.execution_id, EXECUTION_ID);
        assert_eq!(args.organization, ORGANIZATION_ID);
    }

    #[test]
    fn rejects_nil_remote_execution_id() {
        assert!(Cli::try_parse_from([
            "a3s",
            "code",
            "remote",
            "apply",
            "00000000-0000-0000-0000-000000000000",
            "--organization",
            ORGANIZATION_ID,
        ])
        .is_err());
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
        })) = cli.command
        else {
            panic!("expected the code exec route");
        };
        assert_eq!(args.mode, CodeMode::Auto);
        assert_eq!(args.tool_policy, CodeToolPolicy::WorkspaceWrite);
        assert_eq!(args.prompt.as_deref(), Some("update the selected code"));
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

    #[test]
    fn parses_local_schedule_enable_and_notifications() {
        let cli = Cli::try_parse_from([
            "a3s",
            "code",
            "schedule",
            "enable",
            "daily-triage",
            "--every",
            "15m",
            "--model",
            "deepseek/deepseek-v4-flash",
        ])
        .unwrap();
        let Some(RootCommand::Code(CodeArgs {
            command:
                Some(CodeCommand::Schedule(CodeScheduleArgs {
                    command: CodeScheduleCommand::Enable(args),
                })),
        })) = cli.command
        else {
            panic!("expected the code schedule enable route");
        };
        assert_eq!(args.loop_id, "daily-triage");
        assert_eq!(args.every.as_deref(), Some("15m"));
        assert_eq!(args.model.as_deref(), Some("deepseek/deepseek-v4-flash"));

        let cli = Cli::try_parse_from(["a3s", "code", "schedule", "notifications"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(RootCommand::Code(CodeArgs {
                command: Some(CodeCommand::Schedule(CodeScheduleArgs {
                    command: CodeScheduleCommand::Notifications,
                })),
            }))
        ));
    }
}
