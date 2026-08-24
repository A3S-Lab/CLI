mod support;

use std::fs;
use std::process::{Command, Output};

use serde_json::{json, Value};
use support::{a3s_bin, configure_component_env, TempWorkspace};

#[test]
fn plugin_help_exposes_the_complete_command_contract() {
    let output = Command::new(a3s_bin())
        .args(["plugin", "--help"])
        .output()
        .expect("run plugin help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for command in [
        "search",
        "inspect",
        "list",
        "install",
        "upgrade",
        "apply",
        "enable",
        "disable",
        "uninstall",
    ] {
        assert!(
            stdout
                .lines()
                .any(|line| line.trim_start().starts_with(command)),
            "missing plugin command {command:?}:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("mcp-serve"),
        "the host-owned management transport must stay out of user-facing help:\n{stdout}"
    );
}

#[test]
fn plugin_enablement_exposes_review_before_apply_flags() {
    for action in ["enable", "disable"] {
        let output = Command::new(a3s_bin())
            .args(["plugin", action, "--help"])
            .output()
            .expect("run plugin enablement help");
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("--dry-run"), "{action} help:\n{stdout}");
        assert!(stdout.contains("--yes"), "{action} help:\n{stdout}");
    }
}

#[test]
fn plugin_mutations_require_explicit_non_interactive_authority() {
    let temp = TempWorkspace::new("plugin-mutation-authority");
    let digest = "a".repeat(64);
    let cases = [
        (
            vec!["--output", "json", "plugin", "install", "acme/research"],
            "plugin.install",
        ),
        (
            vec!["--output", "json", "plugin", "upgrade", "acme/research"],
            "plugin.upgrade",
        ),
        (
            vec![
                "--output",
                "json",
                "plugin",
                "apply",
                "plugin-install-abc",
                "--plan-digest",
                &digest,
            ],
            "plugin.apply",
        ),
        (
            vec!["--output", "json", "plugin", "enable", "acme/research"],
            "plugin.enable",
        ),
        (
            vec!["--output", "json", "plugin", "disable", "acme/research"],
            "plugin.disable",
        ),
        (
            vec!["--output", "json", "plugin", "uninstall", "acme/research"],
            "plugin.uninstall",
        ),
    ];

    for (args, expected_command) in cases {
        let output = run_isolated(&temp, &args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        let value = output_json(&output);
        assert_eq!(value["command"], expected_command);
        assert_eq!(value["error"]["code"], "usage.invalid");
    }

    assert!(
        !temp.path("state/plugin-manager/operations").exists(),
        "rejected mutations must not persist a plan or intent"
    );
}

#[test]
fn plugin_commands_reject_jsonl_before_observation_or_mutation() {
    let temp = TempWorkspace::new("plugin-jsonl");
    let output = run_isolated(&temp, &["--output", "jsonl", "plugin", "list"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let value = output_json(&output);
    assert_eq!(value["command"], "plugin.list");
    assert_eq!(value["type"], "error");
    assert_eq!(value["error"]["code"], "usage.invalid");
}

#[test]
fn plugin_list_uses_the_standard_manager_installed_page() {
    let temp = TempWorkspace::new("plugin-list-unavailable");
    let output = run_isolated(&temp, &["--output", "json", "plugin", "list"]);

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "machine diagnostics must remain structured"
    );
    let value = output_json(&output);
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["command"], "plugin.list");
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["scope"]["kind"], "user");
    assert_eq!(value["data"]["scope"]["id"], "user/current");
    assert!(value["data"]["snapshotDigest"]
        .as_str()
        .is_some_and(|digest| digest.starts_with("sha256:")));
    assert_eq!(value["data"]["packages"], json!([]));
    assert!(value["data"].get("nextCursor").is_none());
    assert_eq!(value["warnings"], json!([]));
}

#[test]
fn offline_plugin_search_preserves_the_use_registry_error_contract() {
    let temp = TempWorkspace::new("plugin-search-offline");
    let output = run_isolated(
        &temp,
        &[
            "--offline",
            "--output",
            "json",
            "plugin",
            "search",
            "science",
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value = output_json(&output);
    assert_eq!(value["command"], "plugin.search");
    assert_eq!(value["ok"], false);
    assert_eq!(
        value["error"]["code"],
        "use.extension.registry_source_default_missing"
    );
    assert!(value["error"]["suggestion"].is_string());
}

#[test]
fn offline_plugin_inspect_preserves_the_use_registry_error_contract() {
    let temp = TempWorkspace::new("plugin-inspect-missing");
    let output = run_isolated(
        &temp,
        &[
            "--offline",
            "--output",
            "json",
            "plugin",
            "inspect",
            "acme/missing",
        ],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let value = output_json(&output);
    assert_eq!(value["command"], "plugin.inspect");
    assert_eq!(value["ok"], false);
    assert_eq!(
        value["error"]["code"],
        "use.extension.registry_source_default_missing"
    );
}

#[test]
fn plugin_inspect_rejects_noncanonical_package_ids_before_catalog_access() {
    let temp = TempWorkspace::new("plugin-inspect-invalid");
    let output = run_isolated(
        &temp,
        &[
            "--offline",
            "--output",
            "json",
            "plugin",
            "inspect",
            "use/acme/research",
        ],
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let value = output_json(&output);
    assert_eq!(value["command"], "plugin.inspect");
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "usage.invalid");
}

fn run_isolated(temp: &TempWorkspace, args: &[&str]) -> Output {
    let workspace = temp.path("workspace");
    let config = temp.path("config/config.acl");
    fs::create_dir_all(&workspace).expect("create plugin command workspace");
    fs::create_dir_all(config.parent().expect("config parent")).expect("create config directory");
    fs::write(&config, "").expect("write empty plugin command config");

    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, temp);
    command
        .env("A3S_USE_INSTALL_DIR", temp.path("missing-use"))
        .env_remove("A3S_CONFIG_FILE")
        .arg("--config")
        .arg(config)
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .expect("run isolated plugin command")
}

fn output_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "plugin command did not return JSON ({error}):\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}
