//! `a3s` — umbrella CLI for the A3S platform.
//!
//! `a3s <tool> [args...]` runs the matching `a3s-<tool>` binary on PATH (the
//! way `git foo` runs `git-foo`), so every A3S tool is reachable under one
//! command without this crate depending on any of them:
//!
//!   a3s code            →  a3s-code            (launches its TUI with no args)
//!   a3s box  ps         →  a3s-box ps
//!   a3s <x>  [args]     →  a3s-<x> [args]
//!
//! `a3s list` shows which `a3s-*` tools are installed; `a3s <tool> --help`
//! shows a tool's own help.

use std::path::Path;
use std::process::Command;

/// Tools we document in `--help` (informational only — dispatch works for any
/// `a3s-<name>` on PATH, installed or not).
const KNOWN: &[(&str, &str)] = &[
    ("code", "AI coding agent — interactive TUI"),
    ("box", "microVM sandbox runtime"),
    ("gateway", "API gateway / reverse proxy"),
    ("search", "code & content search"),
    ("power", "power/observability tooling"),
];

fn usage() {
    println!(
        "a3s {} — umbrella CLI for the A3S platform\n",
        env!("CARGO_PKG_VERSION")
    );
    println!("usage: a3s <tool> [args...]\n");
    println!("Runs the matching a3s-<tool> binary on PATH. Known tools:");
    for (name, desc) in KNOWN {
        println!("  {name:<9} {desc}");
    }
    println!(
        "\n  a3s list             list installed a3s-* tools\n  \
         a3s <tool> --help    a tool's own help\n  \
         a3s --version        this dispatcher's version"
    );
}

/// Print every `a3s-<tool>` executable found on PATH.
fn list_tools() {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut found: Vec<String> = Vec::new();
    for dir in std::env::split_paths(&path) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(tool) = name.strip_prefix("a3s-") {
                    // Skip helper sub-binaries (e.g. a3s-box-shim) and dupes.
                    if !tool.contains('-') && is_executable(&entry.path()) {
                        let t = tool.to_string();
                        if !found.contains(&t) {
                            found.push(t);
                        }
                    }
                }
            }
        }
    }
    found.sort();
    if found.is_empty() {
        println!("no a3s-* tools found on PATH");
    } else {
        println!("installed a3s tools:");
        for t in found {
            println!("  a3s {t}");
        }
    }
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

fn run(target: &str, rest: &[String]) -> ! {
    // On Unix, exec replaces this process so signals/TTY/exit-code pass straight
    // through to the tool. Elsewhere, spawn and forward the exit code.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = Command::new(target).args(rest).exec();
        eprintln!("a3s: cannot run '{target}': {err}");
        eprintln!("is '{target}' installed and on PATH? try 'a3s list' or 'a3s --help'");
        std::process::exit(127);
    }
    #[cfg(not(unix))]
    {
        match Command::new(target).args(rest).status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(1)),
            Err(err) => {
                eprintln!("a3s: cannot run '{target}': {err}");
                eprintln!("is '{target}' installed and on PATH? try 'a3s list' or 'a3s --help'");
                std::process::exit(127);
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => usage(),
        Some("-V" | "--version") => println!("a3s {}", env!("CARGO_PKG_VERSION")),
        Some("list") => list_tools(),
        Some(sub) => run(&format!("a3s-{sub}"), &args[1..]),
    }
}
