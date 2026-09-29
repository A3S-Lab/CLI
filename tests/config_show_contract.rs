use std::path::PathBuf;
use std::process::Command;

#[path = "support/config_contract.rs"]
mod config_contract_support;
use config_contract_support::test_config;

fn a3s_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_a3s"))
}

#[test]
fn config_show_is_canonical_structured_and_redacted() {
    let directory = tempfile::tempdir().expect("temp directory");
    let config = directory.path().join("config.acl");
    std::fs::write(&config, test_config()).expect("write config");

    let output = Command::new(a3s_binary())
        .arg("--config")
        .arg(&config)
        .args(["--output", "json", "config", "show"])
        .output()
        .expect("run config show");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["command"], "config.show");
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["defaultModel"], "openai/model-a");
    assert!(value["data"].get("workspaceRetrieval").is_none());
    assert!(value["data"].get("knowledgeBase").is_none());
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(!rendered.contains("top-secret-api-key"), "{rendered}");
}
