//! Lazy file-backed Memory initialization shared by TUI and `code exec`.

use std::path::PathBuf;

use a3s_memory::{MemoryItem, MemoryStore, PrunePolicy};
use crate::vec_memory_store::VecMemoryStore;
use anyhow::Context;
use tokio::sync::OnceCell;

/// Preserve the durable a3s-vec backend while deferring collection open until
/// the first real Memory operation. Session construction only needs the typed
/// backend handle; eagerly opening the collection delays terminal takeover
/// without making Memory useful any sooner.
pub(crate) struct LazyFileMemoryStore {
    directory: PathBuf,
    store: OnceCell<VecMemoryStore>,
}

impl LazyFileMemoryStore {
    pub(crate) fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            store: OnceCell::new(),
        }
    }

    async fn inner(&self) -> anyhow::Result<&VecMemoryStore> {
        let directory = self.directory.clone();
        self.store
            .get_or_try_init(|| async {
                let store = VecMemoryStore::open(&directory)?;
                // One-time convergence: carry legacy FileMemoryStore JSON
                // items into the a3s-vec collection, then retire the legacy
                // files so the import runs at most once.
                let legacy_items = directory.join("items");
                let legacy_index = directory.join("index.json");
                if legacy_items.is_dir() {
                    match store.import_legacy_json_items(&legacy_items) {
                        Ok(imported) if imported > 0 => {
                            tracing::info!(
                                imported,
                                dir = %directory.display(),
                                "imported legacy JSON memories into a3s-vec"
                            );
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::warn!(error = %error, "legacy memory import failed; legacy files kept");
                            return Ok::<VecMemoryStore, anyhow::Error>(store);
                        }
                    }
                    let _ = std::fs::rename(
                        &legacy_items,
                        directory.join("items.imported"),
                    );
                    if legacy_index.is_file() {
                        let _ = std::fs::rename(
                            &legacy_index,
                            directory.join("index.json.imported"),
                        );
                    }
                }
                Ok(store)
            })
            .await
            .with_context(|| {
                format!(
                    "failed to initialize a3s-vec Memory store at {}",
                    self.directory.display()
                )
            })
    }

    #[cfg(test)]
    fn is_initialized(&self) -> bool {
        self.store.get().is_some()
    }
}

#[async_trait::async_trait]
impl MemoryStore for LazyFileMemoryStore {
    async fn store(&self, item: MemoryItem) -> anyhow::Result<()> {
        MemoryStore::store(self.inner().await?, item).await
    }

    async fn store_and_return(&self, item: MemoryItem) -> anyhow::Result<MemoryItem> {
        MemoryStore::store_and_return(self.inner().await?, item).await
    }

    async fn retrieve(&self, id: &str) -> anyhow::Result<Option<MemoryItem>> {
        MemoryStore::retrieve(self.inner().await?, id).await
    }

    async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<MemoryItem>> {
        MemoryStore::search(self.inner().await?, query, limit).await
    }

    async fn search_by_tags(
        &self,
        tags: &[String],
        limit: usize,
    ) -> anyhow::Result<Vec<MemoryItem>> {
        MemoryStore::search_by_tags(self.inner().await?, tags, limit).await
    }

    async fn get_recent(&self, limit: usize) -> anyhow::Result<Vec<MemoryItem>> {
        MemoryStore::get_recent(self.inner().await?, limit).await
    }

