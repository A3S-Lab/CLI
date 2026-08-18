use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use super::LocalEmbeddingManifest;

pub(super) const MANAGED_MODEL_NAME: &str = "Xenova/all-MiniLM-L6-v2";
pub(super) const MANAGED_MODEL_REVISION: &str = "751bff37182d3f1213fa05d7196b954e230abad9";
pub(super) const MANAGED_MODEL_DIMENSION: usize = 384;
const MANAGED_MANIFEST_NAME: &str = "model.acl";
#[cfg(any(feature = "local-cpu-embedding", test))]
const MANAGED_MANIFEST_SHA256: &str =
    "6c235dd4cad3a36f5b49ead5be05523d6f113e5b2f71ba319220a6706640906a";
#[cfg(any(feature = "local-cpu-embedding", test))]
const MANAGED_BUNDLE_MAX_BYTES: u64 = 32 * 1024 * 1024;
const MANAGED_MANIFEST: &[u8] = include_bytes!("managed_model.acl");

#[cfg(any(feature = "local-cpu-embedding", test))]
struct ManagedRemoteArtifact {
    source_path: &'static str,
    name: &'static str,
    sha256: &'static str,
    bytes: u64,
}

#[cfg(any(feature = "local-cpu-embedding", test))]
const MANAGED_REMOTE_ARTIFACTS: &[ManagedRemoteArtifact] = &[
    ManagedRemoteArtifact {
        source_path: "onnx/model_quantized.onnx",
        name: "model_quantized.onnx",
        sha256: "afdb6f1a0e45b715d0bb9b11772f032c399babd23bfc31fed1c170afc848bdb1",
        bytes: 22_972_370,
    },
    ManagedRemoteArtifact {
        source_path: "tokenizer.json",
        name: "tokenizer.json",
        sha256: "da0e79933b9ed51798a3ae27893d3c5fa4a201126cef75586296df9b4d2c62a0",
        bytes: 711_661,
    },
    ManagedRemoteArtifact {
        source_path: "config.json",
        name: "config.json",
        sha256: "7135149f7cffa1a573466c6e4d8423ed73b62fd2332c575bf738a0d033f70df7",
        bytes: 650,
    },
    ManagedRemoteArtifact {
        source_path: "special_tokens_map.json",
        name: "special_tokens_map.json",
        sha256: "b6d346be366a7d1d48332dbc9fdf3bf8960b5d879522b7799ddba59e76237ee3",
        bytes: 125,
    },
    ManagedRemoteArtifact {
        source_path: "tokenizer_config.json",
        name: "tokenizer_config.json",
        sha256: "9261e7d79b44c8195c1cada2b453e55b00aeb81e907a6664974b4d7776172ab3",
        bytes: 366,
    },
];

pub(super) fn manifest_path(data_root: &Path) -> PathBuf {
    data_root
        .join("power/artifact-bundles/a3s-code/all-minilm-l6-v2")
        .join(MANAGED_MODEL_REVISION)
        .join(MANAGED_MANIFEST_NAME)
}

pub(super) fn load_installed_manifest(data_root: &Path) -> anyhow::Result<LocalEmbeddingManifest> {
    let path = manifest_path(data_root);
    let installed = std::fs::read(&path)
        .context("could not read the A3S Power-managed local embedding manifest")?;
    if installed != MANAGED_MANIFEST {
        bail!("the A3S Power-managed local embedding manifest failed content admission");
    }
    let manifest = LocalEmbeddingManifest::load(&path)?;
    if manifest.model != MANAGED_MODEL_NAME || manifest.revision != MANAGED_MODEL_REVISION {
        bail!("the A3S Power-managed local embedding manifest has an unexpected identity");
    }
    Ok(manifest)
}

