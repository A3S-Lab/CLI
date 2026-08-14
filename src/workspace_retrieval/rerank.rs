use std::path::Path;

use a3s_acl::Block;
use a3s_code_core::{WorkspaceRerankAlgorithm, WorkspaceRerankMode, WorkspaceRerankOptions};
use anyhow::{anyhow, bail, Context};

use super::config::{bool_value, usize_value};

pub(super) const DETERMINISTIC_RERANKER_BLOCK: &str = "deterministic_reranker";

/// Trusted-host selection and hard limits for deterministic second-stage ranking.
///
/// The typed block is disabled by default. A trusted ACL must explicitly set
/// `enabled = true`; no mode or algorithm string is accepted as input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DeterministicWorkspaceRerankerConfig {
    pub enabled: bool,
    pub max_candidates: usize,
    pub max_feature_bytes_per_candidate: usize,
    pub max_fingerprints_per_candidate: usize,
    pub max_scratch_bytes: usize,
}

impl Default for DeterministicWorkspaceRerankerConfig {
    fn default() -> Self {
        let defaults = WorkspaceRerankOptions::deterministic();
        Self {
            enabled: false,
            max_candidates: defaults.max_candidates,
            max_feature_bytes_per_candidate: defaults.max_feature_bytes_per_candidate,
            max_fingerprints_per_candidate: defaults.max_fingerprints_per_candidate,
            max_scratch_bytes: defaults.max_scratch_bytes,
        }
    }
}

impl DeterministicWorkspaceRerankerConfig {
    pub(super) fn apply_block(&mut self, block: &Block, source: &Path) -> anyhow::Result<()> {
        if !block.labels.is_empty() || !block.blocks.is_empty() {
            bail!(
                "workspace_retrieval {DETERMINISTIC_RERANKER_BLOCK} in A3S ACL {} must be an unlabeled flat block",
                source.display()
            );
        }
        const KNOWN_FIELDS: &[&str] = &[
            "enabled",
            "max_candidates",
            "max_feature_bytes_per_candidate",
            "max_fingerprints_per_candidate",
            "max_scratch_bytes",
        ];
        for field in block.attributes.keys() {
            if !KNOWN_FIELDS.contains(&field.as_str()) {
                bail!(
                    "unknown workspace_retrieval {DETERMINISTIC_RERANKER_BLOCK} field `{field}` in A3S ACL {}",
                    source.display()
                );
            }
        }
        let enabled = block.attributes.get("enabled").with_context(|| {
            format!(
                "workspace_retrieval {DETERMINISTIC_RERANKER_BLOCK} in A3S ACL {} requires an explicit enabled boolean",
                source.display()
            )
        })?;
        self.enabled = bool_value(
            enabled,
            &format!("{DETERMINISTIC_RERANKER_BLOCK}.enabled"),
            source,
        )?;
        if let Some(value) = block.attributes.get("max_candidates") {
            self.max_candidates = usize_value(
                value,
                &format!("{DETERMINISTIC_RERANKER_BLOCK}.max_candidates"),
                source,
            )?;
        }
        if let Some(value) = block.attributes.get("max_feature_bytes_per_candidate") {
            self.max_feature_bytes_per_candidate = usize_value(
                value,
                &format!("{DETERMINISTIC_RERANKER_BLOCK}.max_feature_bytes_per_candidate"),
                source,
            )?;
        }
        if let Some(value) = block.attributes.get("max_fingerprints_per_candidate") {
            self.max_fingerprints_per_candidate = usize_value(
                value,
                &format!("{DETERMINISTIC_RERANKER_BLOCK}.max_fingerprints_per_candidate"),
                source,
            )?;
        }
        if let Some(value) = block.attributes.get("max_scratch_bytes") {
            self.max_scratch_bytes = usize_value(
                value,
                &format!("{DETERMINISTIC_RERANKER_BLOCK}.max_scratch_bytes"),
                source,
            )?;
        }
        Ok(())
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        self.validated_options().map(|_| ())
    }

    pub(crate) fn core_options(&self) -> anyhow::Result<Option<WorkspaceRerankOptions>> {
        let options = self.validated_options()?;
        Ok(self.enabled.then_some(options))
    }

    pub(crate) fn requested_mode(&self) -> &'static str {
        if self.enabled {
            "deterministic"
        } else {
            "rrf_only"
        }
    }

    pub(crate) fn algorithm(&self) -> &'static str {
        if self.enabled {
            WorkspaceRerankAlgorithm::RrfK60DeterministicMmrV1.as_str()
        } else {
            WorkspaceRerankAlgorithm::RrfK60.as_str()
        }
    }

    fn validated_options(&self) -> anyhow::Result<WorkspaceRerankOptions> {
        let options = WorkspaceRerankOptions::deterministic()
            .with_max_candidates(self.max_candidates)
            .with_max_feature_bytes_per_candidate(self.max_feature_bytes_per_candidate)
            .with_max_fingerprints_per_candidate(self.max_fingerprints_per_candidate)
            .with_max_scratch_bytes(self.max_scratch_bytes)
            .validate()
            .map_err(|error| {
                anyhow!("invalid workspace_retrieval deterministic_reranker: {error}")
            })?;
        debug_assert_eq!(options.mode, WorkspaceRerankMode::Deterministic);
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_rrf_only_and_core_bounds() {
        let config = DeterministicWorkspaceRerankerConfig::default();
        let defaults = WorkspaceRerankOptions::deterministic();

        assert!(!config.enabled);
        assert_eq!(config.requested_mode(), "rrf_only");
        assert_eq!(config.algorithm(), "rrf_k60");
        assert_eq!(config.max_candidates, defaults.max_candidates);
        assert_eq!(
            config.max_feature_bytes_per_candidate,
            defaults.max_feature_bytes_per_candidate
        );
        assert_eq!(
            config.max_fingerprints_per_candidate,
            defaults.max_fingerprints_per_candidate
        );
        assert_eq!(config.max_scratch_bytes, defaults.max_scratch_bytes);
        assert!(config.core_options().unwrap().is_none());
    }

    #[test]
    fn every_hard_bound_is_delegated_to_core_validation() {
        let defaults = DeterministicWorkspaceRerankerConfig::default();
        let invalid = [
            DeterministicWorkspaceRerankerConfig {
                max_candidates: 0,
                ..defaults
            },
            DeterministicWorkspaceRerankerConfig {
                max_candidates: defaults.max_candidates + 1,
                ..defaults
            },
            DeterministicWorkspaceRerankerConfig {
                max_feature_bytes_per_candidate: 3,
                ..defaults
            },
            DeterministicWorkspaceRerankerConfig {
                max_feature_bytes_per_candidate: defaults.max_feature_bytes_per_candidate + 1,
                ..defaults
            },
            DeterministicWorkspaceRerankerConfig {
                max_fingerprints_per_candidate: 0,
                ..defaults
            },
            DeterministicWorkspaceRerankerConfig {
                max_fingerprints_per_candidate: defaults.max_fingerprints_per_candidate + 1,
                ..defaults
            },
            DeterministicWorkspaceRerankerConfig {
                max_scratch_bytes: 0,
                ..defaults
            },
            DeterministicWorkspaceRerankerConfig {
                max_scratch_bytes: defaults.max_scratch_bytes + 1,
                ..defaults
            },
        ];

        for config in invalid {
            assert!(config.validate().is_err(), "config must fail: {config:?}");
        }
    }
}
