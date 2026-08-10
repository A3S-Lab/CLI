//! `a3s` — the A3S coding agent CLI.
//!
//! `a3s code` launches the interactive terminal UI (the coding agent); the
//! rest are basic commands.

mod a3s_os;
mod account_providers;
mod api;
mod budget;
mod cli;
mod commands;
mod compact;
mod config;
mod deep_research_checkpoint;
mod evolution;
mod host_command_guardrail;
mod model;
mod plugin_policy_handoff_env;
mod plugin_runtime_task_host;
#[path = "research/code.rs"]
mod research;
mod runtime_tool;
mod session_llm;
mod system_agents;
mod timeline;
mod top;
mod tui;
mod update;
mod use_registry;
mod user_paths;

#[cfg(test)]
#[path = "../tests/support/tuf_test_support.rs"]
mod tuf_test_support;

#[cfg(test)]
static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const RUNTIME_SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
#[cfg(windows)]
const WINDOWS_RUNTIME_WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

fn main() -> std::process::ExitCode {
    let mut runtime_builder = tokio::runtime::Builder::new_multi_thread();
    runtime_builder.enable_all();
    // Windows async state machines require more worker-stack headroom than
    // the Unix baseline during reviewed cognitive-package lifecycle rollback.
    #[cfg(windows)]
    runtime_builder.thread_stack_size(WINDOWS_RUNTIME_WORKER_STACK_BYTES);
    let runtime = match runtime_builder.build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to start the A3S async runtime: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let exit_code = runtime.block_on(cli::run(std::env::args_os()));
    // Tokio waits indefinitely for blocking-pool work during Runtime::drop.
    // Product hosts perform explicit cleanup; this final bound prevents an
    // unresponsive filesystem or child adapter from keeping a finished CLI
    // process alive forever.
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_GRACE);
    exit_code
}