pub(super) async fn prepare_manifest(
    data_root: &Path,
    allow_first_use_install: bool,
) -> anyhow::Result<LocalEmbeddingManifest> {
    if !allow_first_use_install {
        let data_root = data_root.to_path_buf();
        return tokio::task::spawn_blocking(move || {
            let manifest = load_installed_manifest(&data_root).with_context(|| {
                "the A3S Power-managed local embedding bundle is not installed and first-use installation is disabled by offline mode or A3S_NO_AUTO_INSTALL"
            })?;
            manifest.admit().with_context(|| {
                "the installed A3S Power-managed local embedding bundle failed offline admission"
            })?;
            Ok(manifest)
        })
        .await
        .context("the A3S Power-managed offline admission task could not complete")?;
    }

    #[cfg(feature = "local-cpu-embedding")]
    {
        let data_root = data_root.to_path_buf();
        provision(&data_root).await?;
        tokio::task::spawn_blocking(move || load_installed_manifest(&data_root))
            .await
            .context("the A3S Power-managed manifest load task could not complete")?
            .context("could not load the provisioned A3S Power local embedding manifest")
    }

    #[cfg(not(feature = "local-cpu-embedding"))]
    {
        let _ = data_root;
        bail!("local CPU embedding requires the local-cpu-embedding binary feature")
    }
}

#[cfg(feature = "local-cpu-embedding")]
async fn provision(data_root: &Path) -> anyhow::Result<()> {
    use a3s_power::artifact_bundle::{
        provision_artifact_bundle, ArtifactBundle, BundleArtifact, BundleProvisionPolicy,
    };

    let source = |path: &str| {
        format!(
            "https://huggingface.co/{MANAGED_MODEL_NAME}/resolve/{MANAGED_MODEL_REVISION}/{path}?download=true"
        )
    };
    let mut artifacts = vec![BundleArtifact::inline(
        MANAGED_MANIFEST_NAME,
        MANAGED_MANIFEST,
        MANAGED_MANIFEST_SHA256,
    )?];
    for artifact in MANAGED_REMOTE_ARTIFACTS {
        artifacts.push(BundleArtifact::remote(
            artifact.name,
            source(artifact.source_path),
            artifact.sha256,
            artifact.bytes,
        )?);
    }
    let bundle = ArtifactBundle::new(MANAGED_MODEL_NAME, MANAGED_MODEL_REVISION, artifacts)?;
    let destination = manifest_path(data_root)
        .parent()
        .context("the managed local embedding manifest has no bundle directory")?;
    let policy = BundleProvisionPolicy::new(destination)
        .with_network(true)
        .with_max_total_bytes(MANAGED_BUNDLE_MAX_BYTES);
    provision_artifact_bundle(&bundle, &policy)
        .await
        .context("A3S Power could not provision the managed local embedding bundle")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn embedded_manifest_has_the_locked_identity_and_digest() {
        let actual = format!("{:x}", Sha256::digest(MANAGED_MANIFEST));
        assert_eq!(actual, MANAGED_MANIFEST_SHA256);

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(MANAGED_MANIFEST_NAME);
        std::fs::write(&path, MANAGED_MANIFEST).unwrap();
        let manifest = LocalEmbeddingManifest::load(&path).unwrap();
        assert_eq!(manifest.model, MANAGED_MODEL_NAME);
        assert_eq!(manifest.revision, MANAGED_MODEL_REVISION);
        assert_eq!(manifest.dimension(), MANAGED_MODEL_DIMENSION);

        let source = std::str::from_utf8(MANAGED_MANIFEST).unwrap();
        let total_remote_bytes = MANAGED_REMOTE_ARTIFACTS
            .iter()
            .map(|artifact| artifact.bytes)
            .sum::<u64>();
        assert!(total_remote_bytes + MANAGED_MANIFEST.len() as u64 <= MANAGED_BUNDLE_MAX_BYTES);
        for artifact in MANAGED_REMOTE_ARTIFACTS {
            assert!(source.contains(&format!("path = \"{}\"", artifact.name)));
            assert!(source.contains(&format!("sha256 = \"{}\"", artifact.sha256)));
        }
    }

    #[tokio::test]
    async fn disabled_first_use_is_a_strict_no_mutation_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let data_root = temp.path().join("missing-data-root");

        let error = prepare_manifest(&data_root, false)
            .await
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("first-use installation is disabled"),
            "{error}"
        );
        assert!(!data_root.exists());
    }

    #[test]
    fn managed_manifest_substitution_fails_before_artifact_admission() {
        let temp = tempfile::tempdir().unwrap();
        let path = manifest_path(temp.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let substituted = std::str::from_utf8(MANAGED_MANIFEST)
            .unwrap()
            .replace("max_length = 512", "max_length = 256");
        std::fs::write(&path, substituted).unwrap();

        let error = load_installed_manifest(temp.path())
            .unwrap_err()
            .to_string();

        assert!(error.contains("content admission"), "{error}");
    }
}
