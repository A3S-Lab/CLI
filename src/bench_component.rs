//! Managed installation for the private Bench control component.
//!
//! The public command remains `a3s bench`. The downloaded entrypoint lives
//! under `~/.a3s/components/bench` and is never added to PATH. It controls
//! benchmark planning and evaluation; A3S OS Runtime remains the only layer
//! that executes Candidate and Judge Agent Assets.

use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component as PathComponent, Path, PathBuf};
use std::process::Command;

const RELEASE_API: &str = "https://api.github.com/repos/A3S-Lab/a3s-bench/releases/latest";
const COMPONENT_SCHEMA: &str = "a3s.component.v1";
const CURRENT_SCHEMA: &str = "a3s.component-current.v1";
const RECEIPT_SCHEMA: &str = "a3s.component-receipt.v1";
const COMPONENT_ID: &str = "bench";
const CLI_PROTOCOL: &str = "a3s-bench-cli/v1";

#[cfg(windows)]
const ENTRYPOINT_NAME: &str = "a3s-bench.exe";
#[cfg(not(windows))]
const ENTRYPOINT_NAME: &str = "a3s-bench";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledBench {
    pub(crate) version: String,
    pub(crate) target: String,
    pub(crate) path: PathBuf,
    archive_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BenchState {
    Missing,
    Installed(InstalledBench),
    Broken(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseAsset {
    version: String,
    target: String,
    name: String,
    url: String,
    sha256: Option<String>,
    checksum_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BundleManifest {
    version: String,
    target: String,
    entrypoint: PathBuf,
    required_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentRecord {
    package_root: PathBuf,
    version: String,
    target: String,
    archive_sha256: String,
}

pub(crate) fn inspect() -> BenchState {
    let root = match bench_root() {
        Ok(root) => root,
        Err(error) => return BenchState::Broken(error.to_string()),
    };
    inspect_at(&root)
}

pub(crate) fn install() -> anyhow::Result<InstalledBench> {
    match inspect() {
        BenchState::Installed(installed) => {
            println!(
                "✓ a3s bench {} is already installed at {}",
                installed.version,
                installed.path.display()
            );
            return Ok(installed);
        }
        BenchState::Broken(error) => {
            eprintln!("a3s: repairing invalid bench component state: {error}");
        }
        BenchState::Missing => {}
    }

    let target = release_target().ok_or_else(|| {
        anyhow::anyhow!(
            "a3s bench is not published for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let release = fetch_latest_release(target)?;
    install_release(&release)
}

pub(crate) fn update() -> anyhow::Result<InstalledBench> {
    let current = match inspect() {
        BenchState::Installed(installed) => installed,
        BenchState::Missing => {
            return Err(anyhow::anyhow!(
                "a3s bench is not installed; run `a3s install bench` first"
            ));
        }
        BenchState::Broken(error) => {
            return Err(anyhow::anyhow!(
                "bench component state is invalid: {error}; run `a3s install bench` to repair it before updating"
            ));
        }
    };

    let release = fetch_latest_release(&current.target)?;
    if crate::update::version_ge(&current.version, &release.version) {
        println!("✓ a3s bench {} is already up to date", current.version);
        return Ok(current);
    }
    install_release(&release)
}

pub(crate) fn ensure() -> anyhow::Result<InstalledBench> {
    match inspect() {
        BenchState::Installed(installed) => Ok(installed),
        BenchState::Missing => {
            eprintln!("a3s: Bench control component is not installed; installing it now...");
            install()
        }
        BenchState::Broken(error) => {
            eprintln!("a3s: bench component is invalid; repairing it now: {error}");
            install()
        }
    }
}

fn install_release(release: &ReleaseAsset) -> anyhow::Result<InstalledBench> {
    let root = bench_root()?;
    fs::create_dir_all(&root)?;
    let _lock = InstallLock::acquire(&root)?;

    // Reconcile after taking the process-wide installation lock.
    if let BenchState::Installed(installed) = inspect_at(&root) {
        if crate::update::version_ge(&installed.version, &release.version) {
            return Ok(installed);
        }
    }

    let staging = root.join(format!(
        ".staging-{}-{}",
        std::process::id(),
        release.version
    ));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    let _cleanup = CleanupDir(staging.clone());
    let archive = staging.join(&release.name);

    eprintln!(
        "a3s: downloading Bench control component {} for {}...",
        release.version, release.target
    );
    download_to(&release.url, &archive)?;
    let expected_sha = resolve_sha256(release)?;
    let actual_sha = sha256_file(&archive)?;
    if actual_sha != expected_sha {
        return Err(anyhow::anyhow!(
            "bench archive checksum mismatch: expected {expected_sha}, got {actual_sha}"
        ));
    }

    validate_tar_archive(&archive)?;
    let payload = staging.join("payload");
    fs::create_dir_all(&payload)?;
    extract_tar_archive(&archive, &payload)?;
    validate_extracted_tree(&payload)?;

    let manifest_path = find_unique_manifest(&payload)?;
    let package_root = manifest_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("bench component manifest has no package root"))?;
    let manifest = parse_bundle_manifest(&fs::read(&manifest_path)?)?;
    validate_manifest(&manifest, release, package_root)?;
    let entrypoint = package_root.join(&manifest.entrypoint);
    make_executable(&entrypoint)?;
    verify_component_probe(&entrypoint, &manifest)?;

    let versions = root.join("versions");
    fs::create_dir_all(&versions)?;
    let final_dir = versions.join(&release.version).join(&release.target);
    let parent = final_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("bench version directory has no parent"))?;
    fs::create_dir_all(parent)?;
    write_receipt(package_root, release, &actual_sha, &manifest)?;
    replace_package_dir(package_root, &final_dir)?;

    let installed = validate_installed_dir(&final_dir)?;
    verify_component_probe(&installed.path, &manifest)?;
    activate(&root, &final_dir, &installed, &actual_sha)?;
    println!(
        "✓ installed a3s bench {} at {}",
        installed.version,
        installed.path.display()
    );
    Ok(installed)
}

fn replace_package_dir(source: &Path, destination: &Path) -> anyhow::Result<()> {
    if fs::symlink_metadata(destination).is_err() {
        fs::rename(source, destination)?;
        return Ok(());
    }

    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("bench destination has no valid file name"))?;
    let backup =
        destination.with_file_name(format!(".{file_name}.replaced-{}", std::process::id()));
    if fs::symlink_metadata(&backup).is_ok() {
        return Err(anyhow::anyhow!(
            "bench replacement backup already exists: {}",
            backup.display()
        ));
    }
    fs::rename(destination, &backup)?;
    if let Err(error) = fs::rename(source, destination) {
        let restore = fs::rename(&backup, destination);
        return match restore {
            Ok(()) => Err(anyhow::anyhow!(
                "could not activate the verified bench bundle: {error}"
            )),
            Err(restore_error) => Err(anyhow::anyhow!(
                "could not activate the verified bench bundle ({error}) or restore the previous directory ({restore_error})"
            )),
        };
    }
    if let Err(error) = remove_path(&backup) {
        eprintln!(
            "warning: could not remove replaced bench directory {}: {error}",
            backup.display()
        );
    }
    Ok(())
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn bench_root() -> anyhow::Result<PathBuf> {
    if let Some(root) = std::env::var_os("A3S_COMPONENTS_DIR") {
        return Ok(PathBuf::from(root).join(COMPONENT_ID));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set; set A3S_COMPONENTS_DIR"))?;
    Ok(home.join(".a3s").join("components").join(COMPONENT_ID))
}

fn inspect_at(root: &Path) -> BenchState {
    let current = root.join("current.json");
    if !current.exists() {
        return BenchState::Missing;
    }
    match read_current(root, &current).and_then(|record| {
        let installed = validate_installed_dir(&record.package_root)?;
        if installed.version != record.version
            || installed.target != record.target
            || installed.archive_sha256 != record.archive_sha256
        {
            return Err(anyhow::anyhow!(
                "bench current.json identity does not match its installed bundle"
            ));
        }
        let host_target = release_target().ok_or_else(|| {
            anyhow::anyhow!(
                "a3s bench is not supported on {}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
        if installed.target != host_target {
            return Err(anyhow::anyhow!(
                "bench bundle target {} does not match this host ({host_target})",
                installed.target
            ));
        }
        let expected = root
            .join("versions")
            .join(&installed.version)
            .join(&installed.target);
        if record.package_root != expected {
            return Err(anyhow::anyhow!(
                "bench current.json path does not match its version and target"
            ));
        }
        Ok(installed)
    }) {
        Ok(installed) => BenchState::Installed(installed),
        Err(error) => BenchState::Broken(error.to_string()),
    }
}

fn read_current(root: &Path, path: &Path) -> anyhow::Result<CurrentRecord> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    if value.get("schema").and_then(|v| v.as_str()) != Some(CURRENT_SCHEMA)
        || value.get("component").and_then(|v| v.as_str()) != Some(COMPONENT_ID)
    {
        return Err(anyhow::anyhow!(
            "invalid bench current.json schema or component"
        ));
    }
    let relative = value
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("bench current.json is missing path"))?;
    let relative = safe_relative_path(relative)?;
    let version = required_string(&value, "version")?;
    validate_stable_version(&version)?;
    let target = required_string(&value, "target")?;
    validate_release_target_name(&target)?;
    let archive_sha256 = required_string(&value, "archive_sha256")?;
    if !is_sha256(&archive_sha256) {
        return Err(anyhow::anyhow!(
            "bench current.json archive_sha256 is invalid"
        ));
    }
    Ok(CurrentRecord {
        package_root: root.join(relative),
        version,
        target,
        archive_sha256: archive_sha256.to_ascii_lowercase(),
    })
}

fn validate_installed_dir(dir: &Path) -> anyhow::Result<InstalledBench> {
    if !dir.is_dir() {
        return Err(anyhow::anyhow!(
            "bench component directory is missing: {}",
            dir.display()
        ));
    }
    validate_extracted_tree(dir)?;
    let manifest_path = dir.join("component.json");
    let manifest = parse_bundle_manifest(&fs::read(&manifest_path).map_err(|error| {
        anyhow::anyhow!("could not read {}: {error}", manifest_path.display())
    })?)?;
    validate_stable_version(&manifest.version)?;
    validate_release_target_name(&manifest.target)?;
    if manifest
        .entrypoint
        .file_name()
        .and_then(|name| name.to_str())
        != Some(ENTRYPOINT_NAME)
    {
        return Err(anyhow::anyhow!(
            "bench component entrypoint must be {ENTRYPOINT_NAME}"
        ));
    }
    let entrypoint = dir.join(&manifest.entrypoint);
    if !entrypoint.is_file() {
        return Err(anyhow::anyhow!(
            "bench entrypoint is missing: {}",
            entrypoint.display()
        ));
    }
    for required in &manifest.required_files {
        if !dir.join(required).is_file() {
            return Err(anyhow::anyhow!(
                "bench required file is missing: {}",
                required.display()
            ));
        }
    }
    ensure_executable(&entrypoint)?;
    let archive_sha256 = validate_receipt(dir, &manifest)?;
    Ok(InstalledBench {
        version: manifest.version,
        target: manifest.target,
        path: entrypoint,
        archive_sha256,
    })
}

fn activate(
    root: &Path,
    package_root: &Path,
    installed: &InstalledBench,
    archive_sha: &str,
) -> anyhow::Result<()> {
    if installed.archive_sha256 != archive_sha {
        return Err(anyhow::anyhow!(
            "bench archive digest does not match the installed bundle receipt"
        ));
    }
    installed
        .path
        .strip_prefix(package_root)
        .map_err(|_| anyhow::anyhow!("bench entrypoint is outside its package root"))?;
    let relative_package_root = package_root
        .strip_prefix(root)
        .map_err(|_| anyhow::anyhow!("bench package is outside component root"))?;
    let value = serde_json::json!({
        "schema": CURRENT_SCHEMA,
        "component": COMPONENT_ID,
        "version": installed.version,
        "target": installed.target,
        "path": relative_package_root,
        "archive_sha256": archive_sha,
    });
    let temp = root.join(format!(".current-{}.json", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(&value)?)?;
    fs::rename(temp, root.join("current.json"))?;
    Ok(())
}

fn write_receipt(
    package_root: &Path,
    release: &ReleaseAsset,
    archive_sha: &str,
    manifest: &BundleManifest,
) -> anyhow::Result<()> {
    let payload_sha256 = sha256_tree(package_root)?;
    let value = serde_json::json!({
        "schema": RECEIPT_SCHEMA,
        "component": COMPONENT_ID,
        "version": release.version,
        "target": release.target,
        "source_url": release.url,
        "archive_sha256": archive_sha,
        "payload_sha256": payload_sha256,
        "entrypoint": manifest.entrypoint,
        "cli_protocol": CLI_PROTOCOL,
    });
    fs::write(
        package_root.join("receipt.json"),
        serde_json::to_vec_pretty(&value)?,
    )?;
    Ok(())
}

fn validate_receipt(dir: &Path, manifest: &BundleManifest) -> anyhow::Result<String> {
    let receipt_path = dir.join("receipt.json");
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_path).map_err(|error| {
            anyhow::anyhow!("could not read {}: {error}", receipt_path.display())
        })?)?;
    if value.get("schema").and_then(|v| v.as_str()) != Some(RECEIPT_SCHEMA)
        || value.get("component").and_then(|v| v.as_str()) != Some(COMPONENT_ID)
        || value.get("cli_protocol").and_then(|v| v.as_str()) != Some(CLI_PROTOCOL)
    {
        return Err(anyhow::anyhow!(
            "bench receipt has incompatible schema, component, or CLI protocol"
        ));
    }
    let version = required_string(&value, "version")?;
    let target = required_string(&value, "target")?;
    let entrypoint = safe_relative_path(&required_string(&value, "entrypoint")?)?;
    let source_url = required_string(&value, "source_url")?;
    validate_release_download_url(&source_url)?;
    if version != manifest.version || target != manifest.target || entrypoint != manifest.entrypoint
    {
        return Err(anyhow::anyhow!(
            "bench receipt identity does not match component.json"
        ));
    }
    let archive_sha256 = required_string(&value, "archive_sha256")?;
    if !is_sha256(&archive_sha256) {
        return Err(anyhow::anyhow!("bench receipt archive_sha256 is invalid"));
    }
    let payload_sha256 = required_string(&value, "payload_sha256")?;
    if !is_sha256(&payload_sha256) {
        return Err(anyhow::anyhow!("bench receipt payload_sha256 is invalid"));
    }
    let actual_payload_sha256 = sha256_tree(dir)?;
    if !payload_sha256.eq_ignore_ascii_case(&actual_payload_sha256) {
        return Err(anyhow::anyhow!("bench installed payload checksum mismatch"));
    }
    Ok(archive_sha256.to_ascii_lowercase())
}

fn fetch_latest_release(target: &str) -> anyhow::Result<ReleaseAsset> {
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--connect-timeout",
            "5",
            "--max-time",
            "15",
            RELEASE_API,
        ])
        .output()
        .map_err(|error| anyhow::anyhow!("could not run curl: {error}"))?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "could not query the a3s-bench release service; the Bench control component may not be published yet"
        ));
    }
    parse_release_response(&output.stdout, target)
}

fn parse_release_response(bytes: &[u8], target: &str) -> anyhow::Result<ReleaseAsset> {
    validate_release_target_name(target)?;
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let tag = value
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("bench release response is missing tag_name"))?;
    let version = canonical_release_version(tag)?;
    let name = format!("a3s-bench-{version}-{target}.tar.gz");
    let checksum_name = format!("{name}.sha256");
    let assets = value
        .get("assets")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("bench release response is missing assets"))?;
    let archive = assets
        .iter()
        .find(|asset| asset.get("name").and_then(|v| v.as_str()) == Some(name.as_str()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Bench control component {version} is not published for {target}; the repository currently contains design and fixtures only"
            )
        })?;
    let url = archive
        .get("browser_download_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("bench release asset is missing download URL"))?
        .to_string();
    validate_release_download_url(&url)?;
    let sha256 = archive
        .get("digest")
        .and_then(|v| v.as_str())
        .and_then(parse_sha256_digest);
    let checksum_url = assets
        .iter()
        .find(|asset| asset.get("name").and_then(|v| v.as_str()) == Some(checksum_name.as_str()))
        .and_then(|asset| asset.get("browser_download_url"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if let Some(url) = &checksum_url {
        validate_release_download_url(url)?;
    }
    if sha256.is_none() && checksum_url.is_none() {
        return Err(anyhow::anyhow!(
            "bench release asset has no SHA-256 digest or checksum sidecar"
        ));
    }
    Ok(ReleaseAsset {
        version,
        target: target.to_string(),
        name,
        url,
        sha256,
        checksum_url,
    })
}

fn validate_release_download_url(url: &str) -> anyhow::Result<()> {
    const PREFIX: &str = "https://github.com/A3S-Lab/a3s-bench/releases/download/";
    if url.starts_with(PREFIX) {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "bench release asset URL is outside the trusted repository"
        ))
    }
}

