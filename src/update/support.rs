use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use a3s::components::MANAGED_SRT_PAYLOAD_RELATIVE_ROOT;

const MAX_ARCHIVE_ENTRIES: usize = 20_000;
const MAX_SUPPORT_ENTRIES: usize = 2_000;
const MAX_SUPPORT_BYTES: u64 = 32 * 1024 * 1024;

pub(super) fn prepare_support_activation<F>(
    archive_root: &Path,
    executable: &Path,
    validate: F,
) -> Result<SupportActivation, String>
where
    F: Fn(&Path) -> Result<(), String>,
{
    let source = find_release_support(archive_root)?;
    validate(&source)?;

    let binary_directory = executable.parent().ok_or_else(|| {
        format!(
            "cannot derive support directory for {}",
            executable.display()
        )
    })?;
    let target = binary_directory.join(MANAGED_SRT_PAYLOAD_RELATIVE_ROOT);
    let target_parent = target
        .parent()
        .ok_or_else(|| format!("cannot derive support parent for {}", target.display()))?;
    std::fs::create_dir_all(target_parent).map_err(|error| {
        format!(
            "create support directory {}: {error}",
            target_parent.display()
        )
    })?;

    let staging = tempfile::Builder::new()
        .prefix(".a3s-managed-srt-update-")
        .tempdir_in(target_parent)
        .map_err(|error| {
            format!(
                "create support staging directory in {}: {error}",
                target_parent.display()
            )
        })?;
    let staged_root = staging.path().join("managed-srt");
    copy_support_tree(&source, &staged_root)?;
    validate(&staged_root)?;

    SupportActivation::activate(&staged_root, target)
}

fn find_release_support(root: &Path) -> Result<PathBuf, String> {
    let root_metadata = std::fs::symlink_metadata(root)
        .map_err(|error| format!("inspect extracted release {}: {error}", root.display()))?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(format!(
            "extracted release root is not a directory: {}",
            root.display()
        ));
    }

    let mut stack = vec![root.to_path_buf()];
    let mut candidates = Vec::new();
    let mut entries_seen = 0_usize;
    while let Some(directory) = stack.pop() {
        let mut entries = std::fs::read_dir(&directory)
            .map_err(|error| format!("read extracted release {}: {error}", directory.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                format!(
                    "read an entry from extracted release {}: {error}",
                    directory.display()
                )
            })?;
        entries.sort_by_key(std::fs::DirEntry::file_name);

        for entry in entries {
            entries_seen = entries_seen
                .checked_add(1)
                .ok_or_else(|| "extracted release entry count overflowed".to_string())?;
            if entries_seen > MAX_ARCHIVE_ENTRIES {
                return Err(format!(
                    "release archive exceeds the {MAX_ARCHIVE_ENTRIES} entry discovery limit"
                ));
            }

            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("inspect extracted entry {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() {
                if is_support_root_name(&path) {
                    return Err(format!(
                        "release support root must not be a symbolic link: {}",
                        path.display()
                    ));
                }
                continue;
            }
            if !metadata.is_dir() {
                continue;
            }
            if is_support_root_name(&path) {
                candidates.push(path.clone());
            }
            stack.push(path);
        }
    }

    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        count => Err(format!(
            "release archive must contain exactly one support/managed-srt directory; found {count}"
        )),
    }
}

fn is_support_root_name(path: &Path) -> bool {
    path.file_name() == Some(OsStr::new("managed-srt"))
        && path.parent().and_then(Path::file_name) == Some(OsStr::new("support"))
}

#[derive(Default)]
struct CopyBudget {
    entries: usize,
    bytes: u64,
}

fn copy_support_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(source)
        .map_err(|error| format!("inspect release support {}: {error}", source.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "release support root is not a directory: {}",
            source.display()
        ));
    }
    std::fs::create_dir(destination).map_err(|error| {
        format!(
            "create support staging root {}: {error}",
            destination.display()
        )
    })?;
    let mut budget = CopyBudget::default();
    copy_directory(source, destination, &mut budget)?;
    copy_permissions(&metadata, destination)
        .map_err(|error| format!("copy permissions to {}: {error}", destination.display()))
}

