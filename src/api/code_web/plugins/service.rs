use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use a3s::components::{
    CodePluginUiCandidate, CodePluginUiCandidateBroker, CodePluginUiCandidateDecision,
    CodePluginUiCandidateError, CodePluginUiStateError, CodePluginUiStateStore,
};
use a3s::plugin_manager::{
    PluginApplyRequest, PluginEnablementApplyRequest, PluginEnablementPlanRequest, PluginManager,
    PluginManagerError, PluginPlanRequest,
};
use a3s_boot::{BootError, BootResponse, Result as BootResult};
use serde_json::{json, Value};

use super::activity_document;
use super::controller::{
    PluginActivityStateRequest, PluginFlowResolveRequest, PluginFlowRunRequest,
    PluginReloadRequest, PluginToggleRequest,
};
use crate::api::code_web::session_runtime::rebuild_code_web_sessions;
use crate::api::code_web::state::CodeWebState;
use crate::tui::skills::{
    agent_skill_dirs, count_skill_files, load_disabled_skills, load_skills, save_disabled_skills,
};
use crate::use_registry::{UseActivityContentLookup, UseActivityStateAuthorityLookup};

pub(in crate::api::code_web) struct PluginsService {
    state: Arc<CodeWebState>,
    manager: Arc<PluginManager>,
    ui_state: CodePluginUiStateStore,
    ui_candidates: CodePluginUiCandidateBroker,
}