    async fn get_important(&self, threshold: f32, limit: usize) -> anyhow::Result<Vec<MemoryItem>> {
        MemoryStore::get_important(self.inner().await?, threshold, limit).await
    }

    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        MemoryStore::delete(self.inner().await?, id).await
    }

    async fn clear(&self) -> anyhow::Result<()> {
        MemoryStore::clear(self.inner().await?).await
    }

    async fn count(&self) -> anyhow::Result<usize> {
        MemoryStore::count(self.inner().await?).await
    }

    async fn prune(&self, policy: &PrunePolicy) -> anyhow::Result<usize> {
        MemoryStore::prune(self.inner().await?, policy).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_json_items_are_imported_once() {
        let root = tempfile::tempdir().unwrap();
        let memory_dir = root.path().join("memory");
        std::fs::create_dir_all(memory_dir.join("items")).unwrap();
        let item = serde_json::json!({
            "id": "legacy-1",
            "content": "legacy memory about e2e fixtures",
            "timestamp": "2026-01-01T00:00:00Z",
            "importance": 0.7,
            "tags": ["legacy"],
            "memory_type": "semantic",
            "metadata": {},
            "access_count": 0,
            "last_accessed": null
        });
        std::fs::write(
            memory_dir.join("items/legacy-1.json"),
            serde_json::to_vec(&item).unwrap(),
        )
        .unwrap();

        let store = LazyFileMemoryStore::new(&memory_dir);
        let matches = MemoryStore::search(&store, "legacy e2e fixtures", 5)
            .await
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert!(matches[0].content.contains("legacy memory"));
        assert!(memory_dir.join("items.imported").exists());
        assert!(memory_dir.join("index.json.imported").exists() == false);

        // Re-opening must not duplicate or re-import. Drop the first handle
        // first: the engine's collection lock is exclusive per process.
        drop(store);
        let store = LazyFileMemoryStore::new(&memory_dir);
        assert_eq!(MemoryStore::count(&store).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn construction_does_not_touch_an_unopenable_collection() {
        let root = tempfile::tempdir().unwrap();
        let collection_dir = root.path().join("durable_memory");
        std::fs::create_dir(&collection_dir).unwrap();
        std::fs::write(collection_dir.join("garbage"), b"not a collection").unwrap();
        let store = LazyFileMemoryStore::new(root.path());

        assert!(!store.is_initialized());

        let error = MemoryStore::count(&store).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("failed to initialize a3s-vec Memory store"),
            "{error:#}"
        );
        assert!(!store.is_initialized());
    }

    #[tokio::test]
    async fn first_operation_initializes_the_file_backend_once() {
        let root = tempfile::tempdir().unwrap();
        let store = LazyFileMemoryStore::new(root.path());

        assert_eq!(MemoryStore::count(&store).await.unwrap(), 0);
        assert!(store.is_initialized());

        MemoryStore::store(&store, MemoryItem::new("durable lazy Memory"))
            .await
            .unwrap();
        assert_eq!(MemoryStore::count(&store).await.unwrap(), 1);
        assert!(root.path().join("durable_memory").exists());
    }

    #[tokio::test]
    async fn first_search_initializes_and_reads_the_file_backend() {
        let root = tempfile::tempdir().unwrap();
        let eager = VecMemoryStore::open(root.path()).unwrap();
        MemoryStore::store(
            &eager,
            MemoryItem::new("The lazy interactive startup verification codename is ORCHID-7319."),
        )
        .await
        .unwrap();
        drop(eager);

        let store = LazyFileMemoryStore::new(root.path());
        assert!(!store.is_initialized());

        let matches = MemoryStore::search(&store, "startup verification codename", 5)
            .await
            .unwrap();

        assert!(store.is_initialized());
        assert_eq!(matches.len(), 1);
        assert!(matches[0].content.contains("ORCHID-7319"));
    }

    #[tokio::test]
    async fn store_survives_reopen_through_a_fresh_lazy_handle() {
        let root = tempfile::tempdir().unwrap();
        let writer = LazyFileMemoryStore::new(root.path());
        MemoryStore::store(
            &writer,
            MemoryItem::new("The lazy reopen verification codename is MAGNOLIA-4421."),
        )
        .await
        .unwrap();
        assert!(writer.is_initialized());
        assert!(root.path().join("durable_memory").exists());
        drop(writer);

        let reader = LazyFileMemoryStore::new(root.path());
        assert!(!reader.is_initialized());
        let matches = MemoryStore::search(&reader, "reopen verification codename", 5)
            .await
            .unwrap();
        assert!(reader.is_initialized());
        assert_eq!(matches.len(), 1);
        assert!(matches[0].content.contains("MAGNOLIA-4421"));
        assert_eq!(MemoryStore::count(&reader).await.unwrap(), 1);
    }
}