fn copy_directory(
    source: &Path,
    destination: &Path,
    budget: &mut CopyBudget,
) -> Result<(), String> {
    let mut entries = std::fs::read_dir(source)
        .map_err(|error| format!("read release support {}: {error}", source.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read an entry from {}: {error}", source.display()))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        budget.entries = budget
            .entries
            .checked_add(1)
            .ok_or_else(|| "release support entry count overflowed".to_string())?;
        if budget.entries > MAX_SUPPORT_ENTRIES {
            return Err(format!(
                "release support exceeds the {MAX_SUPPORT_ENTRIES} entry limit"
            ));
        }

        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&source_path).map_err(|error| {
            format!("inspect release support {}: {error}", source_path.display())
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "release support contains a symbolic link: {}",
                source_path.display()
            ));
        }
        if metadata.is_dir() {
            std::fs::create_dir(&destination_path).map_err(|error| {
                format!(
                    "create support staging directory {}: {error}",
                    destination_path.display()
                )
            })?;
            copy_directory(&source_path, &destination_path, budget)?;
            copy_permissions(&metadata, &destination_path).map_err(|error| {
                format!(
                    "copy permissions to support staging directory {}: {error}",
                    destination_path.display()
                )
            })?;
        } else if metadata.is_file() {
            copy_regular_file(&source_path, &destination_path, budget)?;
        } else {
            return Err(format!(
                "release support contains an unsupported file type: {}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn copy_regular_file(
    source: &Path,
    destination: &Path,
    budget: &mut CopyBudget,
) -> Result<(), String> {
    let mut source_options = std::fs::OpenOptions::new();
    source_options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        source_options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut input = source_options
        .open(source)
        .map_err(|error| format!("open release support file {}: {error}", source.display()))?;
    let metadata = input
        .metadata()
        .map_err(|error| format!("inspect open support file {}: {error}", source.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "release support entry changed type while copying: {}",
            source.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return Err(format!(
                "release support contains a hard-linked file: {}",
                source.display()
            ));
        }
    }

    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| {
            format!(
                "create support staging file {}: {error}",
                destination.display()
            )
        })?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("read release support file {}: {error}", source.display()))?;
        if read == 0 {
            break;
        }
        budget.bytes = budget
            .bytes
            .checked_add(read as u64)
            .ok_or_else(|| "release support byte count overflowed".to_string())?;
        if budget.bytes > MAX_SUPPORT_BYTES {
            return Err(format!(
                "release support exceeds the {MAX_SUPPORT_BYTES} byte limit"
            ));
        }
        output.write_all(&buffer[..read]).map_err(|error| {
            format!(
                "write support staging file {}: {error}",
                destination.display()
            )
        })?;
    }
    output.flush().map_err(|error| {
        format!(
            "flush support staging file {}: {error}",
            destination.display()
        )
    })?;
    copy_permissions(&metadata, destination)
        .map_err(|error| format!("copy permissions to {}: {error}", destination.display()))
}

