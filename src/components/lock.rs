use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use fs2::FileExt;

use super::id::ComponentId;

/// Cross-process guard for one top-level component family.
///
/// Delegated children share their parent's lock (for example every `use/*`
/// operation uses the `use` lock). This keeps parent installation and child
/// lifecycle calls in one deterministic order and avoids parent/child
/// deadlocks during cascade operations.
pub struct ComponentOperationLock {
    file: File,
}

impl ComponentOperationLock {
    pub async fn acquire(path: PathBuf, component: &ComponentId) -> anyhow::Result<Self> {
        let component = component.to_string();
        tokio::task::spawn_blocking(move || Self::acquire_blocking(&path, &component))
            .await
            .context("component lock task failed")?
    }

    pub fn acquire_sync(path: &Path, component: &ComponentId) -> anyhow::Result<Self> {
        Self::acquire_blocking(path, component.as_str())
    }

    fn acquire_blocking(path: &Path, component: &str) -> anyhow::Result<Self> {
        let parent = path
            .parent()
            .context("component operation lock has no parent directory")?;
        ensure_real_directory(parent)?;
        inspect_lock_path(path)?;

        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;

            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(path)
            .with_context(|| format!("failed to open component lock {}", path.display()))?;
        file.lock_exclusive()
            .with_context(|| format!("failed to acquire component lock {}", path.display()))?;
        validate_open_lock(&file, path)?;
        file.set_len(0)
            .with_context(|| format!("failed to truncate component lock {}", path.display()))?;
        writeln!(file, "pid={} component={component}", std::process::id())
            .with_context(|| format!("failed to write component lock {}", path.display()))?;
        Ok(Self { file })
    }
}

fn ensure_real_directory(path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path).with_context(|| {
        format!(
            "failed to create component lock directory {}",
            path.display()
        )
    })?;
    let metadata = std::fs::symlink_metadata(path).with_context(|| {
        format!(
            "failed to inspect component lock directory {}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "component lock directory '{}' must be a real directory",
            path.display()
        );
    }
    Ok(())
}

fn inspect_lock_path(path: &Path) -> anyhow::Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "component operation lock '{}' must not be a symbolic link",
                path.display()
            )
        }
        Ok(metadata) if !metadata.is_file() => {
            bail!(
                "component operation lock '{}' must be a regular file",
                path.display()
            )
        }
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to inspect component operation lock {}",
                path.display()
            )
        }),
    }
}

fn validate_open_lock(file: &File, path: &Path) -> anyhow::Result<()> {
    let path_metadata = inspect_lock_path(path)?
        .with_context(|| format!("component operation lock '{}' disappeared", path.display()))?;
    let file_metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect open component lock {}", path.display()))?;
    if !file_metadata.is_file() {
        bail!(
            "open component operation lock '{}' is not a regular file",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if path_metadata.dev() != file_metadata.dev() || path_metadata.ino() != file_metadata.ino()
        {
            bail!(
                "component operation lock '{}' changed while it was being acquired",
                path.display()
            );
        }
        if file_metadata.nlink() != 1 {
            bail!(
                "component operation lock '{}' must not have hard links",
                path.display()
            );
        }
    }
    #[cfg(not(unix))]
    let _ = path_metadata;
    Ok(())
}

impl Drop for ComponentOperationLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn concurrent_operations_wait_for_the_existing_lock() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("locks/use.lock");
        let first = ComponentOperationLock::acquire_blocking(&path, "use").unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let thread_path = path.clone();
        let thread = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let _second =
                ComponentOperationLock::acquire_blocking(&thread_path, "use/browser").unwrap();
            acquired_tx.send(()).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(acquired_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        drop(first);
        acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        thread.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_lock_is_rejected_without_modifying_its_target() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("do-not-truncate");
        let lock_path = temp.path().join("locks/use.lock");
        std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        std::fs::write(&target, b"preserve me").unwrap();
        symlink(&target, &lock_path).unwrap();

        let Err(error) = ComponentOperationLock::acquire_blocking(&lock_path, "use") else {
            panic!("symbolic-link lock should be rejected");
        };

        assert!(error.to_string().contains("symbolic link"), "{error:#}");
        assert_eq!(std::fs::read(&target).unwrap(), b"preserve me");
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_lock_directory_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("redirected-locks");
        let lock_directory = temp.path().join("locks");
        std::fs::create_dir(&target).unwrap();
        symlink(&target, &lock_directory).unwrap();

        let error =
            ComponentOperationLock::acquire_blocking(&lock_directory.join("use.lock"), "use")
                .err()
                .expect("symbolic-link lock directory should be rejected");

        assert!(error.to_string().contains("real directory"), "{error:#}");
        assert!(!target.join("use.lock").exists());
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_lock_is_rejected_without_modifying_its_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("do-not-truncate");
        let lock_path = temp.path().join("locks/use.lock");
        std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        std::fs::write(&target, b"preserve me").unwrap();
        std::fs::hard_link(&target, &lock_path).unwrap();

        let error = ComponentOperationLock::acquire_blocking(&lock_path, "use")
            .err()
            .expect("hard-link lock should be rejected");

        assert!(error.to_string().contains("hard links"), "{error:#}");
        assert_eq!(std::fs::read(&target).unwrap(), b"preserve me");
    }
}
