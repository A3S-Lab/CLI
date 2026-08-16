mod manifest;
mod provider;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_acl::{Block, Value};
use a3s_code_core::embedding::{EmbeddingExecutorConfig, EmbeddingProvider};
use anyhow::{bail, Context};

pub(super) use manifest::LocalEmbeddingManifest;

pub(super) const LOCAL_CPU_BLOCK: &str = "local_cpu";
const DEFAULT_INTRA_THREADS: usize = 2;
const MAX_INTRA_THREADS: usize = 64;
const MAX_MANIFEST_PATH_BYTES: usize = 4_096;
const MAX_LOCAL_CPU_BATCH_INPUTS: usize = 2;

pub(super) const fn embedding_batch_input_limit() -> usize {
    MAX_LOCAL_CPU_BATCH_INPUTS
}

pub(super) fn embedding_executor_config(
    request_timeout: std::time::Duration,
) -> EmbeddingExecutorConfig {
    EmbeddingExecutorConfig {
        max_batch_inputs: MAX_LOCAL_CPU_BATCH_INPUTS,
        request_timeout,
        ..EmbeddingExecutorConfig::default()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalCpuRuntimeSupport {
    #[cfg(feature = "local-cpu-embedding")]
    Available,
    #[cfg(not(feature = "local-cpu-embedding"))]
    FeatureDisabled,
    #[cfg(all(
        feature = "local-cpu-embedding",
        not(any(target_arch = "x86_64", target_arch = "aarch64"))
    ))]
    UnsupportedArchitecture,
    #[cfg(all(feature = "local-cpu-embedding", target_arch = "x86_64"))]
    MissingX86_64V3(&'static str),
}

impl LocalCpuRuntimeSupport {
    pub(crate) fn is_available(self) -> bool {
        #[cfg(feature = "local-cpu-embedding")]
        {
            matches!(self, Self::Available)
        }

        #[cfg(not(feature = "local-cpu-embedding"))]
        {
            false
        }
    }

    pub(crate) fn unavailable_reason(self) -> Option<&'static str> {
        match self {
            #[cfg(feature = "local-cpu-embedding")]
            Self::Available => None,
            #[cfg(not(feature = "local-cpu-embedding"))]
            Self::FeatureDisabled => Some("feature_disabled"),
            #[cfg(all(
                feature = "local-cpu-embedding",
                not(any(target_arch = "x86_64", target_arch = "aarch64"))
            ))]
            Self::UnsupportedArchitecture => Some("unsupported_architecture"),
            #[cfg(all(feature = "local-cpu-embedding", target_arch = "x86_64"))]
            Self::MissingX86_64V3(_) => Some("missing_x86_64_v3"),
        }
    }
}

pub(crate) fn local_cpu_runtime_support() -> LocalCpuRuntimeSupport {
    #[cfg(not(feature = "local-cpu-embedding"))]
    {
        LocalCpuRuntimeSupport::FeatureDisabled
    }

    #[cfg(all(feature = "local-cpu-embedding", target_arch = "x86_64"))]
    {
        classify_x86_64_v3(x86_64_v3_features())
    }

    #[cfg(all(feature = "local-cpu-embedding", target_arch = "aarch64"))]
    {
        LocalCpuRuntimeSupport::Available
    }

    #[cfg(all(
        feature = "local-cpu-embedding",
        not(any(target_arch = "x86_64", target_arch = "aarch64"))
    ))]
    {
        LocalCpuRuntimeSupport::UnsupportedArchitecture
    }
}

