use std::fmt;
use std::sync::Arc;

use a3s_code_core::{
    ChunkCatalogLimits, ChunkingConfig, ManifestWorkspaceBackend, WorkspaceChunkingStrategy,
    WorkspaceRetrievalOptions, WorkspaceServices,
};
use anyhow::Context;
use tokio_util::sync::CancellationToken;

/// One-way gate separating semantic background work from the TUI first frame.
/// CancellationToken is used as a persistent latch: activation cannot be lost
/// if it happens before a provider begins waiting.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceRetrievalStartupGate {
    activated: CancellationToken,
}

impl WorkspaceRetrievalStartupGate {
    pub(super) fn new() -> Self {
        Self {
            activated: CancellationToken::new(),
        }
    }

    pub(crate) fn activate(&self) {
        self.activated.cancel();
    }

    pub(super) async fn wait(&self, cancellation: &CancellationToken) -> bool {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => false,
            _ = self.activated.cancelled() => true,
        }
    }
}

/// One host-owned retrieval configuration split across its two ownership
/// boundaries: the shared catalog and the per-session semantic runtime.
#[derive(Clone)]
pub(crate) struct WorkspaceRetrievalHost {
    session: WorkspaceRetrievalOptions,
    catalog_strategy: WorkspaceChunkingStrategy,
    startup_gate: Option<WorkspaceRetrievalStartupGate>,
}

impl WorkspaceRetrievalHost {
    pub(super) fn new(
        session: WorkspaceRetrievalOptions,
        catalog_strategy: WorkspaceChunkingStrategy,
    ) -> Self {
        Self {
            session,
            catalog_strategy,
            startup_gate: None,
        }
    }

    pub(super) fn new_deferred(
        session: WorkspaceRetrievalOptions,
        catalog_strategy: WorkspaceChunkingStrategy,
        startup_gate: WorkspaceRetrievalStartupGate,
    ) -> Self {
        Self {
            session,
            catalog_strategy,
            startup_gate: Some(startup_gate),
        }
    }

    pub(crate) fn activate_background_indexing(&self) {
        if let Some(gate) = &self.startup_gate {
            gate.activate();
        }
    }

    pub(super) fn session_options(&self) -> &WorkspaceRetrievalOptions {
        &self.session
    }

    pub(super) fn workspace_services(
        &self,
        backend: Arc<ManifestWorkspaceBackend>,
    ) -> anyhow::Result<Arc<WorkspaceServices>> {
        // Host owns chunking strategy for the shared catalog. Durable zvec FTS
        // is not opened here — Core opens it on first `search` mode `bm25`
        // demand, so Loading and deferred/eager hosts share one path.
        backend
            .configure_chunk_catalog(
                self.catalog_strategy.clone(),
                ChunkingConfig::default(),
                ChunkCatalogLimits::default(),
            )
            .context("could not configure the host-owned workspace chunk catalog")?;
        Ok(WorkspaceServices::local_with_retrieval_backend(backend))
    }
}

impl fmt::Debug for WorkspaceRetrievalHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkspaceRetrievalHost")
            .field("session", &self.session)
            .field("catalog_strategy", &self.catalog_strategy)
            .field("startup_deferred", &self.startup_gate.is_some())
            .finish()
    }
}

pub(crate) fn workspace_services_for_host(
    backend: Arc<ManifestWorkspaceBackend>,
    retrieval: Option<&WorkspaceRetrievalHost>,
) -> anyhow::Result<Arc<WorkspaceServices>> {
    match retrieval {
        Some(retrieval) => retrieval.workspace_services(backend),
        // Lexical catalog + best-effort persistent zvec FTS do not require
        // semantic/embedding retrieval. Catalog enable stays cheap; durable
        // FTS opens on first `search` mode `bm25` demand.
        None => Ok(WorkspaceServices::local_with_retrieval_backend(backend)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_code_core::ManifestWorkspaceBackend;
    use tempfile::TempDir;

    #[tokio::test]
    async fn startup_gate_is_a_persistent_one_way_latch() {
        let gate = WorkspaceRetrievalStartupGate::new();
        let cancellation = CancellationToken::new();
        let waiting = tokio::time::timeout(
            std::time::Duration::from_millis(10),
            gate.wait(&cancellation),
        )
        .await;
        assert!(waiting.is_err(), "closed gate must keep providers waiting");

        gate.activate();
        assert!(gate.wait(&cancellation).await);
        assert!(gate.wait(&cancellation).await);
    }

    #[tokio::test]
    async fn startup_gate_honors_provider_cancellation() {
        let gate = WorkspaceRetrievalStartupGate::new();
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        assert!(!gate.wait(&cancellation).await);
    }

    #[tokio::test]
    async fn lexical_catalog_enabled_without_semantic_retrieval() {
        let root = TempDir::new().expect("temp workspace");
        let backend = ManifestWorkspaceBackend::new(root.path());
        let services = workspace_services_for_host(backend, None).expect("workspace services");
        assert!(
            services.chunk_catalog().is_some(),
            "Code host must attach the lexical chunk catalog without workspace_retrieval"
        );
        assert!(
            services.persistent_index().is_some(),
            "default product builds must attach best-effort persistent zvec FTS on demand"
        );
    }

    #[tokio::test]
    async fn catalog_configure_does_not_open_durable_zvec() {
        use a3s_code_core::{
            EmbeddingBatchRequest, EmbeddingBatchResponse, EmbeddingProvider,
            EmbeddingProviderDescriptor, EmbeddingProviderError, WorkspaceChunkingStrategy,
        };
        use async_trait::async_trait;
        use tokio_util::sync::CancellationToken;

        struct NoopEmbedder;

        #[async_trait]
        impl EmbeddingProvider for NoopEmbedder {
            fn descriptor(&self) -> EmbeddingProviderDescriptor {
                EmbeddingProviderDescriptor::new("noop", "noop", 8)
            }

            async fn embed(
                &self,
                _request: EmbeddingBatchRequest,
                _cancellation: CancellationToken,
            ) -> Result<EmbeddingBatchResponse, EmbeddingProviderError> {
                Ok(EmbeddingBatchResponse::new(
                    self.descriptor(),
                    Vec::new(),
                ))
            }
        }

        let root = TempDir::new().expect("temp workspace");
        let backend = ManifestWorkspaceBackend::new_deferred(root.path());
        let gate = WorkspaceRetrievalStartupGate::new();
        let host = WorkspaceRetrievalHost::new_deferred(
            a3s_code_core::WorkspaceRetrievalOptions::new(std::sync::Arc::new(NoopEmbedder)),
            WorkspaceChunkingStrategy::Lines,
            gate,
        );
        let services = host
            .workspace_services(Arc::clone(&backend))
            .expect("deferred workspace services");
        assert!(
            services.chunk_catalog().is_some(),
            "host still exposes a lexical catalog"
        );
        assert!(
            backend.persistent_index().is_none(),
            "catalog configure must not attach durable zvec"
        );
        assert!(
            !root.path().join(".a3s-code").join("index").exists(),
            "catalog configure must not create the durable index directory"
        );
    }
}
