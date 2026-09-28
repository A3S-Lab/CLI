//! a3s-vec backed durable memory store — the convergence target for A3S
//! memory.
//!
//! One system, A3S's own engine: memory items live in an in-process a3s-vec
//! collection with a full-text index over their content, so recall is a real
//! relevance query instead of loading every item. Importance and type ride
//! alongside as typed fields, and vector (semantic) retrieval rides in the
//! same collection: an optional embedding provider (the workspace
//! retrieval local-CPU stack) turns store/search into hybrid FTS + ANN
//! recall through reciprocal-rank fusion.
//!
//! This module intentionally depends only on `a3s-vec` (already optional in
//! core behind `a3s-vec-fts`) so the memory layer and the workspace lexical
//! index share one engine and one operational model.

use a3s_code_core::embedding::{
    EmbeddingBatchRequest, EmbeddingInput, EmbeddingProvider, EmbeddingProviderError,
};
use a3s_memory::{MemoryItem, MemoryStore, MemoryType};
use a3s_vec::{
    Collection, CollectionSchema, DataType, Doc, FieldSchema, IndexParams, MetricType, SearchQuery,
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio_util::sync::CancellationToken;

static ENGINE_INIT: OnceLock<Result<(), String>> = OnceLock::new();

fn ensure_initialized() -> Result<(), String> {
    // `a3s_vec::initialize` is idempotent; always call it so the cold path
    // stays reachable under the process-wide OnceLock.
    ENGINE_INIT
        .get_or_init(|| a3s_vec::initialize(None).map_err(|error| error.to_string()))
        .clone()
}

fn display_error(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow!(error.to_string())
}

/// One durable memory record as stored in the a3s-vec collection.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryRecord {
    pub id: String,
    pub content: String,
    pub tags: Vec<String>,
    pub importance: f32,
    pub memory_type: String,
    /// RFC3339 timestamp (lexicographic order == chronological for UTC).
    pub timestamp: String,
    /// Serialized auxiliary state (metadata map, access counters).
    pub meta: String,
}

/// An a3s-vec collection holding durable memory items, opened at a fixed path.
pub struct VecMemoryStore {
    collection: Collection,
    path: PathBuf,
    /// Whether the collection carries the ANN column. Legacy collections
    /// migrate on open; when the migration is refused the store stays FTS-only.
    vector_enabled: bool,
    embedding: Option<Arc<dyn EmbeddingProvider>>,
}

const COLLECTION_NAME: &str = "durable_memory";
const CONTENT_FIELD: &str = "content";
const VECTOR_FIELD: &str = "embedding";
/// Dimension of the managed local-CPU embedding model (MiniLM). The ANN
/// column is sized for it; a provider reporting another dimension is ignored
/// rather than mismatching the index.
const EMBEDDING_DIM: usize = 384;

fn memory_schema() -> Result<CollectionSchema> {
    let mut content =
        FieldSchema::new(CONTENT_FIELD, DataType::String, false, 0).map_err(display_error)?;
    let fts = IndexParams::fts(Some("whitespace"), None, None).map_err(display_error)?;
    content.set_index_params(&fts).map_err(display_error)?;
    CollectionSchema::builder(COLLECTION_NAME)
        // The engine primary key (set on every Doc) IS the memory id; no
        // separate id field is declared.
        .add_field(content)
        .add_field(FieldSchema::new("tags", DataType::String, true, 0).map_err(display_error)?)
        .add_field(FieldSchema::new("importance", DataType::Float, true, 0).map_err(display_error)?)
        .add_field(
            FieldSchema::new("memory_type", DataType::String, true, 0).map_err(display_error)?,
        )
        .add_field(FieldSchema::new("timestamp", DataType::String, true, 0).map_err(display_error)?)
        .add_field(FieldSchema::new("meta", DataType::String, true, 0).map_err(display_error)?)
        .add_vector_field(
            VECTOR_FIELD,
            DataType::VectorFp32,
            EMBEDDING_DIM as u32,
            IndexParams::hnsw(MetricType::Cosine, 16, 100).map_err(display_error)?,
        )
        .build()
        .map_err(display_error)
}

