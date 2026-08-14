use std::fmt;
use std::path::Path;

use a3s_acl::{Block, Document, Value};
use a3s_code_core::embedding::EmbeddingNormalization;
use anyhow::{bail, Context};

const DEFAULT_PROVIDER_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_RECORDS: usize = 100_000;
const DEFAULT_MAX_BYTES: usize = 128 * 1024 * 1024;
const DEFAULT_SHUTDOWN_TIMEOUT_MS: u64 = 5_000;
const MAX_EMBEDDING_DIMENSION: usize = 65_536;
const MAX_PROVIDER_TIMEOUT_MS: u64 = 5 * 60 * 1_000;
const MAX_SHUTDOWN_TIMEOUT_MS: u64 = 30_000;
const MAX_ACL_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
const MAX_INDEX_RECORDS: usize = 5_000_000;
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceRetrievalConfigAuthority {
    Trusted,
    Workspace,
}

/// Host-owned configuration for one session-bound semantic workspace index.
///
/// The default is deliberately disabled. Only a user configuration or an
/// explicitly selected ACL file may authorize source egress. A discovered
/// workspace ACL may turn an inherited configuration off, but cannot enable or
/// route it.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct WorkspaceRetrievalConfig {
    pub enabled: bool,
    pub allow_source_egress: bool,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub revision: Option<String>,
    pub dimension: Option<usize>,
    pub normalization: EmbeddingNormalization,
    pub provider_timeout_ms: u64,
    pub max_records: usize,
    pub max_bytes: usize,
    pub shutdown_timeout_ms: u64,
}

impl fmt::Debug for WorkspaceRetrievalConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkspaceRetrievalConfig")
            .field("enabled", &self.enabled)
            .field("allow_source_egress", &self.allow_source_egress)
            .field("model", &self.model)
            .field("endpoint", &self.endpoint.as_ref().map(|_| "<configured>"))
            .field("revision", &self.revision)
            .field("dimension", &self.dimension)
            .field("normalization", &self.normalization)
            .field("provider_timeout_ms", &self.provider_timeout_ms)
            .field("max_records", &self.max_records)
            .field("max_bytes", &self.max_bytes)
            .field("shutdown_timeout_ms", &self.shutdown_timeout_ms)
            .finish()
    }
}

impl Default for WorkspaceRetrievalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_source_egress: false,
            model: None,
            endpoint: None,
            revision: None,
            dimension: None,
            normalization: EmbeddingNormalization::None,
            provider_timeout_ms: DEFAULT_PROVIDER_TIMEOUT_MS,
            max_records: DEFAULT_MAX_RECORDS,
            max_bytes: DEFAULT_MAX_BYTES,
            shutdown_timeout_ms: DEFAULT_SHUTDOWN_TIMEOUT_MS,
        }
    }
}