fn resolve_sha256(release: &ReleaseAsset) -> anyhow::Result<String> {
    if let Some(digest) = &release.sha256 {
        return Ok(digest.clone());
    }
    let url = release
        .checksum_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("bench release is missing checksum URL"))?;
    let output = Command::new("curl")
        .args(["-fsSL", "--connect-timeout", "5", "--max-time", "15", url])
        .output()
        .map_err(|error| anyhow::anyhow!("could not run curl: {error}"))?;
    if !output.status.success() {
        return Err(anyhow::anyhow!("could not download bench checksum"));
    }
    parse_checksum_file(&output.stdout, &release.name)
}

fn parse_sha256_digest(value: &str) -> Option<String> {
    let digest = value.trim().strip_prefix("sha256:").unwrap_or(value.trim());
    is_sha256(digest).then(|| digest.to_ascii_lowercase())
}

fn parse_checksum_file(bytes: &[u8], expected_name: &str) -> anyhow::Result<String> {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(digest) = parts.next() else {
            continue;
        };
        let name = parts.next().unwrap_or("").trim_start_matches('*');
        if is_sha256(digest) && (name.is_empty() || name == expected_name) {
            return Ok(digest.to_ascii_lowercase());
        }
    }
    Err(anyhow::anyhow!("bench checksum sidecar is invalid"))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn download_to(url: &str, destination: &Path) -> anyhow::Result<()> {
    let status = Command::new("curl")
        .args([
            "-fL",
            "--retry",
            "2",
            "--connect-timeout",
            "5",
            "--max-time",
            "300",
            "--show-error",
            "--progress-bar",
            "-o",
        ])
        .arg(destination)
        .arg(url)
        .status()
        .map_err(|error| anyhow::anyhow!("could not run curl: {error}"))?;
    if !status.success() {
        return Err(anyhow::anyhow!("failed to download {url}"));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_tree(root: &Path) -> anyhow::Result<String> {
    fn collect_files(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(anyhow::anyhow!(
                    "bench payload contains a symbolic link: {}",
                    path.display()
                ));
            }
            if metadata.is_dir() {
                collect_files(root, &path, files)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| anyhow::anyhow!("bench payload path escaped its root"))?;
                if relative != Path::new("receipt.json") {
                    files.push(path);
                }
            } else {
                return Err(anyhow::anyhow!(
                    "bench payload contains a non-regular file: {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|left, right| {
        left.strip_prefix(root)
            .unwrap_or(left)
            .cmp(right.strip_prefix(root).unwrap_or(right))
    });

    let mut hasher = Sha256::new();
    for path in files {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| anyhow::anyhow!("bench payload path escaped its root"))?;
        let relative = relative
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("bench payload path is not valid UTF-8"))?;
        let metadata = fs::metadata(&path)?;
        hasher.update((relative.len() as u64).to_le_bytes());
        hasher.update(relative.as_bytes());
        hasher.update(metadata.len().to_le_bytes());

        let mut file = fs::File::open(&path)?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn validate_tar_archive(path: &Path) -> anyhow::Result<()> {
    let names = Command::new("tar")
        .args(["tzf"])
        .arg(path)
        .output()
        .map_err(|error| anyhow::anyhow!("could not inspect bench archive: {error}"))?;
    if !names.status.success() {
        return Err(anyhow::anyhow!("bench archive is not a readable tar.gz"));
    }
    for line in String::from_utf8_lossy(&names.stdout).lines() {
        safe_relative_path(line.trim_end_matches('/'))?;
    }

    let verbose = Command::new("tar")
        .args(["tvzf"])
        .arg(path)
        .output()
        .map_err(|error| anyhow::anyhow!("could not inspect bench archive types: {error}"))?;
    if !verbose.status.success() {
        return Err(anyhow::anyhow!("could not inspect bench archive types"));
    }
    for line in String::from_utf8_lossy(&verbose.stdout).lines() {
        let kind = line.as_bytes().first().copied().unwrap_or(b'?');
        if kind != b'-' && kind != b'd' {
            return Err(anyhow::anyhow!(
                "bench archive contains unsupported entry type"
            ));
        }
    }
    Ok(())
}

fn extract_tar_archive(path: &Path, destination: &Path) -> anyhow::Result<()> {
    let status = Command::new("tar")
        .arg("xzf")
        .arg(path)
        .arg("-C")
        .arg(destination)
        .status()
        .map_err(|error| anyhow::anyhow!("could not extract bench archive: {error}"))?;
    if !status.success() {
        return Err(anyhow::anyhow!("failed to extract bench archive"));
    }
    Ok(())
}

fn validate_extracted_tree(root: &Path) -> anyhow::Result<()> {
    fn visit(root: &Path, dir: &Path, count: &mut usize) -> anyhow::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            *count += 1;
            if *count > 10_000 {
                return Err(anyhow::anyhow!("bench archive contains too many files"));
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(anyhow::anyhow!(
                    "bench archive contains a symbolic link: {}",
                    path.display()
                ));
            }
            if metadata.is_dir() {
                visit(root, &path, count)?;
            } else if !metadata.is_file() {
                return Err(anyhow::anyhow!(
                    "bench archive contains a non-regular file: {}",
                    path.display()
                ));
            }
            if !path.starts_with(root) {
                return Err(anyhow::anyhow!("bench archive escaped extraction root"));
            }
        }
        Ok(())
    }
    visit(root, root, &mut 0)
}

