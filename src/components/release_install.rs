use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use a3s_updater::{
    download_asset, extract_release_archive, fetch_release, parse_version, uninstall_owned_files,
    verify_sha256, ComponentReceipt, DirectoryActivation, InstallProvenance,
    RECEIPT_SCHEMA_VERSION,
};
use anyhow::{bail, Context};

use super::catalog::ReleaseSpec;
use super::id::ComponentId;
use super::lifecycle::{InstallRequest, OperationRecord};
use super::paths::ComponentPaths;
use super::probe::probe_version;

pub async fn install_release(
    id: &ComponentId,
    release_spec: ReleaseSpec,
    request: &InstallRequest,
    paths: &ComponentPaths,
) -> anyhow::Result<OperationRecord> {
    let target = release_spec.asset_family.target().with_context(|| {
        format!(
            "component '{}' has no release for {}-{}",
            id,
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    super::progress(
        request.progress,
        format!("a3s: resolving release for '{}'...", id),
    );
    let release = fetch_release(
        release_spec.github_owner,
        release_spec.github_repo,
        request.version.as_deref(),
    )
    .await?;
    let version = parse_version(&release.tag_name)?.to_string();
    let archive_name =
        release_spec
            .asset_family
            .archive_name(release_spec.binary, &version, target);
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == archive_name)
        .cloned()
        .with_context(|| {
            format!(
                "release v{} has no asset '{}' for component '{}'",
                version, archive_name, id
            )
        })?;
    let checksum = release_checksum(&release, &asset).await?;
    super::progress(
        request.progress,
        format!("a3s: downloading '{}' {} for {}...", id, version, target),
    );
    let archive = download_asset(&asset.browser_download_url).await?;
    verify_sha256(&archive, &checksum)?;

    let component_root = paths.component_root(id);
    std::fs::create_dir_all(&component_root)?;
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&component_root)?;
    let unpacked = staging.path().join("unpacked");
    extract_release_archive(&archive, &unpacked, &archive_name)?;
    let executable_name = release_spec
        .asset_family
        .executable_name(release_spec.binary, target);
    let staged_executable = find_unique_file(&unpacked, &executable_name)?;
    let actual_version = probe_version(&staged_executable)?;
    if parse_version(&actual_version)? != parse_version(&version)? {
        bail!(
            "downloaded '{}' reported version {}, expected {}",
            id,
            actual_version,
            version
        );
    }
    let relative_executable = staged_executable.strip_prefix(&unpacked)?.to_path_buf();
    let active = paths.version_root(id, &version);
    let old_receipt = paths.receipt_store().read(id.as_str())?;
    let activation = DirectoryActivation::activate(&unpacked, &active)?;
    let executable = active.join(relative_executable);
    let receipt = ComponentReceipt {
        schema_version: RECEIPT_SCHEMA_VERSION,
        component_id: id.to_string(),
        version: version.clone(),
        provenance: InstallProvenance::GithubRelease,
        install_root: active.clone(),
        executable_path: Some(executable.clone()),
        owned_paths: vec![active.clone()],
        source: Some(format!(
            "https://github.com/{}/{}/releases/tag/{}",
            release_spec.github_owner, release_spec.github_repo, release.tag_name
        )),
        artifact_checksums: BTreeMap::from([(archive_name, checksum)]),
        installed_at: chrono::Utc::now().to_rfc3339(),
    };
    paths.receipt_store().write(&receipt)?;
    activation.commit()?;

    if let Some(old) = old_receipt.filter(|old| old.install_root != active) {
        if old.provenance.owns_files() {
            let _ = uninstall_owned_files(&old, &paths.data_root);
        }
    }
    Ok(OperationRecord {
        component: id.clone(),
        action: "install",
        changed: true,
        version: Some(version),
        provenance: Some(InstallProvenance::GithubRelease),
        path: Some(executable),
        message: format!("Installed component '{}' from its verified release.", id),
    })
}

async fn release_checksum(
    release: &a3s_updater::Release,
    asset: &a3s_updater::Asset,
) -> anyhow::Result<String> {
    if let Some(digest) = asset.digest.as_deref() {
        let digest = digest.strip_prefix("sha256:").unwrap_or(digest);
        if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(digest.to_ascii_lowercase());
        }
    }
    for name in [
        format!("{}.sha256", asset.name),
        "checksums.txt".to_string(),
    ] {
        let Some(checksum_asset) = release
            .assets
            .iter()
            .find(|candidate| candidate.name == name)
        else {
            continue;
        };
        let bytes = download_asset(&checksum_asset.browser_download_url).await?;
        if let Some(checksum) = parse_checksum_file(&bytes, &asset.name) {
            return Ok(checksum);
        }
    }
    bail!(
        "release asset '{}' has no trusted SHA-256 digest or checksum file",
        asset.name
    )
}

fn parse_checksum_file(bytes: &[u8], asset_name: &str) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let digest = fields.next()?.trim_start_matches("sha256:");
        let file = fields.next().map(|value| value.trim_start_matches('*'));
        if digest.len() == 64
            && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            && (file.is_none() || file == Some(asset_name))
        {
            return Some(digest.to_ascii_lowercase());
        }
    }
    None
}

fn find_unique_file(root: &Path, file_name: &str) -> anyhow::Result<PathBuf> {
    let mut matches = Vec::new();
    collect_named_files(root, file_name, &mut matches)?;
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => bail!("release archive does not contain '{}'", file_name),
        _ => bail!("release archive contains multiple '{}' files", file_name),
    }
}

fn collect_named_files(
    root: &Path,
    file_name: &str,
    matches: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_named_files(&path, file_name, matches)?;
        } else if path.file_name().and_then(|value| value.to_str()) == Some(file_name) {
            matches.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_checksum_formats() {
        let digest = "a".repeat(64);
        assert_eq!(
            parse_checksum_file(
                format!("{digest}  a3s-use-1.0.0-linux-x86_64.tar.gz\n").as_bytes(),
                "a3s-use-1.0.0-linux-x86_64.tar.gz"
            ),
            Some(digest.clone())
        );
        assert_eq!(
            parse_checksum_file(format!("{digest}\n").as_bytes(), "asset.tar.gz"),
            Some(digest)
        );
        assert_eq!(
            parse_checksum_file(
                format!("{}  other.tar.gz\n", "b".repeat(64)).as_bytes(),
                "asset.tar.gz"
            ),
            None
        );
    }
}