impl WorkspaceRetrievalConfig {
    pub(crate) fn apply_document(
        &mut self,
        document: &Document,
        authority: WorkspaceRetrievalConfigAuthority,
        source: &Path,
    ) -> anyhow::Result<()> {
        let blocks = document
            .blocks
            .iter()
            .filter(|block| block.name == "workspace_retrieval")
            .collect::<Vec<_>>();
        if blocks.len() > 1 {
            bail!(
                "A3S ACL {} contains more than one workspace_retrieval block",
                source.display()
            );
        }
        let Some(block) = blocks.first() else {
            return Ok(());
        };
        validate_block_shape(block, source)?;
        match authority {
            WorkspaceRetrievalConfigAuthority::Trusted => self.apply_trusted_block(block, source),
            WorkspaceRetrievalConfigAuthority::Workspace => {
                self.apply_workspace_block(block, source)
            }
        }
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if !self.allow_source_egress {
            bail!(
                "workspace_retrieval requires allow_source_egress = true in a trusted user or explicit ACL file"
            );
        }
        let model = self.model.as_deref().context(
            "workspace_retrieval requires model = \"provider/embedding-model\" when enabled",
        )?;
        validate_model_route(model)?;
        let dimension = self.dimension.context(
            "workspace_retrieval requires the embedding model output dimension when enabled",
        )?;
        if !(1..=MAX_EMBEDDING_DIMENSION).contains(&dimension) {
            bail!("workspace_retrieval dimension must be between 1 and {MAX_EMBEDDING_DIMENSION}");
        }
        if self
            .revision
            .as_ref()
            .is_some_and(|revision| revision.len() > 256 || revision.chars().any(char::is_control))
        {
            bail!("workspace_retrieval revision must be at most 256 printable characters");
        }
        if self.provider_timeout_ms == 0 || self.provider_timeout_ms > MAX_PROVIDER_TIMEOUT_MS {
            bail!("workspace_retrieval provider_timeout_ms must be between 1 and 300000");
        }
        if self.max_records == 0 || self.max_records > MAX_INDEX_RECORDS {
            bail!("workspace_retrieval max_records must be between 1 and 5000000");
        }
        if self.max_bytes == 0 || self.max_bytes as u64 > MAX_INDEX_BYTES {
            bail!("workspace_retrieval max_bytes must be between 1 and 4294967296");
        }
        if dimension
            .checked_mul(std::mem::size_of::<f32>())
            .is_none_or(|vector_bytes| vector_bytes > self.max_bytes)
        {
            bail!("workspace_retrieval max_bytes cannot hold one embedding vector");
        }
        if self.shutdown_timeout_ms == 0 || self.shutdown_timeout_ms > MAX_SHUTDOWN_TIMEOUT_MS {
            bail!("workspace_retrieval shutdown_timeout_ms must be between 1 and 30000");
        }
        Ok(())
    }

    fn apply_trusted_block(&mut self, block: &Block, source: &Path) -> anyhow::Result<()> {
        const KNOWN_FIELDS: &[&str] = &[
            "enabled",
            "allow_source_egress",
            "model",
            "endpoint",
            "revision",
            "dimension",
            "normalization",
            "provider_timeout_ms",
            "max_records",
            "max_bytes",
            "shutdown_timeout_ms",
        ];
        for field in block.attributes.keys() {
            if !KNOWN_FIELDS.contains(&field.as_str()) {
                bail!(
                    "unknown workspace_retrieval field `{field}` in A3S ACL {}",
                    source.display()
                );
            }
        }
        if let Some(value) = block.attributes.get("enabled") {
            self.enabled = bool_value(value, "enabled", source)?;
        }
        if let Some(value) = block.attributes.get("allow_source_egress") {
            self.allow_source_egress = bool_value(value, "allow_source_egress", source)?;
        }
        if let Some(value) = block.attributes.get("model") {
            self.model = Some(string_value(value, "model", source)?.to_string());
        }
        if let Some(value) = block.attributes.get("endpoint") {
            self.endpoint = Some(string_value(value, "endpoint", source)?.to_string());
        }
        if let Some(value) = block.attributes.get("revision") {
            self.revision = Some(string_value(value, "revision", source)?.to_string());
        }
        if let Some(value) = block.attributes.get("dimension") {
            self.dimension = Some(usize_value(value, "dimension", source)?);
        }
        if let Some(value) = block.attributes.get("normalization") {
            self.normalization = match string_value(value, "normalization", source)? {
                "none" => EmbeddingNormalization::None,
                "unit" => EmbeddingNormalization::Unit,
                _ => bail!(
                    "workspace_retrieval normalization in A3S ACL {} must be `none` or `unit`",
                    source.display()
                ),
            };
        }
        if let Some(value) = block.attributes.get("provider_timeout_ms") {
            self.provider_timeout_ms = u64_value(value, "provider_timeout_ms", source)?;
        }
        if let Some(value) = block.attributes.get("max_records") {
            self.max_records = usize_value(value, "max_records", source)?;
        }
        if let Some(value) = block.attributes.get("max_bytes") {
            self.max_bytes = usize_value(value, "max_bytes", source)?;
        }
        if let Some(value) = block.attributes.get("shutdown_timeout_ms") {
            self.shutdown_timeout_ms = u64_value(value, "shutdown_timeout_ms", source)?;
        }
        Ok(())
    }

