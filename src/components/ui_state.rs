//! A3S Code-owned durable state for cognitive-package UI surfaces.

use std::collections::BTreeMap;
#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use a3s_use_core::{metadata_is_link_or_reparse_point, PlanScope, PluginPackageId};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::ComponentPaths;

pub(crate) const UI_STATE_MAX_ENTRIES: usize = 64;
pub(crate) const UI_STATE_MAX_KEY_BYTES: usize = 128;
pub(crate) const UI_STATE_MAX_VALUE_BYTES: usize = 16 * 1024;
pub(crate) const UI_STATE_MAX_SURFACE_BYTES: usize = 256 * 1024;
const UI_STATE_MAX_FILE_BYTES: u64 = UI_STATE_MAX_SURFACE_BYTES as u64 + 64 * 1024;
const UI_STATE_SCHEMA: &str = "a3s.code.plugin-ui-state.v1";

#[derive(Debug, thiserror::Error)]
pub enum CodePluginUiStateError {
    #[error("plugin UI state identity is invalid")]
    InvalidIdentity,
    #[error("plugin UI state key is invalid")]
    InvalidKey,
    #[error("plugin UI state value exceeds the per-entry limit")]
    ValueTooLarge,
    #[error("plugin UI state exceeds the per-surface capacity")]
    CapacityExceeded,
    #[error("plugin UI state is corrupt or does not match its storage identity")]
    Corrupt,
    #[error("plugin UI state storage path is unsafe")]
    UnsafePath,
    #[error("plugin UI state I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("plugin UI state worker failed: {0}")]
    Worker(String),
}

pub type CodePluginUiStateResult<T> = Result<T, CodePluginUiStateError>;

#[derive(Debug, Clone)]
pub struct CodePluginUiStateStore {
    root: PathBuf,
}

