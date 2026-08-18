use super::*;

fn apply(
    config: &mut WorkspaceRetrievalConfig,
    source: &str,
    authority: WorkspaceRetrievalConfigAuthority,
) -> anyhow::Result<()> {
    let document = a3s_acl::parse_acl(source)?;
    config.apply_document(&document, authority, Path::new("test-config.acl"))
}

#[test]
fn defaults_to_disabled_without_source_egress() {
    let config = WorkspaceRetrievalConfig::default();

    assert!(!config.enabled);
    assert!(!config.allow_source_egress);
    assert_eq!(config.backend_name(), "disabled");
    assert!(!config.reranker.enabled);
    assert_eq!(config.reranker.algorithm(), "rrf_k60");
    assert_eq!(config.chunking.strategy_name(), "line");
    assert_eq!(config.chunking.target_bytes(), None);
    assert!(!config.chunking.uses_default_separators());
    assert_eq!(config.semantic_readiness_timeout_ms, 0);
    assert!(config.validate().is_ok());
}

#[test]
fn trusted_layer_can_select_typed_chunking_and_reset_to_line() {
    let mut config = WorkspaceRetrievalConfig::default();
    apply(
        &mut config,
        r#"workspace_retrieval {
  chunking {
    recursive {
      target_bytes = 8192
      overlap_bytes = 512
      separators = ["\n\n", "\n", ". ", " "]
    }
  }
}"#,
        WorkspaceRetrievalConfigAuthority::Trusted,
    )
    .unwrap();

    config.validate().unwrap();
    assert_eq!(config.chunking.strategy_name(), "recursive");
    assert_eq!(config.chunking.target_bytes(), Some(8192));
    assert_eq!(config.chunking.overlap_bytes(), Some(512));
    assert!(!config.chunking.uses_default_separators());
    let recursive = format!("{config:?}");
    assert!(recursive.contains("Recursive"), "{recursive}");
    assert!(recursive.contains("target_bytes: 8192"), "{recursive}");
    assert!(recursive.contains("overlap_bytes: 512"), "{recursive}");

    apply(
        &mut config,
        "workspace_retrieval { chunking { recursive { target_bytes = 64 } } }",
        WorkspaceRetrievalConfigAuthority::Trusted,
    )
    .unwrap();
    config.validate().unwrap();
    assert!(config.chunking.uses_default_separators());
    assert_eq!(config.chunking.separators(), None);

    apply(
        &mut config,
        "workspace_retrieval { chunking { line {} } }",
        WorkspaceRetrievalConfigAuthority::Trusted,
    )
    .unwrap();
    config.validate().unwrap();
    assert!(format!("{config:?}").contains("Lines"));
}

#[test]
fn fixed_and_recursive_chunking_reuse_core_bounds_while_disabled() {
    for source in [
        "workspace_retrieval { enabled = false chunking { fixed_window { target_bytes = 3 } } }",
        "workspace_retrieval { enabled = false chunking { fixed_window { target_bytes = 8 overlap_bytes = 8 } } }",
        "workspace_retrieval { enabled = false chunking { recursive { target_bytes = 65537 } } }",
        "workspace_retrieval { enabled = false chunking { recursive { target_bytes = 64 separators = [] } } }",
        "workspace_retrieval { enabled = false chunking { recursive { target_bytes = 64 separators = [\"\\n\", \"\\n\"] } } }",
    ] {
        let mut config = WorkspaceRetrievalConfig::default();
        apply(
            &mut config,
            source,
            WorkspaceRetrievalConfigAuthority::Trusted,
        )
        .unwrap();
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("chunking"), "{source}: {error}");
    }
}

