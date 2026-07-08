use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use a3s_boot::{BootError, Result as BootResult};
use serde_json::{json, Value};

use super::controller::{PluginReloadRequest, PluginToggleRequest};
use crate::api::code_web::session_runtime::rebuild_code_web_sessions;
use crate::api::code_web::state::CodeWebState;
use crate::tui::skills::{
    agent_skill_dirs, count_skill_files, load_disabled_skills, load_skills, save_disabled_skills,
};

pub(in crate::api::code_web) struct PluginsService {
    state: Arc<CodeWebState>,
}

impl PluginsService {
    pub(in crate::api::code_web) fn new(state: Arc<CodeWebState>) -> Self {
        Self { state }
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
                    "command": format!("/{name}"),
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
}