impl CodePluginUiStateStore {
    pub(crate) fn from_component_paths(paths: &ComponentPaths) -> Self {
        Self::new(paths.state_root.join("use").join("ui-state"))
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub async fn get(
        &self,
        scope: &PlanScope,
        package_id: &str,
        surface_id: &str,
        key: &str,
    ) -> CodePluginUiStateResult<Option<Value>> {
        let identity = UiStateIdentity::new(scope.clone(), package_id, surface_id)?;
        validate_key(key)?;
        let root = self.root.clone();
        let key = key.to_string();
        run_blocking(move || {
            with_store_lock(&root, |canonical_root| {
                let snapshot = read_snapshot(canonical_root, &identity)?;
                Ok(snapshot.and_then(|snapshot| snapshot.entries.get(&key).cloned()))
            })
        })
        .await
    }

    pub async fn set(
        &self,
        scope: &PlanScope,
        package_id: &str,
        surface_id: &str,
        key: &str,
        value: Value,
    ) -> CodePluginUiStateResult<()> {
        let identity = UiStateIdentity::new(scope.clone(), package_id, surface_id)?;
        validate_key(key)?;
        validate_value(&value)?;
        let root = self.root.clone();
        let key = key.to_string();
        run_blocking(move || {
            with_store_lock(&root, |canonical_root| {
                let mut snapshot = read_snapshot(canonical_root, &identity)?
                    .unwrap_or_else(|| UiStateSnapshot::empty(&identity));
                if !snapshot.entries.contains_key(&key)
                    && snapshot.entries.len() >= UI_STATE_MAX_ENTRIES
                {
                    return Err(CodePluginUiStateError::CapacityExceeded);
                }
                snapshot.entries.insert(key, value);
                snapshot.validate(&identity)?;
                write_snapshot(canonical_root, &identity, &snapshot)
            })
        })
        .await
    }

    pub async fn delete(
        &self,
        scope: &PlanScope,
        package_id: &str,
        surface_id: &str,
        key: &str,
    ) -> CodePluginUiStateResult<bool> {
        let identity = UiStateIdentity::new(scope.clone(), package_id, surface_id)?;
        validate_key(key)?;
        let root = self.root.clone();
        let key = key.to_string();
        run_blocking(move || {
            with_store_lock(&root, |canonical_root| {
                let Some(mut snapshot) = read_snapshot(canonical_root, &identity)? else {
                    return Ok(false);
                };
                if snapshot.entries.remove(&key).is_none() {
                    return Ok(false);
                }
                if snapshot.entries.is_empty() {
                    remove_snapshot(canonical_root, &identity)?;
                } else {
                    snapshot.validate(&identity)?;
                    write_snapshot(canonical_root, &identity, &snapshot)?;
                }
                Ok(true)
            })
        })
        .await
    }

    /// Remove one complete package/scope/surface state namespace.
    ///
    /// Cleanup deliberately does not parse the snapshot. A corrupt file must
    /// fail closed for runtime reads and writes, but uninstall still needs an
    /// idempotent way to remove every byte owned by the retired surface.
    pub async fn clear_surface(
        &self,
        scope: &PlanScope,
        package_id: &str,
        surface_id: &str,
    ) -> CodePluginUiStateResult<bool> {
        let identity = UiStateIdentity::new(scope.clone(), package_id, surface_id)?;
        let root = self.root.clone();
        run_blocking(move || {
            with_store_lock(&root, |canonical_root| {
                remove_snapshot(canonical_root, &identity)
            })
        })
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UiStateIdentity {
    scope: PlanScope,
    package_id: String,
    surface_id: String,
}

impl UiStateIdentity {
    fn new(scope: PlanScope, package_id: &str, surface_id: &str) -> CodePluginUiStateResult<Self> {
        if !valid_machine_id(&scope.id)
            || PluginPackageId::parse(package_id.to_string()).is_err()
            || !valid_surface_id(surface_id)
        {
            return Err(CodePluginUiStateError::InvalidIdentity);
        }
        Ok(Self {
            scope,
            package_id: package_id.to_string(),
            surface_id: surface_id.to_string(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UiStateSnapshot {
    schema: String,
    scope: PlanScope,
    package_id: String,
    surface_id: String,
    entries: BTreeMap<String, Value>,
}

impl UiStateSnapshot {
    fn empty(identity: &UiStateIdentity) -> Self {
        Self {
            schema: UI_STATE_SCHEMA.to_string(),
            scope: identity.scope.clone(),
            package_id: identity.package_id.clone(),
            surface_id: identity.surface_id.clone(),
            entries: BTreeMap::new(),
        }
    }

    fn validate(&self, identity: &UiStateIdentity) -> CodePluginUiStateResult<()> {
        if self.schema != UI_STATE_SCHEMA
            || self.scope != identity.scope
            || self.package_id != identity.package_id
            || self.surface_id != identity.surface_id
            || self.entries.len() > UI_STATE_MAX_ENTRIES
        {
            return Err(CodePluginUiStateError::Corrupt);
        }
        let mut total = 0_usize;
        for (key, value) in &self.entries {
            validate_key(key).map_err(|_| CodePluginUiStateError::Corrupt)?;
            let value_bytes =
                serialized_value_size(value).map_err(|_| CodePluginUiStateError::Corrupt)?;
            if value_bytes > UI_STATE_MAX_VALUE_BYTES {
                return Err(CodePluginUiStateError::Corrupt);
            }
            total = total
                .checked_add(key.len())
                .and_then(|size| size.checked_add(value_bytes))
                .ok_or(CodePluginUiStateError::Corrupt)?;
        }
        if total > UI_STATE_MAX_SURFACE_BYTES {
            return Err(CodePluginUiStateError::CapacityExceeded);
        }
        Ok(())
    }
}

async fn run_blocking<T, F>(operation: F) -> CodePluginUiStateResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CodePluginUiStateResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| CodePluginUiStateError::Worker(error.to_string()))?
}

fn with_store_lock<T>(
    root: &Path,
    operation: impl FnOnce(&Path) -> CodePluginUiStateResult<T>,
) -> CodePluginUiStateResult<T> {
    let canonical_root = ensure_root(root)?;
    let lock_path = canonical_root.join(".lock");
    reject_symlink_or_non_file_if_present(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    secure_file(&lock)?;
    lock.lock_exclusive()?;
    let result = operation(&canonical_root);
    let unlock = FileExt::unlock(&lock);
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(CodePluginUiStateError::Io(error)),
    }
}

fn ensure_root(root: &Path) -> CodePluginUiStateResult<PathBuf> {
    let mut current = root.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if !metadata_is_link_or_reparse_point(&metadata) && metadata.is_dir() => {
                break;
            }
            Ok(_) => return Err(CodePluginUiStateError::UnsafePath),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = current
                    .file_name()
                    .ok_or(CodePluginUiStateError::UnsafePath)?
                    .to_os_string();
                missing.push(name);
                current = current
                    .parent()
                    .ok_or(CodePluginUiStateError::UnsafePath)?
                    .to_path_buf();
            }
            Err(error) => return Err(CodePluginUiStateError::Io(error)),
        }
    }
    let canonical_anchor = std::fs::canonicalize(&current)?;
    for name in missing.into_iter().rev() {
        current.push(name);
        ensure_regular_directory(&current)?;
    }
    let canonical_root = std::fs::canonicalize(root)?;
    if !canonical_root.starts_with(&canonical_anchor) {
        return Err(CodePluginUiStateError::UnsafePath);
    }
    Ok(canonical_root)
}

fn read_snapshot(
    root: &Path,
    identity: &UiStateIdentity,
) -> CodePluginUiStateResult<Option<UiStateSnapshot>> {
    let Some(path) = existing_snapshot_path(root, identity)? else {
        return Ok(None);
    };
    reject_symlink_or_non_file(&path)?;
    let metadata = std::fs::metadata(&path)?;
    if metadata.len() > UI_STATE_MAX_FILE_BYTES {
        return Err(CodePluginUiStateError::Corrupt);
    }
    let bytes = std::fs::read(path)?;
    let snapshot: UiStateSnapshot =
        serde_json::from_slice(&bytes).map_err(|_| CodePluginUiStateError::Corrupt)?;
    snapshot.validate(identity).map_err(|error| match error {
        CodePluginUiStateError::Io(error) => CodePluginUiStateError::Io(error),
        CodePluginUiStateError::UnsafePath => CodePluginUiStateError::UnsafePath,
        CodePluginUiStateError::Worker(message) => CodePluginUiStateError::Worker(message),
        _ => CodePluginUiStateError::Corrupt,
    })?;
    Ok(Some(snapshot))
}

fn write_snapshot(
    root: &Path,
    identity: &UiStateIdentity,
    snapshot: &UiStateSnapshot,
) -> CodePluginUiStateResult<()> {
    snapshot.validate(identity)?;
    let path = writable_snapshot_path(root, identity)?;
    reject_symlink_or_non_file_if_present(&path)?;
    let bytes = serde_json::to_vec(snapshot).map_err(|_| CodePluginUiStateError::Corrupt)?;
    if bytes.len() as u64 > UI_STATE_MAX_FILE_BYTES {
        return Err(CodePluginUiStateError::CapacityExceeded);
    }
    let parent = path.parent().ok_or(CodePluginUiStateError::UnsafePath)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    secure_file(temporary.as_file())?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&path)
        .map_err(|error| CodePluginUiStateError::Io(error.error))?;
    sync_directory(parent)?;
    Ok(())
}

fn remove_snapshot(root: &Path, identity: &UiStateIdentity) -> CodePluginUiStateResult<bool> {
    let Some(path) = existing_snapshot_path(root, identity)? else {
        return Ok(false);
    };
    reject_symlink_or_non_file(&path)?;
    std::fs::remove_file(&path)?;
    let package_dir = path.parent().ok_or(CodePluginUiStateError::UnsafePath)?;
    sync_directory(package_dir)?;
    prune_empty_state_directories(root, package_dir);
    Ok(true)
}

fn existing_snapshot_path(
    root: &Path,
    identity: &UiStateIdentity,
) -> CodePluginUiStateResult<Option<PathBuf>> {
    let segments = state_directory_segments(identity);
    let mut directory = root.to_path_buf();
    for segment in segments {
        directory.push(segment);
        match std::fs::symlink_metadata(&directory) {
            Ok(metadata) if !metadata_is_link_or_reparse_point(&metadata) && metadata.is_dir() => {}
            Ok(_) => return Err(CodePluginUiStateError::UnsafePath),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(CodePluginUiStateError::Io(error)),
        }
    }
    let path = directory.join(snapshot_file_name(identity));
    match std::fs::symlink_metadata(&path) {
        Ok(_) => Ok(Some(path)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CodePluginUiStateError::Io(error)),
    }
}

fn writable_snapshot_path(
    root: &Path,
    identity: &UiStateIdentity,
) -> CodePluginUiStateResult<PathBuf> {
    let mut directory = root.to_path_buf();
    for segment in state_directory_segments(identity) {
        directory.push(segment);
        ensure_regular_directory(&directory)?;
    }
    Ok(directory.join(snapshot_file_name(identity)))
}

fn state_directory_segments(identity: &UiStateIdentity) -> [String; 3] {
    [
        "v1".to_string(),
        digest(&format!(
            "{}\n{}",
            identity.scope.kind.as_str(),
            identity.scope.id
        )),
        digest(&identity.package_id),
    ]
}

fn snapshot_file_name(identity: &UiStateIdentity) -> String {
    format!("{}.json", digest(&identity.surface_id))
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn ensure_regular_directory(path: &Path) -> CodePluginUiStateResult<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata_is_link_or_reparse_point(&metadata) && metadata.is_dir() => {
            Ok(())
        }
        Ok(_) => Err(CodePluginUiStateError::UnsafePath),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(path)?;
            let metadata = std::fs::symlink_metadata(path)?;
            if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
                return Err(CodePluginUiStateError::UnsafePath);
            }
            secure_directory(path)?;
            Ok(())
        }
        Err(error) => Err(CodePluginUiStateError::Io(error)),
    }
}

