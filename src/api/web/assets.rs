use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use a3s_updater::{
    download_asset, extract_release_archive, fetch_release, parse_version, verify_sha256,
    DirectoryActivation,
};
use anyhow::{bail, Context};
use fs2::FileExt;
use tokio::time::{sleep, Instant};

const WEB_RELEASE_OWNER: &str = "A3S-Lab";
const WEB_RELEASE_REPOSITORY: &str = "CLI";
const WEB_ASSET_LOCK_TIMEOUT: Duration = Duration::from_secs(300);
const WEB_ASSET_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(in crate::api) async fn prepare_default_web_dir(
    workspace: &Path,
    offline: bool,
    allow_download: bool,
) -> anyhow::Result<PathBuf> {
    let paths = a3s::components::ComponentPaths::from_env_at(workspace);
    if let Some(directory) =
        find_default_web_dir(paths.as_ref().ok().map(|paths| paths.data_root.as_path()))
    {
        return Ok(directory);
    }

    if offline {
        bail!(
            "web assets were not found and offline mode forbids the first-use download; pass \
             --web-dir, use --api-only, or rerun without --offline"
        );
    }
    if !allow_download {
        bail!(
            "web assets were not found and automatic downloads are disabled by \
             A3S_NO_AUTO_INSTALL=1; pass --web-dir, use --api-only, or allow automatic setup"
        );
    }

    let paths = paths.context(
        "web assets were not found and the A3S data directories could not be resolved; pass \
         --web-dir or use --api-only",
    )?;
    install_matching_web_release(&paths).await.map_err(|error| {
        anyhow::anyhow!(
            "failed to prepare A3S Web assets for a3s {}: {error:#}; pass --web-dir, use \
             --api-only, or retry when the matching GitHub release is available",
            env!("CARGO_PKG_VERSION")
        )
    })
}

fn find_default_web_dir(data_root: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        candidates.extend(packaged_candidates(&executable));
    }
    if let Some(data_root) = data_root {
        candidates.push(cached_web_dir(data_root));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.extend(upward_candidates(&cwd));
        candidates.push(cwd.join("dist/workspace"));
        candidates.push(cwd.join("dist"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist/workspace"));
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/dist"));

    find_existing_web_dir(candidates)
}

fn cached_web_dir(data_root: &Path) -> PathBuf {
    data_root.join("web").join(env!("CARGO_PKG_VERSION"))
}

async fn install_matching_web_release(
    paths: &a3s::components::ComponentPaths,
) -> anyhow::Result<PathBuf> {
    let active = cached_web_dir(&paths.data_root);
    let _lock = acquire_asset_lock(&paths.runtime_root).await?;
    if active.join("index.html").is_file() {
        return Ok(clean_path(active));
    }

    let version = env!("CARGO_PKG_VERSION");
    let archive_name = format!("a3s-web-v{version}.tar.gz");
    let release = fetch_release(WEB_RELEASE_OWNER, WEB_RELEASE_REPOSITORY, Some(version))
        .await
        .with_context(|| format!("failed to resolve release v{version}"))?;
    let expected_version = parse_version(version)?;
    let release_version = parse_version(&release.tag_name)?;
    if release_version != expected_version {
        bail!(
            "release API returned tag '{}' while v{} was requested",
            release.tag_name,
            version
        );
    }
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == archive_name)
        .with_context(|| {
            format!("release v{version} does not contain the required Web asset '{archive_name}'")
        })?;
    if asset.browser_download_url.trim().is_empty() {
        bail!("release Web asset '{archive_name}' has an empty download URL");
    }
    let checksum = web_asset_checksum(&release, asset).await?;
    let archive = download_asset(&asset.browser_download_url).await?;
    verify_sha256(&archive, &checksum)?;

    let web_root = paths.data_root.join("web");
    std::fs::create_dir_all(&web_root)
        .with_context(|| format!("failed to create Web data root {}", web_root.display()))?;
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&web_root)
        .with_context(|| format!("failed to stage Web assets in {}", web_root.display()))?;
    let unpacked = staging.path().join("unpacked");
    let extracted = extract_release_archive(&archive, &unpacked, &archive_name)?;
    let staged_web = unpacked.join("web");
    validate_release_layout(&unpacked, &staged_web, &extracted)?;

    let activation = DirectoryActivation::activate(&staged_web, &active)?;
    activation.commit()?;
    Ok(clean_path(active))
}

