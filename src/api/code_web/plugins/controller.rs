use std::sync::Arc;

use a3s_boot::{controller, Result as BootResult};
use serde::Deserialize;

use super::service::PluginsService;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PluginToggleRequest {
    pub(super) enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PluginReloadRequest {
    pub(super) rebuild_sessions: Option<bool>,
}

impl Default for PluginReloadRequest {
    fn default() -> Self {
        Self {
            rebuild_sessions: Some(true),
        }
    }
}

pub(super) struct PluginsController {
    service: Arc<PluginsService>,
}

impl PluginsController {
    pub(super) fn new(service: Arc<PluginsService>) -> Self {
        Self { service }
    }
}

#[controller("/v1/plugins")]
impl PluginsController {
    #[get("/")]
    async fn list(
        &self,
        #[query("workspace")] workspace: Option<String>,
    ) -> BootResult<serde_json::Value> {
        self.service.list(workspace).await
    }

    #[post("/{name}/enabled")]
    async fn set_enabled(
        &self,
        #[param("name")] name: String,
        #[body] request: PluginToggleRequest,
    ) -> BootResult<serde_json::Value> {
        self.service.set_enabled(&name, request).await
    }

    #[post("/reload")]
    async fn reload(&self, #[body] request: PluginReloadRequest) -> BootResult<serde_json::Value> {
        self.service.reload(request).await
    }
}
