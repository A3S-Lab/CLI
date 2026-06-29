//! `a3s` — the A3S coding agent CLI.
//!
//! `a3s code` launches the interactive terminal UI (the coding agent); the
//! rest are basic commands.

mod a3s_os;
mod box_cmd;
mod claude;
mod codex;
mod tools;
mod top;
mod tui;
mod update;

#[cfg(test)]
static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn usage() {
    println!("a3s {} — A3S coding agent CLI\n", env!("CARGO_PKG_VERSION"));
    println!("usage:");
    println!("  a3s code                  launch the interactive coding agent (TUI)");
    println!("  a3s code resume <id>      resume a saved session by id");
    println!("  a3s box <args...>         run a3s-box, installing it automatically if needed");
    println!("  a3s list                  list installed a3s-* tools on PATH");
    println!("  a3s top                   live monitor for boxes, agents, and diagnostics");
    println!("  a3s update                check for and install a newer version");
    println!("  a3s --version             show version");
    println!("  a3s --help                show this help");
}

/// Check the latest GitHub release and upgrade in place via the shared `update`
/// module (Homebrew, with a direct-download fallback).
async fn self_update() -> anyhow::Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("a3s {current} — checking for updates…");
    let Some(latest) = update::fetch_latest() else {
        eprintln!("a3s: couldn't reach the release server (try again later)");
        std::process::exit(1);
    };
    if update::version_ge(current, &latest) {
        println!("✓ already up to date (a3s {current})");
        return Ok(());
    }
    println!("→ a3s {latest} available (you have {current})");
    if !update::can_self_update() {
        println!("get the new build from: https://github.com/A3S-Lab/Cli/releases/latest");
        return Ok(());
    }
    match update::perform_upgrade(&latest) {
        Some(_) => println!("✓ updated to a3s {latest} — run `a3s code` to use it"),
        None => {
            eprintln!(
                "upgrade failed — get the latest from https://github.com/A3S-Lab/Cli/releases/latest"
            );
            std::process::exit(1);
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("update") => self_update().await,
        Some("box") => box_cmd::run(args.collect()).await,
        Some("list") => {
            tools::print_tool_list();
            Ok(())
        }
        Some("top") => top::run(args.collect()).await,
        // Pass any trailing args (e.g. `resume <id>`) through to the TUI.
        Some("code") => {
            let rest: Vec<String> = args.collect();
            if rest.first().map(String::as_str) == Some("update") {
                self_update().await
            } else {
                tui::run(rest).await
            }
        }
        Some("-V") | Some("--version") => {
            println!("a3s {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        None | Some("-h") | Some("--help") | Some("help") => {
            usage();
            Ok(())
        }
        Some(other) => {
            eprintln!("a3s: unknown command '{other}' — try 'a3s --help'");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    fn cargo_command() -> std::ffi::OsString {
        std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
    }

    #[tokio::test]
    async fn test_help_command() {
        let _guard = cargo_run_guard();
        let output = std::process::Command::new(cargo_command())
            .args(["run", "--", "--help"])
            .output()
            .expect("Failed to execute process");

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("usage:"));
    }

    #[tokio::test]
    async fn test_version_command() {
        let _guard = cargo_run_guard();
        let output = std::process::Command::new(cargo_command())
            .args(["run", "--", "--version"])
            .output()
            .expect("Failed to execute process");

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    }

    fn cargo_run_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }
}