fn find_unique_manifest(root: &Path) -> anyhow::Result<PathBuf> {
    fn visit(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) -> anyhow::Result<()> {
        if depth > 3 {
            return Ok(());
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit(&path, depth + 1, found)?;
            } else if path.file_name().and_then(|name| name.to_str()) == Some("component.json") {
                found.push(path);
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    visit(root, 0, &mut found)?;
    match found.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(anyhow::anyhow!(
            "bench archive does not contain component.json"
        )),
        _ => Err(anyhow::anyhow!(
            "bench archive contains multiple component manifests"
        )),
    }
}

fn parse_bundle_manifest(bytes: &[u8]) -> anyhow::Result<BundleManifest> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("schema").and_then(|v| v.as_str()) != Some(COMPONENT_SCHEMA)
        || value.get("component").and_then(|v| v.as_str()) != Some(COMPONENT_ID)
        || value.get("cli_protocol").and_then(|v| v.as_str()) != Some(CLI_PROTOCOL)
    {
        return Err(anyhow::anyhow!(
            "bench component manifest has incompatible schema, component, or CLI protocol"
        ));
    }
    let version = required_string(&value, "version")?;
    let target = required_string(&value, "target")?;
    let entrypoint = safe_relative_path(&required_string(&value, "entrypoint")?)?;
    let required_files = value
        .get("required_files")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("bench component manifest is missing required_files"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("required_files must contain strings"))
                .and_then(safe_relative_path)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(BundleManifest {
        version,
        target,
        entrypoint,
        required_files,
    })
}

