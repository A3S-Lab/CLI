#![cfg(unix)]

mod support;

use std::path::PathBuf;
use std::process::Command;

use support::{
    a3s_bin, box_release_target, configure_component_env, make_executable, sh_quote,
    start_fake_box_release, TempWorkspace,
};

#[test]
fn box_command_delegates_to_configured_a3s_box() {
    let temp = TempWorkspace::new("delegate");
    let bin_dir = temp.path("bin");
    let args_log = temp.path("args.log");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'a3s-box 2.5.2\\n'\n  exit 0\nfi\nprintf '%s\\n' \"$@\" > {}\nprintf 'delegated:%s\\n' \"$*\"\nexit 0\n",
        sh_quote(&args_log)
    );
    make_executable(&bin_dir.join("a3s-box"), &script);

    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args(["box", "ps", "--format", "json"])
        .env("A3S_BOX_INSTALL_DIR", &bin_dir)
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
    let temp = TempWorkspace::new("exit-status");
    let bin_dir = temp.path("bin");
    make_executable(
        &bin_dir.join("a3s-box"),
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'a3s-box 2.5.2\\n'\n  exit 0\nfi\nprintf 'failing-box:%s\\n' \"$*\"\nexit 7\n",
    );

    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args(["box", "run", "bad"])
        .env("A3S_BOX_INSTALL_DIR", &bin_dir)
        .output()
        .expect("failed to run a3s box");

    assert_eq!(output.status.code(), Some(7));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "failing-box:run bad\n"
    );
}

#[test]
fn box_command_auto_installs_a_verified_release() {
    if box_release_target().is_none() {
        eprintln!("skipping release install test on unsupported host target");
        return;
    }
    let temp = TempWorkspace::new("auto-install");
    let server = start_fake_box_release(&temp, "2.5.2", None);

    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args(["box", "version"])
        .env("A3S_UPDATER_GITHUB_API_BASE", server.api_base())
        .output()
        .expect("failed to run a3s box");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "installed-box:version\n"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("component 'box' is not installed; installing it now"));
    assert!(stderr.contains("resolving release for 'box'"));
    assert!(stderr.contains("downloading 'box' 2.5.2"));

    let receipt = temp.path("state/components/box.json");
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
    assert_eq!(receipt["componentId"], "box");
    assert_eq!(receipt["version"], "2.5.2");
    assert_eq!(receipt["provenance"], "github-release");
    let executable = PathBuf::from(receipt["executablePath"].as_str().unwrap());
    assert!(executable.is_file());
    assert!(executable.starts_with(temp.path("data/components/box/2.5.2")));

    let requests = server.requests();
    assert!(requests
        .iter()
        .any(|path| path.ends_with("/repos/A3S-Lab/Box/releases/latest")));
    assert!(requests
        .iter()
        .any(|path| path.contains("/assets/a3s-box-v2.5.2-")));
}

#[test]
fn box_command_respects_no_auto_install() {
    let temp = TempWorkspace::new("no-auto-install");
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args(["box", "version"])
        .env("A3S_NO_AUTO_INSTALL", "1")
        .output()
        .expect("failed to run a3s box");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("A3S_NO_AUTO_INSTALL"));
    assert!(stderr.contains("a3s install box"));
}
