use std::path::{Path, PathBuf};
use std::process::Command;

use super::support::{portable_release_target, FakeReleaseServer, TempWorkspace};

pub(super) struct RealUseRelease {
    pub(super) server: FakeReleaseServer,
    pub(super) version: String,
}

pub(super) fn start(workspace: &TempWorkspace) -> RealUseRelease {
    let binary = required_path("A3S_USE_E2E_BIN");
    let source_root = required_path("A3S_USE_E2E_SOURCE_ROOT");
    let browser_driver = binary
        .parent()
        .expect("real Use binary must have a parent directory")
        .join("a3s-use-browser-driver");
    assert!(
        browser_driver.is_file(),
        "real Browser driver is missing at {}",
        browser_driver.display()
    );
    let version = use_version(&binary);
    let target = portable_release_target().expect("test host must support a portable Use release");
    let package_name = format!("a3s-use-{version}-{target}");
    let release_root = workspace.path("real-release");
    let package_root = release_root.join(&package_name);
    std::fs::create_dir_all(&package_root).expect("create real Use package root");

    copy_executable(&binary, &package_root.join("a3s-use"));
    copy_executable(
        &browser_driver,
        &package_root.join("a3s-use-browser-driver"),
    );
    for (source, destination) in [
        ("crates/browser-driver/skills", "skills"),
        ("crates/browser-driver/skill-data", "skill-data"),
        ("crates/office/skills", "office-skills"),
        ("crates/ocr/skills", "ocr-skills"),
        ("crates/browser-driver/dashboard/out", "dashboard"),
    ] {
        copy_tree(&source_root.join(source), &package_root.join(destination));
    }
    for source in ["LICENSE", "README.md", "THIRD_PARTY_NOTICES.md"] {
        std::fs::copy(source_root.join(source), package_root.join(source))
            .unwrap_or_else(|error| panic!("failed to package {source}: {error}"));
    }
    for source in ["LICENSE-APACHE-2.0", "UPSTREAM.md"] {
        std::fs::copy(
            source_root.join("crates/browser-driver").join(source),
            package_root.join(source),
        )
        .unwrap_or_else(|error| panic!("failed to package {source}: {error}"));
    }

    let archive_name = format!("{package_name}.tar.gz");
    let archive_path = workspace.path(&archive_name);
    let status = Command::new("tar")
        .arg("czf")
        .arg(&archive_path)
        .arg("-C")
        .arg(&release_root)
        .arg(&package_name)
        .status()
        .expect("create real Use release archive");
    assert!(
        status.success(),
        "failed to create real Use release archive"
    );
    let archive = std::fs::read(archive_path).expect("read real Use release archive");
    RealUseRelease {
        server: FakeReleaseServer::start("Use", &version, &archive_name, archive),
        version,
    }
}

fn required_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must point to the real Use checkout artifact"))
}

fn use_version(binary: &Path) -> String {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", binary.display()));
    assert!(
        output.status.success(),
        "{} --version failed: {}",
        binary.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("Use version output must be UTF-8");
    stdout
        .split_whitespace()
        .last()
        .filter(|version| !version.is_empty())
        .map(str::to_string)
        .expect("Use version output must end with a version")
}

fn copy_executable(source: &Path, destination: &Path) {
    std::fs::copy(source, destination).unwrap_or_else(|error| {
        panic!(
            "failed to copy executable {} to {}: {error}",
            source.display(),
            destination.display()
        )
    });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("failed to chmod {}: {error}", destination.display()));
    }
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", destination.display()));
    for entry in std::fs::read_dir(source)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", source.display()))
    {
        let entry = entry.expect("read release source entry");
        let file_type = entry.file_type().expect("inspect release source entry");
        let target = destination.join(entry.file_name());
        assert!(
            !file_type.is_symlink(),
            "release source must not contain symlinks: {}",
            entry.path().display()
        );
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target);
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &target).unwrap_or_else(|error| {
                panic!(
                    "failed to copy release file {} to {}: {error}",
                    entry.path().display(),
                    target.display()
                )
            });
        } else {
            panic!(
                "release source contains an unsupported entry: {}",
                entry.path().display()
            );
        }
    }
}
