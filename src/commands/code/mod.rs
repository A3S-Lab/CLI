mod exec;
mod exec_policy;
mod hooks;
mod host_must_wires;

mod sandbox;
mod session;

use anyhow::bail;

use crate::cli::args::{CodeArgs, CodeCommand, OutputMode};
use crate::cli::context::InvocationContext;
use crate::cli::output::usage_error;

pub(crate) async fn run(args: CodeArgs, context: &InvocationContext) -> anyhow::Result<()> {
    if args.worktree.is_some() && args.command.is_some() {
        bail!("`--worktree` applies only to interactive `a3s code` (not subcommands)");
    }
    match args.command {
        None => match args.worktree {
            Some(name) => crate::code_pager::run_in_isolated_worktree(name, context).await,
            None => launch_pager(Vec::new(), context).await,
        },
        Some(CodeCommand::Exec(args)) => exec::run(args, context).await,
        Some(CodeCommand::Resume(args)) => {
            let mut argv = vec!["resume".to_string()];
            if let Some(session_id) = args.session_id {
                argv.push(session_id);
            }
            launch_pager(argv, context).await
        }
        Some(CodeCommand::Sandbox(args)) => sandbox::run(args, context).await,
        Some(CodeCommand::Hooks(args)) => hooks::run(args, context),
        Some(CodeCommand::Session(args)) => session::run(args, context).await,
    }
}

async fn launch_pager(args: Vec<String>, context: &InvocationContext) -> anyhow::Result<()> {
    let output = context.output_mode();
    if output != OutputMode::Human {
        return Err(usage_error(
            "interactive `a3s code` requires human output; use `a3s code exec` for automation",
        ));
    }
    crate::code_pager::run_in(args, &context.directory, context).await
}
