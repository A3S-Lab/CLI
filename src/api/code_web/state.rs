use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use a3s_code_core::{Agent, AgentSession, CodeConfig};
use tokio::sync::Mutex;

use crate::budget::DEFAULT_CODE_WEB_EFFORT_ID;

#[derive(Debug, Clone)]
pub(in crate::api::code_web) struct CodeWebSessionControls {
    pub(in crate::api::code_web) effort: String,
    pub(in crate::api::code_web) goal: Option<String>,
}

impl Default for CodeWebSessionControls {
    fn default() -> Self {
        Self {
            effort: DEFAULT_CODE_WEB_EFFORT_ID.to_string(),
            goal: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(in crate::api::code_web) struct CodeWebSessionContext {
    pub(in crate::api::code_web) compact_summary: Option<String>,
}

#[derive(Debug, Clone)]
pub(in crate::api::code_web) struct CodeWebSessionSettings {
    pub(in crate::api::code_web) model: Option<String>,
    pub(in crate::api::code_web) follow_default_model: bool,
    pub(in crate::api::code_web) permission_mode: String,
    pub(in crate::api::code_web) planning_mode: Option<String>,
    pub(in crate::api::code_web) goal_tracking: Option<bool>,
}

impl Default for CodeWebSessionSettings {
    fn default() -> Self {
        Self {
            model: None,
            follow_default_model: true,
            permission_mode: "auto".to_string(),
            planning_mode: None,
            goal_tracking: None,
        }
    }
}

pub(in crate::api) struct CodeWebState {
    pub(in crate::api::code_web) agent: Arc<Agent>,
    pub(in crate::api::code_web) config_path: PathBuf,
    pub(in crate::api::code_web) default_workspace: PathBuf,
    pub(in crate::api::code_web) code_config: RwLock<CodeConfig>,
    pub(in crate::api::code_web) sessions: Mutex<HashMap<String, Arc<AgentSession>>>,
    pub(in crate::api::code_web) messages: Mutex<HashMap<String, Vec<serde_json::Value>>>,
    pub(in crate::api::code_web) session_controls: Mutex<HashMap<String, CodeWebSessionControls>>,
    pub(in crate::api::code_web) session_contexts: Mutex<HashMap<String, CodeWebSessionContext>>,
    pub(in crate::api::code_web) session_settings: Mutex<HashMap<String, CodeWebSessionSettings>>,
}

impl CodeWebState {
    pub(in crate::api) fn new(
        agent: Arc<Agent>,
        config_path: PathBuf,
        default_workspace: PathBuf,
        code_config: CodeConfig,
    ) -> Self {
        Self {
            agent,
            config_path,
            default_workspace,
            code_config: RwLock::new(code_config),
            sessions: Mutex::new(HashMap::new()),
            messages: Mutex::new(HashMap::new()),
            session_controls: Mutex::new(HashMap::new()),
            session_contexts: Mutex::new(HashMap::new()),
            session_settings: Mutex::new(HashMap::new()),
        }
    }

    pub(in crate::api::code_web) fn code_config_snapshot(&self) -> CodeConfig {
        self.code_config
            .read()
            .map(|config| config.clone())
            .unwrap_or_default()
    }

    pub(in crate::api::code_web) fn current_default_model(&self) -> Option<String> {
        self.code_config_snapshot().default_model
    }

    pub(in crate::api::code_web) async fn close(&self) {
        self.agent.close().await;
    }
}