#[test]
fn recursive_separator_count_and_byte_limits_are_core_owned() {
    let too_many = (0..17)
        .map(|index| format!("\"separator-{index}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let too_long = "x".repeat(65);
    for separators in [too_many, format!("\"{too_long}\"")] {
        let source = format!(
            "workspace_retrieval {{ enabled = false chunking {{ recursive {{ target_bytes = 64 separators = [{separators}] }} }} }}"
        );
        let mut config = WorkspaceRetrievalConfig::default();
        apply(
            &mut config,
            &source,
            WorkspaceRetrievalConfigAuthority::Trusted,
        )
        .unwrap();
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("chunking"), "{error}");
    }
}

#[test]
fn trusted_layer_can_enable_a_bounded_embedding_route() {
    let mut config = WorkspaceRetrievalConfig::default();
    apply(
        &mut config,
        r#"
workspace_retrieval {
  enabled = true
  allow_source_egress = true
  model = "local/embed-v1"
  endpoint = "http://127.0.0.1:8080/v1/embeddings"
  revision = "2026-08-14"
  dimension = 384
  normalization = "unit"
  provider_timeout_ms = 2500
  max_records = 25000
  max_bytes = 33554432
  shutdown_timeout_ms = 1000
  semantic_readiness_timeout_ms = 2500
  deterministic_reranker {
    enabled = true
    max_candidates = 24
    max_feature_bytes_per_candidate = 2048
    max_fingerprints_per_candidate = 64
    max_scratch_bytes = 1048576
  }
}
"#,
        WorkspaceRetrievalConfigAuthority::Trusted,
    )
    .unwrap();

    config.validate().unwrap();
    assert!(config.enabled);
    assert_eq!(config.model.as_deref(), Some("local/embed-v1"));
    assert_eq!(config.dimension, Some(384));
    assert_eq!(config.normalization, EmbeddingNormalization::Unit);
    assert_eq!(config.max_records, 25_000);
    assert_eq!(config.semantic_readiness_timeout_ms, 2_500);
    assert!(config.reranker.enabled);
    assert_eq!(config.reranker.max_candidates, 24);
    assert_eq!(config.reranker.algorithm(), "rrf_k60+deterministic_mmr_v1");
}

#[test]
fn trusted_layer_can_enable_local_cpu_without_source_egress() {
    let mut config = WorkspaceRetrievalConfig::default();
    apply(
        &mut config,
        r#"
workspace_retrieval {
  enabled = true
  provider_timeout_ms = 45000
  local_cpu {
    intra_threads = 3
  }
}
"#,
        WorkspaceRetrievalConfigAuthority::Trusted,
    )
    .unwrap();

    config.validate().unwrap();
    assert!(config.enabled);
    assert!(!config.allow_source_egress);
    assert_eq!(config.backend_name(), "local_cpu");
    assert!(config.model.is_none());
    let local_cpu = config.local_cpu.as_ref().unwrap();
    assert_eq!(local_cpu.intra_threads, 3);
    assert!(local_cpu.is_power_managed());

    apply(
        &mut config,
        "workspace_retrieval { enabled = false }",
        WorkspaceRetrievalConfigAuthority::Workspace,
    )
    .unwrap();
    assert_eq!(config.backend_name(), "disabled");
}

#[test]
fn local_cpu_is_typed_mutually_exclusive_and_trusted_only() {
    for source in [
        r#"workspace_retrieval {
  enabled = true
  allow_source_egress = true
  local_cpu { artifact_manifest = "model.acl" }
}"#,
        r#"workspace_retrieval {
  enabled = true
  model = "remote/embed"
  local_cpu { artifact_manifest = "model.acl" }
}"#,
        r#"workspace_retrieval {
  local_cpu { artifact_manifest = "one.acl" }
  local_cpu { artifact_manifest = "two.acl" }
}"#,
    ] {
        let mut config = WorkspaceRetrievalConfig::default();
        assert!(apply(
            &mut config,
            source,
            WorkspaceRetrievalConfigAuthority::Trusted
        )
        .is_err());
    }

    let mut config = WorkspaceRetrievalConfig::default();
    let error = apply(
        &mut config,
        r#"workspace_retrieval {
  enabled = false
  local_cpu { artifact_manifest = "model.acl" }
}"#,
        WorkspaceRetrievalConfigAuthority::Workspace,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("workspace A3S ACL"), "{error}");
}

#[test]
fn workspace_layer_cannot_enable_or_route_source_egress() {
    for source in [
        "workspace_retrieval { enabled = true }",
        "workspace_retrieval { enabled = false model = \"evil/embed\" }",
        "workspace_retrieval { allow_source_egress = true }",
        "workspace_retrieval { enabled = false deterministic_reranker { enabled = true } }",
        "workspace_retrieval { enabled = false chunking { fixed_window { target_bytes = 64 } } }",
        "workspace_retrieval { enabled = false semantic_readiness_timeout_ms = 1000 }",
    ] {
        let mut config = WorkspaceRetrievalConfig::default();
        let error = apply(
            &mut config,
            source,
            WorkspaceRetrievalConfigAuthority::Workspace,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("workspace A3S ACL"), "{error}");
    }
}

#[test]
fn semantic_readiness_timeout_is_bounded_even_while_disabled() {
    let mut config = WorkspaceRetrievalConfig::default();
    apply(
        &mut config,
        "workspace_retrieval { enabled = false semantic_readiness_timeout_ms = 30001 }",
        WorkspaceRetrievalConfigAuthority::Trusted,
    )
    .unwrap();

    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("semantic_readiness_timeout_ms"), "{error}");
}

