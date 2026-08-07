use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use a3s::plugin_manager::{
    PluginApplyRequest, PluginEnablementApplyRequest, PluginEnablementPlanRequest, PluginManager,
    PluginManagerError, PluginPlanRequest,
};
use a3s_boot::{BootError, Result as BootResult};
use serde_json::{json, Value};

use super::controller::{
    PluginFlowResolveRequest, PluginFlowRunRequest, PluginReloadRequest, PluginToggleRequest,
};
use crate::api::code_web::session_runtime::rebuild_code_web_sessions;
use crate::api::code_web::state::CodeWebState;
use crate::tui::skills::{
    agent_skill_dirs, count_skill_files, load_disabled_skills, load_skills, save_disabled_skills,
};

pub(in crate::api::code_web) struct PluginsService {
    state: Arc<CodeWebState>,
    manager: Arc<PluginManager>,
}

impl PluginsService {
    pub(in crate::api::code_web) fn new(state: Arc<CodeWebState>) -> Self {
        let manager = state.plugin_manager();
        Self { state, manager }
    }

    pub(in crate::api::code_web) async fn list(
        &self,
        workspace: Option<String>,
    ) -> BootResult<Value> {
        Ok(self.snapshot(workspace, Vec::new(), false))
    }

    pub(in crate::api::code_web) async fn set_enabled(
        &self,
        raw_name: &str,
        request: PluginToggleRequest,
    ) -> BootResult<Value> {
        let name = normalize_skill_name(raw_name)?;
        let dirs = self.default_skill_dirs();
        let skills = load_skills(&dirs);
        if !skills.iter().any(|(skill_name, _)| skill_name == &name) {
            return Err(BootError::NotFound(format!(
                "skill/plugin `{name}` was not found"
            )));
        }

        let mut disabled = load_disabled_skills();
        let enabled = apply_enabled(&mut disabled, &name, request.enabled);
        save_disabled_skills(&disabled);

        let mut response = self.snapshot(None, Vec::new(), false);
        if let Some(object) = response.as_object_mut() {
            object.insert(
                "updated".to_string(),
                json!({
                    "name": name,
                    "enabled": enabled,
                }),
            );
        }
        Ok(response)
    }

    pub(in crate::api::code_web) async fn reload(
        &self,
        request: PluginReloadRequest,
    ) -> BootResult<Value> {
        let rebuild_sessions = request.rebuild_sessions.unwrap_or(true);
        let rebuilt_sessions = if rebuild_sessions {
            self.rebuild_sessions().await?
        } else {
            Vec::new()
        };

        Ok(self.snapshot(None, rebuilt_sessions, true))
    }

    pub(in crate::api::code_web) fn activities(&self) -> BootResult<Value> {
        let Some(registry) = self.state.use_registry() else {
            return Ok(json!({
                "schemaVersion": 1,
                "available": false,
                "generation": 0,
                "revision": "",
                "items": [],
            }));
        };
        let mut value = serde_json::to_value(registry.activity_catalog())
            .map_err(|error| BootError::Internal(error.to_string()))?;
        if let Some(object) = value.as_object_mut() {
            object.insert("available".to_string(), Value::Bool(true));
        }
        Ok(value)
    }

    pub(in crate::api::code_web) fn flows(&self) -> BootResult<Value> {
        let Some(registry) = self.state.use_registry() else {
            return Ok(json!({
                "schemaVersion": 1,
                "available": false,
                "generation": 0,
                "revision": "",
                "items": [],
            }));
        };
        let catalog = registry.flow_catalog();
        let available = catalog.is_available();
        let mut value = serde_json::to_value(catalog)
            .map_err(|error| BootError::Internal(error.to_string()))?;
        if let Some(object) = value.as_object_mut() {
            object.insert("available".to_string(), Value::Bool(available));
        }
        Ok(value)
    }

