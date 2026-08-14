use std::fmt;
use std::sync::Arc;

use a3s_code_core::{
    ChunkCatalogLimits, ChunkingConfig, ManifestWorkspaceBackend, WorkspaceChunkingStrategy,
    WorkspaceRetrievalOptions, WorkspaceServices,
};
use anyhow::Context;

/// One host-owned retrieval configuration split across its two ownership
/// boundaries: the shared catalog and the per-session semantic runtime.
#[derive(Clone)]
pub(crate) struct WorkspaceRetrievalHost {
    session: WorkspaceRetrievalOptions,
    catalog_strategy: WorkspaceChunkingStrategy,
}

impl WorkspaceRetrievalHost {
    pub(super) fn new(
        session: WorkspaceRetrievalOptions,
        catalog_strategy: WorkspaceChunkingStrategy,
    ) -> Self {
        Self {
            session,
            catalog_strategy,
        }
    }

    pub(super) fn session_options(&self) -> &WorkspaceRetrievalOptions {
        &self.session
    }

    pub(super) fn workspace_services(
        &self,
        backend: Arc<ManifestWorkspaceBackend>,
    ) -> anyhow::Result<Arc<WorkspaceServices>> {
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
            .finish()
    }
}

pub(crate) fn workspace_services_for_host(
    backend: Arc<ManifestWorkspaceBackend>,
    retrieval: Option<&WorkspaceRetrievalHost>,
) -> anyhow::Result<Arc<WorkspaceServices>> {
    match retrieval {
        Some(retrieval) => retrieval.workspace_services(backend),
        None => Ok(WorkspaceServices::local_with_manifest_backend(backend)),
    }
}
