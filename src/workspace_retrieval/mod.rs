mod chunking;
mod config;
mod host;
mod local_cpu;
mod provider;
mod rerank;
mod status;

pub(crate) use config::{WorkspaceRetrievalConfig, WorkspaceRetrievalConfigAuthority};
pub(crate) use host::{workspace_services_for_host, WorkspaceRetrievalHost};
pub(crate) use local_cpu::local_cpu_runtime_support;
pub(crate) use provider::build_workspace_retrieval_options;
pub(crate) use status::{workspace_retrieval_status_report, WorkspaceRetrievalStatusReport};

pub(crate) fn embedding_batch_input_limit(config: &WorkspaceRetrievalConfig) -> usize {
    if config.local_cpu.is_some() {
        local_cpu::embedding_batch_input_limit()
    } else {
        a3s_code_core::embedding::EmbeddingExecutorConfig::default().max_batch_inputs
    }
}

pub(crate) trait SessionOptionsWorkspaceRetrievalExt {
    fn with_optional_workspace_retrieval(self, retrieval: Option<&WorkspaceRetrievalHost>) -> Self;
}

impl SessionOptionsWorkspaceRetrievalExt for a3s_code_core::SessionOptions {
    fn with_optional_workspace_retrieval(self, retrieval: Option<&WorkspaceRetrievalHost>) -> Self {
        match retrieval {
            Some(retrieval) => self.with_workspace_retrieval(retrieval.session_options().clone()),
            None => self,
        }
    }
}

impl SessionOptionsWorkspaceRetrievalExt for Option<a3s_code_core::SessionOptions> {
    fn with_optional_workspace_retrieval(self, retrieval: Option<&WorkspaceRetrievalHost>) -> Self {
        self.map(|options| options.with_optional_workspace_retrieval(retrieval))
    }
}

#[cfg(test)]
mod provider_tests;
