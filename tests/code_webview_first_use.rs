#![cfg(all(windows, target_arch = "x86_64"))]

mod support;

use std::io::{Cursor, Write};
use std::process::Command;

use support::{a3s_bin, FakeReleaseServer, TempWorkspace};

const WEBVIEW_VERSION: &str = "0.1.3";
const WEBVIEW_ARCHIVE: &str = "a3s-webview-v0.1.3-x86_64-pc-windows-msvc.zip";

#[test]
fn native_windows_profile_supplies_the_user_config_path_without_home() {
    let workspace = TempWorkspace::new("config-userprofile");
    let profile = workspace.path("profile");
    let project = workspace.path("project");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::create_dir_all(&project).unwrap();

    let output = Command::new(a3s_bin())
        .current_dir(&project)
        .arg("config")
        .arg("paths")
        .env_remove("HOME")
        .env("USERPROFILE", &profile)
        .env("LOCALAPPDATA", workspace.path("local-app-data"))
        .env("A3S_DATA_HOME", workspace.path("data"))
        .env("A3S_STATE_HOME", workspace.path("state"))
        .env("A3S_CACHE_HOME", workspace.path("cache"))
        .env("A3S_RUNTIME_HOME", workspace.path("runtime"))
        .env_remove("A3S_CONFIG_FILE")
        .output()
        .expect("resolve config paths without HOME");

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let expected = profile.join(".a3s/config.acl");
    assert!(stdout.contains(&expected.display().to_string()), "{stdout}");
}

#[test]
fn component_install_uses_native_windows_profile_when_home_is_unset() {
    let workspace = TempWorkspace::new("webview-userprofile-first-use");
    let release = start_fake_webview_release();
    let profile = workspace.path("profile");
    let project = workspace.path("project");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::create_dir_all(&project).unwrap();

    let output = Command::new(a3s_bin())
        .current_dir(&project)
        .arg("--no-progress")
        .arg("install")
        .arg("webview")
        .env_remove("HOME")
        .env("USERPROFILE", &profile)
        .env("LOCALAPPDATA", workspace.path("local-app-data"))
        .env("A3S_DATA_HOME", workspace.path("data"))
        .env("A3S_STATE_HOME", workspace.path("state"))
        .env("A3S_CACHE_HOME", workspace.path("cache"))
        .env("A3S_RUNTIME_HOME", workspace.path("runtime"))
        .env("A3S_UPDATER_GITHUB_API_BASE", release.api_base())
        .env("PATH", "")
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env_remove("A3S_CONFIG_FILE")
        .env_remove("A3S_OFFLINE")
        .env_remove("A3S_NO_AUTO_INSTALL")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .output()
        .expect("install WebView without HOME");

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt_path = workspace.path("state/components/webview.json");
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    assert_eq!(receipt["componentId"], "webview");
    assert_eq!(receipt["version"], WEBVIEW_VERSION);
}

fn start_fake_webview_release() -> FakeReleaseServer {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("a3s-webview.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(&fake_webview_binary()).unwrap();
    let archive = writer.finish().unwrap().into_inner();
    FakeReleaseServer::start("WebView", WEBVIEW_VERSION, WEBVIEW_ARCHIVE, archive)
}

fn fake_webview_binary() -> Vec<u8> {
    std::fs::read(a3s_bin()).expect("read version-capable executable fixture")
}
