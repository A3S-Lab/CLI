use serde_json::json;

use crate::cli::args::{CodeHooksArgs, CodeHooksCommand, OutputMode};
use crate::cli::context::InvocationContext;
use crate::cli::output::render_value;

pub(super) fn run(args: CodeHooksArgs, context: &InvocationContext) -> anyhow::Result<()> {
    let trust_path = context
        .component_paths
        .state_root
        .join("code/hooks-trust.json");
    let executor = crate::code_hooks::CommandHookExecutor::discover(
        &context.directory,
        context.home.as_deref(),
        trust_path,
    )?;
    let command = match args.command {
        CodeHooksCommand::List => "list".to_string(),
        CodeHooksCommand::Trust(args) => format!("trust {}", args.id),
        CodeHooksCommand::Disable(args) => format!("disable {}", args.id),
        CodeHooksCommand::Enable(args) => format!("enable {}", args.id),
    };
    let text = executor.manage(&command)?;
    let value = executor.status_value();
    if context.output_mode() == OutputMode::Human {
        println!("{text}");
        return Ok(());
    }
    render_value(context.output_mode(), "code.hooks", json!(value), || {})
}
