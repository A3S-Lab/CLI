use std::sync::Arc;

use a3s_boot::{controller, Result as BootResult};
use serde::Deserialize;

use super::service::KnowledgeService;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KbWorkspaceRequest {
    pub(super) workspace: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KbAddNoteRequest {
    pub(super) workspace: Option<String>,
    pub(super) text: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KbImportRequest {
    pub(super) workspace: Option<String>,
    pub(super) path: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KbSearchRequest {
    pub(super) workspace: Option<String>,
    pub(super) query: String,
}

pub(super) struct KnowledgeController {
    service: Arc<KnowledgeService>,
}

impl KnowledgeController {
    pub(super) fn new(service: Arc<KnowledgeService>) -> Self {
        Self { service }
    }
}

#[controller("/v1/knowledge")]
impl KnowledgeController {
    #[get("/kb")]
    async fn kb_home(
        &self,
        #[query("workspace")] workspace: Option<String>,
    ) -> BootResult<serde_json::Value> {
        self.service.kb_home(workspace).await
    }

    #[post("/kb/notes")]
    async fn add_note(&self, #[body] request: KbAddNoteRequest) -> BootResult<serde_json::Value> {
        self.service.add_note(request).await
    }

    #[post("/kb/import/preview")]
    async fn import_preview(
        &self,
        #[body] request: KbImportRequest,
    ) -> BootResult<serde_json::Value> {
        self.service.import_preview(request).await
    }

    #[post("/kb/import")]
    async fn import(&self, #[body] request: KbImportRequest) -> BootResult<serde_json::Value> {
        self.service.import(request).await
    }

    #[post("/kb/search")]
    async fn search(&self, #[body] request: KbSearchRequest) -> BootResult<serde_json::Value> {
        self.service.search(request).await
    }

    #[post("/kb/ensure")]
    async fn ensure(&self, #[body] request: KbWorkspaceRequest) -> BootResult<serde_json::Value> {
        self.service.ensure(request.workspace).await
    }
}
