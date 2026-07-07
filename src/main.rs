//! `a3s` — the A3S coding agent CLI.
//!
//! `a3s code` launches the interactive terminal UI (the coding agent); the
//! rest are basic commands.

mod a3s_os;
mod box_cmd;
mod claude;
mod codex;
mod runtime_tool;
mod tools;
mod top;
mod tui;
mod update;

#[cfg(test)]
static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn usage_text() -> String {
    [
        format!("a3s {} — A3S coding agent CLI", env!("CARGO_PKG_VERSION")),
        String::new(),
        "usage:".to_string(),
        "  a3s code                  launch the interactive coding agent (TUI)".to_string(),
        "  a3s code resume <id>      resume a saved session by id".to_string(),
        "  a3s code login|logout     manage the configured OS account".to_string(),
        "  a3s code config|dirs      inspect config and local asset roots".to_string(),
        "  a3s code serve            start the local API and Shu Xiao'an web UI".to_string(),
        "  a3s code kb|ctx|memory    use TUI knowledge/history tools from scripts".to_string(),
        "  a3s code <family> <cmd>   run asset lifecycle commands (agent/mcp/skill/flow/okf)"
            .to_string(),
        "  a3s box <args...>         run a3s-box, installing it automatically if needed"
            .to_string(),
        "  a3s list                  list installed a3s-* tools on PATH".to_string(),
        "  a3s top                   live monitor for boxes, agents, and diagnostics".to_string(),
        "  a3s update                check for and install a newer version".to_string(),
        "  a3s --version             show version".to_string(),
        "  a3s --help                show this help".to_string(),
    ]
    .join("\n")
        + "\n"
}

fn usage() {
    print!("{}", usage_text());
}

fn version_text() -> String {
    format!("a3s {}\n", env!("CARGO_PKG_VERSION"))
}

/// Check the latest GitHub release and upgrade in place via the shared `update`
/// module (Homebrew, with a direct-download fallback).
async fn self_update() -> anyhow::Result<()> {
    let current = update::current_version();
    println!("a3s {current} — checking for updates…");
    let Some(latest) = update::fetch_latest() else {
        eprintln!("a3s: couldn't reach the release server (try again later)");
        std::process::exit(1);
    };
    if update::version_ge(&current, &latest) {
        println!("✓ already up to date (a3s {current})");
        match update::repair_installation() {
            Ok(items) if items.is_empty() => println!("✓ installation looks healthy"),
            Ok(items) => {
                for item in items {
                    println!("✓ {item}");
                }
            }
            Err(error) => eprintln!("warning: install repair failed: {error}"),
        }
        return Ok(());
    }
    println!("→ a3s {latest} available (you have {current})");
    if !update::can_self_update() {
        println!("get the new build from: https://github.com/A3S-Lab/Cli/releases/latest");
        return Ok(());
    }
    match update::perform_upgrade(&latest) {
        Ok(_) => println!("✓ updated to a3s {latest} — run `a3s code` to use it"),
        Err(error) => {
            eprintln!("upgrade failed: {error}");
            eprintln!("get the latest from https://github.com/A3S-Lab/Cli/releases/latest");
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
            } else if tui::is_code_cli_command(&rest) {
                tui::run_code_cli(rest).await
            } else {
                tui::run(rest).await
            }
        }
        Some("-V") | Some("--version") => {
            print!("{}", version_text());
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
    #[test]
    fn test_help_command() {
        let text = super::usage_text();
        assert!(text.contains("usage:"));
        assert!(text.contains("  a3s code"));
        assert!(text.contains("a3s --version"));
    }

    #[test]
    fn test_version_command() {
        assert_eq!(
            super::version_text(),
            format!("a3s {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}
