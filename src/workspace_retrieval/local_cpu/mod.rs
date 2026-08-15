mod manifest;
mod provider;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_acl::{Block, Value};
use a3s_code_core::embedding::EmbeddingProvider;
use anyhow::{bail, Context};

pub(super) use manifest::LocalEmbeddingManifest;

pub(super) const LOCAL_CPU_BLOCK: &str = "local_cpu";
const DEFAULT_INTRA_THREADS: usize = 2;
const MAX_INTRA_THREADS: usize = 64;
const MAX_MANIFEST_PATH_BYTES: usize = 4_096;

/// Trusted host selection for an offline, in-process CPU embedding model.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct LocalCpuEmbeddingConfig {
    artifact_manifest: PathBuf,
    pub(super) intra_threads: usize,
}

impl fmt::Debug for LocalCpuEmbeddingConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalCpuEmbeddingConfig")
            .field("artifact_manifest", &"<configured>")
            .field("intra_threads", &self.intra_threads)
            .finish()
    }
}

impl LocalCpuEmbeddingConfig {
    pub(super) fn from_block(block: &Block, source: &Path) -> anyhow::Result<Self> {
        if !block.labels.is_empty() || !block.blocks.is_empty() {
            bail!(
                "workspace_retrieval {LOCAL_CPU_BLOCK} in A3S ACL {} must be an unlabeled flat block",
                source.display()
            );
        }
        for field in block.attributes.keys() {
            if !["artifact_manifest", "intra_threads"].contains(&field.as_str()) {
                bail!(
                    "unknown workspace_retrieval {LOCAL_CPU_BLOCK} field `{field}` in A3S ACL {}",
                    source.display()
                );
            }
        }
        let manifest = block
            .attributes
            .get("artifact_manifest")
            .context("workspace_retrieval local_cpu requires artifact_manifest")?;
        let manifest = string_value(manifest, "artifact_manifest", source)?;
        if manifest.len() > MAX_MANIFEST_PATH_BYTES {
            bail!("workspace_retrieval local_cpu artifact_manifest is too long");
        }
        let manifest = PathBuf::from(manifest);
        let artifact_manifest = if manifest.is_absolute() {
            manifest
        } else {
            source
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(manifest)
        };
        let intra_threads = block
            .attributes
            .get("intra_threads")
            .map(|value| integer_value(value, "intra_threads", source))
            .transpose()?
            .unwrap_or(DEFAULT_INTRA_THREADS);
        let config = Self {
            artifact_manifest,
            intra_threads,
        };
        config.validate()?;
        Ok(config)
    }

    pub(super) fn validate(&self) -> anyhow::Result<()> {
        if !(1..=MAX_INTRA_THREADS).contains(&self.intra_threads) {
            bail!("workspace_retrieval local_cpu intra_threads must be between 1 and 64");
        }
        Ok(())
    }

    pub(super) fn load_manifest(&self) -> anyhow::Result<LocalEmbeddingManifest> {
        LocalEmbeddingManifest::load(&self.artifact_manifest)
    }

    pub(super) fn validate_artifacts(&self) -> anyhow::Result<()> {
        self.load_manifest()?.admit().map(|_| ())
    }

    pub(super) fn build_provider(
        &self,
        manifest: LocalEmbeddingManifest,
    ) -> anyhow::Result<Arc<dyn EmbeddingProvider>> {
        provider::build_provider(manifest, self.intra_threads)
    }
}

fn string_value<'a>(value: &'a Value, field: &str, source: &Path) -> anyhow::Result<&'a str> {
    let value = value.as_str().with_context(|| {
        format!(
            "workspace_retrieval local_cpu {field} in A3S ACL {} must be a string",
            source.display()
        )
    })?;
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        bail!(
            "workspace_retrieval local_cpu {field} in A3S ACL {} must be a non-empty printable string",
            source.display()
        );
    }
    Ok(value)
}

fn integer_value(value: &Value, field: &str, source: &Path) -> anyhow::Result<usize> {
    let value = value.as_number().with_context(|| {
        format!(
            "workspace_retrieval local_cpu {field} in A3S ACL {} must be a positive integer",
            source.display()
        )
    })?;
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > usize::MAX as f64 {
        bail!(
            "workspace_retrieval local_cpu {field} in A3S ACL {} must be a positive integer",
            source.display()
        );
    }
    Ok(value as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str, config_path: &Path) -> anyhow::Result<LocalCpuEmbeddingConfig> {
        let document = a3s_acl::parse_acl(source)?;
        LocalCpuEmbeddingConfig::from_block(&document.blocks[0], config_path)
    }

    #[test]
    fn resolves_relative_manifest_against_trusted_config() {
        let config = parse(
            r#"local_cpu { artifact_manifest = "models/embed/model.acl" intra_threads = 3 }"#,
            Path::new("C:/trusted/.a3s/config.acl"),
        )
        .unwrap();

        assert_eq!(config.intra_threads, 3);
        assert_eq!(
            config.artifact_manifest,
            PathBuf::from("C:/trusted/.a3s/models/embed/model.acl")
        );
        assert!(!format!("{config:?}").contains("models/embed"));
    }

    #[test]
    fn rejects_ambiguous_or_unbounded_local_config() {
        for source in [
            "local_cpu {}",
            r#"local_cpu "named" { artifact_manifest = "model.acl" }"#,
            r#"local_cpu { artifact_manifest = "model.acl" threads = 2 }"#,
            r#"local_cpu { artifact_manifest = "model.acl" intra_threads = 0 }"#,
            r#"local_cpu { artifact_manifest = "model.acl" intra_threads = 65 }"#,
            r#"local_cpu { artifact_manifest = "model.acl" nested {} }"#,
        ] {
            assert!(parse(source, Path::new("config.acl")).is_err(), "{source}");
        }
    }
}