#[test]
fn workspace_layer_can_disable_a_trusted_configuration() {
    let mut config = WorkspaceRetrievalConfig::default();
    apply(
        &mut config,
        r#"workspace_retrieval {
  enabled = true
  allow_source_egress = true
  model = "trusted/embed"
  dimension = 3
}"#,
        WorkspaceRetrievalConfigAuthority::Trusted,
    )
    .unwrap();
    apply(
        &mut config,
        "workspace_retrieval { enabled = false }",
        WorkspaceRetrievalConfigAuthority::Workspace,
    )
    .unwrap();

    assert!(!config.enabled);
    config.validate().unwrap();
}

#[test]
fn enabled_configuration_requires_two_explicit_gates_and_shape() {
    let mut config = WorkspaceRetrievalConfig {
        enabled: true,
        ..WorkspaceRetrievalConfig::default()
    };
    assert!(config
        .validate()
        .unwrap_err()
        .to_string()
        .contains("allow_source_egress"));
    config.allow_source_egress = true;
    assert!(config.validate().unwrap_err().to_string().contains("model"));
    config.model = Some("provider/model".to_string());
    assert!(config
        .validate()
        .unwrap_err()
        .to_string()
        .contains("dimension"));
}

#[test]
fn trusted_layer_can_explicitly_return_to_rrf_only() {
    let mut config = WorkspaceRetrievalConfig::default();
    apply(
        &mut config,
        "workspace_retrieval { deterministic_reranker { enabled = true } }",
        WorkspaceRetrievalConfigAuthority::Trusted,
    )
    .unwrap();
    assert!(config.reranker.enabled);

    apply(
        &mut config,
        "workspace_retrieval { deterministic_reranker { enabled = false } }",
        WorkspaceRetrievalConfigAuthority::Trusted,
    )
    .unwrap();

    assert!(!config.reranker.enabled);
    assert_eq!(config.reranker.requested_mode(), "rrf_only");
}

#[test]
fn reranker_bounds_fail_even_when_retrieval_is_disabled() {
    for (field, value) in [
        ("max_candidates", 0),
        ("max_candidates", 101),
        ("max_feature_bytes_per_candidate", 3),
        ("max_feature_bytes_per_candidate", 4_097),
        ("max_fingerprints_per_candidate", 0),
        ("max_fingerprints_per_candidate", 129),
        ("max_scratch_bytes", 0),
        ("max_scratch_bytes", 4 * 1024 * 1024 + 1),
    ] {
        let mut config = WorkspaceRetrievalConfig::default();
        apply(
            &mut config,
            &format!(
                "workspace_retrieval {{ enabled = false deterministic_reranker {{ enabled = true {field} = {value} }} }}"
            ),
            WorkspaceRetrievalConfigAuthority::Trusted,
        )
        .unwrap();

        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("rerank."), "{field}={value}: {error}");
    }
}

#[test]
fn strict_parser_rejects_typos_wrong_types_and_duplicate_blocks() {
    for source in [
        "workspace_retrieval { enabld = true }",
        "workspace_retrieval { enabled = \"true\" }",
        "workspace_retrieval \"named\" { enabled = false }",
        "workspace_retrieval { child { value = true } }",
        "workspace_retrieval { local_cpu { artifact_manifest = \"model.acl\" typo = true } }",
        "workspace_retrieval { deterministic_reranker { max_candidates = 10 } }",
        "workspace_retrieval { deterministic_reranker { enabled = true mode = \"deterministic\" } }",
        "workspace_retrieval { deterministic_reranker { enabled = true } deterministic_reranker { enabled = false } }",
        "workspace_retrieval { chunking { strategy = \"recursive\" } }",
        "workspace_retrieval { chunking {} }",
        "workspace_retrieval { chunking \"named\" { line {} } }",
        "workspace_retrieval { chunking { custom { target_bytes = 64 } } }",
        "workspace_retrieval { chunking { line { target_bytes = 64 } } }",
        "workspace_retrieval { chunking { fixed_window {} } }",
        "workspace_retrieval { chunking { recursive {} } }",
        "workspace_retrieval { chunking { recursive { target_bytes = 64 nested {} } } }",
        "workspace_retrieval { chunking { fixed_window { target_bytes = 64 } recursive { target_bytes = 64 } } }",
        "workspace_retrieval { chunking { fixed_window { target_bytes = 64 unknown = 1 } } }",
        "workspace_retrieval { chunking { recursive { target_bytes = 64 separators = \"\\n\" } } }",
        "workspace_retrieval { chunking { recursive { target_bytes = 64 separators = [\"\\n\", 1] } } }",
        "workspace_retrieval { chunking { line {} } chunking { line {} } }",
        "workspace_retrieval { enabled = false } workspace_retrieval { enabled = false }",
    ] {
        let mut config = WorkspaceRetrievalConfig::default();
        assert!(apply(
            &mut config,
            source,
            WorkspaceRetrievalConfigAuthority::Trusted
        )
        .is_err());
    }
}
