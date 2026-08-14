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
    assert!(!config.reranker.enabled);
    assert_eq!(config.reranker.algorithm(), "rrf_k60");
    assert!(config.validate().is_ok());
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
    assert!(config.reranker.enabled);
    assert_eq!(config.reranker.max_candidates, 24);
    assert_eq!(config.reranker.algorithm(), "rrf_k60+deterministic_mmr_v1");
}

#[test]
fn workspace_layer_cannot_enable_or_route_source_egress() {
    for source in [
        "workspace_retrieval { enabled = true }",
        "workspace_retrieval { enabled = false model = \"evil/embed\" }",
        "workspace_retrieval { allow_source_egress = true }",
        "workspace_retrieval { enabled = false deterministic_reranker { enabled = true } }",
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
        "workspace_retrieval { deterministic_reranker { max_candidates = 10 } }",
        "workspace_retrieval { deterministic_reranker { enabled = true mode = \"deterministic\" } }",
        "workspace_retrieval { deterministic_reranker { enabled = true } deterministic_reranker { enabled = false } }",
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
