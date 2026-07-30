mod support;

use std::fs;
use std::process::{Command, Output};

use serde_json::{json, Value};
use support::{a3s_bin, configure_component_env, TempWorkspace};

#[test]
fn plugin_help_exposes_the_read_only_command_contract() {
    let output = Command::new(a3s_bin())
        .args(["plugin", "--help"])
        .output()
        .expect("run plugin help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for command in ["search", "inspect", "list"] {
        assert!(
            stdout
                .lines()
                .any(|line| line.trim_start().starts_with(command)),
            "missing plugin command {command:?}:\n{stdout}"
        );
    }
}

#[test]
fn plugin_list_distinguishes_unavailable_use_from_an_empty_installation() {
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
    assert_eq!(value["data"]["schemaVersion"], 1);
    assert_eq!(value["data"]["available"], false);
    assert_eq!(value["data"]["items"], json!([]));
    assert!(value["data"]["error"].is_string(), "{value:#}");
    assert_eq!(value["warnings"].as_array().map(Vec::len), Some(1));
}

#[test]
fn offline_plugin_search_uses_only_local_verified_state() {
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

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let value = output_json(&output);
    assert_eq!(value["command"], "plugin.search");
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["query"]["text"], "science");
    assert_eq!(value["data"]["query"]["offline"], true);
    assert_eq!(value["data"]["items"], json!([]));
    assert_eq!(value["data"]["totalMatches"], 0);
    assert_eq!(value["data"]["returnedItems"], 0);
    assert!(
        value["warnings"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "unavailable installation state must remain explicit: {value:#}"
    );
}

#[test]
fn offline_plugin_inspect_has_a_typed_not_found_result() {
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
    assert_eq!(value["error"]["code"], "plugin.not_found");
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
