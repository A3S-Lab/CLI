#![cfg(unix)]

#[allow(dead_code)]
mod support;

use std::process::{Command, Output};

use support::{a3s_bin, make_executable, TempWorkspace};

fn run(home: &std::path::Path, config: &std::path::Path, args: &[&str]) -> Output {
    Command::new(a3s_bin())
        .args(args)
        .env("HOME", home)
        .env("A3S_CONFIG_FILE", config)
        .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .env_remove("CODEX_HOME")
        .env("PATH", home.join("bin"))
        .env("RUST_BACKTRACE", "0")
        .output()
        .unwrap_or_else(|error| panic!("failed to run a3s {args:?}: {error}"))
}

#[test]
fn product_account_commands_delegate_authentication_to_the_owning_cli() {
    let workspace = TempWorkspace::new("account-product-delegation");
    let home = workspace.path("home");
    let config = workspace.path("config.acl");
    let log = workspace.path("product.log");
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(&config, "").unwrap();
    for product in ["claude", "codex"] {
        make_executable(
            &home.join("bin").join(product),
            &format!(
                "#!/bin/sh\nprintf '{}:%s\\n' \"$*\" >> '{}'\n",
                product,
                log.display()
            ),
        );
    }

    for args in [
        &["account", "login", "claude-code"][..],
        &["account", "logout", "claude-code"][..],
        &["account", "login", "codex"][..],
        &["account", "logout", "codex"][..],
    ] {
        let output = run(&home, &config, args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        "claude:auth login\nclaude:auth logout\ncodex:login\ncodex:logout\n"
    );
}

#[test]
fn account_list_reports_product_owned_login_sources_without_secrets() {
    let workspace = TempWorkspace::new("account-list");
    let home = workspace.path("home");
    let config = workspace.path("config.acl");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(&config, "os = \"https://os.example.test\"\n").unwrap();

    let login = run(&home, &config, &["account", "login", "a3s-os", "secret"]);
    assert!(login.status.success());

    let output = run(&home, &config, &["account", "list"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("claude-code\tsigned-out"));
    assert!(stdout.contains("codex\tsigned-out"));
    assert!(stdout.contains("a3s-os\tsigned-in\thttps://os.example.test"));
    assert!(!stdout.contains("secret"));

    let logout = run(&home, &config, &["account", "logout", "a3s-os"]);
    assert!(logout.status.success());
    assert!(!home.join(".a3s/os-auth.json").exists());
}
