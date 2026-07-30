use std::sync::Arc;

use a3s_boot::{BootError, Result as BootResult};
use serde_json::{json, Value};

use crate::api::code_web::session_runtime::rebuild_code_web_sessions_for_workspace;
use crate::api::code_web::state::CodeWebState;
use crate::config;
use crate::evolution::SkillOptimizationRequest;
use crate::evolution::WorkspaceEvolution;

pub(in crate::api::code_web) struct EvolutionService {
    state: Arc<CodeWebState>,
}

impl EvolutionService {
    pub(in crate::api::code_web) fn new(state: Arc<CodeWebState>) -> Self {
        Self { state }
    }

    pub(in crate::api::code_web) async fn overview(&self) -> BootResult<Value> {
        serialize(self.evolution().overview().await.map_err(internal_error)?)
    }

    pub(in crate::api::code_web) async fn scan(&self) -> BootResult<Value> {
        let evolution = self.evolution();
        let observed = evolution
            .synchronize_memory_store(config::memory_dir())
            .await
            .map_err(internal_error)?;
        let overview = evolution.overview().await.map_err(internal_error)?;
        Ok(json!({
            "observed": observed,
            "overview": overview,
        }))
    }

    pub(in crate::api::code_web) async fn optimizations(&self) -> BootResult<Value> {
        serialize(
            self.evolution()
                .skill_optimizations(None)
                .await
                .map_err(internal_error)?,
        )
    }

    pub(in crate::api::code_web) async fn optimization(&self, run_id: String) -> BootResult<Value> {
        serialize(
            self.evolution()
                .skill_optimization(run_id)
                .await
                .map_err(action_error)?,
        )
    }

    pub(in crate::api::code_web) async fn optimize(
        &self,
        id: String,
        request: Value,
    ) -> BootResult<Value> {
        let request = parse_optimization_request(request)?;
        let session_id = format!(
            "code-web-skill-opt-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        );
        let client = crate::session_llm::resolve_config_llm_client(
            &self.state.code_config_snapshot(),
            &a3s_code_core::SessionOptions::new(),
            &session_id,
        )
        .map_err(BootError::BadRequest)?;
        let evolution = self.evolution();
        let run = evolution
            .queue_skill_optimization(id, request)
            .await
            .map_err(action_error)?;
        let run_id = run.id.clone();
        tokio::spawn(async move {
            if let Err(error) = evolution
                .execute_skill_optimization(run_id.clone(), client)
                .await
            {
                tracing::warn!(%error, %run_id, "background Skill optimization failed");
            }
        });
        Ok(json!({
            "run": run,
            "execution": "background",
            "approvalRequired": true,
        }))
    }

    pub(in crate::api::code_web) async fn adopt_optimization(
        &self,
        run_id: String,
    ) -> BootResult<Value> {
        let _refresh = self.state.evolution_refresh_lock.lock().await;
        let evolution = self.evolution();
        let result = evolution
            .adopt_skill_optimization(run_id)
            .await
            .map_err(action_error)?;
        let rebuilt_sessions = if result.requires_session_reload {
            let rebuilt = rebuild_code_web_sessions_for_workspace(
                &self.state,
                Some(&self.state.default_workspace),
            )
            .await?;
            if !rebuilt.is_empty() {
                evolution
                    .mark_session_assets_activated()
                    .await
                    .map_err(internal_error)?;
            }
            rebuilt
        } else {
            Vec::new()
        };
        Ok(json!({
            "result": result,
            "rebuiltSessions": rebuilt_sessions,
        }))
    }

    pub(in crate::api::code_web) async fn dismiss_optimization(
        &self,
        run_id: String,
    ) -> BootResult<Value> {
        serialize(
            self.evolution()
                .dismiss_skill_optimization(run_id)
                .await
                .map_err(action_error)?,
        )
    }

    pub(in crate::api::code_web) async fn materialize(
        &self,
        id: String,
        request: Value,
    ) -> BootResult<Value> {
        let _refresh = self.state.evolution_refresh_lock.lock().await;
        let force = request
            .get("force")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let evolution = self.evolution();
        let result = evolution
            .materialize(id, force)
            .await
            .map_err(action_error)?;
        let rebuilt_sessions = if result.requires_session_reload {
            let rebuilt = rebuild_code_web_sessions_for_workspace(
                &self.state,
                Some(&self.state.default_workspace),
            )
            .await?;
            if !rebuilt.is_empty() {
                evolution
                    .mark_session_assets_activated()
                    .await
                    .map_err(internal_error)?;
            }
            rebuilt
        } else {
            Vec::new()
        };
        Ok(json!({
            "result": result,
            "rebuiltSessions": rebuilt_sessions,
        }))
    }

    pub(in crate::api::code_web) async fn reject(
        &self,
        id: String,
        request: Value,
    ) -> BootResult<Value> {
        let reason = request
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        serialize(
            self.evolution()
                .reject(id, reason)
                .await
                .map_err(action_error)?,
        )
    }

    pub(in crate::api::code_web) async fn reopen(&self, id: String) -> BootResult<Value> {
        serialize(self.evolution().reopen(id).await.map_err(action_error)?)
    }

    pub(in crate::api::code_web) async fn rollback(
        &self,
        id: String,
        request: Value,
    ) -> BootResult<Value> {
        let _refresh = self.state.evolution_refresh_lock.lock().await;
        let target_version = request
            .get("targetVersion")
            .and_then(Value::as_u64)
            .map(u32::try_from)
            .transpose()
            .map_err(|_| BootError::BadRequest("targetVersion is too large".to_string()))?;
        let evolution = self.evolution();
        let result = evolution
            .rollback(id, target_version)
            .await
            .map_err(action_error)?;
        let rebuilt_sessions = if result.requires_session_reload {
            let rebuilt = rebuild_code_web_sessions_for_workspace(
                &self.state,
                Some(&self.state.default_workspace),
            )
            .await?;
            if !rebuilt.is_empty() {
                evolution
                    .mark_session_assets_activated()
                    .await
                    .map_err(internal_error)?;
            }
            rebuilt
        } else {
            Vec::new()
        };
        Ok(json!({
            "result": result,
            "rebuiltSessions": rebuilt_sessions,
        }))
    }

    fn evolution(&self) -> WorkspaceEvolution {
        WorkspaceEvolution::new(&self.state.default_workspace)
    }
}

fn serialize(value: impl serde::Serialize) -> BootResult<Value> {
    serde_json::to_value(value).map_err(|error| BootError::Internal(error.to_string()))
}

fn internal_error(error: anyhow::Error) -> BootError {
    BootError::Internal(error.to_string())
}

fn action_error(error: anyhow::Error) -> BootError {
    BootError::BadRequest(error.to_string())
}

fn parse_optimization_request(request: Value) -> BootResult<SkillOptimizationRequest> {
    if request.is_null() {
        return Ok(SkillOptimizationRequest::default());
    }
    serde_json::from_value(request).map_err(|error| BootError::BadRequest(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimization_request_defaults_and_rejects_unknown_fields() {
        let request = parse_optimization_request(json!({})).unwrap();
        assert_eq!(request.edit_budget, 3);
        assert_eq!(request.task_count, 4);
        assert!(parse_optimization_request(json!({"autoPublish": true})).is_err());
    }
}