fn reject_symlink_or_non_file(path: &Path) -> CodePluginUiStateResult<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(CodePluginUiStateError::UnsafePath);
    }
    Ok(())
}

fn reject_symlink_or_non_file_if_present(path: &Path) -> CodePluginUiStateResult<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata_is_link_or_reparse_point(&metadata) && metadata.is_file() => {
            Ok(())
        }
        Ok(_) => Err(CodePluginUiStateError::UnsafePath),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CodePluginUiStateError::Io(error)),
    }
}

fn prune_empty_state_directories(root: &Path, package_dir: &Path) {
    let _ = std::fs::remove_dir(package_dir);
    if let Some(scope_dir) = package_dir.parent() {
        let _ = std::fs::remove_dir(scope_dir);
        if let Some(version_dir) = scope_dir.parent() {
            if version_dir != root {
                let _ = std::fs::remove_dir(version_dir);
            }
        }
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> CodePluginUiStateResult<()> {
    let directory = File::open(path)?;
    directory.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> CodePluginUiStateResult<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_file(file: &File) -> CodePluginUiStateResult<()> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn secure_file(_file: &std::fs::File) -> CodePluginUiStateResult<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_directory(path: &Path) -> CodePluginUiStateResult<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn secure_directory(_path: &Path) -> CodePluginUiStateResult<()> {
    Ok(())
}

fn validate_key(key: &str) -> CodePluginUiStateResult<()> {
    if key.is_empty()
        || key.len() > UI_STATE_MAX_KEY_BYTES
        || !key
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !key.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b':' | b'/')
        })
    {
        return Err(CodePluginUiStateError::InvalidKey);
    }
    Ok(())
}

