//! `a3s` — the A3S coding agent CLI.
//!
//! `a3s code` launches the interactive terminal UI (the coding agent); the
//! rest are basic commands.

mod a3s_os;
mod agent_skills;
mod cli;
mod code_hooks;
mod code_pager;
mod commands;
mod config;
mod git_snapshot;
mod host_command_guardrail;
mod image_input;
mod lazy_memory_store;
mod model;
mod session_llm;
mod session_paths;
mod update;
mod user_paths;
mod vec_memory_store;

#[cfg(test)]
static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const RUNTIME_SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
const RUNTIME_WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

fn main() -> std::process::ExitCode {
    let mut runtime_builder = tokio::runtime::Builder::new_multi_thread();
    runtime_builder.enable_all();
    // Component install and package rollback share this worker stack headroom.
    runtime_builder.thread_stack_size(RUNTIME_WORKER_STACK_BYTES);
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
