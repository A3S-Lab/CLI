use std::sync::Arc;

use a3s_boot::{controller, Result as BootResult};

use super::service::KernelService;
use crate::api::code_web::dto::{
    CreateSessionRequest, KernelSessionResponse, SessionListResponse, SessionResponse,
};

pub(super) struct KernelSessionsController {
    service: Arc<KernelService>,
}

impl KernelSessionsController {
    pub(super) fn new(service: Arc<KernelService>) -> Self {
        Self { service }
    }
}

#[controller("/")]
impl KernelSessionsController {
    #[post("/sessions")]
    async fn create_session(
        &self,
        #[body] request: CreateSessionRequest,
    ) -> BootResult<SessionResponse> {
        self.service.create_session(request).await
    }

    #[get("/v1/kernel/sessions")]
    async fn list_kernel_sessions(&self) -> BootResult<SessionListResponse> {
        self.service.list_sessions().await
    }

    #[post("/v1/kernel/sessions")]
    async fn create_kernel_session(
        &self,
        #[body] request: CreateSessionRequest,
    ) -> BootResult<KernelSessionResponse> {
        self.service.create_kernel_session(request).await
    }

    #[get("/v1/kernel/sessions/{session_id}")]
    async fn get_kernel_session(
        &self,
        #[param("session_id")] session_id: String,
    ) -> BootResult<SessionResponse> {
        self.service.get_session(&session_id).await
    }

    #[patch("/v1/kernel/sessions/{session_id}")]
    async fn update_kernel_session(
        &self,
        #[param("session_id")] session_id: String,
        #[body] request: serde_json::Value,
    ) -> BootResult<SessionResponse> {
        self.service.update_session(&session_id, request).await
    }

    #[delete("/v1/kernel/sessions/{session_id}")]
    async fn delete_kernel_session(
        &self,
        #[param("session_id")] session_id: String,
    ) -> BootResult<()> {
        self.service.delete_session(&session_id).await
    }

    #[get("/v1/kernel/sessions/{session_id}/messages")]
    async fn kernel_session_messages(
        &self,
        #[param("session_id")] session_id: String,
    ) -> BootResult<serde_json::Value> {
        self.service.session_messages(&session_id).await
    }

    #[delete("/v1/kernel/sessions/{session_id}/messages")]
    async fn clear_kernel_session_messages(
        &self,
        #[param("session_id")] session_id: String,
    ) -> BootResult<serde_json::Value> {
        self.service.clear_session_messages(&session_id).await
    }

    #[post("/v1/kernel/sessions/{session_id}/messages")]
    async fn run_kernel_session_message(
        &self,
        #[param("session_id")] session_id: String,
        #[body] request: serde_json::Value,
    ) -> BootResult<serde_json::Value> {
        self.service.run_session_message(&session_id, request).await
    }

    #[get("/v1/kernel/sessions/{session_id}/status")]
    async fn kernel_session_status(
        &self,
        #[param("session_id")] session_id: String,
    ) -> BootResult<serde_json::Value> {
        self.service.session_status(&session_id).await
    }
}