fn required_string(value: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("bench component manifest is missing {key}"))
}

fn canonical_release_version(tag: &str) -> anyhow::Result<String> {
    let version = tag.trim().strip_prefix('v').unwrap_or(tag.trim());
    validate_stable_version(version)?;
    Ok(version.to_string())
}

fn validate_stable_version(version: &str) -> anyhow::Result<()> {
    let parts = version.split('.').collect::<Vec<_>>();
    let valid = parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == &"0" || !part.starts_with('0'))
                && part.parse::<u32>().is_ok()
        });
    if !valid {
        return Err(anyhow::anyhow!(
            "bench release version must be a stable SemVer (for example 1.2.3)"
        ));
    }
    Ok(())
}

fn validate_release_target_name(target: &str) -> anyhow::Result<()> {
    if matches!(
        target,
        "darwin-arm64" | "darwin-x86_64" | "linux-arm64" | "linux-x86_64"
    ) {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "unsupported bench release target: {target}"
        ))
    }
}

fn validate_manifest(
    manifest: &BundleManifest,
    release: &ReleaseAsset,
    package_root: &Path,
) -> anyhow::Result<()> {
    validate_stable_version(&manifest.version)?;
    validate_release_target_name(&manifest.target)?;
    if manifest.version != release.version || manifest.target != release.target {
        return Err(anyhow::anyhow!(
            "bench component manifest version or target does not match release"
        ));
    }
    if manifest
        .entrypoint
        .file_name()
        .and_then(|name| name.to_str())
        != Some(ENTRYPOINT_NAME)
    {
        return Err(anyhow::anyhow!(
            "bench component entrypoint must be {ENTRYPOINT_NAME}"
        ));
    }
    if !package_root.join(&manifest.entrypoint).is_file() {
        return Err(anyhow::anyhow!("bench component entrypoint is missing"));
    }
    for required in &manifest.required_files {
        if !package_root.join(required).is_file() {
            return Err(anyhow::anyhow!(
                "bench component required file is missing: {}",
                required.display()
            ));
        }
    }
    Ok(())
}