    pub(in crate::api::code_web) fn resolve_flow(
        &self,
        request: PluginFlowResolveRequest,
    ) -> BootResult<Value> {
        let parsed = crate::use_registry::flow::parse_flow_design(&request.design_json)
            .map_err(|error| BootError::BadRequest(error.to_string()))?;
        if parsed.installed_flow.is_none() {
            return Err(BootError::Conflict(
                "workflow design has no installedFlow identity; bind an exact A3S Use Flow before deployment"
                    .to_string(),
            ));
        }
        let registry = self.state.use_registry().ok_or_else(|| {
            BootError::ServiceUnavailable("A3S Use is not installed or ready".to_string())
        })?;
        let catalog = registry.flow_catalog();
        if !catalog.is_available() {
            return Err(BootError::ServiceUnavailable(
                "A3S Use is not installed or ready".to_string(),
            ));
        }
        let resolved = catalog
            .resolve_design(&parsed)
            .map_err(|error| BootError::Conflict(error.to_string()))?;
        Ok(json!({
            "schemaVersion": 1,
            "available": true,
            "flow": resolved.to_json(),
        }))
    }

    pub(in crate::api::code_web) async fn run_flow(
        &self,
        request: PluginFlowRunRequest,
    ) -> BootResult<Value> {
        let parsed = crate::use_registry::flow::parse_flow_design(&request.design_json)
            .map_err(|error| BootError::BadRequest(error.to_string()))?;
        if parsed.installed_flow.is_none() {
            return Err(BootError::Conflict(
                "workflow design has no installedFlow identity; bind an exact A3S Use Flow before run"
                    .to_string(),
            ));
        }
        let registry = self.state.use_registry().ok_or_else(|| {
            BootError::ServiceUnavailable("A3S Use is not installed or ready".to_string())
        })?;
        let catalog = registry.flow_catalog();
        if !catalog.is_available() {
            return Err(BootError::ServiceUnavailable(
                "A3S Use is not installed or ready".to_string(),
            ));
        }
        let runtime = crate::use_registry::flow_runtime::InstalledFlowRuntime::new(
            self.state.default_workspace.clone(),
        );
        let run = runtime
            .run(
                &catalog,
                &parsed,
                request.input.unwrap_or_else(|| json!({})),
                request.run_id,
            )
            .await
            .map_err(flow_runtime_error)?;
        Ok(json!({
            "schemaVersion": 1,
            "run": run,
        }))
    }

    pub(in crate::api::code_web) async fn flow_runs(
        &self,
        limit: Option<usize>,
    ) -> BootResult<Value> {
        let runtime = crate::use_registry::flow_runtime::InstalledFlowRuntime::new(
            self.state.default_workspace.clone(),
        );
        let runs = runtime.list(limit).await.map_err(flow_runtime_error)?;
        Ok(json!({
            "schemaVersion": 1,
            "items": runs,
        }))
    }

    pub(in crate::api::code_web) async fn flow_run(&self, run_id: &str) -> BootResult<Value> {
        let runtime = crate::use_registry::flow_runtime::InstalledFlowRuntime::new(
            self.state.default_workspace.clone(),
        );
        let run = runtime.get(run_id).await.map_err(flow_runtime_error)?;
        Ok(json!({
            "schemaVersion": 1,
            "run": run,
        }))
    }

    pub(in crate::api::code_web) async fn flow_run_events(
        &self,
        run_id: &str,
    ) -> BootResult<Value> {
        let runtime = crate::use_registry::flow_runtime::InstalledFlowRuntime::new(
            self.state.default_workspace.clone(),
        );
        let events = runtime.events(run_id).await.map_err(flow_runtime_error)?;
        Ok(json!({
            "schemaVersion": 1,
            "runId": run_id,
            "items": events,
        }))
    }

    pub(in crate::api::code_web) fn activity_content(&self, key: &str) -> BootResult<Value> {
        let key = normalize_activity_key(key)?;
        let registry = self.state.use_registry().ok_or_else(|| {
            BootError::ServiceUnavailable("A3S Use is not installed or ready".to_string())
        })?;
        let content = registry.activity_content(&key).ok_or_else(|| {
            BootError::NotFound(format!(
                "enabled Activity Bar contribution `{key}` was not found"
            ))
        })?;
        serde_json::to_value(content).map_err(|error| BootError::Internal(error.to_string()))
    }

