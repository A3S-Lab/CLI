use super::*;

impl KernelService {
    pub(in crate::api::code_web) async fn session_status(
        &self,
        session_id: &str,
    ) -> BootResult<serde_json::Value> {
        let session = self.kernel_session(session_id).await?;
        let settings = self.session_settings_snapshot(session_id).await;
        let os_status = code_web_os_status(self.state.as_ref()).await?;
        let runtime_connected = os_status
            .get("runtimeToolActive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(json!({
            "sessionId": session.session_id(),
            "status": "idle",
            "cwd": session.workspace().display().to_string(),
            "model": self.effective_model(&settings),
            "followDefaultModel": settings.follow_default_model,
            "permissionMode": settings.permission_mode,
            "planningMode": settings.planning_mode,
            "goalTracking": settings.goal_tracking,
            "mcpServers": [],
            "runtime": {
                "connected": runtime_connected,
                "transport": "rest",
                "os": os_status,
            },
            "commands": ["clear", "compact", "cost", "help", "history", "mcp", "model", "tools"],
        }))
    }

    pub(in crate::api::code_web) async fn effort_levels(&self) -> BootResult<serde_json::Value> {
        Ok(json!({
            "items": effort_levels_json(),
        }))
    }

    pub(in crate::api::code_web) async fn session_controls(
        &self,
        session_id: &str,
    ) -> BootResult<serde_json::Value> {
        self.kernel_session(session_id).await?;
        let controls = self.session_controls_snapshot(session_id).await;
        let settings = self.session_settings_snapshot(session_id).await;
        let model = self.effective_model(&settings);
        let context_limit = code_web_context_limit_for_model(self.state.as_ref(), model.as_deref());
        Ok(controls_json(
            session_id,
            &controls,
            Some(context_limit),
            self.state.auto_compact_threshold,
        ))
    }

    pub(in crate::api::code_web) async fn update_session_controls(
        &self,
        session_id: &str,
        request: serde_json::Value,
    ) -> BootResult<serde_json::Value> {
        self.kernel_session(session_id).await?;
        let (controls_snapshot, effort_changed) = {
            let mut controls_by_session = self.state.session_controls.lock().await;
            let controls = controls_by_session
                .entry(session_id.to_string())
                .or_default();
            let original_effort = controls.effort.clone();

            if let Some(effort_value) = request.get("effort") {
                let effort = effort_value
                    .as_str()
                    .ok_or_else(|| BootError::BadRequest("effort must be a string".to_string()))?;
                let profile = normalize_effort(effort).ok_or_else(|| {
                    BootError::BadRequest(format!("unsupported effort level `{effort}`"))
                })?;
                controls.effort = profile.id.to_string();
            }

            if let Some(goal_value) = request.get("goal") {
                match goal_value {
                    Value::Null => controls.goal = None,
                    Value::String(goal) => controls.goal = normalize_goal(goal),
                    _ => {
                        return Err(BootError::BadRequest(
                            "goal must be a string or null".to_string(),
                        ));
                    }
                }
            }

            (controls.clone(), controls.effort != original_effort)
        };

        if effort_changed {
            let settings = self.session_settings_snapshot(session_id).await;
            self.rebuild_session_with_settings(session_id, &settings)
                .await?;
        }

        self.persist_session_state(session_id).await?;

        let settings = self.session_settings_snapshot(session_id).await;
        let model = self.effective_model(&settings);
        let context_limit = code_web_context_limit_for_model(self.state.as_ref(), model.as_deref());
        Ok(controls_json(
            session_id,
            &controls_snapshot,
            Some(context_limit),
            self.state.auto_compact_threshold,
        ))
    }
}
