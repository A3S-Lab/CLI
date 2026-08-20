use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use a3s_acl::Block;
use a3s_code_core::embedding::{EmbeddingNormalization, EmbeddingProviderDescriptor};
use anyhow::{bail, Context};
use sha2::{Digest, Sha256};

const MANIFEST_BLOCK: &str = "local_embedding_model";
const FILE_BLOCK: &str = "file";
const SCHEMA_VERSION: usize = 1;
const RUNTIME: &str = "fastembed-onnx-v1";
const RUNTIME_VERSION: &str = "5.17.3";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_MODEL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TOKENIZER_BYTES: u64 = 64 * 1024 * 1024;
const MAX_METADATA_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_ARTIFACT_BYTES: u64 = 384 * 1024 * 1024;
const MAX_MODEL_TEXT_BYTES: usize = 256;
const MAX_LICENSE_TEXT_BYTES: usize = 512;
const MAX_ARTIFACT_PATH_BYTES: usize = 1_024;
const MAX_EMBEDDING_DIMENSION: usize = 65_536;
const MIN_MODEL_LENGTH: usize = 8;
const MAX_MODEL_LENGTH: usize = 8_192;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum PoolingKind {
    Cls,
    Mean,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum QuantizationKind {
    None,
    Static,
    Dynamic,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ArtifactRole {
    Model,
    Tokenizer,
    Config,
    SpecialTokensMap,
    TokenizerConfig,
}

impl ArtifactRole {
    const ALL: [Self; 5] = [
        Self::Model,
        Self::Tokenizer,
        Self::Config,
        Self::SpecialTokensMap,
        Self::TokenizerConfig,
    ];

    fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "model" => Ok(Self::Model),
            "tokenizer" => Ok(Self::Tokenizer),
            "config" => Ok(Self::Config),
            "special_tokens_map" => Ok(Self::SpecialTokensMap),
            "tokenizer_config" => Ok(Self::TokenizerConfig),
            _ => bail!("unsupported local embedding artifact role `{value}`"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Tokenizer => "tokenizer",
            Self::Config => "config",
            Self::SpecialTokensMap => "special_tokens_map",
            Self::TokenizerConfig => "tokenizer_config",
        }
    }

    fn byte_limit(self) -> u64 {
        match self {
            Self::Model => MAX_MODEL_BYTES,
            Self::Tokenizer => MAX_TOKENIZER_BYTES,
            Self::Config | Self::SpecialTokensMap | Self::TokenizerConfig => {
                MAX_METADATA_FILE_BYTES
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArtifactFile {
    relative_path: PathBuf,
    sha256: String,
}

/// Immutable, content-bound local model manifest.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct LocalEmbeddingManifest {
    root: PathBuf,
    pub(super) model: String,
    pub(super) revision: String,
    pub(super) dimension: usize,
    pub(super) max_length: usize,
    pub(super) pooling: PoolingKind,
    pub(super) quantization: QuantizationKind,
    files: BTreeMap<ArtifactRole, ArtifactFile>,
}

impl fmt::Debug for LocalEmbeddingManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalEmbeddingManifest")
            .field("model", &self.model)
            .field("revision", &self.revision)
            .field("dimension", &self.dimension)
            .field("max_length", &self.max_length)
            .field("pooling", &self.pooling)
            .field("quantization", &self.quantization)
            .field("artifacts", &self.files.len())
            .finish()
    }
}

#[derive(Debug)]
#[cfg_attr(not(feature = "local-cpu-embedding"), allow(dead_code))]
pub(super) struct AdmittedModelArtifacts {
    pub(super) model: Vec<u8>,
    pub(super) tokenizer: Vec<u8>,
    pub(super) config: Vec<u8>,
    pub(super) special_tokens_map: Vec<u8>,
    pub(super) tokenizer_config: Vec<u8>,
}

impl LocalEmbeddingManifest {
    pub(super) fn load(path: &Path) -> anyhow::Result<Self> {
        let metadata = std::fs::symlink_metadata(path)
            .with_context(|| "could not inspect the local embedding artifact manifest")?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_MANIFEST_BYTES
        {
            bail!(
                "local embedding artifact manifest must be a non-symlink regular file at most 65536 bytes"
            );
        }
        let source = std::fs::read_to_string(path)
            .with_context(|| "could not read the local embedding artifact manifest as UTF-8")?;
        let document =
            a3s_acl::parse_acl(&source).context("invalid local embedding artifact manifest ACL")?;
        if document.blocks.len() != 1 || document.blocks[0].name != MANIFEST_BLOCK {
            bail!("local embedding artifact manifest must contain exactly one local_embedding_model block");
        }
        let root = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .context("could not resolve the local embedding artifact directory")?;
        Self::from_block(&document.blocks[0], &root)
    }

    fn from_block(block: &Block, root: &Path) -> anyhow::Result<Self> {
        if !block.labels.is_empty() {
            bail!("local_embedding_model must be unlabeled");
        }
        const KNOWN_FIELDS: &[&str] = &[
            "schema_version",
            "model",
            "revision",
            "runtime",
            "runtime_version",
            "dimension",
            "normalization",
            "pooling",
            "quantization",
            "license",
            "max_length",
        ];
        for field in block.attributes.keys() {
            if !KNOWN_FIELDS.contains(&field.as_str()) {
                bail!("unknown local_embedding_model field `{field}`");
            }
        }
        if block.blocks.iter().any(|child| child.name != FILE_BLOCK) {
            bail!("local_embedding_model supports only labeled file blocks");
        }

        let schema_version = integer_attribute(block, "schema_version")?;
        if schema_version != SCHEMA_VERSION {
            bail!("unsupported local embedding manifest schema_version `{schema_version}`");
        }
        let runtime = string_attribute(block, "runtime", MAX_MODEL_TEXT_BYTES)?;
        let runtime_version = string_attribute(block, "runtime_version", MAX_MODEL_TEXT_BYTES)?;
        if runtime != RUNTIME || runtime_version != RUNTIME_VERSION {
            bail!("local embedding artifact requires runtime {RUNTIME} {RUNTIME_VERSION}");
        }
        let normalization = string_attribute(block, "normalization", MAX_MODEL_TEXT_BYTES)?;
        if normalization != "unit" {
            bail!("fastembed-onnx-v1 requires normalization = \"unit\"");
        }
        let dimension = integer_attribute(block, "dimension")?;
        if !(1..=MAX_EMBEDDING_DIMENSION).contains(&dimension) {
            bail!("local embedding dimension must be between 1 and 65536");
        }
        let max_length = integer_attribute(block, "max_length")?;
        if !(MIN_MODEL_LENGTH..=MAX_MODEL_LENGTH).contains(&max_length) {
            bail!("local embedding max_length must be between 8 and 8192");
        }
        let pooling = match string_attribute(block, "pooling", MAX_MODEL_TEXT_BYTES)? {
            "cls" => PoolingKind::Cls,
            "mean" => PoolingKind::Mean,
            _ => bail!("local embedding pooling must be `cls` or `mean`"),
        };
        let quantization = match string_attribute(block, "quantization", MAX_MODEL_TEXT_BYTES)? {
            "none" => QuantizationKind::None,
            "static" => QuantizationKind::Static,
            "dynamic" => QuantizationKind::Dynamic,
            _ => bail!("local embedding quantization must be `none`, `static`, or `dynamic`"),
        };
        let model = string_attribute(block, "model", MAX_MODEL_TEXT_BYTES)?.to_owned();
        let revision = string_attribute(block, "revision", MAX_MODEL_TEXT_BYTES)?.to_owned();
        let _license = string_attribute(block, "license", MAX_LICENSE_TEXT_BYTES)?;

        let mut files = BTreeMap::new();
        for child in &block.blocks {
            if child.labels.len() != 1 || !child.blocks.is_empty() {
                bail!("local embedding file blocks require exactly one role label and no child blocks");
            }
            for field in child.attributes.keys() {
                if !["path", "sha256"].contains(&field.as_str()) {
                    bail!("unknown local embedding file field `{field}`");
                }
            }
            let role = ArtifactRole::parse(&child.labels[0])?;
            let relative_path = string_attribute(child, "path", MAX_ARTIFACT_PATH_BYTES)?;
            validate_artifact_path(relative_path)?;
            let sha256 = string_attribute(child, "sha256", 64)?;
            validate_sha256(sha256)?;
            if files
                .insert(
                    role,
                    ArtifactFile {
                        relative_path: PathBuf::from(relative_path),
                        sha256: sha256.to_owned(),
                    },
                )
                .is_some()
            {
                bail!("duplicate local embedding artifact role `{}`", role.name());
            }
        }
        for role in ArtifactRole::ALL {
            if !files.contains_key(&role) {
                bail!("local embedding artifact is missing `{}`", role.name());
            }
        }

        Ok(Self {
            root: root.to_path_buf(),
            model,
            revision,
            dimension,
            max_length,
            pooling,
            quantization,
            files,
        })
    }

    pub(super) fn descriptor(&self) -> EmbeddingProviderDescriptor {
        EmbeddingProviderDescriptor::new("local-cpu", self.model.clone(), self.dimension)
            .with_revision(self.revision.clone())
            .with_normalization(EmbeddingNormalization::Unit)
    }

    pub(crate) fn dimension(&self) -> usize {
        self.dimension
    }

    #[cfg(feature = "local-cpu-embedding")]
    pub(super) fn cache_key(&self, intra_threads: usize) -> String {
        let mut digest = Sha256::new();
        for value in [
            self.model.as_bytes(),
            self.revision.as_bytes(),
            RUNTIME.as_bytes(),
            RUNTIME_VERSION.as_bytes(),
        ] {
            digest.update(value);
            digest.update([0]);
        }
        digest.update(self.dimension.to_le_bytes());
        digest.update(self.max_length.to_le_bytes());
        digest.update(intra_threads.to_le_bytes());
        digest.update([self.pooling as u8, self.quantization as u8]);
        for role in ArtifactRole::ALL {
            digest.update([role as u8]);
            digest.update(self.files[&role].sha256.as_bytes());
        }
        hex_digest(digest.finalize())
    }

    pub(super) fn admit(&self) -> anyhow::Result<AdmittedModelArtifacts> {
        let mut admitted = BTreeMap::new();
        let mut total_bytes = 0u64;
        for role in ArtifactRole::ALL {
            let artifact = &self.files[&role];
            let path = self.root.join(&artifact.relative_path);
            let bytes = read_bounded_file(&path, &self.root, role.byte_limit(), role.name())?;
            total_bytes = total_bytes
                .checked_add(bytes.len() as u64)
                .context("local embedding artifact byte accounting overflowed")?;
            if total_bytes > MAX_TOTAL_ARTIFACT_BYTES {
                bail!("local embedding artifacts exceed the 402653184-byte total limit");
            }
            let actual = hex_digest(Sha256::digest(&bytes));
            if actual != artifact.sha256 {
                bail!(
                    "local embedding artifact `{}` failed SHA-256 admission",
                    role.name()
                );
            }
            admitted.insert(role, bytes);
        }
        Ok(AdmittedModelArtifacts {
            model: admitted.remove(&ArtifactRole::Model).unwrap_or_default(),
            tokenizer: admitted
                .remove(&ArtifactRole::Tokenizer)
                .unwrap_or_default(),
            config: admitted.remove(&ArtifactRole::Config).unwrap_or_default(),
            special_tokens_map: admitted
                .remove(&ArtifactRole::SpecialTokensMap)
                .unwrap_or_default(),
            tokenizer_config: admitted
                .remove(&ArtifactRole::TokenizerConfig)
                .unwrap_or_default(),
        })
    }
}

fn read_bounded_file(
    path: &Path,
    manifest_root: &Path,
    limit: u64,
    role: &str,
) -> anyhow::Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("could not inspect local embedding artifact `{role}`"))?;
    if metadata.file_type().is_symlink() {
        bail!("local embedding artifact `{role}` must not be a symbolic link");
    }
    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("could not resolve local embedding artifact `{role}`"))?;
    let canonical_root = manifest_root
        .canonicalize()
        .context("could not resolve the local embedding artifact directory")?;
    if !canonical_path.starts_with(&canonical_root) {
        bail!("local embedding artifact `{role}` must stay below the manifest directory");
    }
    let file = File::open(&canonical_path)
        .with_context(|| format!("could not open local embedding artifact `{role}`"))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("could not inspect local embedding artifact `{role}`"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
        bail!("local embedding artifact `{role}` is empty, non-regular, or exceeds its byte limit");
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("could not read local embedding artifact `{role}`"))?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        bail!("local embedding artifact `{role}` is empty or exceeds its byte limit");
    }
    Ok(bytes)
}

