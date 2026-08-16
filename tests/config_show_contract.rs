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
    assert_eq!(
        value["data"]["workspaceRetrieval"]["rerank"]["requestedMode"],
        "rrf_only"
    );
    assert_eq!(
        value["data"]["workspaceRetrieval"]["rerank"]["algorithm"],
        "rrf_k60"
    );
    assert_eq!(
        value["data"]["workspaceRetrieval"]["rerank"]["active"],
        false
    );
    assert_eq!(
        value["data"]["workspaceRetrieval"]["chunking"]["strategy"],
        "line"
    );
    let retrieval = &value["data"]["workspaceRetrieval"];
    assert_eq!(retrieval["maxEmbeddingBatchInputs"], 64);
    let local_cpu_available = retrieval["localCpuAvailable"].as_bool().unwrap();
    let local_cpu_reason = &retrieval["localCpuUnavailableReason"];
    if cfg!(feature = "local-cpu-embedding") {
        assert_eq!(local_cpu_available, local_cpu_reason.is_null());
        if !local_cpu_available {
            assert!(["missing_x86_64_v3", "unsupported_architecture"]
                .contains(&local_cpu_reason.as_str().unwrap()));
        }
    } else {
        assert!(!local_cpu_available);
        assert_eq!(local_cpu_reason, "feature_disabled");
    }
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(!rendered.contains("top-secret-api-key"), "{rendered}");
}

#[test]
fn config_show_reports_effective_chunking_and_reranking_without_route_secrets() {
    let directory = tempfile::tempdir().expect("temp directory");
    let config = directory.path().join("config.acl");
    let source = format!(
        r#"{}
workspace_retrieval {{
  enabled = true
  allow_source_egress = true
  model = "openai/embed-v1"
  dimension = 3
  semantic_readiness_timeout_ms = 4321
  deterministic_reranker {{
    enabled = true
    max_candidates = 12
  }}
  chunking {{
    recursive {{
      target_bytes = 8192
      overlap_bytes = 512
      separators = ["\n\n", "\n", ". ", " "]
    }}
  }}
}}
"#,
        test_config()
    );
    std::fs::write(&config, source).expect("write config");

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
    let rerank = &value["data"]["workspaceRetrieval"]["rerank"];
    assert_eq!(
        value["data"]["workspaceRetrieval"]["maxEmbeddingBatchInputs"],
        64
    );
    assert_eq!(rerank["active"], true);
    assert_eq!(rerank["requestedMode"], "deterministic");
    assert_eq!(rerank["algorithm"], "rrf_k60+deterministic_mmr_v1");
    assert_eq!(rerank["maxCandidates"], 12);
    assert_eq!(
        value["data"]["workspaceRetrieval"]["semanticReadinessTimeoutMs"],
        4321
    );
    let chunking = &value["data"]["workspaceRetrieval"]["chunking"];
    assert_eq!(chunking["strategy"], "recursive");
    assert_eq!(chunking["targetBytes"], 8192);
    assert_eq!(chunking["overlapBytes"], 512);
    assert_eq!(
        chunking["separators"],
        serde_json::json!(["\n\n", "\n", ". ", " "])
    );
    let rendered = String::from_utf8_lossy(&output.stdout);
    assert!(!rendered.contains("top-secret-api-key"), "{rendered}");
    assert!(!rendered.contains("https://example.com"), "{rendered}");
}
