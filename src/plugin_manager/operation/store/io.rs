use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::super::super::{PluginManagerError, PluginManagerResult};
use super::{PluginOperationStore, MAX_OPERATION_DIRECTORY_ENTRIES, MAX_OPERATION_RECORD_BYTES};

pub(super) fn ensure_store_directories(store: &PluginOperationStore) -> PluginManagerResult<()> {
    ensure_real_directory(&store.root)?;
    ensure_real_directory(&store.plans_root())?;
    ensure_real_directory(&store.intents_root())?;
    ensure_real_directory(&store.lifecycles_root())?;
    ensure_real_directory(&store.results_root())
}

pub(in crate::plugin_manager) fn ensure_real_directory(path: &Path) -> PluginManagerResult<()> {
    std::fs::create_dir_all(path).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to create plugin operation directory {}: {error}",
            path.display()
        ))
    })?;
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to inspect plugin operation directory {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PluginManagerError::Infrastructure(format!(
            "plugin operation directory '{}' must be a real directory",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| {
                PluginManagerError::Infrastructure(format!(
                    "failed to secure plugin operation directory {}: {error}",
                    path.display()
                ))
            },
        )?;
    }
    Ok(())
}

pub(super) fn read_directory_records<T: DeserializeOwned>(
    directory: &Path,
) -> PluginManagerResult<Vec<(PathBuf, T)>> {
    ensure_real_directory(directory)?;
    let mut records = Vec::new();
    for (index, entry) in std::fs::read_dir(directory)
        .map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "failed to list plugin operation directory {}: {error}",
                directory.display()
            ))
        })?
        .enumerate()
    {
        if index >= MAX_OPERATION_DIRECTORY_ENTRIES {
            return Err(PluginManagerError::Infrastructure(format!(
                "plugin operation directory exceeds the {MAX_OPERATION_DIRECTORY_ENTRIES}-entry limit"
            )));
        }
        let entry = entry.map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "failed to inspect plugin operation directory entry: {error}"
            ))
        })?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "failed to inspect plugin operation record {}: {error}",
                path.display()
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(PluginManagerError::Infrastructure(format!(
                "plugin operation record '{}' must be a regular file",
                path.display()
            )));
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            return Err(PluginManagerError::Infrastructure(format!(
                "plugin operation directory contains unsupported file '{}'",
                path.display()
            )));
        }
        let record = read_required_record(&path)?;
        records.push((path, record));
    }
    Ok(records)
}

pub(super) fn read_required_record<T: DeserializeOwned>(path: &Path) -> PluginManagerResult<T> {
    read_optional_record(path)?.ok_or_else(|| {
        PluginManagerError::InvalidRequest(format!(
            "plugin operation record '{}' was not found",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown")
        ))
    })
}

pub(in crate::plugin_manager) fn read_optional_record<T: DeserializeOwned>(
    path: &Path,
) -> PluginManagerResult<Option<T>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(PluginManagerError::Infrastructure(format!(
                "failed to inspect plugin operation record {}: {error}",
                path.display()
            )));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PluginManagerError::Infrastructure(format!(
            "plugin operation record '{}' must be a regular file",
            path.display()
        )));
    }
    if metadata.len() == 0 || metadata.len() > MAX_OPERATION_RECORD_BYTES {
        return Err(PluginManagerError::Infrastructure(format!(
            "plugin operation record '{}' has an invalid size",
            path.display()
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|file| {
            file.take(MAX_OPERATION_RECORD_BYTES + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "failed to read plugin operation record {}: {error}",
                path.display()
            ))
        })?;
    if bytes.len() as u64 > MAX_OPERATION_RECORD_BYTES {
        return Err(PluginManagerError::Infrastructure(format!(
            "plugin operation record '{}' exceeds its size limit",
            path.display()
        )));
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "plugin operation record '{}' is invalid: {error}",
            path.display()
        ))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::plugin_manager) enum WriteDisposition {
    Created,
    AlreadyExists,
}

