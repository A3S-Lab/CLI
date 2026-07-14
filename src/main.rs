//! `a3s` — the A3S coding agent CLI.
//!
//! `a3s code` launches the interactive terminal UI (the coding agent); the
//! rest are basic commands.

mod a3s_os;
mod account;
mod account_providers;
mod api;
mod bench_component;
mod box_cmd;
mod budget;
mod compact;
mod components;
mod config;
mod model;
mod os_cmd;
mod runtime_tool;
mod search_cmd;
mod session_llm;
mod timeline;
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
        "  a3s code deepresearch     run DeepResearch and write .md/.html reports".to_string(),
        "  a3s code kb|ctx|memory    use TUI knowledge/history tools from scripts".to_string(),
        "  a3s code <family> <cmd>   run asset lifecycle commands (agent/mcp/skill/flow/okf)"
            .to_string(),
        "  a3s login [token]         sign in to A3S OS (browser OAuth or bearer token)"
            .to_string(),
        "  a3s logout                sign out from A3S OS".to_string(),
        "  a3s model <command>       list, select, or reset the A3S Code model route"
            .to_string(),
        "  a3s account <command>     inspect local Claude, Codex, WorkBuddy, and A3S OS logins"
            .to_string(),
        "  a3s box <args...>         run Box, installing it automatically on first use".to_string(),
        "  a3s bench <args...>       run Bench, installing its private control component on first real use"
            .to_string(),
        "  a3s search <command>      manage search engines and headless browser runtimes".to_string(),
        "  a3s install <code|box|bench> install a component".to_string(),
        "  a3s list                  show code, box, bench, and other a3s-* tools".to_string(),
        "  a3s top                   live monitor for boxes, agents, and diagnostics".to_string(),
        "  a3s update [code|box|bench] update a component; defaults to code".to_string(),
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

fn install_usage_text() -> &'static str {
    "usage: a3s install <code|box|bench>\n\n\
     `code` is included with a3s; installing it verifies and repairs its companion tools.\n\
     Box and Bench are downloaded only by explicit install or first real use.\n"
}

fn update_usage_text() -> &'static str {
    "usage: a3s update [code|box|bench]\n\n\
     With no component, this updates Code for compatibility with earlier releases.\n"
}

fn box_usage_text() -> &'static str {
    "usage: a3s box <args...>\n\n\
     Arguments are forwarded to a3s-box. Box is installed automatically on first use.\n"
}

fn bench_usage_text() -> &'static str {
    "usage:\n\
       a3s bench list [--all] [--json]\n\
       a3s bench info <task-id|./path> [--all] [--json]\n\
       a3s bench run <task-id|./path> --agent <asset> [--json]\n\
       a3s bench result [run-id] [--json]\n\
       a3s bench advanced <command> ...\n\n\
     Bench is a private control component installed automatically on first real use;\n\
     it is never added to PATH. Candidate and Judge Agent Assets are executed only\n\
     by A3S OS Runtime. Local task paths must start with ./ or ../.\n"
}

fn list_usage_text() -> &'static str {
    "usage: a3s list\n\nShow managed Code, Box, and Bench components plus other a3s-* tools on PATH.\n"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComponentSelection {
    Help,
    Target(components::ComponentId),
}

fn parse_component_selection(
    args: &[String],
    default: Option<components::ComponentId>,
) -> Result<ComponentSelection, String> {
    match args {
        [] => default
            .map(ComponentSelection::Target)
            .ok_or_else(|| "missing component; choose code, box, or bench".to_string()),
        [arg] if matches!(arg.as_str(), "-h" | "--help" | "help") => Ok(ComponentSelection::Help),
        [arg] => components::ComponentId::parse(arg)
            .map(ComponentSelection::Target)
            .ok_or_else(|| format!("unknown component '{arg}'; choose code, box, or bench")),
        _ => Err("expected exactly one component: code, box, or bench".to_string()),
    }
}

fn exact_help_request(args: &[String]) -> bool {
    matches!(args, [arg] if matches!(arg.as_str(), "-h" | "--help" | "help"))
}

fn read_only_product_request(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
        || matches!(args.first().map(String::as_str), Some("help"))
        || matches!(args, [arg] if matches!(arg.as_str(), "-V" | "--version"))
}

fn exact_version_request(args: &[String]) -> bool {
    matches!(args, [arg] if matches!(arg.as_str(), "-V" | "--version"))
}

fn usage_error(message: &str, usage: &str) -> ! {
    eprintln!("a3s: {message}\n\n{usage}");
    std::process::exit(2);
}

