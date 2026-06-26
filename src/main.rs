//! `a3s` — the A3S coding agent CLI.
//!
//! `a3s code` launches the interactive terminal UI (the coding agent); the
//! rest are basic commands.

mod codex;
mod top;
mod tui;

fn usage() {
    println!("a3s {} — A3S coding agent CLI\n", env!("CARGO_PKG_VERSION"));
    println!("usage:");
    println!("  a3s code                  launch the interactive coding agent (TUI)");
    println!("  a3s code resume <id>      resume a saved session by id");
    println!("  a3s top                   live monitor for agents, containers, and processes");
    println!("  a3s update                check for and install a newer version");
    println!("  a3s --version             show version");
    println!("  a3s --help                show this help");
}

/// `[0,2,6] >= [0,2,5]` — Vec<u32> compares lexicographically = semver order.
fn version_ge(a: &str, b: &str) -> bool {
    let parse = |s: &str| {
        s.split('.')
            .filter_map(|x| x.parse::<u32>().ok())
            .collect::<Vec<_>>()
    };
    parse(a) >= parse(b)
}

/// Check the latest GitHub release; upgrade via Homebrew (how a3s is installed).
async fn self_update() -> anyhow::Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("a3s {current} — checking for updates…");
    let latest = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "https://api.github.com/repos/A3S-Lab/Cli/releases/latest",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .and_then(|v| {
            v["tag_name"]
                .as_str()
                .map(|s| s.trim_start_matches('v').to_string())
        });
    let Some(latest) = latest else {
        eprintln!("a3s: couldn't reach the release server (try again later)");
        std::process::exit(1);
    };
    if version_ge(current, &latest) {
        println!("✓ already up to date (a3s {current})");
        return Ok(());
    }
    println!("→ a3s {latest} available (you have {current})");
    let exe = std::env::current_exe().unwrap_or_default();
    let exe_s = exe.to_string_lossy();
    if exe_s.contains("/Cellar/") || exe_s.contains("/homebrew/") || exe_s.contains("/usr/local/") {
        println!("upgrading via Homebrew…");
        // `brew upgrade` reads the cached formula, so refresh the tap first —
        // otherwise it sees the old version and no-ops with "already installed".
        if let Some(repo) = std::process::Command::new("brew")
            .args(["--repo", "a3s-lab/tap"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
        {
            let _ = std::process::Command::new("git")
                .args(["-C", &repo, "pull", "--quiet", "--ff-only"])
                .status();
        }
        let _ = std::process::Command::new("brew")
            .args(["upgrade", "a3s"])
            .status();
        // `brew upgrade` exits 0 even on a no-op, so confirm the new version is
        // actually installed before claiming success.
        let installed = std::process::Command::new("brew")
            .args(["list", "--versions", "a3s"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        if installed.contains(&latest) {
            println!("✓ updated to a3s {latest}");
        } else {
            eprintln!("upgrade didn't take — run manually: brew update && brew upgrade a3s");
            std::process::exit(1);
        }
    } else {
        println!("get the new build from:");
        println!("  https://github.com/A3S-Lab/Cli/releases/latest");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("update") => self_update().await,
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
    use super::*;

    #[tokio::test]
    async fn test_help_command() {
        let output = std::process::Command::new("cargo")
            .args(["run", "--", "--help"])
            .output()
            .expect("Failed to execute process");

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("usage:"));
    }

    #[tokio::test]
    async fn test_version_command() {
        let output = std::process::Command::new("cargo")
            .args(["run", "--", "--version"])
            .output()
            .expect("Failed to execute process");

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn version_compare() {
        assert!(version_ge("0.2.6", "0.2.5")); // newer
        assert!(version_ge("0.2.6", "0.2.6")); // equal = up to date
        assert!(!version_ge("0.2.6", "0.3.0")); // older -> update
        assert!(!version_ge("0.2.6", "1.0.0"));
        assert!(version_ge("1.0.0", "0.9.9"));
    }
}