    pub(in crate::api::code_web) async fn marketplace(&self) -> BootResult<Value> {
        let installed = self
            .state
            .use_registry()
            .map(|registry| registry.package_statuses())
            .unwrap_or_default();
        let snapshot = self
            .manager
            .marketplace(&installed)
            .await
            .map_err(manager_error)?;
        serde_json::to_value(snapshot).map_err(|error| BootError::Internal(error.to_string()))
    }

    pub(in crate::api::code_web) async fn plan_operation(
        &self,
        request: PluginPlanRequest,
    ) -> BootResult<Value> {
        self.manager
            .plan_operation(&request)
            .await
            .map_err(manager_error)
    }

    pub(in crate::api::code_web) async fn apply_operation(
        &self,
        request: PluginApplyRequest,
    ) -> BootResult<Value> {
        self.manager
            .apply_confirmed_operation(&request)
            .await
            .map_err(manager_error)
    }

    pub(in crate::api::code_web) async fn plan_package_enablement(
        &self,
        request: PluginEnablementPlanRequest,
    ) -> BootResult<Value> {
        self.manager
            .plan_package_enablement(&request)
            .await
            .map_err(manager_error)
    }

    pub(in crate::api::code_web) async fn apply_package_enablement(
        &self,
        request: PluginEnablementApplyRequest,
    ) -> BootResult<Value> {
        self.manager
            .apply_confirmed_package_enablement(&request)
            .await
            .map_err(manager_error)
    }

