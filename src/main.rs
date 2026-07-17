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
mod model;
mod runtime_tool;
mod session_llm;
mod system_agents;
mod timeline;
mod top;
mod tui;
mod update;
mod use_registry;

#[cfg(test)]
static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tokio::main]
async fn main() -> std::process::ExitCode {
    cli::run(std::env::args_os()).await
}