#[cfg(all(feature = "local-cpu-embedding", target_arch = "x86_64"))]
fn x86_64_v3_features() -> [(&'static str, bool); 13] {
    [
        ("sse3", std::arch::is_x86_feature_detected!("sse3")),
        ("ssse3", std::arch::is_x86_feature_detected!("ssse3")),
        ("sse4.1", std::arch::is_x86_feature_detected!("sse4.1")),
        ("sse4.2", std::arch::is_x86_feature_detected!("sse4.2")),
        ("popcnt", std::arch::is_x86_feature_detected!("popcnt")),
        ("avx", std::arch::is_x86_feature_detected!("avx")),
        ("avx2", std::arch::is_x86_feature_detected!("avx2")),
        ("bmi1", std::arch::is_x86_feature_detected!("bmi1")),
        ("bmi2", std::arch::is_x86_feature_detected!("bmi2")),
        ("f16c", std::arch::is_x86_feature_detected!("f16c")),
        ("fma", std::arch::is_x86_feature_detected!("fma")),
        ("lzcnt", std::arch::is_x86_feature_detected!("lzcnt")),
        ("movbe", std::arch::is_x86_feature_detected!("movbe")),
    ]
}

#[cfg(all(feature = "local-cpu-embedding", target_arch = "x86_64"))]
fn classify_x86_64_v3(features: [(&'static str, bool); 13]) -> LocalCpuRuntimeSupport {
    features
        .into_iter()
        .find_map(|(name, available)| (!available).then_some(name))
        .map_or(
            LocalCpuRuntimeSupport::Available,
            LocalCpuRuntimeSupport::MissingX86_64V3,
        )
}

fn ensure_runtime_supported() -> anyhow::Result<()> {
    ensure_runtime_support(local_cpu_runtime_support())
}

fn ensure_runtime_support(support: LocalCpuRuntimeSupport) -> anyhow::Result<()> {
    match support {
        #[cfg(feature = "local-cpu-embedding")]
        LocalCpuRuntimeSupport::Available => Ok(()),
        #[cfg(not(feature = "local-cpu-embedding"))]
        LocalCpuRuntimeSupport::FeatureDisabled => {
            bail!("local CPU embedding requires the local-cpu-embedding binary feature")
        }
        #[cfg(all(
            feature = "local-cpu-embedding",
            not(any(target_arch = "x86_64", target_arch = "aarch64"))
        ))]
        LocalCpuRuntimeSupport::UnsupportedArchitecture => {
            bail!("local CPU embedding supports only x86_64 and aarch64 release targets")
        }
        #[cfg(all(feature = "local-cpu-embedding", target_arch = "x86_64"))]
        LocalCpuRuntimeSupport::MissingX86_64V3(feature) => bail!(
            "local CPU embedding requires the x86-64-v3 baseline; the current CPU is missing `{feature}`"
        ),
    }
}

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

    pub(super) fn validate_runtime_support(&self) -> anyhow::Result<()> {
        ensure_runtime_supported()
    }

    pub(super) fn validate_artifacts(&self) -> anyhow::Result<()> {
        self.validate_runtime_support()?;
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

    #[test]
    fn runtime_support_reports_a_stable_non_sensitive_reason() {
        let support = local_cpu_runtime_support();
        assert_eq!(
            support.is_available(),
            support.unavailable_reason().is_none()
        );
        if let Some(reason) = support.unavailable_reason() {
            assert!([
                "feature_disabled",
                "unsupported_architecture",
                "missing_x86_64_v3"
            ]
            .contains(&reason));
        }
        assert_eq!(ensure_runtime_supported().is_ok(), support.is_available());
    }

    #[test]
    fn local_cpu_executor_uses_a_memory_bounded_microbatch() {
        let config = embedding_executor_config(std::time::Duration::from_secs(9));
        assert_eq!(config.max_batch_inputs, MAX_LOCAL_CPU_BATCH_INPUTS);
        assert_eq!(config.request_timeout, std::time::Duration::from_secs(9));
        assert_eq!(
            config.max_request_inputs,
            EmbeddingExecutorConfig::default().max_request_inputs
        );
    }

    #[cfg(all(feature = "local-cpu-embedding", target_arch = "x86_64"))]
    #[test]
    fn missing_x86_64_v3_feature_fails_before_model_loading() {
        let mut features = x86_64_v3_features();
        for (_, available) in &mut features {
            *available = true;
        }
        features[7].1 = false;
        let support = classify_x86_64_v3(features);
        assert_eq!(support, LocalCpuRuntimeSupport::MissingX86_64V3("bmi1"));
        assert_eq!(support.unavailable_reason(), Some("missing_x86_64_v3"));
        let error = ensure_runtime_support(support).unwrap_err().to_string();
        assert!(error.contains("x86-64-v3"), "{error}");
        assert!(error.contains("bmi1"), "{error}");
    }
}
