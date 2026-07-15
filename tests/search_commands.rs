#![cfg(unix)]

mod support;

use std::process::Command;

use support::{a3s_bin, TempWorkspace};

fn run(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(a3s_bin())
        .args(args)
        .env("HOME", home)
        .env_remove("A3S_CONFIG_FILE")
        .output()
        .unwrap_or_else(|error| panic!("failed to run a3s {args:?}: {error}"))
}

#[test]
fn search_read_only_commands_do_not_create_runtime_state() {
    let workspace = TempWorkspace::new("search-read-only");
    let home = workspace.path("home");
    std::fs::create_dir_all(&home).unwrap();

    for args in [
        &["search", "--help"][..],
        &["search", "engines"][..],
        &["search", "status"][..],
        &["search", "browser", "list"][..],
        &["search", "doctor"][..],
    ] {
        let output = run(&home, args);
        assert!(
            output.status.success(),
            "a3s {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    assert!(
        !home.join(".a3s").exists(),
        "read-only search commands created managed state"
    );
}

#[test]
fn search_engine_catalog_and_browser_lifecycle_are_visible() {
    let workspace = TempWorkspace::new("search-catalog");
    let home = workspace.path("home");
    std::fs::create_dir_all(&home).unwrap();

    let engines = run(&home, &["search", "engines"]);
    let text = String::from_utf8_lossy(&engines.stdout);
    for expected in ["ddg", "baidu", "bing_cn", "chrome", "lightpanda"] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }

    let help = run(&home, &["search", "--help"]);
    let text = String::from_utf8_lossy(&help.stdout);
    for expected in [
        "browser list",
        "browser install",
        "browser update",
        "browser repair",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
}
