#![cfg(unix)]

mod support;

use std::process::Command;

use support::{
    a3s_bin, host_supports_standalone_box_asset, install_fake_download_tools, TempWorkspace,
};

#[test]
#[ignore = "soak test; run with `cargo test --test box_command_soak -- --ignored`"]
fn box_command_soak_installs_once_then_reuses_binary() {
    if !host_supports_standalone_box_asset() {
        eprintln!("skipping standalone install soak test on unsupported host target");
        return;
    }

    let tmp = TempWorkspace::new("soak");
    let bin_dir = tmp.path("install-bin");
    let home_dir = tmp.path("home");
    let tool_dir = tmp.path("tools");
    let curl_log = tmp.path("curl.log");
    let args_log = tmp.path("installed-args.log");
    install_fake_download_tools(&tool_dir, &curl_log, Some(&args_log));

    for index in 0..50 {
        let output = Command::new(a3s_bin())
            .args(["box", "run", "--iteration"])
            .arg(index.to_string())
            .env("A3S_BOX_INSTALL_DIR", &bin_dir)
            .env("HOME", &home_dir)
            .env("PATH", &tool_dir)
            .output()
            .expect("failed to run a3s box soak iteration");

        assert!(
            output.status.success(),
            "iteration {index} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("installed-box:run --iteration {index}\n")
        );
    }

    let curl_invocations = std::fs::read_to_string(curl_log).expect("curl log should be written");
    assert_eq!(
        curl_invocations
            .lines()
            .filter(|line| line.contains("/releases/latest"))
            .count(),
        1
    );
    assert_eq!(
        curl_invocations
            .lines()
            .filter(|line| line.contains("--progress-bar"))
            .count(),
        1
    );

    let delegated_args = std::fs::read_to_string(args_log).expect("args log should be written");
    assert_eq!(delegated_args.lines().count(), 50 * 3);
    assert!(delegated_args.contains("--iteration"));
    assert!(bin_dir.join("a3s-box").is_file());
}
