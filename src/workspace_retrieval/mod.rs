mod config;
mod provider;
mod rerank;
mod status;

pub(crate) use config::{WorkspaceRetrievalConfig, WorkspaceRetrievalConfigAuthority};
pub(crate) use provider::build_workspace_retrieval_options;
pub(crate) use status::format_workspace_retrieval_status;

pub(crate) trait SessionOptionsWorkspaceRetrievalExt {
    fn with_optional_workspace_retrieval(
        self,
        retrieval: Option<&a3s_code_core::WorkspaceRetrievalOptions>,
    ) -> Self;
}

impl SessionOptionsWorkspaceRetrievalExt for a3s_code_core::SessionOptions {
    fn with_optional_workspace_retrieval(
        self,
        retrieval: Option<&a3s_code_core::WorkspaceRetrievalOptions>,
    ) -> Self {
        match retrieval {
            Some(retrieval) => self.with_workspace_retrieval(retrieval.clone()),
            None => self,
        }
    }
}

impl SessionOptionsWorkspaceRetrievalExt for Option<a3s_code_core::SessionOptions> {
    fn with_optional_workspace_retrieval(
        self,
        retrieval: Option<&a3s_code_core::WorkspaceRetrievalOptions>,
    ) -> Self {
        self.map(|options| options.with_optional_workspace_retrieval(retrieval))
    }
}

#[cfg(test)]
mod provider_tests;