    fn snapshot(
        &self,
        workspace: Option<String>,
        rebuilt_sessions: Vec<Value>,
        reloaded: bool,
    ) -> Value {
        let workspace = self.workspace_from_request(workspace);
        let workspace_text = workspace.display().to_string();
        let dirs = agent_skill_dirs(&workspace_text);
        let disabled = load_disabled_skills();
        let mut sources_by_name: BTreeMap<String, Vec<Value>> = BTreeMap::new();

        let dir_summaries = dirs
            .iter()
            .map(|dir| {
                let dir_skills = load_skills(std::slice::from_ref(dir));
                for (name, _) in &dir_skills {
                    sources_by_name
                        .entry(name.clone())
                        .or_default()
                        .push(json!({
                            "path": dir.display().to_string(),
                        }));
                }
                json!({
                    "path": dir.display().to_string(),
                    "exists": dir.is_dir(),
                    "itemCount": count_skill_files(std::slice::from_ref(dir)),
                })
            })
            .collect::<Vec<_>>();

        let skills = load_skills(&dirs);
        let total = skills.len();
        let items = skills
            .into_iter()
            .map(|(name, description)| {
                let enabled = !disabled.contains(&name);
                json!({
                    "name": name,
                    "command": format!("${name}"),
                    "description": description,
                    "enabled": enabled,
                    "sources": sources_by_name.remove(&name).unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();
        let disabled_count = items
            .iter()
            .filter(|item| item.get("enabled").and_then(Value::as_bool) == Some(false))
            .count();

        json!({
            "workspaceRoot": workspace.display().to_string(),
            "dirs": dir_summaries,
            "items": items,
            "total": total,
            "enabledCount": total.saturating_sub(disabled_count),
            "disabledCount": disabled_count,
            "reloaded": reloaded,
            "reloadedAt": if reloaded { Some(chrono::Utc::now().to_rfc3339()) } else { None },
            "rebuiltSessions": rebuilt_sessions,
        })
    }

    async fn rebuild_sessions(&self) -> BootResult<Vec<Value>> {
        rebuild_code_web_sessions(self.state.as_ref()).await
    }

    fn workspace_from_request(&self, workspace: Option<String>) -> PathBuf {
        workspace
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.state.default_workspace.clone())
    }

    fn default_skill_dirs(&self) -> Vec<PathBuf> {
        agent_skill_dirs(&self.state.default_workspace.display().to_string())
    }
}

fn manager_error(error: PluginManagerError) -> BootError {
    match error {
        PluginManagerError::InvalidRequest(message) => BootError::BadRequest(message),
        PluginManagerError::Timeout(message) => BootError::GatewayTimeout(message),
        PluginManagerError::OperationFailed(message) => BootError::Conflict(message),
        PluginManagerError::Upstream(message) => BootError::BadGateway(message),
        PluginManagerError::Infrastructure(message) => BootError::Internal(message),
    }
}

fn flow_runtime_error(
    error: crate::use_registry::flow_runtime::InstalledFlowRuntimeError,
) -> BootError {
    use crate::use_registry::flow_runtime::InstalledFlowRuntimeError;
    match error {
        InstalledFlowRuntimeError::InvalidRequest(message) => BootError::BadRequest(message),
        InstalledFlowRuntimeError::Conflict(message) => BootError::Conflict(message),
        InstalledFlowRuntimeError::NotFound(message) => BootError::NotFound(message),
        InstalledFlowRuntimeError::Execution(message) => BootError::Conflict(message),
        InstalledFlowRuntimeError::State(message) => BootError::Internal(message),
    }
}

fn normalize_activity_key(value: &str) -> BootResult<String> {
    let value = value.trim();
    let segments = value.split(':').collect::<Vec<_>>();
    if segments.len() != 2 || !segments.into_iter().all(valid_segment) {
        return Err(BootError::BadRequest(
            "invalid Activity Bar contribution key".to_string(),
        ));
    }
    Ok(value.to_string())
}

fn valid_segment(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn normalize_skill_name(raw_name: &str) -> BootResult<String> {
    let name = raw_name.trim().trim_start_matches('/').trim();
    if name.is_empty() {
        return Err(BootError::BadRequest(
            "skill/plugin name is required".to_string(),
        ));
    }
    Ok(name.to_string())
}

fn apply_enabled(disabled: &mut HashSet<String>, name: &str, enabled: Option<bool>) -> bool {
    let target_enabled = enabled.unwrap_or_else(|| disabled.contains(name));
    if target_enabled {
        disabled.remove(name);
    } else {
        disabled.insert(name.to_string());
    }
    target_enabled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_enabled_matches_plugin_toggle_semantics() {
        let mut disabled = HashSet::from(["reviewer".to_string()]);
        assert!(apply_enabled(&mut disabled, "reviewer", None));
        assert!(!disabled.contains("reviewer"));

        assert!(!apply_enabled(&mut disabled, "reviewer", None));
        assert!(disabled.contains("reviewer"));

        assert!(apply_enabled(&mut disabled, "reviewer", Some(true)));
        assert!(!disabled.contains("reviewer"));

        assert!(!apply_enabled(&mut disabled, "reviewer", Some(false)));
        assert!(disabled.contains("reviewer"));
    }

    #[tokio::test]
    async fn service_reuses_the_state_manager_policy_and_lock_boundary() {
        let temporary = tempfile::tempdir().expect("create Plugin Manager fixture");
        let workspace = temporary.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("create fixture workspace");
        let config_path = temporary.path().join("config.acl");
        let code_config = a3s_code_core::CodeConfig::from_acl(
            r#"
                default_model = "openai/test-model"
                providers "openai" {
                  apiKey = "sk-test"
                  baseUrl = "https://example.com/v1"
                  models "test-model" {}
                }
            "#,
        )
        .expect("parse fixture config");
        let agent = Arc::new(
            a3s_code_core::Agent::from_config(code_config.clone())
                .await
                .expect("create fixture agent"),
        );
        let repository = Arc::new(
            crate::api::code_web::session_store::CodeWebSessionRepository::open(
                temporary.path().join("sessions"),
            )
            .await
            .expect("open fixture session repository"),
        );
        let manager = Arc::new(
            PluginManager::from_host_with_policy(
                &config_path,
                &workspace,
                a3s::plugin_manager::PluginManagerPolicy {
                    offline: true,
                    authorization: a3s::plugin_manager::PluginAuthorizationPolicy::default(),
                },
            )
            .expect("create fixture Plugin Manager"),
        );
        let state = Arc::new(CodeWebState::new(
            agent,
            config_path,
            workspace,
            code_config,
            repository,
            Arc::clone(&manager),
        ));

        let service = PluginsService::new(state);

        assert!(Arc::ptr_eq(&service.manager, &manager));
        assert!(service.manager.policy().offline);
    }
}