pub(in crate::plugin_manager) fn write_new_record<T: Serialize>(
    path: &Path,
    record: &T,
) -> PluginManagerResult<WriteDisposition> {
    let parent = path.parent().ok_or_else(|| {
        PluginManagerError::Infrastructure("plugin operation record path has no parent".to_string())
    })?;
    ensure_real_directory(parent)?;
    match std::fs::symlink_metadata(path) {
        Ok(_) => return Ok(WriteDisposition::AlreadyExists),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(PluginManagerError::Infrastructure(format!(
                "failed to inspect plugin operation record {}: {error}",
                path.display()
            )));
        }
    }
    let bytes = serde_json::to_vec(record).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to encode plugin operation record: {error}"
        ))
    })?;
    if bytes.len() as u64 > MAX_OPERATION_RECORD_BYTES {
        return Err(PluginManagerError::Infrastructure(
            "plugin operation record exceeds its size limit".to_string(),
        ));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to create temporary plugin operation record in {}: {error}",
            parent.display()
        ))
    })?;
    set_private_file(temporary.as_file())?;
    temporary.write_all(&bytes).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to write plugin operation record: {error}"
        ))
    })?;
    temporary.as_file().sync_all().map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to sync plugin operation record: {error}"
        ))
    })?;
    match temporary.persist_noclobber(path) {
        Ok(_) => {
            sync_parent(path)?;
            Ok(WriteDisposition::Created)
        }
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(WriteDisposition::AlreadyExists)
        }
        Err(error) => Err(PluginManagerError::Infrastructure(format!(
            "failed to publish plugin operation record {}: {}",
            path.display(),
            error.error
        ))),
    }
}

pub(in crate::plugin_manager) fn write_replace_record<T: Serialize>(
    path: &Path,
    record: &T,
) -> PluginManagerResult<()> {
    let parent = path.parent().ok_or_else(|| {
        PluginManagerError::Infrastructure("plugin operation record path has no parent".to_string())
    })?;
    ensure_real_directory(parent)?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(PluginManagerError::Infrastructure(format!(
                "plugin operation record '{}' must be a regular file",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(PluginManagerError::Infrastructure(format!(
                "failed to inspect plugin operation record {}: {error}",
                path.display()
            )));
        }
    }
    let bytes = serde_json::to_vec(record).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to encode plugin operation record: {error}"
        ))
    })?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_OPERATION_RECORD_BYTES {
        return Err(PluginManagerError::Infrastructure(
            "plugin operation record exceeds its size limit".to_string(),
        ));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to create temporary plugin operation record in {}: {error}",
            parent.display()
        ))
    })?;
    set_private_file(temporary.as_file())?;
    temporary.write_all(&bytes).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to write plugin operation record: {error}"
        ))
    })?;
    temporary.as_file().sync_all().map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to sync plugin operation record: {error}"
        ))
    })?;
    temporary.persist(path).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to publish plugin operation record {}: {}",
            path.display(),
            error.error
        ))
    })?;
    sync_parent(path)
}

fn set_private_file(file: &File) -> PluginManagerResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                PluginManagerError::Infrastructure(format!(
                    "failed to secure plugin operation record: {error}"
                ))
            })?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

fn sync_parent(path: &Path) -> PluginManagerResult<()> {
    #[cfg(unix)]
    {
        let parent = path.parent().ok_or_else(|| {
            PluginManagerError::Infrastructure(
                "plugin operation record has no parent directory".to_string(),
            )
        })?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                PluginManagerError::Infrastructure(format!(
                    "failed to sync plugin operation directory {}: {error}",
                    parent.display()
                ))
            })?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(super) fn remove_file_if_present(path: &Path) -> PluginManagerResult<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(PluginManagerError::Infrastructure(format!(
            "failed to prune plugin operation record {}: {error}",
            path.display()
        ))),
    }
}