async fn web_asset_checksum(
    release: &a3s_updater::Release,
    asset: &a3s_updater::Asset,
) -> anyhow::Result<String> {
    if let Some(digest) = asset.digest.as_deref() {
        let digest = digest.strip_prefix("sha256:").unwrap_or(digest);
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!(
                "release Web asset '{}' has an invalid SHA-256 digest",
                asset.name
            );
        }
        return Ok(digest.to_ascii_lowercase());
    }

    let checksum_name = format!("{}.sha256", asset.name);
    let checksum_asset = release
        .assets
        .iter()
        .find(|candidate| candidate.name == checksum_name)
        .with_context(|| {
            format!(
                "release Web asset '{}' has neither a GitHub SHA-256 digest nor companion '{}'",
                asset.name, checksum_name
            )
        })?;
    let bytes = download_asset(&checksum_asset.browser_download_url).await?;
    parse_checksum_file(&bytes, &asset.name)
}

fn parse_checksum_file(bytes: &[u8], asset_name: &str) -> anyhow::Result<String> {
    let text = std::str::from_utf8(bytes).context("Web asset checksum file is not UTF-8")?;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let mut fields = line.split_whitespace();
        let Some(digest) = fields.next() else {
            continue;
        };
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        if let Some(name) = fields.next() {
            if name.trim_start_matches('*') != asset_name {
                continue;
            }
        }
        return Ok(digest.to_ascii_lowercase());
    }
    bail!(
        "companion checksum does not contain a valid SHA-256 entry for '{}'",
        asset_name
    )
}

fn validate_release_layout(
    unpacked: &Path,
    staged_web: &Path,
    extracted: &[PathBuf],
) -> anyhow::Result<()> {
    if !staged_web.join("index.html").is_file() {
        bail!("release Web archive does not contain web/index.html");
    }
    for file in extracted {
        let relative = file
            .strip_prefix(unpacked)
            .with_context(|| format!("extracted path {} escaped staging", file.display()))?;
        if !relative.starts_with("web") {
            bail!(
                "release Web archive contains a file outside the web directory: {}",
                relative.display()
            );
        }
    }
    Ok(())
}

async fn acquire_asset_lock(runtime_root: &Path) -> anyhow::Result<WebAssetLock> {
    let path = runtime_root.join("web/assets.lock");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create Web asset lock directory {}",
                parent.display()
            )
        })?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open Web asset lock {}", path.display()))?;
    let deadline = Instant::now() + WEB_ASSET_LOCK_TIMEOUT;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(WebAssetLock { file }),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    bail!(
                        "timed out waiting for another process to prepare A3S Web assets at {}",
                        path.display()
                    );
                }
                sleep(WEB_ASSET_LOCK_POLL_INTERVAL).await;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to lock Web assets at {}", path.display()))
            }
        }
    }
}

struct WebAssetLock {
    file: File,
}

impl Drop for WebAssetLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn packaged_candidates(executable: &Path) -> Vec<PathBuf> {
    let mut candidates = packaged_layout_candidates(executable);
    if let Ok(canonical) = executable.canonicalize() {
        if canonical != executable {
            candidates.extend(packaged_layout_candidates(&canonical));
        }
    }
    candidates
}

fn packaged_layout_candidates(executable: &Path) -> Vec<PathBuf> {
    let Some(bin_dir) = executable.parent() else {
        return Vec::new();
    };

    let mut candidates = vec![bin_dir.join("web")];
    if let Some(prefix) = bin_dir.parent() {
        candidates.push(prefix.join("share/a3s/web"));
    }
    candidates
}

fn find_existing_web_dir(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates
        .into_iter()
        .map(clean_path)
        .find(|candidate| candidate.join("index.html").is_file())
}

fn upward_candidates(start: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut current = Some(start);
    while let Some(dir) = current {
        candidates.push(dir.join("apps/web/dist/workspace"));
        candidates.push(dir.join("apps/web/dist"));
        current = dir.parent();
    }
    candidates
}

