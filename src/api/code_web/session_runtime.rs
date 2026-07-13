use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_boot::{BootError, Result as BootResult};
use a3s_code_core::host_env::HostEnv;
use a3s_code_core::{AgentSession, CodeConfig, LlmClient, SessionOptions, SystemPromptSlots};
use serde_json::{json, Value};

use super::state::{CodeWebSessionControls, CodeWebSessionSettings, CodeWebState};
use crate::budget::{self, BudgetWorkload};
use crate::config;
use crate::tui::skills::{agent_skill_dirs, ensure_builtin_skills_dir};

#[derive(Debug, Clone)]
pub(in crate::api::code_web) struct CodeWebSessionRuntime {
    pub(in crate::api::code_web) skill_dirs: Vec<PathBuf>,
    pub(in crate::api::code_web) os_session: Option<crate::a3s_os::StoredOsSession>,
    os_configured: bool,
    os_address: Option<String>,
    builtin_skill_active: bool,
    capability_skill_active: bool,
    refresh_error: Option<String>,
}

impl CodeWebSessionRuntime {
    pub(in crate::api::code_web) fn os_status_json(&self) -> Value {
        let session = self.os_session.as_ref();
        let address = session
            .map(|session| session.address.clone())
            .or_else(|| self.os_address.clone());
        let origin = address.as_deref().map(crate::a3s_os::os_origin);
        json!({
            "configured": self.os_configured,
            "address": address,
            "origin": origin,
            "signedIn": session.is_some(),
            "label": session.map(crate::a3s_os::StoredOsSession::display_label),
            "loginAtMs": session.map(|session| session.login_at_ms),
            "expiresAtMs": session.and_then(|session| session.expires_at_ms),
            "tokenType": session.and_then(|session| session.token_type.clone()),
            "needsRefresh": session.is_some_and(crate::a3s_os::needs_refresh),
            "capabilitySkillActive": self.capability_skill_active,
            "builtinSkillActive": self.builtin_skill_active,
            "runtimeToolActive": session.is_some(),
            "refreshError": self.refresh_error,
        })
    }
}

pub(in crate::api::code_web) async fn code_web_os_status(
    state: &CodeWebState,
) -> BootResult<Value> {
    Ok(
        code_web_session_runtime_for_workspace(state, &state.default_workspace)
            .await
            .os_status_json(),
    )
}

pub(in crate::api::code_web) async fn code_web_session_options(
    state: &CodeWebState,
    workspace: &Path,
    session_id: Option<&str>,
    model: Option<String>,
    effort: &str,
) -> BootResult<(SessionOptions, CodeWebSessionRuntime, Arc<dyn LlmClient>)> {
    let runtime = code_web_session_runtime_for_workspace(state, workspace).await;
    let context_limit = code_web_context_limit_for_model(state, model.as_deref());
    let budget =
        budget::budget_plan_for_effort_id(effort, Some(context_limit), BudgetWorkload::Interactive);
    let mut options = SessionOptions::new()
        .with_auto_save(false)
        .with_auto_compact(true)
        .with_max_context_tokens(context_limit as usize)
        .with_auto_compact_threshold(state.auto_compact_threshold as f32)
        .with_file_memory(config::memory_dir())
        .with_skill_dirs(runtime.skill_dirs.clone())
        .with_max_tool_rounds(budget.max_tool_rounds)
        .with_max_parallel_tasks(budget.max_parallel_tasks)
        .with_max_continuation_turns(budget.max_continuation_turns);

    let session_id = session_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| HostEnv::default().next_id());
    options = options.with_session_id(session_id.clone());
    if let Some(model) = model {
        options = options.with_model(model);
    }
    if let Some(session) = runtime.os_session.as_ref() {
        options = options.with_prompt_slots(
            SystemPromptSlots::default().with_extra(os_platform_guide(&session.address)),
        );
    }

    let llm_client = crate::session_llm::resolve_session_llm_client(
        &state.code_config_snapshot(),
        &options,
        &session_id,
    )
    .map_err(BootError::Internal)?;
    options = options.with_llm_client(Arc::clone(&llm_client));

    Ok((options, runtime, llm_client))
}