fn validate_artifact_path(value: &str) -> anyhow::Result<()> {
    if value.contains('\\') || value.contains(':') {
        bail!("local embedding artifact paths must use portable relative forward-slash syntax");
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("local embedding artifact paths must stay below the manifest directory");
    }
    Ok(())
}

fn validate_sha256(value: &str) -> anyhow::Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("local embedding artifact sha256 must be 64 lowercase hexadecimal characters");
    }
    Ok(())
}

fn string_attribute<'a>(
    block: &'a Block,
    field: &str,
    max_bytes: usize,
) -> anyhow::Result<&'a str> {
    let value = block
        .attributes
        .get(field)
        .with_context(|| format!("local embedding manifest requires `{field}`"))?
        .as_str()
        .with_context(|| format!("local embedding manifest `{field}` must be a string"))?;
    if value.trim().is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        bail!("local embedding manifest `{field}` is empty, oversized, or contains control characters");
    }
    Ok(value)
}

fn integer_attribute(block: &Block, field: &str) -> anyhow::Result<usize> {
    let value = block
        .attributes
        .get(field)
        .with_context(|| format!("local embedding manifest requires `{field}`"))?
        .as_number()
        .with_context(|| format!("local embedding manifest `{field}` must be an integer"))?;
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > usize::MAX as f64 {
        bail!("local embedding manifest `{field}` must be a non-negative integer");
    }
    Ok(value as usize)
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    let mut encoded = String::with_capacity(bytes.as_ref().len() * 2);
    for byte in bytes.as_ref() {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_fixture(root: &Path) -> PathBuf {
        let files = [
            ("model", "model.onnx", b"model".as_slice()),
            ("tokenizer", "tokenizer.json", b"tokenizer".as_slice()),
            ("config", "config.json", b"config".as_slice()),
            (
                "special_tokens_map",
                "special_tokens_map.json",
                b"special".as_slice(),
            ),
            (
                "tokenizer_config",
                "tokenizer_config.json",
                b"tokenizer-config".as_slice(),
            ),
        ];
        let mut file_blocks = String::new();
        for (role, path, bytes) in files {
            std::fs::write(root.join(path), bytes).unwrap();
            file_blocks.push_str(&format!(
                "file \"{role}\" {{ path = \"{path}\" sha256 = \"{}\" }}\n",
                hex_digest(Sha256::digest(bytes))
            ));
        }
        let manifest = root.join("model.acl");
        std::fs::write(
            &manifest,
            format!(
                r#"local_embedding_model {{
  schema_version = 1
  model = "sentence-transformers/test"
  revision = "0123456789abcdef"
  runtime = "fastembed-onnx-v1"
  runtime_version = "5.17.3"
  dimension = 384
  normalization = "unit"
  pooling = "mean"
  quantization = "static"
  license = "Apache-2.0"
  max_length = 512
  {file_blocks}
}}
"#
            ),
        )
        .unwrap();
        manifest
    }

    #[test]
    fn admits_exact_content_bound_artifacts() {
        let fixture = tempfile::tempdir().unwrap();
        let manifest = LocalEmbeddingManifest::load(&write_fixture(fixture.path())).unwrap();

        let admitted = manifest.admit().unwrap();

        assert_eq!(manifest.dimension, 384);
        #[cfg(feature = "local-cpu-embedding")]
        assert_eq!(
            manifest.descriptor().normalization,
            EmbeddingNormalization::Unit
        );
        assert_eq!(admitted.model, b"model");
        assert_eq!(admitted.tokenizer, b"tokenizer");
        #[cfg(feature = "local-cpu-embedding")]
        assert!(!manifest.cache_key(2).is_empty());
        let debug = format!("{manifest:?}");
        assert!(!debug.contains(&fixture.path().display().to_string()));
        assert!(!debug.contains("model.onnx"));
    }

    #[test]
    fn rejects_substitution_and_path_escape() {
        let fixture = tempfile::tempdir().unwrap();
        let path = write_fixture(fixture.path());
        let manifest = LocalEmbeddingManifest::load(&path).unwrap();
        std::fs::write(fixture.path().join("model.onnx"), b"substituted").unwrap();
        let error = manifest.admit().unwrap_err().to_string();
        assert!(error.contains("SHA-256"), "{error}");

        let source = std::fs::read_to_string(&path)
            .unwrap()
            .replace("path = \"model.onnx\"", "path = \"../model.onnx\"");
        std::fs::write(&path, source).unwrap();
        let error = LocalEmbeddingManifest::load(&path).unwrap_err().to_string();
        assert!(error.contains("manifest directory"), "{error}");
    }

    #[test]
    fn rejects_runtime_drift_and_unknown_fields() {
        let fixture = tempfile::tempdir().unwrap();
        let path = write_fixture(fixture.path());
        let original = std::fs::read_to_string(&path).unwrap();
        for source in [
            original.replace(
                "runtime_version = \"5.17.3\"",
                "runtime_version = \"6.0.0\"",
            ),
            original.replace(
                "max_length = 512",
                "max_length = 512\n  endpoint = \"https://example.test\"",
            ),
        ] {
            std::fs::write(&path, source).unwrap();
            assert!(LocalEmbeddingManifest::load(&path).is_err());
        }
    }

    #[test]
    fn rejects_missing_empty_unknown_and_duplicate_artifact_fixtures() {
        let missing_role = tempfile::tempdir().unwrap();
        let missing_role_manifest = write_fixture(missing_role.path());
        let source = std::fs::read_to_string(&missing_role_manifest).unwrap();
        let without_config = source
            .lines()
            .filter(|line| !line.contains("file \"config\""))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&missing_role_manifest, without_config).unwrap();
        let error = LocalEmbeddingManifest::load(&missing_role_manifest)
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing `config`"), "{error}");

        let unknown_role = tempfile::tempdir().unwrap();
        let unknown_role_manifest = write_fixture(unknown_role.path());
        let source = std::fs::read_to_string(&unknown_role_manifest)
            .unwrap()
            .replace("file \"config\"", "file \"weights\"");
        std::fs::write(&unknown_role_manifest, source).unwrap();
        let error = LocalEmbeddingManifest::load(&unknown_role_manifest)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unsupported"), "{error}");

        let duplicate_role = tempfile::tempdir().unwrap();
        let duplicate_role_manifest = write_fixture(duplicate_role.path());
        let source = std::fs::read_to_string(&duplicate_role_manifest).unwrap();
        let config_line = source
            .lines()
            .find(|line| line.contains("file \"config\""))
            .unwrap();
        let source = source.replace(config_line, &format!("{config_line}\n{config_line}"));
        std::fs::write(&duplicate_role_manifest, source).unwrap();
        let error = LocalEmbeddingManifest::load(&duplicate_role_manifest)
            .unwrap_err()
            .to_string();
        assert!(error.contains("duplicate"), "{error}");

        let missing_file = tempfile::tempdir().unwrap();
        let missing_file_manifest = write_fixture(missing_file.path());
        let manifest = LocalEmbeddingManifest::load(&missing_file_manifest).unwrap();
        std::fs::remove_file(missing_file.path().join("model.onnx")).unwrap();
        let error = manifest.admit().unwrap_err().to_string();
        assert!(error.contains("inspect"), "{error}");

        let empty_file = tempfile::tempdir().unwrap();
        let empty_file_manifest = write_fixture(empty_file.path());
        let manifest = LocalEmbeddingManifest::load(&empty_file_manifest).unwrap();
        std::fs::write(empty_file.path().join("tokenizer.json"), []).unwrap();
        let error = manifest.admit().unwrap_err().to_string();
        assert!(error.contains("empty"), "{error}");
    }

    #[test]
    fn rejects_oversized_model_before_reading_or_hashing_it() {
        let fixture = tempfile::tempdir().unwrap();
        let path = write_fixture(fixture.path());
        let model = std::fs::OpenOptions::new()
            .write(true)
            .open(fixture.path().join("model.onnx"))
            .unwrap();
        model.set_len(MAX_MODEL_BYTES + 1).unwrap();

        let manifest = LocalEmbeddingManifest::load(&path).unwrap();
        let error = manifest.admit().unwrap_err().to_string();
        assert!(error.contains("byte limit"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_manifest_and_artifact_escape() {
        use std::os::unix::fs::symlink;

        let fixture = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let manifest_path = write_fixture(fixture.path());
        let manifest_link = fixture.path().join("linked-model.acl");
        symlink(&manifest_path, &manifest_link).unwrap();
        let error = LocalEmbeddingManifest::load(&manifest_link)
            .unwrap_err()
            .to_string();
        assert!(error.contains("non-symlink"), "{error}");

        std::fs::write(outside.path().join("model.onnx"), b"model").unwrap();
        std::fs::remove_file(fixture.path().join("model.onnx")).unwrap();
        symlink(
            outside.path().join("model.onnx"),
            fixture.path().join("model.onnx"),
        )
        .unwrap();
        let manifest = LocalEmbeddingManifest::load(&manifest_path).unwrap();
        let error = manifest.admit().unwrap_err().to_string();
        assert!(
            error.contains("symbolic link") || error.contains("manifest directory"),
            "{error}"
        );
    }
}
