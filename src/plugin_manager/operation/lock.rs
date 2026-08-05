use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use super::super::{PluginManagerError, PluginManagerResult};

/// Cross-process serialization boundary for reviewed plugin mutations.
///
/// The umbrella component lifecycle has its own side-effect locks. This lock
/// covers the manager's surrounding plan/intent/result transaction so another
/// adapter cannot start the same reviewed apply while its durable result is
/// still being published.
pub(in crate::plugin_manager) struct PluginMutationLock {
    file: File,
}

impl PluginMutationLock {
    pub(in crate::plugin_manager) async fn acquire(path: PathBuf) -> PluginManagerResult<Self> {
        tokio::task::spawn_blocking(move || Self::acquire_sync(&path))
            .await
            .map_err(|error| {
                PluginManagerError::Infrastructure(format!(
                    "plugin mutation lock task failed: {error}"
                ))
            })?
    }

    fn acquire_sync(path: &Path) -> PluginManagerResult<Self> {
        let parent = path.parent().ok_or_else(|| {
            PluginManagerError::Infrastructure(
                "plugin mutation lock path has no parent".to_string(),
            )
        })?;
        ensure_real_directory(parent)?;
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(PluginManagerError::Infrastructure(format!(
                    "plugin mutation lock '{}' must be a regular file",
                    path.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(PluginManagerError::Infrastructure(format!(
                    "failed to inspect plugin mutation lock {}: {error}",
                    path.display()
                )));
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| {
                PluginManagerError::Infrastructure(format!(
                    "failed to open plugin mutation lock {}: {error}",
                    path.display()
                ))
            })?;
        let metadata = std::fs::symlink_metadata(path).map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "failed to inspect plugin mutation lock {}: {error}",
                path.display()
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(PluginManagerError::Infrastructure(format!(
                "plugin mutation lock '{}' must be a regular file",
                path.display()
            )));
        }
        secure_file(&file)?;
        file.lock_exclusive().map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "failed to acquire plugin mutation lock {}: {error}",
                path.display()
            ))
        })?;
        Ok(Self { file })
    }
}

impl Drop for PluginMutationLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn ensure_real_directory(path: &Path) -> PluginManagerResult<()> {
    std::fs::create_dir_all(path).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to create plugin mutation lock directory {}: {error}",
            path.display()
        ))
    })?;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to inspect plugin mutation lock directory {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PluginManagerError::Infrastructure(format!(
            "plugin mutation lock directory '{}' must be a real directory",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| {
                PluginManagerError::Infrastructure(format!(
                    "failed to secure plugin mutation lock directory {}: {error}",
                    path.display()
                ))
            },
        )?;
    }
    Ok(())
}

fn secure_file(file: &File) -> PluginManagerResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                PluginManagerError::Infrastructure(format!(
                    "failed to secure plugin mutation lock: {error}"
                ))
            })?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn concurrent_manager_mutations_share_one_cross_process_lock() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("plugin-manager/mutation.lock");
        let first = PluginMutationLock::acquire_sync(&path).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let thread_path = path.clone();
        let thread = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let _second = PluginMutationLock::acquire_sync(&thread_path).unwrap();
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
}