/// The ANN column on its own, used to migrate legacy collections in place.
fn vector_field_schema() -> Result<FieldSchema> {
    let mut field =
        FieldSchema::new(VECTOR_FIELD, DataType::VectorFp32, false, EMBEDDING_DIM as u32)
            .map_err(display_error)?;
    field
        .set_index_params(&IndexParams::hnsw(MetricType::Cosine, 16, 100).map_err(display_error)?)
        .map_err(display_error)?;
    Ok(field)
}

fn collection_has_vector_field(collection: &Collection) -> Result<bool> {
    let schema = collection.schema().map_err(display_error)?;
    Ok(schema.vectors().iter().any(|field| field.name() == VECTOR_FIELD))
}

fn record_to_doc(record: &MemoryRecord) -> Result<Doc> {
    let mut doc = Doc::new().map_err(display_error)?;
    doc.set_pk(&record.id);
    doc.add_string(CONTENT_FIELD, &record.content)
        .map_err(display_error)?;
    doc.add_string("tags", &record.tags.join(" "))
        .map_err(display_error)?;
    doc.add_f32("importance", record.importance)
        .map_err(display_error)?;
    doc.add_string("memory_type", &record.memory_type)
        .map_err(display_error)?;
    doc.add_string("timestamp", &record.timestamp)
        .map_err(display_error)?;
    doc.add_string("meta", &record.meta)
        .map_err(display_error)?;
    Ok(doc)
}

fn memory_id(document: &Doc) -> Result<String> {
    document
        .get_pk()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("a3s-vec memory result omitted its primary key"))
}