pub(in crate::api::code_web) fn code_web_context_limit_for_model(
    state: &CodeWebState,
    model: Option<&str>,
) -> u32 {
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else {
        return budget::resolve_ctx_limit(None);
    };
    let config = state.code_config_snapshot();
    budget::context_limit_for_model(model, configured_context_limit(&config, model), None)
}

fn configured_context_limit(config: &CodeConfig, model: &str) -> Option<u32> {
    config
        .list_models()
        .into_iter()
        .find_map(|(provider, item)| {
            let qualified = format!("{}/{}", provider.name, item.id);
            (model == qualified || model == item.id)
                .then_some(item.limit.context)
                .filter(|context| *context > 0)
        })
}

pub(in crate::api::code_web) fn activate_session_runtime(
    session: &AgentSession,
    runtime: &CodeWebSessionRuntime,
) {
    let _ = session.register_dynamic_workflow_runtime();
    match runtime.os_session.as_ref() {
        Some(os_session) => {
            let _ = session
                .register_dynamic_tool(Arc::new(crate::runtime_tool::RuntimeTool::new(os_session)));
        }
        None => {
            let _ = session.unregister_dynamic_tool("runtime");
        }
    }
}

pub(in crate::api::code_web) async fn rebuild_code_web_sessions(
    state: &CodeWebState,
) -> BootResult<Vec<Value>> {
    let sessions = current_sessions(state).await;
    let mut rebuilt = Vec::new();

    for (session_id, old_session, workspace) in sessions {
        let settings = session_settings(state, &session_id).await;
        let controls = session_controls(state, &session_id).await;
        let model = effective_session_model(state, &settings);
        let (options, runtime, llm_client) = code_web_session_options(
            state,
            &workspace,
            Some(&session_id),
            model,
            &controls.effort,
        )
        .await?;
        let new_session = Arc::new(
            state
                .agent
                .session_async(workspace.display().to_string(), Some(options))
                .await
                .map_err(|error| BootError::Internal(error.to_string()))?,
        );
        activate_session_runtime(new_session.as_ref(), &runtime);
        old_session.close().await;
        state
            .sessions
            .lock()
            .await
            .insert(session_id.clone(), new_session);
        state
            .session_contexts
            .lock()
            .await
            .entry(session_id.clone())
            .or_default()
            .set_llm_client(llm_client);
        rebuilt.push(json!({
            "sessionId": session_id,
            "workspace": workspace.display().to_string(),
            "skillDirCount": runtime.skill_dirs.len(),
            "builtinSkillActive": runtime.builtin_skill_active,
            "capabilitySkillActive": runtime.capability_skill_active,
            "runtimeToolActive": runtime.os_session.is_some(),
        }));
    }

    Ok(rebuilt)
}

pub(in crate::api::code_web) fn effective_session_model(
    state: &CodeWebState,
    settings: &CodeWebSessionSettings,
) -> Option<String> {
    if settings.follow_default_model {
        return state.current_default_model();
    }
    settings
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| state.current_default_model())
}

async fn code_web_session_runtime_for_workspace(
    state: &CodeWebState,
    workspace: &Path,
) -> CodeWebSessionRuntime {
    let code_config = state.code_config_snapshot();
    let os_config = code_config.os.clone();
    let os_configured = os_config.is_some();
    let os_address = os_config.as_ref().map(|config| config.address.clone());

    let mut skill_dirs = agent_skill_dirs(&workspace.display().to_string());
    let builtin_skill_active = if let Some(dir) = ensure_builtin_skills_dir() {
        skill_dirs.push(dir);
        true
    } else {
        false
    };

    let (os_session, refresh_error) = match os_config.as_ref() {
        Some(config) => current_os_session(config).await,
        None => (None, None),
    };
    let capability_skill_active = if os_session.is_some() {
        os_config
            .as_ref()
            .and_then(crate::a3s_os::ensure_capability_skill_dir)
            .inspect(|dir| skill_dirs.push(dir.clone()))
            .is_some()
    } else {
        false
    };

    if let Some(session) = os_session.as_ref() {
        crate::a3s_os::export_os_env(session);
    }

    skill_dirs.sort();
    skill_dirs.dedup();

    CodeWebSessionRuntime {
        skill_dirs,
        os_session,
        os_configured,
        os_address,
        builtin_skill_active,
        capability_skill_active,
        refresh_error,
    }
}