fn clean_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_layouts_cover_archives_and_install_prefixes() {
        let executable = Path::new("prefix").join("bin").join("a3s");

        assert_eq!(
            packaged_layout_candidates(&executable),
            vec![
                Path::new("prefix").join("bin/web"),
                Path::new("prefix").join("share/a3s/web"),
            ]
        );
    }

    #[test]
    fn packaged_layouts_are_checked_before_development_fallbacks() {
        let executable = Path::new("prefix").join("bin").join("a3s");
        let mut candidates = packaged_layout_candidates(&executable);
        candidates.extend(upward_candidates(Path::new("workspace/project")));

        assert_eq!(candidates[0], Path::new("prefix").join("bin/web"));
        assert_eq!(candidates[1], Path::new("prefix").join("share/a3s/web"));
        assert!(candidates[2].ends_with("apps/web/dist/workspace"));
    }

    #[test]
    fn existing_packaged_assets_win_over_development_fallbacks() {
        let root = tempfile::tempdir().expect("temporary asset layouts");
        let packaged = root.path().join("bin/web");
        let development = root.path().join("apps/web/dist/workspace");
        std::fs::create_dir_all(&packaged).expect("packaged Web directory");
        std::fs::create_dir_all(&development).expect("development Web directory");
        std::fs::write(packaged.join("index.html"), "packaged").expect("packaged index");
        std::fs::write(development.join("index.html"), "development").expect("development index");

        let found =
            find_existing_web_dir([packaged.clone(), development]).expect("existing Web directory");

        assert_eq!(found, packaged.canonicalize().expect("canonical package"));
    }

    #[test]
    fn cached_assets_are_version_scoped() {
        let root = tempfile::tempdir().expect("temporary Web data root");

        assert_eq!(
            cached_web_dir(root.path()),
            root.path().join("web").join(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn checksum_files_bind_the_digest_to_the_expected_asset() {
        let digest = "0123456789abcdef".repeat(4);

        assert_eq!(
            parse_checksum_file(
                format!("{digest}  a3s-web-v0.9.7.tar.gz\n").as_bytes(),
                "a3s-web-v0.9.7.tar.gz"
            )
            .expect("matching checksum"),
            digest
        );
        assert!(parse_checksum_file(
            format!("{digest}  other.tar.gz\n").as_bytes(),
            "a3s-web-v0.9.7.tar.gz"
        )
        .is_err());
    }

    #[test]
    fn release_extraction_rejects_links_and_path_traversal() {
        let linked_archive = {
            let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_path("web/escape").expect("set link path");
            header
                .set_link_name("../../outside")
                .expect("set escaping link target");
            header.set_size(0);
            header.set_cksum();
            archive
                .append(&header, std::io::empty())
                .expect("append link fixture");
            archive
                .into_inner()
                .expect("finish link tar")
                .finish()
                .expect("finish link gzip")
        };
        let traversal_archive = {
            let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let body = b"outside";
            let mut header = tar::Header::new_gnu();
            let path = b"web/../../outside";
            header.as_mut_bytes()[..path.len()].copy_from_slice(path);
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append(&header, &body[..])
                .expect("append traversal fixture");
            archive
                .into_inner()
                .expect("finish traversal tar")
                .finish()
                .expect("finish traversal gzip")
        };
        let root = tempfile::tempdir().expect("temporary extraction root");

        assert!(extract_release_archive(
            &linked_archive,
            &root.path().join("linked"),
            "a3s-web.tar.gz"
        )
        .is_err());
        assert!(extract_release_archive(
            &traversal_archive,
            &root.path().join("traversal"),
            "a3s-web.tar.gz"
        )
        .is_err());
        assert!(
            !root.path().join("outside").exists(),
            "malicious archive escaped extraction root"
        );
    }

    #[cfg(unix)]
    #[test]
    fn executable_symlink_can_find_assets_in_its_cellar_prefix() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary Homebrew layout");
        let linked_bin = root.path().join("bin");
        let cellar_bin = root.path().join("Cellar/a3s/0.9.2/bin");
        let cellar_web = root.path().join("Cellar/a3s/0.9.2/share/a3s/web");
        std::fs::create_dir_all(&linked_bin).expect("linked bin directory");
        std::fs::create_dir_all(&cellar_bin).expect("Cellar bin directory");
        std::fs::create_dir_all(&cellar_web).expect("Cellar Web directory");
        std::fs::write(cellar_bin.join("a3s"), "binary").expect("Cellar binary");
        std::fs::write(cellar_web.join("index.html"), "web").expect("Cellar Web index");
        let executable = linked_bin.join("a3s");
        symlink(cellar_bin.join("a3s"), &executable).expect("binary symlink");

        let found = find_existing_web_dir(packaged_candidates(&executable))
            .expect("Web assets beside the canonical executable prefix");

        assert_eq!(
            found,
            cellar_web.canonicalize().expect("canonical Web path")
        );
    }
}
