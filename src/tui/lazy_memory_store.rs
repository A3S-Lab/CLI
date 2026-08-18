//! Lazy file-backed Memory initialization for the interactive first frame.

use std::path::PathBuf;

use a3s_memory::{FileMemoryStore, MemoryItem, MemoryStore, PrunePolicy};
use anyhow::Context;
use tokio::sync::OnceCell;

/// Preserve the durable file backend while deferring its index read until the
/// first real Memory operation. Session construction only needs the typed
/// backend handle; eagerly decoding a large `index.json` delays terminal
/// takeover without making Memory useful any sooner.
pub(super) struct LazyFileMemoryStore {
    directory: PathBuf,
    store: OnceCell<FileMemoryStore>,
}

impl LazyFileMemoryStore {
    pub(super) fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            store: OnceCell::new(),
        }
    }

    async fn inner(&self) -> anyhow::Result<&FileMemoryStore> {
        self.store
            .get_or_try_init(|| async { FileMemoryStore::new(&self.directory).await })
            .await
            .with_context(|| {
                format!(
                    "failed to initialize file Memory store at {}",
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
    async fn construction_does_not_touch_an_unreadable_index() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("index.json")).unwrap();
        let store = LazyFileMemoryStore::new(root.path());

        assert!(!store.is_initialized());

        let error = MemoryStore::count(&store).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("failed to initialize file Memory store"),
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
        assert!(root.path().join("index.json").is_file());
    }

    #[tokio::test]
    async fn first_search_initializes_and_reads_the_file_backend() {
        let root = tempfile::tempdir().unwrap();
        let eager = FileMemoryStore::new(root.path()).await.unwrap();
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
}