impl PluginsService {
    pub(in crate::api::code_web) fn new(state: Arc<CodeWebState>) -> Self {
        let manager = state.plugin_manager();
        let ui_state = manager.plugin_ui_state_store();
        let ui_candidates = manager.plugin_ui_candidate_broker();
        Self {
            state,
            manager,
            ui_state,
            ui_candidates,
        }
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
        let catalog = registry.activity_catalog();
        let generation = catalog.generation;
        let revision = catalog.revision.clone();
        let mut value = serde_json::to_value(catalog)
            .map_err(|error| BootError::Internal(error.to_string()))?;
        if let Some(object) = value.as_object_mut() {
            object.insert("available".to_string(), Value::Bool(true));
        }
        if let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
            for item in items {
                if item.get("enabled").and_then(Value::as_bool) != Some(true) {
                    continue;
                }
                let Some(key) = item.get("key").and_then(Value::as_str) else {
                    return Err(BootError::Internal(
                        "Activity catalog item has no stable key".to_string(),
                    ));
                };
                let document_url = activity_document::url(key, generation, &revision);
                if let Some(object) = item.as_object_mut() {
                    object.insert("documentUrl".to_string(), Value::String(document_url));
                }
            }
        }
        Ok(value)
    }

    pub(in crate::api::code_web) async fn activity_candidates(&self) -> BootResult<Value> {
        activity_candidate_catalog(self.ui_candidates.pending().await)
    }

    pub(in crate::api::code_web) async fn activity_candidate_document(
        &self,
        token: &str,
    ) -> BootResult<BootResponse> {
        self.ui_candidates
            .content(token)
            .await
            .map(|content| activity_document::candidate_response(&content))
            .map_err(ui_candidate_error)
    }

    pub(in crate::api::code_web) async fn decide_activity_candidate(
        &self,
        token: &str,
        decision: CodePluginUiCandidateDecision,
    ) -> BootResult<Value> {
        self.ui_candidates
            .decide(token, decision)
            .await
            .map_err(ui_candidate_error)?;
        Ok(json!({
            "schemaVersion": 1,
            "accepted": true,
            "decision": decision,
        }))
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

    pub(in crate::api::code_web) fn activity_document(
        &self,
        key: &str,
        generation: u64,
        revision: &str,
    ) -> BootResult<BootResponse> {
        let key = normalize_activity_key(key)?;
        if generation == 0 || !activity_document::valid_registry_revision(revision) {
            return Err(BootError::BadRequest(
                "invalid Activity document Registry identity".to_string(),
            ));
        }
        let registry = self.state.use_registry().ok_or_else(|| {
            BootError::ServiceUnavailable("A3S Use is not installed or ready".to_string())
        })?;
        match registry.activity_content_at(&key, generation, revision) {
            UseActivityContentLookup::Current(content) => Ok(activity_document::response(&content)),
            UseActivityContentLookup::Unavailable => Err(BootError::ServiceUnavailable(
                "A3S Use Registry is still converging".to_string(),
            )),
            UseActivityContentLookup::Stale => Err(BootError::Gone(format!(
                "Activity Bar contribution `{key}` document generation is no longer current"
            ))),
            UseActivityContentLookup::Missing => Err(BootError::NotFound(format!(
                "enabled Activity Bar contribution `{key}` was not found"
            ))),
        }
    }

    pub(in crate::api::code_web) async fn activity_state(
        &self,
        key: &str,
        generation: u64,
        revision: &str,
        request: PluginActivityStateRequest,
    ) -> BootResult<Value> {
        let key = normalize_activity_key(key)?;
        if generation == 0 || !activity_document::valid_registry_revision(revision) {
            return Err(BootError::BadRequest(
                "invalid Activity state Registry identity".to_string(),
            ));
        }
        let registry = self.state.use_registry().ok_or_else(|| {
            BootError::ServiceUnavailable("A3S Use is not installed or ready".to_string())
        })?;
        let authority = match registry
            .activity_state_authority_at(&key, generation, revision)
            .await
            .map_err(|error| BootError::Internal(error.to_string()))?
        {
            UseActivityStateAuthorityLookup::Current(authority) => authority,
            UseActivityStateAuthorityLookup::Unavailable => {
                return Err(BootError::ServiceUnavailable(
                    "A3S Use Registry is still converging".to_string(),
                ));
            }
            UseActivityStateAuthorityLookup::Stale => {
                return Err(BootError::Gone(format!(
                    "Activity Bar contribution `{key}` state generation is no longer current"
                )));
            }
            UseActivityStateAuthorityLookup::Missing => {
                return Err(BootError::NotFound(format!(
                    "enabled Activity Bar contribution `{key}` was not found"
                )));
            }
        };
        let scope = self.manager.plugin_ui_state_scope();
        let result = match request {
            PluginActivityStateRequest::Get { key } => {
                let value = self
                    .ui_state
                    .get(&scope, &authority.package_id, &authority.surface_id, &key)
                    .await
                    .map_err(ui_state_error)?;
                json!({
                    "schemaVersion": 1,
                    "operation": "get",
                    "found": value.is_some(),
                    "value": value,
                })
            }
            PluginActivityStateRequest::Set { key, value } => {
                self.ui_state
                    .set(
                        &scope,
                        &authority.package_id,
                        &authority.surface_id,
                        &key,
                        value,
                    )
                    .await
                    .map_err(ui_state_error)?;
                json!({
                    "schemaVersion": 1,
                    "operation": "set",
                    "stored": true,
                })
            }
            PluginActivityStateRequest::Delete { key } => {
                let deleted = self
                    .ui_state
                    .delete(&scope, &authority.package_id, &authority.surface_id, &key)
                    .await
                    .map_err(ui_state_error)?;
                json!({
                    "schemaVersion": 1,
                    "operation": "delete",
                    "deleted": deleted,
                })
            }
            PluginActivityStateRequest::Clear => {
                let cleared = self
                    .ui_state
                    .clear_surface(&scope, &authority.package_id, &authority.surface_id)
                    .await
                    .map_err(ui_state_error)?;
                json!({
                    "schemaVersion": 1,
                    "operation": "clear",
                    "cleared": cleared,
                })
            }
        };
        // Keep the exact generation lease alive until the state operation is
        // complete. Retirement cannot pass its drain boundary before this.
        drop(authority);
        Ok(result)
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

fn ui_state_error(error: CodePluginUiStateError) -> BootError {
    match error {
        CodePluginUiStateError::InvalidKey => BootError::BadRequest(error.to_string()),
        CodePluginUiStateError::ValueTooLarge | CodePluginUiStateError::CapacityExceeded => {
            BootError::PayloadTooLarge(error.to_string())
        }
        CodePluginUiStateError::InvalidIdentity
        | CodePluginUiStateError::Corrupt
        | CodePluginUiStateError::UnsafePath
        | CodePluginUiStateError::Io(_)
        | CodePluginUiStateError::Worker(_) => BootError::Internal(error.to_string()),
    }
}

fn activity_candidate_catalog(candidates: Vec<CodePluginUiCandidate>) -> BootResult<Value> {
    let mut items =
        serde_json::to_value(candidates).map_err(|error| BootError::Internal(error.to_string()))?;
    for item in items.as_array_mut().into_iter().flatten() {
        let token = item
            .get("token")
            .and_then(Value::as_str)
            .ok_or_else(|| BootError::Internal("UI candidate has no stable token".to_string()))?
            .to_string();
        let document_url = activity_document::candidate_url(&token);
        item.as_object_mut()
            .ok_or_else(|| BootError::Internal("UI candidate is not an object".to_string()))?
            .insert("documentUrl".to_string(), Value::String(document_url));
    }
    Ok(json!({
        "schemaVersion": 1,
        "items": items,
    }))
}

fn ui_candidate_error(error: CodePluginUiCandidateError) -> BootError {
    match error {
        CodePluginUiCandidateError::InvalidToken => BootError::BadRequest(error.to_string()),
        CodePluginUiCandidateError::NotFound => BootError::Gone(error.to_string()),
        CodePluginUiCandidateError::AlreadyDecided => BootError::Conflict(error.to_string()),
    }
}

fn normalize_activity_key(value: &str) -> BootResult<String> {
    let value = value.trim();
    let Some((route, id)) = value.split_once(':') else {
        return Err(BootError::BadRequest(
            "invalid Activity Bar contribution key".to_string(),
        ));
    };
    if id.contains(':') || !valid_route_segment(route) || !valid_segment(id) {
        return Err(BootError::BadRequest(
            "invalid Activity Bar contribution key".to_string(),
        ));
    }
    Ok(value.to_string())
}

fn valid_route_segment(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
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
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use a3s::components::ComponentPaths;
    use a3s::plugin_manager::{PluginAuthorizationPolicy, PluginManagerPolicy};
    use a3s::registry::RegistryStore;
    use a3s_use_core::{PlanScope, PlanScopeKind};
    use a3s_use_extension::ExtensionPaths;

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

    #[test]
    fn candidate_catalog_exposes_only_path_free_exact_evidence() {
        let token = "a".repeat(64);
        let catalog = activity_candidate_catalog(vec![CodePluginUiCandidate {
            token: token.clone(),
            scope: PlanScope {
                kind: PlanScopeKind::User,
                id: "user/current".to_string(),
            },
            package_id: "acme/research".to_string(),
            surface_id: "review".to_string(),
            generation: 8,
            title: "Research review".to_string(),
            asset_digest: format!("sha256:{}", "b".repeat(64)),
        }])
        .expect("serialize candidate catalog");

        assert_eq!(catalog["schemaVersion"], 1);
        let item = catalog["items"][0]
            .as_object()
            .expect("candidate catalog item");
        assert_eq!(
            item.get("documentUrl"),
            Some(&json!(format!(
                "/api/v1/plugins/activities/candidates/{token}/document"
            )))
        );
        assert_eq!(
            item.get("scope"),
            Some(&json!({"kind": "user", "id": "user/current"}))
        );
        assert_eq!(item.get("generation"), Some(&json!(8)));
        assert_eq!(
            item.get("assetDigest"),
            Some(&json!(format!("sha256:{}", "b".repeat(64))))
        );
        assert!(!item.contains_key("packageRoot"));
        assert!(!item.contains_key("entry"));
        assert!(!item.contains_key("styles"));
        assert!(!item.contains_key("scripts"));
    }

    #[test]
    fn stale_candidate_tokens_map_to_http_gone() {
        assert!(matches!(
            ui_candidate_error(CodePluginUiCandidateError::NotFound),
            BootError::Gone(_)
        ));
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

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn activity_state_api_enforces_exact_identity_limits_and_corruption_boundaries() {
        let _lock = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temporary = tempfile::tempdir().expect("create Activity state fixture");
        let _environment = TestComponentEnvironment::install(temporary.path());
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
        let component_paths =
            ComponentPaths::from_env_at(&workspace).expect("resolve fixture paths");
        let registry_store = RegistryStore::from_component_paths(&component_paths, true);
        let manager = Arc::new(PluginManager::new_with_policy(
            config_path.clone(),
            workspace.clone(),
            component_paths.clone(),
            registry_store,
            PluginManagerPolicy {
                offline: true,
                authorization: PluginAuthorizationPolicy::default(),
            },
        ));
        let state = Arc::new(CodeWebState::new(
            agent,
            config_path,
            workspace,
            code_config,
            repository,
            manager,
        ));
        let extension_paths = ExtensionPaths::new(
            temporary.path().join("use-data"),
            temporary.path().join("use-state"),
        );
        state
            .install_use_registry(
                crate::use_registry::UseRegistryHandle::for_test_activity(extension_paths.clone()),
                None,
            )
            .await;
        let service = PluginsService::new(Arc::clone(&state));
        let revision = "b".repeat(64);

        assert!(matches!(
            service
                .activity_state(
                    "science:research",
                    1,
                    &revision,
                    PluginActivityStateRequest::Get {
                        key: "draft/current".to_string(),
                    },
                )
                .await,
            Err(BootError::Gone(_))
        ));
        assert!(matches!(
            service
                .activity_state(
                    "science:missing",
                    2,
                    &revision,
                    PluginActivityStateRequest::Get {
                        key: "draft/current".to_string(),
                    },
                )
                .await,
            Err(BootError::NotFound(_))
        ));

        service
            .activity_state(
                "science:research",
                2,
                &revision,
                PluginActivityStateRequest::Set {
                    key: "draft/current".to_string(),
                    value: json!({"query": "CRISPR"}),
                },
            )
            .await
            .expect("store exact-generation state");
        let loaded = service
            .activity_state(
                "science:research",
                2,
                &revision,
                PluginActivityStateRequest::Get {
                    key: "draft/current".to_string(),
                },
            )
            .await
            .expect("load exact-generation state");
        assert_eq!(loaded["found"], true);
        assert_eq!(loaded["value"], json!({"query": "CRISPR"}));

        assert!(matches!(
            service
                .activity_state(
                    "science:research",
                    2,
                    &revision,
                    PluginActivityStateRequest::Set {
                        key: "../escape".to_string(),
                        value: Value::Null,
                    },
                )
                .await,
            Err(BootError::BadRequest(_))
        ));
        assert!(matches!(
            service
                .activity_state(
                    "science:research",
                    2,
                    &revision,
                    PluginActivityStateRequest::Set {
                        key: "oversized".to_string(),
                        value: Value::String("x".repeat(16 * 1024)),
                    },
                )
                .await,
            Err(BootError::PayloadTooLarge(_))
        ));

        let snapshot = only_json_file(&component_paths.state_root.join("use").join("ui-state"));
        std::fs::write(&snapshot, b"{not-json").expect("corrupt Activity state fixture");
        assert!(matches!(
            service
                .activity_state(
                    "science:research",
                    2,
                    &revision,
                    PluginActivityStateRequest::Get {
                        key: "draft/current".to_string(),
                    },
                )
                .await,
            Err(BootError::Internal(_))
        ));
        let cleared = service
            .activity_state(
                "science:research",
                2,
                &revision,
                PluginActivityStateRequest::Clear,
            )
            .await
            .expect("clear corrupt state without parsing it");
        assert_eq!(cleared["cleared"], true);

        state
            .install_use_registry(
                crate::use_registry::UseRegistryHandle::for_test_knowledge(
                    extension_paths,
                    0,
                    Vec::new(),
                ),
                None,
            )
            .await;
        assert!(matches!(
            service
                .activity_state(
                    "science:research",
                    2,
                    &revision,
                    PluginActivityStateRequest::Get {
                        key: "draft/current".to_string(),
                    },
                )
                .await,
            Err(BootError::ServiceUnavailable(_))
        ));
    }

    struct TestComponentEnvironment {
        previous: Vec<(&'static str, Option<OsString>)>,
    }

    impl TestComponentEnvironment {
        fn install(root: &Path) -> Self {
            let values = [
                ("A3S_DATA_HOME", root.join("data")),
                ("A3S_STATE_HOME", root.join("state")),
                ("A3S_CACHE_HOME", root.join("cache")),
                ("A3S_RUNTIME_HOME", root.join("runtime")),
            ];
            let mut previous = Vec::with_capacity(values.len());
            for (name, value) in values {
                previous.push((name, std::env::var_os(name)));
                std::env::set_var(name, value);
            }
            Self { previous }
        }
    }

    impl Drop for TestComponentEnvironment {
        fn drop(&mut self) {
            for (name, previous) in self.previous.drain(..).rev() {
                if let Some(value) = previous {
                    std::env::set_var(name, value);
                } else {
                    std::env::remove_var(name);
                }
            }
        }
    }

    fn only_json_file(root: &Path) -> PathBuf {
        let mut directories = vec![root.to_path_buf()];
        let mut files = Vec::new();
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(&directory).expect("read Activity state directory") {
                let path = entry.expect("read Activity state entry").path();
                if path.is_dir() {
                    directories.push(path);
                } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
                    files.push(path);
                }
            }
        }
        assert_eq!(files.len(), 1, "expected one Activity state snapshot");
        files.pop().unwrap()
    }
}
