use std::sync::Arc;

use a3s::plugin_manager::{
    PluginApplyRequest, PluginEnablementApplyRequest, PluginEnablementPlanRequest,
    PluginPlanRequest,
};
use a3s_boot::{controller, BootResponse, Result as BootResult};
use serde::Deserialize;
use serde_json::Value;

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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PluginFlowResolveRequest {
    pub(super) design_json: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PluginFlowRunRequest {
    pub(super) design_json: String,
    #[serde(default)]
    pub(super) input: Option<Value>,
    #[serde(default)]
    pub(super) run_id: Option<String>,
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

    #[get("/activities")]
    async fn activities(&self) -> BootResult<serde_json::Value> {
        self.service.activities()
    }

    #[get("/flows")]
    async fn flows(&self) -> BootResult<serde_json::Value> {
        self.service.flows()
    }

    #[post("/flows/resolve")]
    async fn resolve_flow(
        &self,
        #[body] request: PluginFlowResolveRequest,
    ) -> BootResult<serde_json::Value> {
        self.service.resolve_flow(request)
    }

    #[post("/flows/run")]
    async fn run_flow(
        &self,
        #[body] request: PluginFlowRunRequest,
    ) -> BootResult<serde_json::Value> {
        self.service.run_flow(request).await
    }

    #[get("/flows/runs")]
    async fn flow_runs(
        &self,
        #[query("limit")] limit: Option<usize>,
    ) -> BootResult<serde_json::Value> {
        self.service.flow_runs(limit).await
    }

    #[get("/flows/runs/{run_id}")]
    async fn flow_run(&self, #[param("run_id")] run_id: String) -> BootResult<serde_json::Value> {
        self.service.flow_run(&run_id).await
    }

    #[get("/flows/runs/{run_id}/events")]
    async fn flow_run_events(
        &self,
        #[param("run_id")] run_id: String,
    ) -> BootResult<serde_json::Value> {
        self.service.flow_run_events(&run_id).await
    }

    #[get("/activities/{key}/document", raw)]
    async fn activity_document(
        &self,
        #[param("key")] key: String,
        #[query("generation")] generation: u64,
        #[query("revision")] revision: String,
    ) -> BootResult<BootResponse> {
        self.service.activity_document(&key, generation, &revision)
    }

    #[get("/activities/{key}")]
    async fn activity_content(&self, #[param("key")] key: String) -> BootResult<serde_json::Value> {
        self.service.activity_content(&key)
    }

    #[get("/marketplace")]
    async fn marketplace(&self) -> BootResult<serde_json::Value> {
        self.service.marketplace().await
    }

    #[post("/operations/plan")]
    async fn plan_operation(
        &self,
        #[body] request: PluginPlanRequest,
    ) -> BootResult<serde_json::Value> {
        self.service.plan_operation(request).await
    }

    #[post("/operations/apply")]
    async fn apply_operation(
        &self,
        #[body] request: PluginApplyRequest,
    ) -> BootResult<serde_json::Value> {
        self.service.apply_operation(request).await
    }

    #[post("/packages/enablement/plan")]
    async fn plan_package_enablement(
        &self,
        #[body] request: PluginEnablementPlanRequest,
    ) -> BootResult<serde_json::Value> {
        self.service.plan_package_enablement(request).await
    }

    #[post("/packages/enablement/apply")]
    async fn apply_package_enablement(
        &self,
        #[body] request: PluginEnablementApplyRequest,
    ) -> BootResult<serde_json::Value> {
        self.service.apply_package_enablement(request).await
    }
}