    fn apply_workspace_block(&mut self, block: &Block, source: &Path) -> anyhow::Result<()> {
        if block.attributes.len() != 1 || !block.attributes.contains_key("enabled") {
            bail!(
                "workspace A3S ACL {} may only set workspace_retrieval enabled = false; enable and route source egress from a user ACL or --config file",
                source.display()
            );
        }
        if bool_value(&block.attributes["enabled"], "enabled", source)? {
            bail!(
                "workspace A3S ACL {} cannot enable workspace_retrieval; use a user ACL or an explicit --config file",
                source.display()
            );
        }
        self.enabled = false;
        Ok(())
    }
}

fn validate_block_shape(block: &Block, source: &Path) -> anyhow::Result<()> {
    if !block.labels.is_empty() || !block.blocks.is_empty() {
        bail!(
            "workspace_retrieval in A3S ACL {} must be an unlabeled flat block",
            source.display()
        );
    }
    Ok(())
}

fn validate_model_route(model: &str) -> anyhow::Result<()> {
    let Some((provider, model_id)) = model.split_once('/') else {
        bail!("workspace_retrieval model must use the provider/model format");
    };
    if provider.is_empty()
        || model_id.is_empty()
        || model_id.contains('/')
        || model.len() > 256
        || model.chars().any(char::is_control)
    {
        bail!("workspace_retrieval model must use one non-empty provider/model identifier");
    }
    Ok(())
}

fn bool_value(value: &Value, field: &str, source: &Path) -> anyhow::Result<bool> {
    value.as_bool().with_context(|| {
        format!(
            "workspace_retrieval {field} in A3S ACL {} must be a boolean",
            source.display()
        )
    })
}

fn string_value<'a>(value: &'a Value, field: &str, source: &Path) -> anyhow::Result<&'a str> {
    let value = value.as_str().with_context(|| {
        format!(
            "workspace_retrieval {field} in A3S ACL {} must be a string",
            source.display()
        )
    })?;
    if value.trim().is_empty() {
        bail!(
            "workspace_retrieval {field} in A3S ACL {} must not be empty",
            source.display()
        );
    }
    Ok(value)
}

fn usize_value(value: &Value, field: &str, source: &Path) -> anyhow::Result<usize> {
    let value = numeric_value(value, field, source)?;
    if value > usize::MAX as u64 {
        bail!(
            "workspace_retrieval {field} in A3S ACL {} is too large",
            source.display()
        );
    }
    Ok(value as usize)
}

fn u64_value(value: &Value, field: &str, source: &Path) -> anyhow::Result<u64> {
    numeric_value(value, field, source)
}

fn numeric_value(value: &Value, field: &str, source: &Path) -> anyhow::Result<u64> {
    let number = value.as_number().with_context(|| {
        format!(
            "workspace_retrieval {field} in A3S ACL {} must be a non-negative integer",
            source.display()
        )
    })?;
    if !number.is_finite() || number < 0.0 || number.fract() != 0.0 || number > MAX_ACL_SAFE_INTEGER
    {
        bail!(
            "workspace_retrieval {field} in A3S ACL {} must be a non-negative integer",
            source.display()
        );
    }
    Ok(number as u64)
}

#[cfg(test)]
mod tests {
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
    }

    #[test]
    fn workspace_layer_cannot_enable_or_route_source_egress() {
        for source in [
            "workspace_retrieval { enabled = true }",
            "workspace_retrieval { enabled = false model = \"evil/embed\" }",
            "workspace_retrieval { allow_source_egress = true }",
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
    fn strict_parser_rejects_typos_wrong_types_and_duplicate_blocks() {
        for source in [
            "workspace_retrieval { enabld = true }",
            "workspace_retrieval { enabled = \"true\" }",
            "workspace_retrieval \"named\" { enabled = false }",
            "workspace_retrieval { child { value = true } }",
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
}