/// Check the latest GitHub release and upgrade in place via the shared `update`
/// module (Homebrew, with a direct-download fallback).
fn self_update() -> anyhow::Result<()> {
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
        Some("login") => os_cmd::login(&args.collect::<Vec<_>>()).await,
        Some("logout") => os_cmd::logout(&args.collect::<Vec<_>>()),
        Some("model" | "models") => model::command::run(&args.collect::<Vec<_>>()).await,
        Some("account" | "accounts") => account::command::run(&args.collect::<Vec<_>>()).await,
        Some("install") => {
            let rest = args.collect::<Vec<_>>();
            match parse_component_selection(&rest, None) {
                Ok(ComponentSelection::Help) => {
                    print!("{}", install_usage_text());
                    Ok(())
                }
                Ok(ComponentSelection::Target(component)) => components::install(component),
                Err(error) => usage_error(&error, install_usage_text()),
            }
        }
        Some("update") => {
            let rest = args.collect::<Vec<_>>();
            match parse_component_selection(&rest, Some(components::ComponentId::Code)) {
                Ok(ComponentSelection::Help) => {
                    print!("{}", update_usage_text());
                    Ok(())
                }
                Ok(ComponentSelection::Target(component)) => components::update(component),
                Err(error) => usage_error(&error, update_usage_text()),
            }
        }
        Some("box") => {
            let rest = args.collect::<Vec<_>>();
            if read_only_product_request(&rest) {
                if box_cmd::run_installed(rest.clone())? {
                    Ok(())
                } else if exact_version_request(&rest) {
                    println!("a3s box engine: not installed (run `a3s install box`)");
                    Ok(())
                } else {
                    print!("{}", box_usage_text());
                    Ok(())
                }
            } else {
                box_cmd::run(rest).await
            }
        }
        Some("bench") => {
            let rest = args.collect::<Vec<_>>();
            if read_only_product_request(&rest) {
                if components::run_bench_installed(rest.clone())? {
                    Ok(())
                } else if exact_version_request(&rest) {
                    println!(
                        "a3s bench control component: not installed (run `a3s install bench`)"
                    );
                    Ok(())
                } else {
                    print!("{}", bench_usage_text());
                    Ok(())
                }
            } else {
                components::run_bench(rest)
            }
        }
        Some("search") => search_cmd::run(args.collect()).await,
        Some("list") => {
            let rest = args.collect::<Vec<_>>();
            if rest.is_empty() {
                tools::print_tool_list();
                Ok(())
            } else if exact_help_request(&rest) {
                print!("{}", list_usage_text());
                Ok(())
            } else {
                usage_error("list does not accept arguments", list_usage_text())
            }
        }
        Some("top") => top::run(args.collect()).await,
        // Pass any trailing args (e.g. `resume <id>`) through to the TUI.
        Some("code") => {
            let rest: Vec<String> = args.collect();
            if rest.first().map(String::as_str) == Some("update") {
                if rest.len() == 1 {
                    self_update()
                } else {
                    usage_error(
                        "code update does not accept arguments",
                        "usage: a3s code update",
                    )
                }
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
        assert!(text.contains("  a3s install <code|box|bench>"));
        assert!(text.contains("  a3s bench <args...>"));
        assert!(text.contains("  a3s account <command>"));
        assert!(text.contains("  a3s model <command>"));
        assert!(text.contains("a3s --version"));
    }

    #[test]
    fn component_selection_requires_a_known_install_target() {
        assert_eq!(
            super::parse_component_selection(&["bench".to_string()], None),
            Ok(super::ComponentSelection::Target(
                super::components::ComponentId::Bench
            ))
        );
        assert!(super::parse_component_selection(&[], None).is_err());
        assert!(super::parse_component_selection(&["other".to_string()], None).is_err());
        assert!(
            super::parse_component_selection(&["box".to_string(), "extra".to_string()], None)
                .is_err()
        );
    }

    #[test]
    fn bare_update_keeps_code_compatibility_alias() {
        assert_eq!(
            super::parse_component_selection(&[], Some(super::components::ComponentId::Code)),
            Ok(super::ComponentSelection::Target(
                super::components::ComponentId::Code
            ))
        );
        assert_eq!(
            super::parse_component_selection(&["--help".to_string()], None),
            Ok(super::ComponentSelection::Help)
        );
    }

    #[test]
    fn product_help_detection_does_not_capture_a_task_named_help() {
        assert!(super::read_only_product_request(&[
            "run".to_string(),
            "--help".to_string()
        ]));
        assert!(super::read_only_product_request(&[
            "help".to_string(),
            "run".to_string()
        ]));
        assert!(!super::read_only_product_request(&[
            "run".to_string(),
            "help".to_string()
        ]));
    }

    #[test]
    fn test_version_command() {
        assert_eq!(
            super::version_text(),
            format!("a3s {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}
