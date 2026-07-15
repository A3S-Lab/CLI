#![cfg(unix)]

mod support;

use std::path::PathBuf;
use std::process::Command;

use support::{
    a3s_bin, box_release_target, configure_component_env, start_fake_box_release, TempWorkspace,
};

#[test]
fn managed_component_installs_updates_and_uninstalls_by_receipt() {
    if box_release_target().is_none() {
        eprintln!("skipping component lifecycle test on unsupported host target");
        return;
    }
    let temp = TempWorkspace::new("managed-lifecycle");
    let first_release = start_fake_box_release(&temp, "2.5.2", None);
    let mut install = Command::new(a3s_bin());
    configure_component_env(&mut install, &temp);
    let output = install
        .args(["install", "box", "--source", "release", "--json"])
        .env("A3S_UPDATER_GITHUB_API_BASE", first_release.api_base())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "JSON install emitted progress: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["data"]["operations"][0]["version"], "2.5.2");
    drop(first_release);

    let second_release = start_fake_box_release(&temp, "2.6.0", None);
    let mut update = Command::new(a3s_bin());
    configure_component_env(&mut update, &temp);
    let output = update
        .args(["update", "box", "--json"])
        .env("A3S_UPDATER_GITHUB_API_BASE", second_release.api_base())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "legacy JSON update emitted prose: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["data"]["operations"][0]["version"], "2.6.0");
    let receipt_path = temp.path("state/components/box.json");
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    let executable = PathBuf::from(receipt["executablePath"].as_str().unwrap());
    assert!(executable.starts_with(temp.path("data/components/box/2.6.0")));
    assert!(!temp.path("data/components/box/2.5.2").exists());
    drop(second_release);

    let mut uninstall = Command::new(a3s_bin());
    configure_component_env(&mut uninstall, &temp);
    let output = uninstall
        .args(["uninstall", "box", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "status: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["data"]["operations"][0]["action"], "uninstall");
    assert!(!receipt_path.exists());
    assert!(!temp.path("data/components/box/2.6.0").exists());
}