fn verify_component_probe(path: &Path, manifest: &BundleManifest) -> anyhow::Result<()> {
    let output = Command::new(path)
        .args(["--component-info", "--json"])
        .output()
        .map_err(|error| anyhow::anyhow!("could not run bench component probe: {error}"))?;
    if !output.status.success() {
        return Err(anyhow::anyhow!("bench component probe failed"));
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    if value.get("component").and_then(|v| v.as_str()) != Some(COMPONENT_ID)
        || value.get("version").and_then(|v| v.as_str()) != Some(manifest.version.as_str())
        || value.get("target").and_then(|v| v.as_str()) != Some(manifest.target.as_str())
        || value.get("cli_protocol").and_then(|v| v.as_str()) != Some(CLI_PROTOCOL)
    {
        return Err(anyhow::anyhow!(
            "bench component probe returned incompatible identity"
        ));
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> anyhow::Result<PathBuf> {
    let path = Path::new(value);
    if value.is_empty() || path.is_absolute() {
        return Err(anyhow::anyhow!("component path must be relative"));
    }
    for component in path.components() {
        if !matches!(component, PathComponent::Normal(_)) {
            return Err(anyhow::anyhow!(
                "component path contains an unsafe segment: {value}"
            ));
        }
    }
    Ok(path.to_path_buf())
}

fn ensure_executable(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)?.permissions().mode();
        if mode & 0o111 == 0 {
            return Err(anyhow::anyhow!(
                "bench entrypoint is not executable: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn release_target() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86_64") => "linux-x86_64",
        _ => return None,
    })
}

fn make_executable(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

struct InstallLock {
    path: PathBuf,
}

impl InstallLock {
    fn acquire(root: &Path) -> anyhow::Result<Self> {
        let path = root.join("install.lock");
        for _ in 0..2 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    if let Err(error) = writeln!(file, "{}", std::process::id()) {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(error.into());
                    }
                    return Ok(Self { path });
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists
                        && reclaim_stale_lock(&path)? =>
                {
                    continue;
                }
                Err(error) => {
                    return Err(anyhow::anyhow!(
                        "another bench install may be running ({}): {error}",
                        path.display()
                    ));
                }
            }
        }
        Err(anyhow::anyhow!(
            "could not acquire bench install lock: {}",
            path.display()
        ))
    }
}

fn reclaim_stale_lock(path: &Path) -> anyhow::Result<bool> {
    let pid = fs::read_to_string(path)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok());
    let stale = match pid {
        Some(pid) if pid == std::process::id() => {
            lock_older_than(path, std::time::Duration::from_secs(60 * 60))?
        }
        Some(pid) => {
            let too_old = lock_older_than(path, std::time::Duration::from_secs(60 * 60))?;
            too_old
                || match process_is_running(pid) {
                    Some(running) => !running,
                    None => false,
                }
        }
        None => lock_older_than(path, std::time::Duration::from_secs(10 * 60))?,
    };
    if !stale {
        return Ok(false);
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn lock_older_than(path: &Path, age: std::time::Duration) -> anyhow::Result<bool> {
    let modified = fs::metadata(path)?.modified()?;
    Ok(modified.elapsed().unwrap_or_default() >= age)
}

fn process_is_running(pid: u32) -> Option<bool> {
    #[cfg(unix)]
    {
        let pid = pid.to_string();
        let output = Command::new("/bin/ps")
            .args(["-p", pid.as_str(), "-o", "pid="])
            .output()
            .ok()?;
        Some(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == pid.as_str(),
        )
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct CleanupDir(PathBuf);

impl Drop for CleanupDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_with_github_digest() {
        let json = br#"{
          "tag_name": "v1.2.3",
          "assets": [{
            "name": "a3s-bench-1.2.3-linux-x86_64.tar.gz",
            "browser_download_url": "https://github.com/A3S-Lab/a3s-bench/releases/download/v1.2.3/a3s-bench-1.2.3-linux-x86_64.tar.gz",
            "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
          }]
        }"#;
        let release = parse_release_response(json, "linux-x86_64").unwrap();
        assert_eq!(release.version, "1.2.3");
        assert_eq!(release.sha256, Some("a".repeat(64)));
        assert_eq!(release.checksum_url, None);
    }

    #[test]
    fn parses_release_with_checksum_sidecar() {
        let json = br#"{
          "tag_name": "v1.2.3",
          "assets": [
            {
              "name": "a3s-bench-1.2.3-darwin-arm64.tar.gz",
              "browser_download_url": "https://github.com/A3S-Lab/a3s-bench/releases/download/v1.2.3/a3s-bench-1.2.3-darwin-arm64.tar.gz"
            },
            {
              "name": "a3s-bench-1.2.3-darwin-arm64.tar.gz.sha256",
              "browser_download_url": "https://github.com/A3S-Lab/a3s-bench/releases/download/v1.2.3/a3s-bench-1.2.3-darwin-arm64.tar.gz.sha256"
            }
          ]
        }"#;
        let release = parse_release_response(json, "darwin-arm64").unwrap();
        assert_eq!(release.sha256, None);
        assert_eq!(
            release.checksum_url.as_deref(),
            Some("https://github.com/A3S-Lab/a3s-bench/releases/download/v1.2.3/a3s-bench-1.2.3-darwin-arm64.tar.gz.sha256")
        );
    }

    #[test]
    fn release_without_target_asset_is_honest() {
        let error = parse_release_response(br#"{"tag_name":"v0.1.0","assets":[]}"#, "linux-x86_64")
            .unwrap_err()
            .to_string();
        assert!(error.contains("not published"));
        assert!(error.contains("design and fixtures"));
    }

    #[test]
    fn release_asset_url_must_stay_in_the_bench_repository() {
        let json = br#"{
          "tag_name": "v1.2.3",
          "assets": [{
            "name": "a3s-bench-1.2.3-linux-x86_64.tar.gz",
            "browser_download_url": "https://example.test/bench.tar.gz",
            "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
          }]
        }"#;
        let error = parse_release_response(json, "linux-x86_64")
            .unwrap_err()
            .to_string();
        assert!(error.contains("trusted repository"));
    }

    #[test]
    fn release_version_must_be_a_safe_stable_semver() {
        for tag in ["../escape", "1.2", "1.2.3-alpha", "01.2.3", "1/2/3"] {
            let json = format!(r#"{{"tag_name":"{tag}","assets":[]}}"#);
            let error = parse_release_response(json.as_bytes(), "linux-x86_64")
                .unwrap_err()
                .to_string();
            assert!(error.contains("stable SemVer"), "unexpected error: {error}");
        }
        assert_eq!(canonical_release_version("v1.2.3").unwrap(), "1.2.3");
    }

    #[test]
    fn checksum_parser_matches_expected_asset() {
        let digest = "b".repeat(64);
        let text = format!(
            "{}  other.tar.gz\n{}  wanted.tar.gz\n",
            "a".repeat(64),
            digest
        );
        assert_eq!(
            parse_checksum_file(text.as_bytes(), "wanted.tar.gz").unwrap(),
            digest
        );
    }

    #[test]
    fn rejects_unsafe_component_paths() {
        assert!(safe_relative_path("../escape").is_err());
        assert!(safe_relative_path("/absolute").is_err());
        assert!(safe_relative_path("safe/bin").is_ok());
    }

    #[test]
    fn parses_compatible_bundle_manifest() {
        let value = serde_json::json!({
            "schema": COMPONENT_SCHEMA,
            "component": COMPONENT_ID,
            "version": "1.2.3",
            "target": "linux-x86_64",
            "cli_protocol": CLI_PROTOCOL,
            "entrypoint": format!("bin/{ENTRYPOINT_NAME}"),
            "required_files": ["bin/judge-runner"]
        });
        let manifest = parse_bundle_manifest(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(manifest.version, "1.2.3");
        assert_eq!(
            manifest.required_files,
            vec![PathBuf::from("bin/judge-runner")]
        );
    }

    #[test]
    fn missing_current_pointer_does_not_create_state() {
        let root = std::env::temp_dir().join(format!(
            "a3s-bench-component-missing-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        assert_eq!(inspect_at(&root), BenchState::Missing);
        assert!(!root.exists());
    }

    #[test]
    fn current_pointer_resolves_installed_bundle() {
        let Some(target) = release_target() else {
            return;
        };
        let archive_sha = "c".repeat(64);
        let root = std::env::temp_dir().join(format!(
            "a3s-bench-component-current-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("versions/1.2.3").join(target);
        fs::create_dir_all(dir.join("bin")).unwrap();
        let entrypoint = dir.join("bin").join(ENTRYPOINT_NAME);
        fs::write(&entrypoint, b"binary").unwrap();
        make_executable(&entrypoint).unwrap();
        let manifest = serde_json::json!({
            "schema": COMPONENT_SCHEMA,
            "component": COMPONENT_ID,
            "version": "1.2.3",
            "target": target,
            "cli_protocol": CLI_PROTOCOL,
            "entrypoint": format!("bin/{ENTRYPOINT_NAME}"),
            "required_files": []
        });
        fs::write(
            dir.join("component.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        write_test_receipt(&dir, "1.2.3", target, &archive_sha);
        let mut current = serde_json::json!({
            "schema": CURRENT_SCHEMA,
            "component": COMPONENT_ID,
            "version": "1.2.3",
            "target": target,
            "path": format!("versions/1.2.3/{target}"),
            "archive_sha256": archive_sha,
        });
        fs::write(
            root.join("current.json"),
            serde_json::to_vec(&current).unwrap(),
        )
        .unwrap();

        let state = inspect_at(&root);
        assert!(matches!(
            state,
            BenchState::Installed(InstalledBench { ref version, .. }) if version == "1.2.3"
        ));

        fs::write(&entrypoint, b"tampered").unwrap();
        assert!(matches!(inspect_at(&root), BenchState::Broken(_)));
        fs::write(&entrypoint, b"binary").unwrap();
        assert!(matches!(inspect_at(&root), BenchState::Installed(_)));

        current["version"] = serde_json::Value::String("1.2.4".to_string());
        fs::write(
            root.join("current.json"),
            serde_json::to_vec(&current).unwrap(),
        )
        .unwrap();
        assert!(matches!(inspect_at(&root), BenchState::Broken(_)));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn activation_records_package_root_not_entrypoint_parent() {
        let Some(target) = release_target() else {
            return;
        };
        let archive_sha = "c".repeat(64);
        let root = std::env::temp_dir().join(format!(
            "a3s-bench-component-activate-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let package_root = root.join("versions/1.2.3").join(target);
        fs::create_dir_all(package_root.join("bin")).unwrap();
        let entrypoint = package_root.join("bin").join(ENTRYPOINT_NAME);
        fs::write(&entrypoint, b"binary").unwrap();
        make_executable(&entrypoint).unwrap();
        let manifest = serde_json::json!({
            "schema": COMPONENT_SCHEMA,
            "component": COMPONENT_ID,
            "version": "1.2.3",
            "target": target,
            "cli_protocol": CLI_PROTOCOL,
            "entrypoint": format!("bin/{ENTRYPOINT_NAME}"),
            "required_files": []
        });
        fs::write(
            package_root.join("component.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        write_test_receipt(&package_root, "1.2.3", target, &archive_sha);
        let installed = validate_installed_dir(&package_root).unwrap();

        activate(&root, &package_root, &installed, &archive_sha).unwrap();

        let current: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("current.json")).unwrap()).unwrap();
        assert_eq!(
            current.get("path").and_then(|value| value.as_str()),
            Some(format!("versions/1.2.3/{target}").as_str())
        );
        assert_eq!(inspect_at(&root), BenchState::Installed(installed));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn verified_package_replaces_an_existing_version_directory() {
        let root = std::env::temp_dir().join(format!(
            "a3s-bench-component-replace-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("staging/package");
        let destination = root.join("versions/1.2.3/linux-x86_64");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(source.join("marker"), "verified").unwrap();
        fs::write(destination.join("marker"), "stale").unwrap();

        replace_package_dir(&source, &destination).unwrap();

        assert_eq!(
            fs::read_to_string(destination.join("marker")).unwrap(),
            "verified"
        );
        assert!(!source.exists());
        assert!(!destination
            .with_file_name(format!(".linux-x86_64.replaced-{}", std::process::id()))
            .exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn dead_process_install_lock_is_reclaimed() {
        let root =
            std::env::temp_dir().join(format!("a3s-bench-component-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let mut exited = Command::new("/usr/bin/true").spawn().unwrap();
        let exited_pid = exited.id();
        exited.wait().unwrap();
        fs::write(root.join("install.lock"), format!("{exited_pid}\n")).unwrap();

        let lock = InstallLock::acquire(&root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("install.lock"))
                .unwrap()
                .trim(),
            std::process::id().to_string()
        );
        drop(lock);
        assert!(!root.join("install.lock").exists());
        let _ = fs::remove_dir_all(root);
    }

    fn write_test_receipt(dir: &Path, version: &str, target: &str, archive_sha256: &str) {
        let payload_sha256 = sha256_tree(dir).unwrap();
        let receipt = serde_json::json!({
            "schema": RECEIPT_SCHEMA,
            "component": COMPONENT_ID,
            "version": version,
            "target": target,
            "source_url": format!("https://github.com/A3S-Lab/a3s-bench/releases/download/v{version}/a3s-bench-{version}-{target}.tar.gz"),
            "archive_sha256": archive_sha256,
            "payload_sha256": payload_sha256,
            "entrypoint": format!("bin/{ENTRYPOINT_NAME}"),
            "cli_protocol": CLI_PROTOCOL,
        });
        fs::write(
            dir.join("receipt.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
    }
}