impl VecMemoryStore {
    /// Open the memory collection at `root`, creating it on first use.
    pub fn open(root: &Path) -> Result<Self> {
        ensure_initialized().map_err(|error| anyhow!("a3s-vec engine init failed: {error}"))?;
        fs_err_create_dir_all(root)?;
        let collection_path = root.join(COLLECTION_NAME);
        let collection_path = collection_path
            .to_str()
            .ok_or_else(|| anyhow!("memory collection path is not UTF-8"))?;
        let vector_enabled;
        let collection = if collection_path_exists(&collection_path) {
            let collection = Collection::open(collection_path, None).map_err(display_error)?;
            // Collections created before the ANN column existed try an
            // in-place migration; when the engine refuses (non-nullable
            // vector over legacy documents), this store degrades to FTS-only
            // recall instead of failing Memory entirely.
            vector_enabled = if collection_has_vector_field(&collection)? {
                true
            } else {
                match collection.add_column(&vector_field_schema()?, None) {
                    Ok(()) => true,
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            "memory collection lacks the embedding column and the migration failed; vector recall disabled"
                        );
                        false
                    }
                }
            };
            collection
        } else {
            let schema = memory_schema()?;
            vector_enabled = true;
            Collection::create_and_open(collection_path, &schema, None).map_err(display_error)?
        };
        Ok(Self {
            collection,
            path: collection_path.into(),
            vector_enabled,
            embedding: None,
        })
    }

    /// Attach the hybrid-recall embedding source. A provider whose dimension
    /// disagrees with the ANN column is rejected fail-open (FTS-only).
    pub fn with_embedding_provider(mut self, provider: Arc<dyn EmbeddingProvider>) -> Self {
        self.embedding = if provider.descriptor().dimension == EMBEDDING_DIM {
            Some(provider)
        } else {
            tracing::warn!(
                reported = provider.descriptor().dimension,
                expected = EMBEDDING_DIM,
                "memory embedding provider dimension mismatch; vector recall disabled"
            );
            None
        };
        self
    }

    fn vector_recall_enabled(&self) -> bool {
        self.vector_enabled && self.embedding.is_some()
    }

    /// Embed one text with the attached provider. `None` when no provider is
    /// wired or the provider fails: recall degrades to FTS instead of erroring.
    async fn embed_text(&self, text: &str) -> Option<Vec<f32>> {
        let provider = self.embedding.as_ref()?;
        let request = EmbeddingBatchRequest::new(vec![EmbeddingInput::new("query", text)]);
        match provider.embed(request, CancellationToken::new()).await {
            Ok(response) => response.vectors.first().map(|vector| vector.values.clone()),
            Err(error @ EmbeddingProviderError::Cancelled) => {
                tracing::warn!(error = %error, "memory query embedding cancelled; falling back to FTS");
                None
            }
            Err(error) => {
                tracing::warn!(error = %error, "memory query embedding failed; falling back to FTS");
                None
            }
        }
    }

    /// Insert or replace one memory record by id. Records written without an
    /// embedding carry a zero vector: the ANN column is non-nullable, and a
    /// zero vector never surfaces in ANN recall, which is exactly right for
    /// FTS-only records.
    pub fn put(&self, record: &MemoryRecord) -> Result<()> {
        if self.vector_enabled {
            return self.put_with_vector(record, &vec![0.0; EMBEDDING_DIM]);
        }
        let doc = record_to_doc(record)?;
        let references = [&doc];
        let result = self.collection.insert(&references).map_err(display_error)?;
        if result.error_count != 0 {
            let reasons: Vec<String> = result
                .results
                .iter()
                .filter(|outcome| !outcome.success)
                .map(|outcome| outcome.message.clone())
                .collect();
            return Err(anyhow!(
                "a3s-vec memory insert rejected {} document(s): {}",
                result.error_count,
                reasons.join("; ")
            ));
        }
        self.collection.flush().map_err(display_error)?;
        Ok(())
    }

    /// Full-text recall: rank memory records by FTS relevance to `query`.
    /// Returns `(id, score)` pairs, best first.
    pub fn recall(&self, query: &str, limit: usize) -> Result<Vec<(String, f32)>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut fts = a3s_vec::Fts::new().map_err(display_error)?;
        fts.set_match_string(query).map_err(display_error)?;
        let topk = i32::try_from(limit).map_err(|_| anyhow!("memory recall limit exceeds i32"))?;
        let mut search = SearchQuery::fts(CONTENT_FIELD, &fts, topk).map_err(display_error)?;
        search.set_output_fields(&[]).map_err(display_error)?;
        let documents = self.collection.query(&search).map_err(display_error)?;
        Ok(documents
            .iter()
            .map(memory_id)
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .zip(documents.iter().map(Doc::get_score))
            .collect())
    }

    /// ANN recall over the embedding column: `(id, cosine-distance score)`
    /// pairs, nearest first. Empty unless the collection carries the ANN
    /// column; the caller decides what a degraded answer means.
    pub fn recall_vector(&self, vector: &[f32], limit: usize) -> Result<Vec<(String, f32)>> {
        if !self.vector_enabled || vector.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let topk = i32::try_from(limit).map_err(|_| anyhow!("memory recall limit exceeds i32"))?;
        let mut search = SearchQuery::new(VECTOR_FIELD, vector, topk).map_err(display_error)?;
        search.set_output_fields(&[]).map_err(display_error)?;
        let documents = self.collection.query(&search).map_err(display_error)?;
        Ok(documents
            .iter()
            .map(memory_id)
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .zip(documents.iter().map(Doc::get_score))
            .collect())
    }

    /// Hybrid recall: FTS ranking fused with ANN ranking through reciprocal
    /// rank fusion, so lexical hits and semantic neighbours both surface.
    /// `query_vector` comes from the caller's embedding step; `None` (or an
    /// ANN-less collection) degrades to plain FTS.
    pub fn recall_hybrid(
        &self,
        query: &str,
        query_vector: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        let mut fused: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
        let mut rrf = |ranking: &[(String, f32)]| {
            ranking
                .iter()
                .enumerate()
                .for_each(|(index, (id, _))| *fused.entry(id.clone()).or_default() += 1.0 / (60.0 + index as f32));
        };
        let fts_hits = self.recall(query, limit)?;
        let vector_hits = match query_vector {
            Some(vector) if self.vector_enabled => self.recall_vector(vector, limit)?,
            _ => Vec::new(),
        };
        if vector_hits.is_empty() {
            return Ok(fts_hits);
        }
        rrf(&fts_hits);
        rrf(&vector_hits);
        let mut ranked: Vec<(String, f32)> = fused.into_iter().collect();
        ranked.sort_by(|left, right| right.1.partial_cmp(&left.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(limit);
        Ok(ranked)
    }

    /// Store one record together with its content embedding, so the record is
    /// reachable through ANN recall. Requires the ANN column; otherwise this
    /// is a plain `put`.
    pub fn put_with_vector(&self, record: &MemoryRecord, vector: &[f32]) -> Result<()> {
        let mut doc = record_to_doc(record)?;
        if self.vector_enabled {
            doc.add_vector_f32(VECTOR_FIELD, vector)
                .map_err(display_error)?;
        }
        let references = [&doc];
        let result = self.collection.insert(&references).map_err(display_error)?;
        if result.error_count != 0 {
            let reasons: Vec<String> = result
                .results
                .iter()
                .filter(|outcome| !outcome.success)
                .map(|outcome| outcome.message.clone())
                .collect();
            return Err(anyhow!(
                "a3s-vec memory insert rejected {} document(s): {}",
                result.error_count,
                reasons.join("; ")
            ));
        }
        self.collection.flush().map_err(display_error)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn fs_err_create_dir_all(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)
        .map_err(|error| anyhow!("create memory root {}: {error}", root.display()))
}

#[cfg(windows)]
fn collection_path_exists(path: &str) -> bool {
    Path::new(path).exists()
}

#[cfg(not(windows))]
fn collection_path_exists(path: &str) -> bool {
    Path::new(path).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, content: &str, importance: f32) -> MemoryRecord {
        MemoryRecord {
            id: id.to_owned(),
            content: content.to_owned(),
            tags: vec!["testing".to_owned()],
            importance,
            memory_type: "semantic".to_owned(),
            timestamp: "2026-01-01T00:00:00+00:00".to_owned(),
            meta: "{}".to_owned(),
        }
    }

    #[test]
    fn insert_and_fts_recall_roundtrip() {
        let root = tempfile::tempdir().unwrap();
        let store = VecMemoryStore::open(root.path()).unwrap();
        store
            .put(&record(
                "m1",
                "the workspace uses rust-analyzer for navigation",
                0.8,
            ))
            .unwrap();
        store
            .put(&record(
                "m2",
                "e2e fixtures live under tests/e2e/fixtures",
                0.6,
            ))
            .unwrap();

        let hits = store.recall("rust-analyzer navigation", 5).unwrap();
        assert!(!hits.is_empty(), "expected at least one hit");
        assert_eq!(hits[0].0, "m1");
    }

    #[test]
    fn reopen_sees_persisted_records() {
        let root = tempfile::tempdir().unwrap();
        {
            let store = VecMemoryStore::open(root.path()).unwrap();
            store
                .put(&record("m3", "memory survives process restart", 0.5))
                .unwrap();
        }
        let reopened = VecMemoryStore::open(root.path()).unwrap();
        let hits = reopened.recall("process restart", 5).unwrap();
        assert!(hits.iter().any(|(id, _)| id == "m3"));
    }

    #[tokio::test]
    async fn memory_store_trait_roundtrip() {
        use a3s_memory::MemoryStore as _;
        let root = tempfile::tempdir().unwrap();
        let store = VecMemoryStore::open(root.path()).unwrap();
        let mut item = MemoryItem::new("the workspace prefers rust-analyzer navigation");
        item.importance = 0.9;
        item.tags = vec!["testing".to_owned()];
        item.memory_type = MemoryType::Procedural;
        store.store(item.clone()).await.unwrap();

        assert_eq!(store.count().await.unwrap(), 1);
        let fetched = store
            .retrieve(&item.id)
            .await
            .unwrap()
            .expect("stored item");
        assert_eq!(fetched.content, item.content);
        assert_eq!(fetched.memory_type, item.memory_type);

        let searched = store.search("rust-analyzer navigation", 5).await.unwrap();
        assert_eq!(searched.len(), 1);
        let by_tag = store
            .search_by_tags(&["testing".to_owned()], 5)
            .await
            .unwrap();
        assert_eq!(by_tag.len(), 1);
        let recent = store.get_recent(5).await.unwrap();
        assert_eq!(recent.len(), 1);
        let important = store.get_important(0.8, 5).await.unwrap();
        assert_eq!(important.len(), 1);
        let not_important = store.get_important(0.95, 5).await.unwrap();
        assert!(not_important.is_empty());

        store.delete(&item.id).await.unwrap();
        assert_eq!(store.count().await.unwrap(), 0);
        assert!(store.retrieve(&item.id).await.unwrap().is_none());
    }

    #[test]
    fn empty_query_returns_nothing() {
        let root = tempfile::tempdir().unwrap();
        let store = VecMemoryStore::open(root.path()).unwrap();
        store.put(&record("m4", "something", 0.5)).unwrap();
        assert!(store.recall("   ", 5).unwrap().is_empty());
    }

    /// Deterministic stand-in for the local-CPU provider: synonym classes map
    /// to orthogonal axes, so texts share semantics without sharing tokens.
    struct FakeSynonymEmbedding;

    const SYNONYM_CLASSES: &[(&str, &str, usize)] = &[
        // (token, synonym, class axis)
        ("rocket", "spacecraft", 7),
        ("automobile", "car", 11),
        ("settlement", "village", 19),
    ];

    impl FakeSynonymEmbedding {
        fn embed_text(text: &str) -> Vec<f32> {
            let mut vector = vec![0.0_f32; 384];
            for token in text.split_whitespace() {
                let token = token.trim_matches(|c: char| !c.is_alphanumeric());
                for (left, right, axis) in SYNONYM_CLASSES {
                    if token.eq_ignore_ascii_case(left) || token.eq_ignore_ascii_case(right) {
                        vector[*axis] += 1.0;
                    }
                }
            }
            vector
        }
    }

    #[async_trait::async_trait]
    impl EmbeddingProvider for FakeSynonymEmbedding {
        fn descriptor(&self) -> a3s_code_core::embedding::EmbeddingProviderDescriptor {
            a3s_code_core::embedding::EmbeddingProviderDescriptor::new("test", "fake", 384)
        }

        async fn embed(
            &self,
            request: EmbeddingBatchRequest,
            _cancellation: CancellationToken,
        ) -> std::result::Result<
            a3s_code_core::embedding::EmbeddingBatchResponse,
            EmbeddingProviderError,
        > {
            use a3s_code_core::embedding::{EmbeddingBatchResponse, EmbeddingVector};
            let vectors = request
                .inputs()
                .iter()
                .map(|input| EmbeddingVector::new(input.id(), Self::embed_text(input.text())))
                .collect();
            Ok(EmbeddingBatchResponse::new(
                a3s_code_core::embedding::EmbeddingProviderDescriptor::new("test", "fake", 384),
                vectors,
            ))
        }
    }

    fn store_with_embeddings(root: &Path) -> VecMemoryStore {
        VecMemoryStore::open(root)
            .unwrap()
            .with_embedding_provider(Arc::new(FakeSynonymEmbedding))
    }

    #[tokio::test]
    async fn hybrid_recall_hits_without_lexical_overlap() {
        let root = tempfile::tempdir().unwrap();
        let store = store_with_embeddings(root.path());
        store
            .put_with_vector(
                &record("space", "the spacecraft docked quietly", 0.5),
                &FakeSynonymEmbedding::embed_text("the spacecraft docked quietly"),
            )
            .unwrap();
        store
            .put_with_vector(
                &record("road", "the automobile parked outside", 0.5),
                &FakeSynonymEmbedding::embed_text("the automobile parked outside"),
            )
            .unwrap();

        // "rocket" shares no token with the stored texts; only the synonym
        // axis makes the spacecraft record reachable.
        let hits = MemoryStore::search(&store, "rocket launch", 5).await.unwrap();
        assert_eq!(hits.first().map(|item| item.id.as_str()), Some("space"));

        // A lexical query keeps working through the same surface.
        let lexical = MemoryStore::search(&store, "automobile parked", 5).await.unwrap();
        assert_eq!(lexical.first().map(|item| item.id.as_str()), Some("road"));
    }

    #[tokio::test]
    async fn embedding_failure_degrades_to_fts() {
        struct FailingProvider;

        #[async_trait::async_trait]
        impl EmbeddingProvider for FailingProvider {
            fn descriptor(&self) -> a3s_code_core::embedding::EmbeddingProviderDescriptor {
                a3s_code_core::embedding::EmbeddingProviderDescriptor::new("test", "failing", 384)
            }

            async fn embed(
                &self,
                _request: EmbeddingBatchRequest,
                _cancellation: CancellationToken,
            ) -> std::result::Result<
                a3s_code_core::embedding::EmbeddingBatchResponse,
                EmbeddingProviderError,
            > {
                Err(EmbeddingProviderError::Unavailable { retry_after: None })
            }
        }

        let root = tempfile::tempdir().unwrap();
        let store = VecMemoryStore::open(root.path())
            .unwrap()
            .with_embedding_provider(Arc::new(FailingProvider));
        store
            .put(&record("m5", "resilient lexical fallback memory", 0.5))
            .unwrap();
        let hits = MemoryStore::search(&store, "resilient lexical fallback", 5)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn dimension_mismatch_disables_vector_recall() {
        struct WrongDimension;

        #[async_trait::async_trait]
        impl EmbeddingProvider for WrongDimension {
            fn descriptor(&self) -> a3s_code_core::embedding::EmbeddingProviderDescriptor {
                a3s_code_core::embedding::EmbeddingProviderDescriptor::new("test", "wrong", 7)
            }

            async fn embed(
                &self,
                _request: EmbeddingBatchRequest,
                _cancellation: CancellationToken,
            ) -> std::result::Result<
                a3s_code_core::embedding::EmbeddingBatchResponse,
                EmbeddingProviderError,
            > {
                unimplemented!("a dimension-mismatched provider must never be called")
            }
        }

        let root = tempfile::tempdir().unwrap();
        let store = VecMemoryStore::open(root.path())
            .unwrap()
            .with_embedding_provider(Arc::new(WrongDimension));
        assert!(!store.vector_recall_enabled());
    }

    #[tokio::test]
    async fn legacy_collection_without_vector_column_degrades_to_fts_only() {
        use a3s_vec::CollectionSchemaBuilder as _;

        let root = tempfile::tempdir().unwrap();
        // Build a pre-vector-era collection by hand: the scalar fields of
        // memory_schema() without the ANN column.
        let legacy_schema = CollectionSchema::builder(COLLECTION_NAME)
            .add_field(
                FieldSchema::new(CONTENT_FIELD, DataType::String, false, 0)
                    .map_err(display_error)
                    .unwrap(),
            )
            .add_field(
                FieldSchema::new("tags", DataType::String, true, 0)
                    .map_err(display_error)
                    .unwrap(),
            )
            .add_field(
                FieldSchema::new("importance", DataType::Float, true, 0)
                    .map_err(display_error)
                    .unwrap(),
            )
            .add_field(
                FieldSchema::new("memory_type", DataType::String, true, 0)
                    .map_err(display_error)
                    .unwrap(),
            )
            .add_field(
                FieldSchema::new("timestamp", DataType::String, true, 0)
                    .map_err(display_error)
                    .unwrap(),
            )
            .add_field(
                FieldSchema::new("meta", DataType::String, true, 0)
                    .map_err(display_error)
                    .unwrap(),
            )
            .build()
            .unwrap();
        {
            let collection_path = root.path().join(COLLECTION_NAME);
            let collection = Collection::create_and_open(
                collection_path.to_str().unwrap(),
                &legacy_schema,
                None,
            )
            .unwrap();
            let mut doc = Doc::new().unwrap();
            doc.set_pk("old-1");
            doc.add_string(CONTENT_FIELD, "a pre-vector memory").unwrap();
            let references = [&doc];
            collection.insert(&references).unwrap();
            collection.flush().unwrap();
            collection.close().unwrap();
        }
        let store = VecMemoryStore::open(root.path()).unwrap();
        // The engine refuses to add a non-nullable ANN column over documents
        // that predate it, so opening degrades fail-safe: FTS recall stays
        // fully functional, ANN recall stays off, nothing is lost.
        assert!(!store.vector_enabled);
        assert!(store.recall_vector(&[0.0; 384], 5).unwrap().is_empty());
        let hits = MemoryStore::search(&store, "pre-vector memory", 5).await.unwrap();
        assert_eq!(hits.len(), 1);
        // New writes still land (zero-vector) and remain searchable.
        store
            .put(&record("new-1", "a fresh post-open memory", 0.5))
            .unwrap();
        let fresh = MemoryStore::search(&store, "fresh post-open", 5).await.unwrap();
        assert_eq!(fresh.len(), 1);
    }
}

/// Metadata that rides with a record but does not need to be queryable.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct RecordMeta {
    #[serde(default)]
    metadata: std::collections::HashMap<String, String>,
    #[serde(default)]
    access_count: u32,
    #[serde(default)]
    last_accessed: Option<String>,
}

fn memory_type_to_string(memory_type: &MemoryType) -> Result<String> {
    serde_json::to_string(memory_type).map_err(display_error)
}

fn memory_type_from_string(raw: &str) -> Result<MemoryType> {
    serde_json::from_str(raw).map_err(display_error)
}

fn doc_to_item(document: &Doc) -> Result<MemoryItem> {
    let id = memory_id(document)?;
    let content = document
        .get_string(CONTENT_FIELD)
        .map_err(display_error)?
        .unwrap_or_default();
    let tags = document
        .get_string("tags")
        .map_err(display_error)?
        .unwrap_or_default();
    let importance = document
        .get_f32("importance")
        .map_err(display_error)?
        .unwrap_or_default();
    let memory_type = document
        .get_string("memory_type")
        .map_err(display_error)?
        .unwrap_or_default();
    let timestamp = document
        .get_string("timestamp")
        .map_err(display_error)?
        .unwrap_or_default();
    let meta_raw = document
        .get_string("meta")
        .map_err(display_error)?
        .unwrap_or_default();
    let meta: RecordMeta = serde_json::from_str(&meta_raw).unwrap_or_default();
    let timestamp = DateTime::parse_from_rfc3339(&timestamp)
        .map(|parsed| parsed.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let memory_type = memory_type_from_string(&memory_type).unwrap_or(MemoryType::Semantic);
    Ok(MemoryItem {
        id,
        content,
        timestamp,
        importance,
        tags: tags.split_whitespace().map(str::to_owned).collect(),
        memory_type,
        metadata: meta.metadata,
        access_count: meta.access_count,
        last_accessed: meta.last_accessed.and_then(|raw| {
            DateTime::parse_from_rfc3339(&raw)
                .map(|parsed| parsed.with_timezone(&Utc))
                .ok()
        }),
        content_lower: String::new(),
    })
}

fn item_to_record(item: &MemoryItem) -> Result<MemoryRecord> {
    let meta = RecordMeta {
        metadata: item.metadata.clone(),
        access_count: item.access_count,
        last_accessed: item.last_accessed.map(|stamp| stamp.to_rfc3339()),
    };
    Ok(MemoryRecord {
        id: item.id.clone(),
        content: item.content.clone(),
        tags: item.tags.clone(),
        importance: item.importance,
        memory_type: memory_type_to_string(&item.memory_type)?,
        timestamp: item.timestamp.to_rfc3339(),
        meta: serde_json::to_string(&meta).map_err(display_error)?,
    })
}

#[async_trait::async_trait]
impl MemoryStore for VecMemoryStore {
    async fn store(&self, item: MemoryItem) -> Result<()> {
        let record = item_to_record(&item)?;
        match self.embed_text(&record.content).await {
            Some(vector) => self.put_with_vector(&record, &vector),
            None => self.put(&record),
        }
    }

    async fn retrieve(&self, id: &str) -> Result<Option<MemoryItem>> {
        for document in self.collection.iter().map_err(display_error)? {
            let document = document.map_err(display_error)?;
            if memory_id(&document)?.as_str() == id {
                return Ok(Some(doc_to_item(&document)?));
            }
        }
        Ok(None)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryItem>> {
        let query_vector = if self.vector_recall_enabled() {
            self.embed_text(query).await
        } else {
            None
        };
        let hits = self.recall_hybrid(query, query_vector.as_deref(), limit)?;
        let wanted: std::collections::HashSet<&str> =
            hits.iter().map(|(id, _)| id.as_str()).collect();
        let mut items = Vec::with_capacity(hits.len());
        for document in self.collection.iter().map_err(display_error)? {
            let document = document.map_err(display_error)?;
            if memory_id(&document)?.as_str().is_empty()
                || !wanted.contains(memory_id(&document)?.as_str())
            {
                continue;
            }
            items.push(doc_to_item(&document)?);
            if items.len() == hits.len() {
                break;
            }
        }
        // Preserve FTS ranking order.
        items.sort_by_cached_key(|item| {
            hits.iter()
                .position(|(id, _)| id == &item.id)
                .unwrap_or(usize::MAX)
        });
        Ok(items)
    }

    async fn search_by_tags(&self, tags: &[String], limit: usize) -> Result<Vec<MemoryItem>> {
        if tags.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: std::collections::HashSet<&str> = tags.iter().map(|tag| tag.as_str()).collect();
        let mut items = Vec::new();
        for document in self.collection.iter().map_err(display_error)? {
            let document = document.map_err(display_error)?;
            let item = doc_to_item(&document)?;
            if item.tags.iter().any(|tag| wanted.contains(tag.as_str())) {
                items.push(item);
                if items.len() == limit {
                    break;
                }
            }
        }
        Ok(items)
    }

    async fn get_recent(&self, limit: usize) -> Result<Vec<MemoryItem>> {
        let mut items = Vec::new();
        for document in self.collection.iter().map_err(display_error)? {
            items.push(doc_to_item(&document.map_err(display_error)?)?);
        }
        items.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));
        items.truncate(limit);
        Ok(items)
    }

    async fn get_important(&self, threshold: f32, limit: usize) -> Result<Vec<MemoryItem>> {
        let mut items = Vec::new();
        for document in self.collection.iter().map_err(display_error)? {
            let item = doc_to_item(&document.map_err(display_error)?)?;
            if item.importance >= threshold {
                items.push(item);
            }
        }
        items.sort_by(|left, right| {
            right
                .importance
                .partial_cmp(&left.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        items.truncate(limit);
        Ok(items)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let result = self.collection.delete(&[id]).map_err(display_error)?;
        if result.error_count != 0 {
            return Err(anyhow!("a3s-vec memory delete rejected {id}"));
        }
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        let mut ids = Vec::new();
        for document in self.collection.iter().map_err(display_error)? {
            ids.push(memory_id(&document.map_err(display_error)?)?);
        }
        if ids.is_empty() {
            return Ok(());
        }
        let refs = ids.iter().map(String::as_str).collect::<Vec<_>>();
        let result = self.collection.delete(&refs).map_err(display_error)?;
        if result.error_count != 0 {
            return Err(anyhow!(
                "a3s-vec memory clear rejected {} document(s)",
                result.error_count
            ));
        }
        Ok(())
    }

    async fn count(&self) -> Result<usize> {
        self.collection.count().map_err(display_error)
    }
}

impl VecMemoryStore {
    /// One-time import of the legacy JSON memory store (`items/*.json`, full
    /// `MemoryItem` serializations written by FileMemoryStore). Items whose id
    /// already exists in the collection are skipped, so re-running is
    /// harmless. Returns the number of imported items.
    pub fn import_legacy_json_items(&self, legacy_items_dir: &Path) -> Result<usize> {
        let entries = match std::fs::read_dir(legacy_items_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => {
                return Err(anyhow!(
                    "read legacy memory items {}: {error}",
                    legacy_items_dir.display()
                ))
            }
        };
        let mut existing = std::collections::HashSet::new();
        for document in self.collection.iter().map_err(display_error)? {
            existing.insert(memory_id(&document.map_err(display_error)?)?);
        }
        let mut imported = 0usize;
        let mut corrupt = 0usize;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let raw = match std::fs::read_to_string(&path) {
                Ok(raw) => raw,
                Err(_) => {
                    corrupt += 1;
                    continue;
                }
            };
            let item: MemoryItem = match serde_json::from_str(&raw) {
                Ok(item) => item,
                Err(_) => {
                    corrupt += 1;
                    continue;
                }
            };
            if existing.contains(item.id.as_str()) {
                continue;
            }
            let record = item_to_record(&item)?;
            let mut doc = record_to_doc(&record)?;
            if self.vector_enabled {
                // Imported records have no embedding yet; the zero vector
                // satisfies the non-nullable ANN column and never surfaces
                // in vector recall.
                doc.add_vector_f32(VECTOR_FIELD, &vec![0.0; EMBEDDING_DIM])
                    .map_err(display_error)?;
            }
            let references = [&doc];
            let result = self.collection.insert(&references).map_err(display_error)?;
            if result.error_count != 0 {
                return Err(anyhow!(
                    "a3s-vec memory import rejected {}",
                    item.id
                ));
            }
            imported += 1;
        }
        if imported > 0 || corrupt > 0 {
            self.collection.flush().map_err(display_error)?;
        }
        Ok(imported)
    }
}

