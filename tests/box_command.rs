#![cfg(unix)]

mod support;

use std::process::Command;

use support::{
    a3s_bin, host_supports_standalone_box_asset, install_fake_download_tools, make_executable,
    sh_quote, TempWorkspace,
};

#[test]
fn box_command_delegates_to_configured_a3s_box() {
    let tmp = TempWorkspace::new("delegate");
    let bin_dir = tmp.path("bin");
    let args_log = tmp.path("args.log");
    make_executable(
        &bin_dir.join("a3s-box"),
        &format!(
            r#"#!/bin/sh
printf '%s\n' "$@" > {}
printf 'delegated:%s\n' "$*"
exit 0
"#,
            sh_quote(&args_log)
        ),
    );

    let output = Command::new(a3s_bin())
        .args(["box", "ps", "--format", "json"])
        .env("A3S_BOX_INSTALL_DIR", &bin_dir)
        .env("PATH", "")
        .output()
        .expect("failed to run a3s box");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "delegated:ps --format json\n"
    );
    assert_eq!(
        std::fs::read_to_string(args_log).expect("args log should be written"),
        "ps\n--format\njson\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn box_command_propagates_a3s_box_exit_status() {
    let tmp = TempWorkspace::new("exit-status");
    let bin_dir = tmp.path("bin");
    make_executable(
        &bin_dir.join("a3s-box"),
        r#"#!/bin/sh
printf 'failing-box:%s\n' "$*"
exit 7
"#,
    );

    let output = Command::new(a3s_bin())
        .args(["box", "run", "bad"])
        .env("A3S_BOX_INSTALL_DIR", &bin_dir)
        .env("PATH", "")
        .output()
        .expect("failed to run a3s box");

    assert_eq!(output.status.code(), Some(7));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "failing-box:run bad\n"
    );
}

#[test]
fn box_command_auto_installs_with_download_progress() {
    if !host_supports_standalone_box_asset() {
        eprintln!("skipping standalone install test on unsupported host target");
        return;
    }

    let tmp = TempWorkspace::new("auto-install");
    let bin_dir = tmp.path("install-bin");
    let home_dir = tmp.path("home");
    let tool_dir = tmp.path("tools");
    let curl_log = tmp.path("curl.log");
    install_fake_download_tools(&tool_dir, &curl_log, None);

    let output = Command::new(a3s_bin())
        .args(["box", "version"])
        .env("A3S_BOX_INSTALL_DIR", &bin_dir)
        .env("HOME", &home_dir)
        .env("PATH", &tool_dir)
        .output()
        .expect("failed to run a3s box");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "installed-box:version\n"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("a3s: a3s-box is not installed; installing it now..."));
    assert!(stderr.contains("a3s: downloading a3s-box 2.5.2"));
    assert!(stderr.contains("#### download progress 100.0%"));
    assert!(stderr.contains("a3s: extracting a3s-box 2.5.2..."));
    assert!(stderr.contains("a3s: installing a3s-box into"));
    assert!(stderr.contains("a3s: installed a3s-box to"));

    assert!(bin_dir.join("a3s-box").is_file());
    let curl_invocations = std::fs::read_to_string(curl_log).expect("curl log should be written");
    assert!(curl_invocations.contains("/releases/latest"));
    assert!(curl_invocations.contains("--progress-bar"));
    assert!(curl_invocations.contains("--show-error"));
    assert!(curl_invocations.contains("a3s-box-v2.5.2-"));
}