fn validate_value(value: &Value) -> CodePluginUiStateResult<()> {
    if serialized_value_size(value)? > UI_STATE_MAX_VALUE_BYTES {
        return Err(CodePluginUiStateError::ValueTooLarge);
    }
    Ok(())
}

fn serialized_value_size(value: &Value) -> CodePluginUiStateResult<usize> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|_| CodePluginUiStateError::Corrupt)
}

fn valid_machine_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b':' | b'/' | b'@')
        })
}

fn valid_surface_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use a3s_use_core::PlanScopeKind;

    use super::*;

    fn scope(kind: PlanScopeKind, id: &str) -> PlanScope {
        PlanScope {
            kind,
            id: id.to_string(),
        }
    }

    #[tokio::test]
    async fn state_survives_store_recreation_and_isolates_every_identity_axis() {
        let temp = tempfile::tempdir().unwrap();
        let user = scope(PlanScopeKind::User, "user/current");
        let store = CodePluginUiStateStore::new(temp.path());
        store
            .set(
                &user,
                "acme/research",
                "review",
                "filters.active",
                serde_json::json!({"year": 2026}),
            )
            .await
            .unwrap();

        let restarted = CodePluginUiStateStore::new(temp.path());
        assert_eq!(
            restarted
                .get(&user, "acme/research", "review", "filters.active")
                .await
                .unwrap(),
            Some(serde_json::json!({"year": 2026}))
        );
        assert_eq!(
            restarted
                .get(
                    &scope(PlanScopeKind::Workspace, "workspace/research"),
                    "acme/research",
                    "review",
                    "filters.active"
                )
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            restarted
                .get(&user, "acme/other", "review", "filters.active")
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            restarted
                .get(&user, "acme/research", "status", "filters.active")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn state_rejects_invalid_keys_values_capacity_and_corrupt_snapshots() {
        let temp = tempfile::tempdir().unwrap();
        let scope = scope(PlanScopeKind::User, "user/current");
        let store = CodePluginUiStateStore::new(temp.path());
        assert!(matches!(
            store
                .set(&scope, "acme/research", "review", "../escape", Value::Null)
                .await,
            Err(CodePluginUiStateError::InvalidKey)
        ));
        assert!(matches!(
            store
                .set(
                    &scope,
                    "acme/research",
                    "review",
                    "oversized",
                    Value::String("x".repeat(UI_STATE_MAX_VALUE_BYTES))
                )
                .await,
            Err(CodePluginUiStateError::ValueTooLarge)
        ));
        for index in 0..UI_STATE_MAX_ENTRIES {
            store
                .set(
                    &scope,
                    "acme/research",
                    "review",
                    &format!("entry.{index}"),
                    serde_json::json!(index),
                )
                .await
                .unwrap();
        }
        assert!(matches!(
            store
                .set(
                    &scope,
                    "acme/research",
                    "review",
                    "entry.overflow",
                    Value::Null
                )
                .await,
            Err(CodePluginUiStateError::CapacityExceeded)
        ));

        let identity = UiStateIdentity::new(scope.clone(), "acme/research", "corrupt").unwrap();
        with_store_lock(temp.path(), |root| {
            let path = writable_snapshot_path(root, &identity)?;
            std::fs::write(path, b"{not-json")?;
            Ok(())
        })
        .unwrap();
        assert!(matches!(
            store.get(&scope, "acme/research", "corrupt", "state").await,
            Err(CodePluginUiStateError::Corrupt)
        ));
        assert!(store
            .clear_surface(&scope, "acme/research", "corrupt")
            .await
            .unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn state_rejects_symlinked_ancestor_and_snapshot_paths() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let scope = scope(PlanScopeKind::User, "user/current");
        let root = temp.path().join("state/use/ui-state");
        std::fs::create_dir_all(temp.path().join("state")).unwrap();
        symlink(outside.path(), temp.path().join("state/use")).unwrap();
        let store = CodePluginUiStateStore::new(&root);
        assert!(matches!(
            store
                .set(&scope, "acme/research", "review", "draft", Value::Null)
                .await,
            Err(CodePluginUiStateError::UnsafePath)
        ));
        assert!(!outside.path().join("ui-state").exists());

        std::fs::remove_file(temp.path().join("state/use")).unwrap();
        let store = CodePluginUiStateStore::new(&root);
        store
            .set(&scope, "acme/research", "review", "draft", Value::Null)
            .await
            .unwrap();
        let identity = UiStateIdentity::new(scope.clone(), "acme/research", "review").unwrap();
        let snapshot = with_store_lock(&root, |canonical_root| {
            existing_snapshot_path(canonical_root, &identity)?
                .ok_or(CodePluginUiStateError::Corrupt)
        })
        .unwrap();
        std::fs::remove_file(&snapshot).unwrap();
        let outside_file = outside.path().join("state.json");
        std::fs::write(&outside_file, b"{}").unwrap();
        symlink(&outside_file, &snapshot).unwrap();
        assert!(matches!(
            store.get(&scope, "acme/research", "review", "draft").await,
            Err(CodePluginUiStateError::UnsafePath)
        ));
    }
}
