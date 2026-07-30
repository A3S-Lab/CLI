use std::sync::Arc;

use a3s_boot::{controller, Result as BootResult};

use super::service::EvolutionService;

pub(super) struct EvolutionController {
    service: Arc<EvolutionService>,
}

impl EvolutionController {
    pub(super) fn new(service: Arc<EvolutionService>) -> Self {
        Self { service }
    }
}

#[controller("/v1/evolution")]
impl EvolutionController {
    #[get("/")]
    async fn overview(&self) -> BootResult<serde_json::Value> {
        self.service.overview().await
    }

    #[post("/scan")]
    async fn scan(&self, #[body] _request: serde_json::Value) -> BootResult<serde_json::Value> {
        self.service.scan().await
    }

    #[get("/optimizations")]
    async fn optimizations(&self) -> BootResult<serde_json::Value> {
        self.service.optimizations().await
    }

    #[get("/optimizations/{runId}")]
    async fn optimization(
        &self,
        #[param("runId")] run_id: String,
    ) -> BootResult<serde_json::Value> {
        self.service.optimization(run_id).await
    }

    #[post("/{id}/optimize")]
    async fn optimize(
        &self,
        #[param("id")] id: String,
        #[body] request: serde_json::Value,
    ) -> BootResult<serde_json::Value> {
        self.service.optimize(id, request).await
    }

    #[post("/optimizations/{runId}/adopt")]
    async fn adopt_optimization(
        &self,
        #[param("runId")] run_id: String,
        #[body] _request: serde_json::Value,
    ) -> BootResult<serde_json::Value> {
        self.service.adopt_optimization(run_id).await
    }

    #[post("/optimizations/{runId}/dismiss")]
    async fn dismiss_optimization(
        &self,
        #[param("runId")] run_id: String,
        #[body] _request: serde_json::Value,
    ) -> BootResult<serde_json::Value> {
        self.service.dismiss_optimization(run_id).await
    }

    #[post("/{id}/materialize")]
    async fn materialize(
        &self,
        #[param("id")] id: String,
        #[body] request: serde_json::Value,
    ) -> BootResult<serde_json::Value> {
        self.service.materialize(id, request).await
    }

    #[post("/{id}/reject")]
    async fn reject(
        &self,
        #[param("id")] id: String,
        #[body] request: serde_json::Value,
    ) -> BootResult<serde_json::Value> {
        self.service.reject(id, request).await
    }

    #[post("/{id}/reopen")]
    async fn reopen(
        &self,
        #[param("id")] id: String,
        #[body] _request: serde_json::Value,
    ) -> BootResult<serde_json::Value> {
        self.service.reopen(id).await
    }

    #[post("/{id}/rollback")]
    async fn rollback(
        &self,
        #[param("id")] id: String,
        #[body] request: serde_json::Value,
    ) -> BootResult<serde_json::Value> {
        self.service.rollback(id, request).await
    }
}