async fn current_os_session(
    config: &a3s_code_core::config::OsConfig,
) -> (Option<crate::a3s_os::StoredOsSession>, Option<String>) {
    let Some(session) = crate::a3s_os::current_session(config) else {
        return (None, None);
    };

    if !crate::a3s_os::needs_refresh(&session) {
        return (Some(session), None);
    }

    match crate::a3s_os::refresh_session(&session).await {
        Ok(refreshed) => (Some(refreshed), None),
        Err(error) => (Some(session), Some(error.to_string())),
    }
}

async fn current_sessions(state: &CodeWebState) -> Vec<(String, Arc<AgentSession>, PathBuf)> {
    state
        .sessions
        .lock()
        .await
        .iter()
        .map(|(session_id, session)| {
            (
                session_id.clone(),
                Arc::clone(session),
                session.workspace().to_path_buf(),
            )
        })
        .collect()
}

async fn session_settings(state: &CodeWebState, session_id: &str) -> CodeWebSessionSettings {
    state
        .session_settings
        .lock()
        .await
        .get(session_id)
        .cloned()
        .unwrap_or_default()
}

async fn session_controls(state: &CodeWebState, session_id: &str) -> CodeWebSessionControls {
    state
        .session_controls
        .lock()
        .await
        .get(session_id)
        .cloned()
        .unwrap_or_default()
}

fn os_platform_guide(base_url: &str) -> String {
    format!(
        "[OS platform] You are signed in to the A3S OS platform at {base_url}. \
When the user says OS, they mean this platform, not the local operating system. \
Use the exported A3S_OS_BASE_URL and A3S_OS_TOKEN environment variables, the \
a3s-os-capabilities skill, and the runtime tool for login-gated platform work. \
For progressive API execute calls, preserve shaped view data so the host can \
show the user an authenticated Open view action."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_code_core::config::{ModelConfig, ModelLimit, ProviderConfig};
    use std::collections::HashMap;

    #[test]
    fn os_status_does_not_expose_tokens() {
        let runtime = CodeWebSessionRuntime {
            skill_dirs: Vec::new(),
            os_session: Some(crate::a3s_os::StoredOsSession {
                address: "https://os.example.com/platform".to_string(),
                access_token: "secret-access-token".to_string(),
                refresh_token: Some("secret-refresh-token".to_string()),
                token_type: Some("Bearer".to_string()),
                expires_at_ms: Some(42),
                account_label: Some("Ada".to_string()),
                login_at_ms: 1,
            }),
            os_configured: true,
            os_address: Some("https://os.example.com/platform".to_string()),
            builtin_skill_active: true,
            capability_skill_active: true,
            refresh_error: None,
        };

        let status = runtime.os_status_json();
        let encoded = status.to_string();
        assert_eq!(status["signedIn"], true);
        assert_eq!(status["label"], "Ada");
        assert_eq!(status["origin"], "https://os.example.com");
        assert!(!encoded.contains("secret-access-token"));
        assert!(!encoded.contains("secret-refresh-token"));
    }

    #[test]
    fn configured_context_limit_matches_qualified_and_plain_model_ids() {
        let config = CodeConfig {
            providers: vec![ProviderConfig {
                name: "openai".to_string(),
                api_key: None,
                base_url: None,
                headers: HashMap::new(),
                session_id_header: None,
                models: vec![ModelConfig {
                    id: "tiny".to_string(),
                    name: "Tiny".to_string(),
                    family: String::new(),
                    api_key: None,
                    base_url: None,
                    headers: HashMap::new(),
                    session_id_header: None,
                    attachment: false,
                    reasoning: false,
                    tool_call: true,
                    temperature: true,
                    release_date: None,
                    modalities: Default::default(),
                    cost: Default::default(),
                    limit: ModelLimit {
                        context: 32_768,
                        output: 0,
                    },
                }],
            }],
            ..Default::default()
        };

        assert_eq!(
            configured_context_limit(&config, "openai/tiny"),
            Some(32_768)
        );
        assert_eq!(configured_context_limit(&config, "tiny"), Some(32_768));
        assert_eq!(
            budget::context_limit_for_model(
                "openai/tiny",
                configured_context_limit(&config, "openai/tiny"),
                None,
            ),
            32_768
        );
    }
}