#[cfg(unix)]
fn copy_permissions(metadata: &std::fs::Metadata, destination: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let mode = metadata.mode() & 0o777;
    std::fs::set_permissions(destination, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn copy_permissions(metadata: &std::fs::Metadata, destination: &Path) -> std::io::Result<()> {
    std::fs::set_permissions(destination, metadata.permissions())
}

pub(super) struct SupportActivation {
    target: PathBuf,
    backup: Option<PathBuf>,
    active: bool,
}

impl SupportActivation {
    fn activate(staged: &Path, target: PathBuf) -> Result<Self, String> {
        let backup = match std::fs::symlink_metadata(&target) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(format!(
                        "existing managed SRT support is not a directory: {}",
                        target.display()
                    ));
                }
                let backup = unique_backup_path(&target)?;
                std::fs::rename(&target, &backup).map_err(|error| {
                    format!(
                        "move existing support {} to {}: {error}",
                        target.display(),
                        backup.display()
                    )
                })?;
                Some(backup)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "inspect existing support {}: {error}",
                    target.display()
                ));
            }
        };

        if let Err(error) = std::fs::rename(staged, &target) {
            let restore_error = backup.as_ref().and_then(|backup| {
                std::fs::rename(backup, &target)
                    .err()
                    .map(|restore| restore.to_string())
            });
            return Err(match restore_error {
                Some(restore) => format!(
                    "activate managed SRT support at {}: {error}; restore failed: {restore}",
                    target.display()
                ),
                None => format!(
                    "activate managed SRT support at {}: {error}",
                    target.display()
                ),
            });
        }

        Ok(Self {
            target,
            backup,
            active: true,
        })
    }

    pub(super) fn commit(mut self) -> Result<(), String> {
        self.active = false;
        if let Some(backup) = self.backup.take() {
            remove_path(&backup)
                .map_err(|error| format!("remove old support {}: {error}", backup.display()))?;
        }
        Ok(())
    }

    pub(super) fn rollback(&mut self) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        remove_path(&self.target)
            .map_err(|error| format!("remove new support {}: {error}", self.target.display()))?;
        if let Some(backup) = self.backup.take() {
            std::fs::rename(&backup, &self.target).map_err(|error| {
                format!(
                    "restore support {} from {}: {error}",
                    self.target.display(),
                    backup.display()
                )
            })?;
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for SupportActivation {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

fn unique_backup_path(target: &Path) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("cannot derive backup directory for {}", target.display()))?;
    let name = target
        .file_name()
        .ok_or_else(|| format!("cannot derive backup name for {}", target.display()))?
        .to_string_lossy();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let backup = parent.join(format!(
        ".{name}.a3s-update-{}-{nonce}.bak",
        std::process::id()
    ));
    if backup.exists() {
        return Err(format!(
            "support backup path already exists: {}",
            backup.display()
        ));
    }
    Ok(backup)
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            std::fs::remove_dir_all(path)
        }
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker_validator(root: &Path) -> Result<(), String> {
        let marker = root.join("marker");
        marker
            .is_file()
            .then_some(())
            .ok_or_else(|| format!("support fixture has no marker: {}", marker.display()))
    }

    fn write_fixture(root: &Path, marker: &str) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join("marker"), marker).unwrap();
    }

    #[test]
    fn release_support_discovery_requires_exactly_one_payload() {
        let temp = tempfile::tempdir().unwrap();
        let error = find_release_support(temp.path()).unwrap_err();
        assert!(error.contains("found 0"), "{error}");

        write_fixture(&temp.path().join("one/support/managed-srt"), "one");
        assert_eq!(
            find_release_support(temp.path()).unwrap(),
            temp.path().join("one/support/managed-srt")
        );

        write_fixture(&temp.path().join("two/support/managed-srt"), "two");
        let error = find_release_support(temp.path()).unwrap_err();
        assert!(error.contains("found 2"), "{error}");
    }

    #[test]
    fn activation_rolls_back_the_previous_support_tree() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        write_fixture(&archive.join("support/managed-srt"), "new");
        let executable = temp.path().join("install/bin/a3s");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        let target = executable
            .parent()
            .unwrap()
            .join(MANAGED_SRT_PAYLOAD_RELATIVE_ROOT);
        write_fixture(&target, "old");

        let mut activation =
            prepare_support_activation(&archive, &executable, marker_validator).unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("marker")).unwrap(),
            "new"
        );
        activation.rollback().unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("marker")).unwrap(),
            "old"
        );
    }

    #[test]
    fn committed_activation_removes_its_backup() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        write_fixture(&archive.join("support/managed-srt"), "new");
        let executable = temp.path().join("install/bin/a3s");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        let target = executable
            .parent()
            .unwrap()
            .join(MANAGED_SRT_PAYLOAD_RELATIVE_ROOT);
        write_fixture(&target, "old");

        let activation =
            prepare_support_activation(&archive, &executable, marker_validator).unwrap();
        activation.commit().unwrap();

        assert_eq!(
            std::fs::read_to_string(target.join("marker")).unwrap(),
            "new"
        );
        let support_parent = target.parent().unwrap();
        assert!(std::fs::read_dir(support_parent)
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".bak")));
    }

    #[cfg(unix)]
    #[test]
    fn support_copy_rejects_symbolic_and_hard_links() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        write_fixture(&source, "fixture");
        symlink("marker", source.join("marker-link")).unwrap();
        let error = copy_support_tree(&source, &destination).unwrap_err();
        assert!(error.contains("symbolic link"), "{error}");

        std::fs::remove_file(source.join("marker-link")).unwrap();
        std::fs::hard_link(source.join("marker"), source.join("marker-hardlink")).unwrap();
        let destination = temp.path().join("destination-hardlink");
        let error = copy_support_tree(&source, &destination).unwrap_err();
        assert!(error.contains("hard-linked"), "{error}");
    }
}
