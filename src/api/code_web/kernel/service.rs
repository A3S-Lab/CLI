use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use a3s_boot::{BootError, Result as BootResult};
use a3s_code_core::{AgentEvent, AgentSession, ContentBlock, Message, TokenUsage};
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::controls::{
    compose_controlled_prompt, controls_json, effort_levels_json, normalize_effort, normalize_goal,
};
use super::sleep::{parse_sleep_report, sleep_directive, sleep_today, store_sleep_memories};
use crate::api::code_web::dto::{
    ChatRequest, ChatResponse, CreateSessionRequest, ForkSessionRequest, KernelSessionResponse,
    SessionListResponse, SessionResponse, ShellSessionRequest, SleepSessionRequest, UsageResponse,
};
use crate::api::code_web::session_runtime::{
    activate_session_runtime, code_web_context_limit_for_model, code_web_os_status,
    code_web_session_options, effective_session_model,
};
use crate::api::code_web::state::{
    CodeWebSessionContext, CodeWebSessionControls, CodeWebSessionSettings, CodeWebState,
};

pub(in crate::api::code_web) struct KernelService {
    state: Arc<CodeWebState>,
}

const SHELL_COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const SHELL_OUTPUT_MAX_CHARS: usize = 128_000;

struct CodeWebStreamResult {
    text: String,
    usage: TokenUsage,
    tool_calls_count: usize,
    last_prompt_tokens: usize,
}

#[derive(Default)]
struct CodeWebStreamAccumulator {
    streamed_text: String,
    end_text: Option<String>,
    usage: Option<TokenUsage>,
    tool_calls_count: usize,
    last_prompt_tokens: usize,
    error: Option<String>,
}

impl CodeWebStreamAccumulator {
    fn observe(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TextDelta { text } => self.streamed_text.push_str(&text),
            AgentEvent::ToolStart { .. } => {
                self.tool_calls_count = self.tool_calls_count.saturating_add(1)
            }
            AgentEvent::TurnEnd { usage, .. } => {
                self.last_prompt_tokens = usage.prompt_tokens;
            }
            AgentEvent::End { text, usage, .. } => {
                self.end_text = Some(text);
                self.usage = Some(usage);
            }
            AgentEvent::Error { message } => self.error = Some(message),
            _ => {}
        }
    }

    fn finish(self) -> Result<CodeWebStreamResult, String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let usage = self
            .usage
            .ok_or_else(|| "agent stream ended without a final response".to_string())?;
        let end_text = self.end_text.unwrap_or_default();
        Ok(CodeWebStreamResult {
            text: if end_text.trim().is_empty() {
                self.streamed_text
            } else {
                end_text
            },
            usage,
            tool_calls_count: self.tool_calls_count,
            last_prompt_tokens: self.last_prompt_tokens,
        })
    }
}

async fn run_code_web_stream(
    session: &AgentSession,
    prompt: &str,
    history: &[Message],
) -> Result<CodeWebStreamResult, String> {
    let (mut events, join) = session
        .stream(prompt, Some(history))
        .await
        .map_err(|error| error.to_string())?;
    let mut accumulator = CodeWebStreamAccumulator::default();
    while let Some(event) = events.recv().await {
        let finished = matches!(event, AgentEvent::End { .. } | AgentEvent::Error { .. });
        accumulator.observe(event);
        if finished {
            break;
        }
    }
    join.await.map_err(|error| error.to_string())?;
    accumulator.finish()
}

impl KernelService {
    pub(in crate::api::code_web) fn new(state: Arc<CodeWebState>) -> Self {
        Self { state }
    }

    pub(in crate::api::code_web) async fn create_session(
        &self,
        request: CreateSessionRequest,
    ) -> BootResult<SessionResponse> {
        let session = self.create_or_get_session(None, request).await?;
        let settings = self.session_settings_snapshot(session.session_id()).await;
        Ok(SessionResponse::from_session(
            &session,
            self.session_response_model(session.session_id()).await,
            settings.follow_default_model,
            settings.permission_mode,
            None,
            None,
        ))
    }

    pub(in crate::api::code_web) async fn create_kernel_session(
        &self,
        request: CreateSessionRequest,
    ) -> BootResult<KernelSessionResponse> {
        let title = request.title.clone();
        let agent_id = request.agent_id.clone();
        let session = self.create_or_get_session(None, request).await?;
        let settings = self.session_settings_snapshot(session.session_id()).await;
        Ok(KernelSessionResponse {
            success: true,
            session: SessionResponse::from_session(
                &session,
                self.session_response_model(session.session_id()).await,
                settings.follow_default_model,
                settings.permission_mode,
                title,
                agent_id,
            ),
        })
    }

    pub(in crate::api::code_web) async fn list_agents(&self) -> Vec<serde_json::Value> {
        vec![json!({
            "id": "default",
            "name": "书小安",
            "description": "A3S Code local assistant",
            "tags": ["local", "a3s-code"],
        })]
    }

    pub(in crate::api::code_web) async fn list_sessions(&self) -> BootResult<SessionListResponse> {
        let session_ids: Vec<String> = self
            .state
            .sessions
            .lock()
            .await
            .values()
            .map(|session| session.session_id().to_string())
            .collect();
        let mut sessions = Vec::new();
        for session_id in session_ids {
            let session = self.kernel_session(&session_id).await?;
            let settings = self.session_settings_snapshot(&session_id).await;
            sessions.push(SessionResponse::from_session(
                session.as_ref(),
                self.session_response_model(&session_id).await,
                settings.follow_default_model,
                settings.permission_mode,
                None,
                None,
            ));
        }
        Ok(SessionListResponse {
            total: sessions.len(),
            items: sessions,
        })
    }

    pub(in crate::api::code_web) async fn get_session(
        &self,
        session_id: &str,
    ) -> BootResult<SessionResponse> {
        let session = self.kernel_session(session_id).await?;
        let settings = self.session_settings_snapshot(session_id).await;
        Ok(SessionResponse::from_session(
            session.as_ref(),
            self.session_response_model(session_id).await,
            settings.follow_default_model,
            settings.permission_mode,
            None,
            None,
        ))
    }

    pub(in crate::api::code_web) async fn update_session(
        &self,
        session_id: &str,
        patch: serde_json::Value,
    ) -> BootResult<SessionResponse> {
        self.apply_session_update(session_id, patch).await?;
        self.get_session(session_id).await
    }

    pub(in crate::api::code_web) async fn delete_session(
        &self,
        session_id: &str,
    ) -> BootResult<()> {
        let removed = self.state.sessions.lock().await.remove(session_id);
        self.state.messages.lock().await.remove(session_id);
        self.state.session_controls.lock().await.remove(session_id);
        self.state.session_contexts.lock().await.remove(session_id);
        self.state.session_settings.lock().await.remove(session_id);
        if removed.is_none() {
            return Err(BootError::NotFound(format!(
                "session `{session_id}` was not found"
            )));
        }
        Ok(())
    }

    pub(in crate::api::code_web) async fn session_messages(
        &self,
        session_id: &str,
    ) -> BootResult<serde_json::Value> {
        self.kernel_session(session_id).await?;
        let messages = self
            .state
            .messages
            .lock()
            .await
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        Ok(json!({
            "items": messages,
            "total": messages.len(),
            "page": 1,
            "limit": messages.len().max(1),
        }))
    }

    pub(in crate::api::code_web) async fn session_output(
        &self,
        session_id: &str,
    ) -> BootResult<serde_json::Value> {
        self.kernel_session(session_id).await?;
        let messages = self
            .state
            .messages
            .lock()
            .await
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        Ok(session_output_json(session_id, &messages))
    }

    pub(in crate::api::code_web) async fn run_shell_command(
        &self,
        session_id: &str,
        request: ShellSessionRequest,
    ) -> BootResult<serde_json::Value> {
        let session = self.kernel_session(session_id).await?;
        let command = request.command.trim();
        if command.is_empty() {
            return Err(BootError::BadRequest("command is required".to_string()));
        }
        if command.chars().count() > 20_000 {
            return Err(BootError::BadRequest("command is too long".to_string()));
        }

        let cwd = session.workspace().to_path_buf();
        let started_at = chrono::Utc::now();
        let timer = Instant::now();
        let output = timeout(SHELL_COMMAND_TIMEOUT, async {
            let mut process = Command::new("sh");
            process
                .arg("-c")
                .arg(command)
                .current_dir(&cwd)
                .kill_on_drop(true);
            process.output().await
        })
        .await;
        let completed_at = chrono::Utc::now();
        let duration_ms = timer.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;

        let (stdout, stderr, exit_code, timed_out, spawn_error) = match output {
            Ok(Ok(output)) => (
                shell_text(&output.stdout),
                shell_text(&output.stderr),
                output.status.code(),
                false,
                None,
            ),
            Ok(Err(error)) => (
                String::new(),
                format!("failed to run: {error}"),
                None,
                false,
                Some(error.to_string()),
            ),
            Err(_) => (
                String::new(),
                format!(
                    "command timed out after {} seconds",
                    SHELL_COMMAND_TIMEOUT.as_secs()
                ),
                None,
                true,
                None,
            ),
        };
        let mut combined = String::new();
        combined.push_str(&stdout);
        combined.push_str(&stderr);
        let output_text = if combined.trim().is_empty() {
            match exit_code {
                Some(code) => format!("(exit {code})"),
                None => stderr.clone(),
            }
        } else {
            truncate_chars(&combined, SHELL_OUTPUT_MAX_CHARS)
        };
        let is_error = timed_out || spawn_error.is_some() || exit_code.is_none_or(|code| code != 0);
        let record = shell_output_record(ShellOutputRecordInput {
            session_id,
            command,
            cwd: &cwd.display().to_string(),
            stdout: &truncate_chars(&stdout, SHELL_OUTPUT_MAX_CHARS),
            stderr: &truncate_chars(&stderr, SHELL_OUTPUT_MAX_CHARS),
            output: &output_text,
            exit_code,
            is_error,
            timed_out,
            duration_ms,
            started_at: &started_at.to_rfc3339(),
            completed_at: &completed_at.to_rfc3339(),
        });
        self.append_shell_output_message(session_id, &record)
            .await?;

        Ok(json!({
            "sessionId": session_id,
            "command": command,
            "cwd": cwd.display().to_string(),
            "stdout": truncate_chars(&stdout, SHELL_OUTPUT_MAX_CHARS),
            "stderr": truncate_chars(&stderr, SHELL_OUTPUT_MAX_CHARS),
            "output": output_text,
            "exitCode": exit_code,
            "success": !is_error,
            "isError": is_error,
            "timedOut": timed_out,
            "durationMs": duration_ms,
            "startedAt": started_at.to_rfc3339(),
            "completedAt": completed_at.to_rfc3339(),
            "record": record,
        }))
    }

    pub(in crate::api::code_web) async fn clear_session_messages(
        &self,
        session_id: &str,
    ) -> BootResult<serde_json::Value> {
        let old_session = self.kernel_session(session_id).await?;
        let workspace = old_session.workspace().to_path_buf();
        old_session.close().await;
        let store_dir = code_web_store_dir(&workspace);
        crate::timeline::TimelineJsonlStore::for_session(&store_dir, session_id)
            .clear()
            .map_err(|error| BootError::Internal(error.to_string()))?;
        crate::compact::ContextJsonStore::for_session(&store_dir, session_id)
            .clear()
            .map_err(|error| BootError::Internal(error.to_string()))?;

        let controls = self.session_controls_snapshot(session_id).await;
        let new_session = self
            .create_agent_session(
                &workspace,
                Some(session_id),
                self.session_response_model(session_id).await,
                &controls.effort,
            )
            .await?;
        self.state
            .sessions
            .lock()
            .await
            .insert(session_id.to_string(), new_session);
        self.state
            .messages
            .lock()
            .await
            .insert(session_id.to_string(), Vec::new());
        self.state
            .session_contexts
            .lock()
            .await
            .insert(session_id.to_string(), CodeWebSessionContext::default());
        Ok(json!({
            "sessionId": session_id,
            "cleared": true,
            "items": [],
            "total": 0,
        }))
    }

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

    pub(in crate::api::code_web) async fn compact_session(
        &self,
        session_id: &str,
    ) -> BootResult<serde_json::Value> {
        let session = self.kernel_session(session_id).await?;
        self.model_history_for_session(session.as_ref()).await?;
        let workspace = session.workspace().to_path_buf();
        let timeline_store = crate::timeline::TimelineJsonlStore::for_session(
            code_web_store_dir(&workspace),
            session_id,
        );
        let history_messages = timeline_store
            .metadata()
            .map_err(|error| BootError::Internal(error.to_string()))?
            .source_message_count;
        if history_messages == 0 {
            return Err(BootError::BadRequest("nothing to compact yet".to_string()));
        }

        let summary = crate::compact::compact_timeline(session.llm_client(), &timeline_store)
            .await
            .map_err(|error| BootError::Internal(error.to_string()))?;
        let summary =
            summary.ok_or_else(|| BootError::BadRequest("nothing to compact yet".to_string()))?;
        self.finalize_code_web_compact(session_id, session.as_ref(), &summary)
            .await?;

        Ok(json!({
            "sessionId": session_id,
            "compacted": true,
            "summary": summary,
            "historyMessages": history_messages,
            "completedAt": chrono::Utc::now().to_rfc3339(),
        }))
    }

    pub(in crate::api::code_web) async fn sleep_session(
        &self,
        session_id: &str,
        request: SleepSessionRequest,
    ) -> BootResult<serde_json::Value> {
        let session = self.kernel_session(session_id).await?;
        let focus = request
            .focus
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("")
            .to_string();
        let today = sleep_today();
        let directive = sleep_directive(&focus, false, &today);
        let history = self.model_history_for_session(session.as_ref()).await?;
        self.save_code_web_message_to_timeline(session_id, &Message::user(&directive))
            .await?;
        let result = run_code_web_stream(session.as_ref(), &directive, &history)
            .await
            .map_err(|error| BootError::Internal(error.to_string()))?;
        let assistant_text = result.text.clone();
        let usage = UsageResponse::from_usage(result.usage.clone());
        let tool_calls_count = result.tool_calls_count;
        self.save_code_web_message_to_timeline(session_id, &Message::assistant(&assistant_text))
            .await?;
        let (report_captured, saved_memories) = match parse_sleep_report(&assistant_text) {
            Some(memories) => (
                true,
                store_sleep_memories(session.as_ref(), memories, &today).await?,
            ),
            None => (false, Vec::new()),
        };

        let display = if focus.is_empty() {
            "Sleep consolidation".to_string()
        } else {
            format!("Sleep consolidation focus: {focus}")
        };
        let summary = if !report_captured {
            "Sleep consolidation finished without a parseable memory report.".to_string()
        } else if saved_memories.is_empty() {
            "Sleep consolidation finished with no durable memories to save.".to_string()
        } else {
            format!(
                "Sleep consolidation saved {} durable memories.",
                saved_memories.len()
            )
        };
        self.append_visible_message(session_id, "user", &display, None)
            .await;
        self.append_visible_message(session_id, "system", &summary, None)
            .await;
        self.maybe_auto_compact(session_id, session.as_ref(), result.last_prompt_tokens)
            .await;

        Ok(json!({
            "sessionId": session_id,
            "focus": focus,
            "date": today,
            "reportCaptured": report_captured,
            "savedCount": saved_memories.len(),
            "memories": saved_memories,
            "assistantText": assistant_text,
            "usage": usage,
            "toolCallsCount": tool_calls_count,
            "completedAt": chrono::Utc::now().to_rfc3339(),
        }))
    }

    pub(in crate::api::code_web) async fn fork_session(
        &self,
        session_id: &str,
        request: ForkSessionRequest,
    ) -> BootResult<serde_json::Value> {
        let source_session = self.kernel_session(session_id).await?;
        let focus = request
            .focus
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("")
            .to_string();
        let source_history = source_session.history();
        let source_messages = self
            .state
            .messages
            .lock()
            .await
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        if source_history.is_empty() && source_messages.is_empty() {
            return Err(BootError::BadRequest(
                "nothing to fork yet - start a conversation first".to_string(),
            ));
        }

        let workspace = source_session.workspace().to_path_buf();
        let source_settings = self.session_settings_snapshot(session_id).await;
        let model = self.effective_model(&source_settings);
        let target_settings = source_settings.clone();
        let source_controls = self.session_controls_snapshot(session_id).await;
        let target_session = self
            .create_agent_session(&workspace, None, model.clone(), &source_controls.effort)
            .await?;
        let target_session_id = target_session.session_id().to_string();
        let source_context = self.session_context_snapshot(session_id).await;
        let target_context = CodeWebSessionContext {
            compact_summary: build_fork_context(
                session_id,
                &focus,
                source_context.compact_summary.as_deref(),
                &source_messages,
                &source_history,
            ),
            ..CodeWebSessionContext::default()
        };
        let target_messages =
            fork_messages(session_id, &target_session_id, &source_messages, &focus);

        self.state
            .sessions
            .lock()
            .await
            .insert(target_session_id.clone(), Arc::clone(&target_session));
        self.state
            .messages
            .lock()
            .await
            .insert(target_session_id.clone(), target_messages.clone());
        self.state
            .session_controls
            .lock()
            .await
            .insert(target_session_id.clone(), source_controls);
        self.state
            .session_contexts
            .lock()
            .await
            .insert(target_session_id.clone(), target_context);
        self.state
            .session_settings
            .lock()
            .await
            .insert(target_session_id.clone(), target_settings.clone());

        let title = fork_title(&focus);
        Ok(json!({
            "sourceSessionId": session_id,
            "sessionId": target_session_id,
            "focus": focus,
            "workspace": workspace.display().to_string(),
            "model": model,
            "title": title,
            "copiedMessages": source_messages.len(),
            "messages": target_messages,
            "session": SessionResponse::from_session(
                target_session.as_ref(),
                model.clone(),
                target_settings.follow_default_model,
                target_settings.permission_mode,
                Some(title),
                None,
            ),
            "createdAt": chrono::Utc::now().to_rfc3339(),
        }))
    }

    pub(in crate::api::code_web) async fn run_session_message(
        &self,
        session_id: &str,
        request: serde_json::Value,
    ) -> BootResult<serde_json::Value> {
        let content = request
            .get("content")
            .or_else(|| request.get("message"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| BootError::BadRequest("content is required".to_string()))?;
        let session = self.kernel_session(session_id).await?;
        let prompt = self.compose_session_prompt(session_id, content).await;
        let history = self.model_history_for_session(session.as_ref()).await?;
        self.append_message(session_id, "user", content, None)
            .await?;
        let result = run_code_web_stream(session.as_ref(), &prompt, &history)
            .await
            .map_err(|error| BootError::Internal(error.to_string()))?;
        self.append_message(
            session_id,
            "assistant",
            &result.text,
            self.session_response_model(session_id).await,
        )
        .await?;
        self.maybe_auto_compact(session_id, session.as_ref(), result.last_prompt_tokens)
            .await;
        Ok(json!({
            "sessionId": session_id,
            "accepted": true,
            "events": [],
            "completedAt": chrono::Utc::now().to_rfc3339(),
        }))
    }

    pub(in crate::api::code_web) async fn chat(
        &self,
        request: ChatRequest,
    ) -> BootResult<ChatResponse> {
        let message = request.message.trim().to_string();
        if message.is_empty() {
            return Err(BootError::BadRequest("message cannot be empty".to_string()));
        }

        let session_request = CreateSessionRequest {
            workspace: request.workspace,
            cwd: None,
            model: request.model,
            follow_default_model: None,
            permission_mode: None,
            planning_mode: None,
            goal_tracking: None,
            title: None,
            agent_id: None,
        };
        let session = self
            .create_or_get_session(request.session_id, session_request)
            .await?;
        let session_id = session.session_id().to_string();
        let prompt = self.compose_session_prompt(&session_id, &message).await;
        let history = self.model_history_for_session(session.as_ref()).await?;
        self.append_message(&session_id, "user", &message, None)
            .await?;
        let result = run_code_web_stream(session.as_ref(), &prompt, &history)
            .await
            .map_err(|e| BootError::Internal(e.to_string()))?;
        self.append_message(
            &session_id,
            "assistant",
            &result.text,
            self.session_response_model(&session_id).await,
        )
        .await?;
        self.maybe_auto_compact(&session_id, session.as_ref(), result.last_prompt_tokens)
            .await;

        Ok(ChatResponse {
            session_id,
            workspace: session.workspace().display().to_string(),
            model: self.session_response_model(session.session_id()).await,
            text: result.text,
            usage: UsageResponse::from_usage(result.usage),
            tool_calls_count: result.tool_calls_count,
        })
    }

    async fn create_or_get_session(
        &self,
        requested_id: Option<String>,
        request: CreateSessionRequest,
    ) -> BootResult<Arc<AgentSession>> {
        if let Some(id) = requested_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            let existing_session = self.state.sessions.lock().await.get(id).cloned();
            if let Some(session) = existing_session {
                self.state
                    .session_controls
                    .lock()
                    .await
                    .entry(id.to_string())
                    .or_default();
                self.state
                    .session_contexts
                    .lock()
                    .await
                    .entry(id.to_string())
                    .or_default();
                self.state
                    .session_settings
                    .lock()
                    .await
                    .entry(id.to_string())
                    .or_default();
                return Ok(session);
            }
        }

        let workspace = request
            .workspace
            .as_deref()
            .or(request.cwd.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.state.default_workspace.clone());
        let settings = self.settings_from_create_request(&request);
        let requested_session_id = requested_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty());
        let default_controls = CodeWebSessionControls::default();

        let session = self
            .create_agent_session(
                &workspace,
                requested_session_id,
                self.effective_model(&settings),
                &default_controls.effort,
            )
            .await?;
        self.state
            .sessions
            .lock()
            .await
            .insert(session.session_id().to_string(), Arc::clone(&session));
        self.state
            .messages
            .lock()
            .await
            .entry(session.session_id().to_string())
            .or_default();
        self.state
            .session_controls
            .lock()
            .await
            .entry(session.session_id().to_string())
            .or_insert(default_controls);
        self.state
            .session_contexts
            .lock()
            .await
            .entry(session.session_id().to_string())
            .or_default();
        self.state
            .session_settings
            .lock()
            .await
            .entry(session.session_id().to_string())
            .or_insert(settings);
        Ok(session)
    }

    async fn kernel_session(&self, session_id: &str) -> BootResult<Arc<AgentSession>> {
        self.state
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| BootError::NotFound(format!("session `{session_id}` was not found")))
    }

    async fn session_controls_snapshot(&self, session_id: &str) -> CodeWebSessionControls {
        self.state
            .session_controls
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    async fn session_context_snapshot(&self, session_id: &str) -> CodeWebSessionContext {
        self.state
            .session_contexts
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    async fn session_settings_snapshot(&self, session_id: &str) -> CodeWebSessionSettings {
        self.state
            .session_settings
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    fn settings_from_create_request(
        &self,
        request: &CreateSessionRequest,
    ) -> CodeWebSessionSettings {
        let follow_default_model = request.follow_default_model.unwrap_or_else(|| {
            request
                .model
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
        });
        let model = if follow_default_model {
            None
        } else {
            request
                .model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .or_else(|| self.state.current_default_model())
        };
        CodeWebSessionSettings {
            model,
            follow_default_model,
            permission_mode: request
                .permission_mode
                .as_deref()
                .and_then(normalize_permission_mode)
                .unwrap_or_else(|| "auto".to_string()),
            planning_mode: request
                .planning_mode
                .as_deref()
                .and_then(normalize_planning_mode),
            goal_tracking: request.goal_tracking,
        }
    }

    fn effective_model(&self, settings: &CodeWebSessionSettings) -> Option<String> {
        effective_session_model(self.state.as_ref(), settings)
    }

    async fn create_agent_session(
        &self,
        workspace: &Path,
        session_id: Option<&str>,
        model: Option<String>,
        effort: &str,
    ) -> BootResult<Arc<AgentSession>> {
        let (options, runtime) =
            code_web_session_options(self.state.as_ref(), workspace, session_id, model, effort)
                .await;
        let session = Arc::new(
            self.state
                .agent
                .session(workspace.display().to_string(), Some(options))
                .map_err(|error| BootError::Internal(error.to_string()))?,
        );
        activate_session_runtime(session.as_ref(), &runtime);
        Ok(session)
    }

    async fn session_response_model(&self, session_id: &str) -> Option<String> {
        let settings = self.session_settings_snapshot(session_id).await;
        self.effective_model(&settings)
    }

    async fn apply_session_update(
        &self,
        session_id: &str,
        patch: serde_json::Value,
    ) -> BootResult<()> {
        self.kernel_session(session_id).await?;
        let default_model = self.state.current_default_model();
        let (settings, model_changed) = {
            let mut settings_by_session = self.state.session_settings.lock().await;
            let settings = settings_by_session
                .entry(session_id.to_string())
                .or_default();
            let model_changed = apply_settings_patch(settings, &patch, default_model)?;
            (settings.clone(), model_changed)
        };

        if model_changed {
            self.rebuild_session_with_settings(session_id, &settings)
                .await?;
        }
        Ok(())
    }

    async fn rebuild_session_with_settings(
        &self,
        session_id: &str,
        settings: &CodeWebSessionSettings,
    ) -> BootResult<()> {
        let old_session = self.kernel_session(session_id).await?;
        let workspace = old_session.workspace().to_path_buf();
        let history = old_session.history();
        let messages = self
            .state
            .messages
            .lock()
            .await
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        let previous_context = self.session_context_snapshot(session_id).await;
        if let Some(carried_context) = build_reconfigure_context(
            previous_context.compact_summary.as_deref(),
            &messages,
            &history,
        ) {
            self.state.session_contexts.lock().await.insert(
                session_id.to_string(),
                CodeWebSessionContext {
                    compact_summary: Some(carried_context),
                    ..CodeWebSessionContext::default()
                },
            );
        }

        old_session.close().await;
        let controls = self.session_controls_snapshot(session_id).await;
        let new_session = self
            .create_agent_session(
                &workspace,
                Some(session_id),
                self.effective_model(settings),
                &controls.effort,
            )
            .await?;
        self.state
            .sessions
            .lock()
            .await
            .insert(session_id.to_string(), new_session);
        Ok(())
    }

    async fn compose_session_prompt(&self, session_id: &str, user_prompt: &str) -> String {
        if user_prompt.trim_start().starts_with('/') {
            return user_prompt.to_string();
        }

        let controls = self.session_controls_snapshot(session_id).await;
        compose_controlled_prompt(&controls, user_prompt)
    }

    async fn model_history_for_session(&self, session: &AgentSession) -> BootResult<Vec<Message>> {
        let session_id = session.session_id();
        let store_dir = code_web_store_dir(session.workspace());
        let timeline_store =
            crate::timeline::TimelineJsonlStore::for_session(&store_dir, session_id);
        let context_limit = self.context_limit_for_session(session_id).await;
        let threshold = self.state.auto_compact_threshold;
        let mut metadata = timeline_store
            .metadata()
            .map_err(|error| BootError::Internal(error.to_string()))?;
        if metadata.source_message_count == 0 {
            let legacy_history = session.history();
            if !legacy_history.is_empty() {
                seed_code_web_timeline(
                    &store_dir,
                    session_id,
                    &legacy_history,
                    context_limit,
                    threshold,
                )
                .map_err(|error| BootError::Internal(error.to_string()))?;
                metadata = timeline_store
                    .metadata()
                    .map_err(|error| BootError::Internal(error.to_string()))?;
            }
        }

        let context_store = crate::compact::ContextJsonStore::for_session(&store_dir, session_id);
        if let Some(mut context) = context_store
            .load()
            .map_err(|error| BootError::Internal(error.to_string()))?
            .filter(|context| {
                context.matches_timeline(metadata)
                    && context.context_limit as usize == context_limit
            })
        {
            context.update_runtime_metadata(
                context.last_prompt_tokens,
                context_limit.min(u32::MAX as usize) as u32,
                threshold,
            );
            context_store
                .save(&context)
                .map_err(|error| BootError::Internal(error.to_string()))?;
            return Ok(context.messages);
        }

        let timeline = timeline_store
            .load_all()
            .map(|events| crate::timeline::messages_from_events(&events))
            .map_err(|error| BootError::Internal(error.to_string()))?;
        let context = crate::compact::ModelContextState::rebuild_from_timeline_with_metadata(
            &timeline,
            crate::compact::ProjectionBudget::for_token_limit(context_limit),
            metadata,
            0,
            context_limit.min(u32::MAX as usize) as u32,
            threshold,
        );
        context_store
            .save(&context)
            .map_err(|error| BootError::Internal(error.to_string()))?;
        Ok(context.messages)
    }

    async fn maybe_auto_compact(
        &self,
        session_id: &str,
        session: &AgentSession,
        last_prompt_tokens: usize,
    ) {
        if last_prompt_tokens == 0 {
            return;
        }
        let context_limit = self.context_limit_for_session(session_id).await;
        let threshold = self.state.auto_compact_threshold;
        let should_compact = {
            let mut contexts = self.state.session_contexts.lock().await;
            let context = contexts.entry(session_id.to_string()).or_default();
            let controller = context.auto_compact.get_or_insert_with(|| {
                crate::compact::auto_compact::AutoCompactController::new(
                    threshold,
                    context_limit.min(u32::MAX as usize) as u32,
                )
            });
            controller.update_policy(threshold, context_limit.min(u32::MAX as usize) as u32);
            controller.observe_prompt_tokens(last_prompt_tokens) && controller.start()
        };
        self.persist_code_web_context_usage(
            session_id,
            session.workspace(),
            last_prompt_tokens,
            context_limit,
            threshold,
        );
        if !should_compact {
            return;
        }

        let timeline_store = crate::timeline::TimelineJsonlStore::for_session(
            code_web_store_dir(session.workspace()),
            session_id,
        );
        let result =
            match crate::compact::compact_timeline(session.llm_client(), &timeline_store).await {
                Ok(Some(summary)) => {
                    self.finalize_code_web_compact(session_id, session, &summary)
                        .await
                }
                Ok(None) => Err(BootError::Internal(
                    "automatic compact found an empty timeline".to_string(),
                )),
                Err(error) => Err(BootError::Internal(error)),
            };

        let mut contexts = self.state.session_contexts.lock().await;
        let context = contexts.entry(session_id.to_string()).or_default();
        if let Some(controller) = context.auto_compact.as_mut() {
            if result.is_ok() {
                controller.finish_success(0);
            } else {
                controller.finish_failure();
            }
        }
        if let Err(error) = result {
            eprintln!("warning: automatic compact failed for {session_id}: {error}");
        }
    }

    async fn finalize_code_web_compact(
        &self,
        session_id: &str,
        session: &AgentSession,
        summary: &str,
    ) -> BootResult<()> {
        let workspace = session.workspace().to_path_buf();
        let context_limit = self.context_limit_for_session(session_id).await;
        let threshold = self.state.auto_compact_threshold;
        persist_code_web_compact_summary(
            &code_web_store_dir(&workspace),
            session_id,
            summary,
            context_limit,
            threshold,
        )
        .map_err(|error| BootError::Internal(error.to_string()))?;

        let controls = self.session_controls_snapshot(session_id).await;
        let new_session = self
            .create_agent_session(
                &workspace,
                Some(session_id),
                self.session_response_model(session_id).await,
                &controls.effort,
            )
            .await?;
        session.close().await;
        self.state
            .sessions
            .lock()
            .await
            .insert(session_id.to_string(), new_session);
        {
            let mut messages_by_session = self.state.messages.lock().await;
            let current_messages = messages_by_session.remove(session_id).unwrap_or_default();
            messages_by_session.insert(
                session_id.to_string(),
                compact_visible_messages_after_success(session_id, current_messages, summary),
            );
        }
        self.state
            .session_contexts
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .compact_summary = Some(summary.to_string());
        Ok(())
    }

    fn persist_code_web_context_usage(
        &self,
        session_id: &str,
        workspace: &Path,
        last_prompt_tokens: usize,
        context_limit: usize,
        threshold: f64,
    ) {
        let store = crate::compact::ContextJsonStore::for_session(
            code_web_store_dir(workspace),
            session_id,
        );
        if let Ok(Some(mut context)) = store.load() {
            context.update_runtime_metadata(
                last_prompt_tokens,
                context_limit.min(u32::MAX as usize) as u32,
                threshold,
            );
            let _ = store.save(&context);
        }
    }

    async fn append_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        model: Option<String>,
    ) -> BootResult<()> {
        self.append_visible_message(session_id, role, content, model)
            .await;
        self.save_code_web_message_to_timeline(session_id, &message_for_role(role, content))
            .await?;
        Ok(())
    }

    async fn append_visible_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        model: Option<String>,
    ) {
        let id = format!("{}-{}", chrono::Utc::now().timestamp_millis(), role);
        let mut message = json!({
            "id": id,
            "sessionId": session_id,
            "role": role,
            "content": content,
            "createdAt": chrono::Utc::now().to_rfc3339(),
        });
        if let Some(model) = model {
            message["model"] = Value::String(model);
        }
        self.state
            .messages
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .push(message);
    }

    async fn append_shell_output_message(
        &self,
        session_id: &str,
        record: &Value,
    ) -> BootResult<()> {
        let id = format!("{}-shell", chrono::Utc::now().timestamp_millis());
        let command = record
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let output = record
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let tool_use_id = record
            .get("toolUseId")
            .and_then(Value::as_str)
            .unwrap_or(&id);
        let message = json!({
            "id": id,
            "sessionId": session_id,
            "role": "assistant",
            "content": format!("Shell command finished: {command}"),
            "createdAt": chrono::Utc::now().to_rfc3339(),
            "source": "command:!",
            "contentBlocks": [
                {
                    "type": "tool_use",
                    "id": tool_use_id,
                    "name": "shell_command",
                    "input": {
                        "command": command,
                        "cwd": record.get("cwd").cloned().unwrap_or(Value::Null),
                    }
                },
                {
                    "type": "tool_result",
                    "toolUseId": tool_use_id,
                    "content": output,
                    "isError": record.get("isError").cloned().unwrap_or(Value::Bool(false)),
                    "exitCode": record.get("exitCode").cloned().unwrap_or(Value::Null),
                    "durationMs": record.get("durationMs").cloned().unwrap_or(Value::Null),
                }
            ],
        });
        self.state
            .messages
            .lock()
            .await
            .entry(session_id.to_string())
            .or_default()
            .push(message);
        let is_error = record
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.save_code_web_message_to_timeline(
            session_id,
            &Message::tool_result(tool_use_id, output, is_error),
        )
        .await?;
        Ok(())
    }

    async fn save_code_web_message_to_timeline(
        &self,
        session_id: &str,
        message: &Message,
    ) -> BootResult<()> {
        let session = self.kernel_session(session_id).await?;
        let store_dir = code_web_store_dir(session.workspace());
        let context_limit = self.context_limit_for_session(session_id).await;
        save_code_web_timeline_message(
            &store_dir,
            session_id,
            message,
            context_limit,
            self.state.auto_compact_threshold,
        )
        .map_err(|error| BootError::Internal(error.to_string()))
    }

    async fn context_limit_for_session(&self, session_id: &str) -> usize {
        let settings = self.session_settings_snapshot(session_id).await;
        let model = self.effective_model(&settings);
        code_web_context_limit_for_model(self.state.as_ref(), model.as_deref()) as usize
    }
}

fn message_for_role(role: &str, content: &str) -> Message {
    match role {
        "assistant" => Message::assistant(content),
        "user" => Message::user(content),
        _ => Message {
            role: role.to_string(),
            content: vec![ContentBlock::Text {
                text: content.to_string(),
            }],
            reasoning_content: None,
        },
    }
}

fn code_web_store_dir(workspace: &Path) -> PathBuf {
    workspace.join(".a3s").join("tui-sessions")
}

fn seed_code_web_timeline(
    store_dir: &Path,
    session_id: &str,
    history: &[Message],
    context_limit: usize,
    auto_compact_threshold: f64,
) -> anyhow::Result<()> {
    let timeline_store = crate::timeline::TimelineJsonlStore::for_session(store_dir, session_id);
    if timeline_store.metadata()?.source_message_count > 0 {
        return Ok(());
    }

    let mut source_event_count = 0;
    for (message_index, message) in history.iter().enumerate() {
        let events = crate::timeline::events_for_message(
            session_id,
            message_index,
            source_event_count as u64,
            message,
            chrono::Utc::now().timestamp_millis(),
        );
        source_event_count += events.len();
        for event in &events {
            timeline_store.append(event)?;
        }
    }
    let metadata = crate::timeline::TimelineMetadata {
        source_file_bytes: timeline_store.file_len()?,
        source_event_count,
        source_message_count: history.len(),
        active_summary_index: history.iter().rposition(crate::compact::is_compact_message),
        compact_generation: history
            .iter()
            .filter(|message| crate::compact::is_compact_message(message))
            .count() as u32,
    };
    let context = crate::compact::ModelContextState::rebuild_from_timeline_with_metadata(
        history,
        crate::compact::ProjectionBudget::for_token_limit(context_limit),
        metadata,
        0,
        context_limit.min(u32::MAX as usize) as u32,
        auto_compact_threshold,
    );
    crate::compact::ContextJsonStore::for_session(store_dir, session_id).save(&context)
}

fn save_code_web_timeline_message(
    store_dir: &Path,
    session_id: &str,
    message: &Message,
    context_limit: usize,
    auto_compact_threshold: f64,
) -> anyhow::Result<()> {
    let timeline_store = crate::timeline::TimelineJsonlStore::for_session(store_dir, session_id);
    let context_store = crate::compact::ContextJsonStore::for_session(store_dir, session_id);
    let source_file_bytes = timeline_store.file_len()?;
    let cached_context = context_store.load()?.filter(|context| {
        context.context_version == 2
            && context.source_file_bytes == source_file_bytes
            && context.context_limit as usize == context_limit
    });
    let next_index = cached_context
        .as_ref()
        .map(|context| context.source_message_count)
        .unwrap_or(0);
    let next_seq = cached_context
        .as_ref()
        .map(|context| context.source_event_count as u64)
        .unwrap_or(0);
    let appended_events = crate::timeline::events_for_message(
        session_id,
        next_index,
        next_seq,
        message,
        chrono::Utc::now().timestamp_millis(),
    );
    for event in &appended_events {
        timeline_store.append(event)?;
    }

    let budget = crate::compact::ProjectionBudget::for_token_limit(context_limit);
    if let Some(mut context) = cached_context {
        let last_prompt_tokens = context.last_prompt_tokens;
        context.append_timeline_message(
            message,
            appended_events.len(),
            timeline_store.file_len()?,
            budget,
        );
        context.update_runtime_metadata(
            last_prompt_tokens,
            context_limit.min(u32::MAX as usize) as u32,
            auto_compact_threshold,
        );
        context_store.save(&context)?;
        return Ok(());
    }

    let all_events = timeline_store.load_all()?;
    let timeline_messages = crate::timeline::messages_from_events(&all_events);
    let metadata = timeline_store.metadata()?;
    let context = crate::compact::ModelContextState::rebuild_from_timeline_with_metadata(
        &timeline_messages,
        budget,
        metadata,
        0,
        context_limit.min(u32::MAX as usize) as u32,
        auto_compact_threshold,
    );
    context_store.save(&context)?;
    Ok(())
}

fn persist_code_web_compact_summary(
    store_dir: &Path,
    session_id: &str,
    summary: &str,
    context_limit: usize,
    auto_compact_threshold: f64,
) -> anyhow::Result<()> {
    let mut messages = Vec::new();
    crate::compact::append_compact_summary(&mut messages, summary);
    let Some(message) = messages.first() else {
        return Ok(());
    };
    save_code_web_timeline_message(
        store_dir,
        session_id,
        message,
        context_limit,
        auto_compact_threshold,
    )
}

fn compact_visible_messages_after_success(
    session_id: &str,
    mut messages: Vec<Value>,
    _summary: &str,
) -> Vec<Value> {
    messages.push(json!({
        "id": format!("{}-compact", chrono::Utc::now().timestamp_millis()),
        "sessionId": session_id,
        "role": "system",
        "content": "Context compacted for the model.",
        "createdAt": chrono::Utc::now().to_rfc3339(),
    }));
    messages
}

fn apply_settings_patch(
    settings: &mut CodeWebSessionSettings,
    patch: &Value,
    default_model: Option<String>,
) -> BootResult<bool> {
    let Some(patch) = patch.as_object() else {
        return Err(BootError::BadRequest(
            "session update body must be an object".to_string(),
        ));
    };
    let previous_model = settings.model.clone();
    let previous_follow_default_model = settings.follow_default_model;

    let follow_default_patch = match patch.get("followDefaultModel") {
        Some(Value::Bool(value)) => Some(*value),
        Some(Value::Null) | None => None,
        Some(_) => {
            return Err(BootError::BadRequest(
                "followDefaultModel must be a boolean".to_string(),
            ));
        }
    };

    if let Some(model_value) = patch.get("model") {
        match model_value {
            Value::String(model) => {
                let model = model.trim();
                if model.is_empty() {
                    settings.model = None;
                    settings.follow_default_model = follow_default_patch.unwrap_or(true);
                } else {
                    settings.model = Some(model.to_string());
                    settings.follow_default_model = follow_default_patch.unwrap_or(false);
                }
            }
            Value::Null => {
                settings.model = None;
                settings.follow_default_model = follow_default_patch.unwrap_or(true);
            }
            _ => return Err(BootError::BadRequest("model must be a string".to_string())),
        }
    } else if let Some(follow_default_model) = follow_default_patch {
        settings.follow_default_model = follow_default_model;
        if follow_default_model {
            settings.model = None;
        } else if settings.model.is_none() {
            settings.model = default_model;
        }
    }

    if let Some(permission_mode_value) = patch.get("permissionMode") {
        let permission_mode = permission_mode_value
            .as_str()
            .ok_or_else(|| BootError::BadRequest("permissionMode must be a string".to_string()))?;
        settings.permission_mode = normalize_permission_mode(permission_mode).ok_or_else(|| {
            BootError::BadRequest(format!("unsupported permissionMode `{permission_mode}`"))
        })?;
    }

    if let Some(planning_mode_value) = patch.get("planningMode") {
        match planning_mode_value {
            Value::String(planning_mode) => {
                settings.planning_mode =
                    Some(normalize_planning_mode(planning_mode).ok_or_else(|| {
                        BootError::BadRequest(format!("unsupported planningMode `{planning_mode}`"))
                    })?);
            }
            Value::Null => settings.planning_mode = None,
            _ => {
                return Err(BootError::BadRequest(
                    "planningMode must be a string or null".to_string(),
                ));
            }
        }
    }

    if let Some(goal_tracking_value) = patch.get("goalTracking") {
        match goal_tracking_value {
            Value::Bool(goal_tracking) => settings.goal_tracking = Some(*goal_tracking),
            Value::Null => settings.goal_tracking = None,
            _ => {
                return Err(BootError::BadRequest(
                    "goalTracking must be a boolean or null".to_string(),
                ));
            }
        }
    }

    Ok(previous_model != settings.model
        || previous_follow_default_model != settings.follow_default_model)
}

fn normalize_permission_mode(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "default" | "plan" | "auto" => Some(value.trim().to_ascii_lowercase()),
        _ => None,
    }
}

fn normalize_planning_mode(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" | "enabled" | "disabled" => Some(value.trim().to_ascii_lowercase()),
        _ => None,
    }
}

fn build_reconfigure_context(
    previous_summary: Option<&str>,
    messages: &[serde_json::Value],
    history: &[Message],
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(summary) = previous_summary
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("Existing compacted summary:\n{summary}"));
    }
    let visible_transcript = fork_transcript_from_messages(messages);
    if !visible_transcript.is_empty() {
        parts.push(format!(
            "Visible conversation transcript:\n{visible_transcript}"
        ));
    } else {
        let history_transcript = fork_transcript_from_history(history);
        if !history_transcript.is_empty() {
            parts.push(format!("Core conversation history:\n{history_transcript}"));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(truncate_chars(
            &format!(
                "This session was reconfigured. Continue with this carried context:\n\n{}",
                parts.join("\n\n")
            ),
            12_000,
        ))
    }
}

fn fork_title(focus: &str) -> String {
    let focus = focus.trim();
    if focus.is_empty() {
        "Forked session".to_string()
    } else {
        format!("Fork: {}", truncate_chars(focus, 48))
    }
}

fn fork_messages(
    source_session_id: &str,
    target_session_id: &str,
    source_messages: &[serde_json::Value],
    focus: &str,
) -> Vec<serde_json::Value> {
    let now = chrono::Utc::now();
    let mut messages: Vec<serde_json::Value> = source_messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let mut cloned = message.clone();
            if let Value::Object(map) = &mut cloned {
                map.insert(
                    "id".to_string(),
                    Value::String(format!("{}-fork-copy-{index}", now.timestamp_millis())),
                );
                map.insert(
                    "sessionId".to_string(),
                    Value::String(target_session_id.to_string()),
                );
            }
            cloned
        })
        .collect();
    let focus_line = focus
        .trim()
        .is_empty()
        .then(String::new)
        .unwrap_or_else(|| format!("\nFocus: {}", focus.trim()));
    messages.push(json!({
        "id": format!("{}-fork", now.timestamp_millis()),
        "sessionId": target_session_id,
        "role": "system",
        "content": format!("Forked from session `{source_session_id}`.{focus_line}"),
        "createdAt": now.to_rfc3339(),
    }));
    messages
}

fn build_fork_context(
    source_session_id: &str,
    focus: &str,
    previous_summary: Option<&str>,
    source_messages: &[serde_json::Value],
    source_history: &[Message],
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(summary) = previous_summary
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("Existing compacted summary:\n{summary}"));
    }
    let visible_transcript = fork_transcript_from_messages(source_messages);
    if !visible_transcript.is_empty() {
        parts.push(format!(
            "Visible conversation transcript:\n{visible_transcript}"
        ));
    } else {
        let history_transcript = fork_transcript_from_history(source_history);
        if !history_transcript.is_empty() {
            parts.push(format!("Core conversation history:\n{history_transcript}"));
        }
    }
    if parts.is_empty() {
        return None;
    }
    let focus_line = focus
        .trim()
        .is_empty()
        .then(String::new)
        .unwrap_or_else(|| format!("\nFork focus: {}", focus.trim()));
    Some(truncate_chars(
        &format!(
            "This session was forked from `{source_session_id}`.{focus_line}\n\n{}",
            parts.join("\n\n")
        ),
        12_000,
    ))
}

fn fork_transcript_from_messages(messages: &[serde_json::Value]) -> String {
    let mut lines = Vec::new();
    for message in messages.iter().rev().take(24).rev() {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("message");
        let Some(content) = message.get("content").and_then(Value::as_str) else {
            continue;
        };
        let content = content.trim();
        if content.is_empty() {
            continue;
        }
        lines.push(format!("{role}: {}", truncate_chars(content, 1200)));
    }
    truncate_chars(&lines.join("\n\n"), 10_000)
}

fn fork_transcript_from_history(history: &[Message]) -> String {
    let mut lines = Vec::new();
    for message in history.iter().rev().take(24).rev() {
        let content = message_text(message);
        if content.trim().is_empty() {
            continue;
        }
        lines.push(format!(
            "{}: {}",
            message.role,
            truncate_chars(content.trim(), 1200)
        ));
    }
    truncate_chars(&lines.join("\n\n"), 10_000)
}

fn message_text(message: &Message) -> String {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            ContentBlock::ToolUse { name, .. } => Some(name.as_str()),
            ContentBlock::ToolResult { .. } | ContentBlock::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone)]
struct PendingToolUse {
    tool_name: String,
    input: String,
    created_at: Option<String>,
    source_message_id: String,
}

struct ShellOutputRecordInput<'a> {
    session_id: &'a str,
    command: &'a str,
    cwd: &'a str,
    stdout: &'a str,
    stderr: &'a str,
    output: &'a str,
    exit_code: Option<i32>,
    is_error: bool,
    timed_out: bool,
    duration_ms: u64,
    started_at: &'a str,
    completed_at: &'a str,
}

fn shell_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn shell_output_record(input: ShellOutputRecordInput<'_>) -> Value {
    let tool_use_id = format!(
        "shell-{}",
        input
            .started_at
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>()
    );
    json!({
        "id": tool_use_id,
        "index": 0,
        "toolUseId": tool_use_id,
        "toolName": "shell_command",
        "input": input.command,
        "output": input.output,
        "stdout": input.stdout,
        "stderr": input.stderr,
        "cwd": input.cwd,
        "exitCode": input.exit_code,
        "success": !input.is_error,
        "isError": input.is_error,
        "timedOut": input.timed_out,
        "durationMs": input.duration_ms,
        "createdAt": input.started_at,
        "completedAt": input.completed_at,
        "sourceMessageId": Value::Null,
        "resultMessageId": Value::Null,
        "sessionId": input.session_id,
    })
}

fn session_output_json(session_id: &str, messages: &[Value]) -> Value {
    let items = tool_output_records_from_messages(messages);
    let total = items.len();
    json!({
        "sessionId": session_id,
        "items": items,
        "total": total,
        "format": "structured-tool-log",
    })
}

fn tool_output_records_from_messages(messages: &[Value]) -> Vec<Value> {
    let mut records = Vec::new();
    let mut pending: HashMap<String, PendingToolUse> = HashMap::new();
    let mut last_tool_use_id: Option<String> = None;

    for (message_index, message) in messages.iter().enumerate() {
        let source_message_id = string_field(message, &["id", "messageId", "message_id"])
            .unwrap_or_else(|| format!("message-{message_index}"));
        let created_at = string_field(
            message,
            &["createdAt", "created_at", "timestamp", "created"],
        );

        for (block_index, block) in message_blocks(message).into_iter().enumerate() {
            let block_type = string_field(block, &["type"]).unwrap_or_default();
            match block_type.as_str() {
                "tool_use" | "toolUse" | "tool-call-input" => {
                    let tool_use_id = string_field(
                        block,
                        &[
                            "id",
                            "toolUseId",
                            "tool_use_id",
                            "toolCallId",
                            "tool_call_id",
                        ],
                    )
                    .unwrap_or_else(|| format!("tool-{message_index}-{block_index}"));
                    let tool_name = string_field(
                        block,
                        &["name", "toolName", "tool_name", "tool", "function"],
                    )
                    .unwrap_or_else(|| "tool".to_string());
                    let input = first_field(
                        block,
                        &["input", "toolInput", "tool_input", "args", "arguments"],
                    )
                    .map(stringify_json_value)
                    .unwrap_or_default();
                    pending.insert(
                        tool_use_id.clone(),
                        PendingToolUse {
                            tool_name,
                            input,
                            created_at: created_at.clone(),
                            source_message_id: source_message_id.clone(),
                        },
                    );
                    last_tool_use_id = Some(tool_use_id);
                }
                "tool_result" | "toolResult" | "tool-call-output" => {
                    let tool_use_id = string_field(
                        block,
                        &[
                            "toolUseId",
                            "tool_use_id",
                            "toolCallId",
                            "tool_call_id",
                            "id",
                        ],
                    )
                    .or_else(|| last_tool_use_id.clone())
                    .unwrap_or_else(|| format!("tool-result-{message_index}-{block_index}"));
                    let pending_use = pending.remove(&tool_use_id);
                    let tool_name = pending_use
                        .as_ref()
                        .map(|tool_use| tool_use.tool_name.clone())
                        .or_else(|| {
                            string_field(
                                block,
                                &["name", "toolName", "tool_name", "tool", "function"],
                            )
                        })
                        .unwrap_or_else(|| "result".to_string());
                    let input = pending_use
                        .as_ref()
                        .map(|tool_use| tool_use.input.clone())
                        .or_else(|| {
                            first_field(
                                block,
                                &["input", "toolInput", "tool_input", "args", "arguments"],
                            )
                            .map(stringify_json_value)
                        })
                        .unwrap_or_default();
                    records.push(json!({
                        "id": tool_use_id,
                        "index": records.len(),
                        "toolUseId": tool_use_id,
                        "toolName": tool_name,
                        "input": input,
                        "output": first_field(
                            block,
                            &["content", "output", "toolOutput", "tool_output", "result"],
                        )
                        .map(stringify_tool_output)
                        .unwrap_or_default(),
                        "isError": bool_field(block, &["isError", "is_error", "error"]).unwrap_or(false),
                        "exitCode": first_field(block, &["exitCode", "exit_code", "status"]).cloned().unwrap_or(Value::Null),
                        "before": first_field(block, &["before"]).cloned().unwrap_or(Value::Null),
                        "after": first_field(block, &["after"]).cloned().unwrap_or(Value::Null),
                        "filePath": first_field(block, &["filePath", "file_path", "path"]).cloned().unwrap_or(Value::Null),
                        "durationMs": first_field(block, &["durationMs", "duration_ms", "elapsedMs", "elapsed_ms"]).cloned().unwrap_or(Value::Null),
                        "createdAt": pending_use
                            .as_ref()
                            .and_then(|tool_use| tool_use.created_at.clone())
                            .or_else(|| created_at.clone()),
                        "sourceMessageId": pending_use
                            .as_ref()
                            .map(|tool_use| tool_use.source_message_id.clone())
                            .unwrap_or_else(|| source_message_id.clone()),
                        "resultMessageId": source_message_id.clone(),
                    }));
                }
                "tool_call" | "tool" | "completed_tool" | "completed_tool_call" => {
                    if first_field(
                        block,
                        &["output", "content", "toolOutput", "tool_output", "result"],
                    )
                    .is_none()
                    {
                        continue;
                    }
                    let tool_use_id = string_field(
                        block,
                        &[
                            "toolUseId",
                            "tool_use_id",
                            "toolCallId",
                            "tool_call_id",
                            "id",
                        ],
                    )
                    .unwrap_or_else(|| format!("tool-{message_index}-{block_index}"));
                    records.push(json!({
                        "id": tool_use_id,
                        "index": records.len(),
                        "toolUseId": tool_use_id,
                        "toolName": string_field(block, &["toolName", "tool_name", "name", "tool"])
                            .unwrap_or_else(|| "tool".to_string()),
                        "input": first_field(
                            block,
                            &["input", "toolInput", "tool_input", "args", "arguments"],
                        )
                        .map(stringify_json_value)
                        .unwrap_or_default(),
                        "output": first_field(
                            block,
                            &["output", "content", "toolOutput", "tool_output", "result"],
                        )
                        .map(stringify_tool_output)
                        .unwrap_or_default(),
                        "isError": bool_field(block, &["isError", "is_error", "error"]).unwrap_or(false),
                        "exitCode": first_field(block, &["exitCode", "exit_code", "status"]).cloned().unwrap_or(Value::Null),
                        "before": first_field(block, &["before"]).cloned().unwrap_or(Value::Null),
                        "after": first_field(block, &["after"]).cloned().unwrap_or(Value::Null),
                        "filePath": first_field(block, &["filePath", "file_path", "path"]).cloned().unwrap_or(Value::Null),
                        "durationMs": first_field(block, &["durationMs", "duration_ms", "elapsedMs", "elapsed_ms"]).cloned().unwrap_or(Value::Null),
                        "createdAt": created_at.clone(),
                        "sourceMessageId": source_message_id.clone(),
                        "resultMessageId": source_message_id,
                    }));
                }
                _ => {}
            }
        }
    }

    records
}

fn message_blocks(message: &Value) -> Vec<&Value> {
    if let Some(blocks) = first_field(message, &["contentBlocks", "content_blocks", "blocks"])
        .and_then(Value::as_array)
    {
        return blocks.iter().collect();
    }

    if let Some(blocks) = message.get("content").and_then(Value::as_array) {
        return blocks.iter().collect();
    }

    if let Some(nested_message) = message.get("message") {
        if let Some(blocks) = first_field(
            nested_message,
            &["contentBlocks", "content_blocks", "blocks"],
        )
        .and_then(Value::as_array)
        {
            return blocks.iter().collect();
        }
        if let Some(blocks) = nested_message.get("content").and_then(Value::as_array) {
            return blocks.iter().collect();
        }
    }

    match message.get("type").and_then(Value::as_str) {
        Some("tool_use" | "toolUse" | "tool_result" | "toolResult" | "tool_call" | "tool") => {
            vec![message]
        }
        _ => Vec::new(),
    }
}

fn first_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let map = value.as_object()?;
    keys.iter()
        .filter_map(|key| map.get(*key))
        .find(|field| !field.is_null())
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    first_field(value, keys).and_then(json_scalar_to_string)
}

fn bool_field(value: &Value, keys: &[&str]) -> Option<bool> {
    match first_field(value, keys)? {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Some(true),
            "false" | "no" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn json_scalar_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn stringify_json_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

fn stringify_tool_output(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(|item| {
                    if let Value::String(value) = item {
                        return Some(value.as_str());
                    }
                    item.get("text")
                        .or_else(|| item.get("content"))
                        .or_else(|| item.get("message"))
                        .and_then(Value::as_str)
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.trim().is_empty() {
                stringify_json_value(value)
            } else {
                text
            }
        }
        _ => stringify_json_value(value),
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_accumulator_uses_last_turn_prompt_tokens_and_final_usage() {
        let mut accumulator = CodeWebStreamAccumulator::default();
        accumulator.observe(AgentEvent::TurnEnd {
            turn: 0,
            usage: TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 10,
                total_tokens: 110,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
        });
        accumulator.observe(AgentEvent::ToolStart {
            id: "tool-1".to_string(),
            name: "read".to_string(),
        });
        accumulator.observe(AgentEvent::TurnEnd {
            turn: 1,
            usage: TokenUsage {
                prompt_tokens: 170,
                completion_tokens: 20,
                total_tokens: 190,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
        });
        accumulator.observe(AgentEvent::TextDelta {
            text: "streamed".to_string(),
        });
        accumulator.observe(AgentEvent::End {
            text: "final answer".to_string(),
            usage: TokenUsage {
                prompt_tokens: 270,
                completion_tokens: 30,
                total_tokens: 300,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            verification_summary: Box::new(
                a3s_code_core::verification::VerificationSummary::from_reports(&[]),
            ),
            meta: None,
        });

        let result = accumulator.finish().expect("completed stream");
        assert_eq!(result.last_prompt_tokens, 170);
        assert_eq!(result.usage.prompt_tokens, 270);
        assert_eq!(result.text, "final answer");
        assert_eq!(result.tool_calls_count, 1);
    }

    fn temp_code_web_store_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "a3s-code-web-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn fork_messages_rekeys_copied_messages_and_marks_the_branch() {
        let source_messages = vec![json!({
            "id": "old",
            "sessionId": "source",
            "role": "user",
            "content": "continue the UI parity work",
        })];
        let messages = fork_messages("source", "target", &source_messages, "sleep UI");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["sessionId"], "target");
        assert_ne!(messages[0]["id"], "old");
        assert_eq!(messages[1]["role"], "system");
        assert!(messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("Focus: sleep UI"));
    }

    #[test]
    fn fork_context_prefers_visible_messages_and_keeps_focus() {
        let source_messages = vec![json!({
            "role": "assistant",
            "content": "The toolbar now uses GUI actions for sleep.",
        })];
        let history = vec![Message::user("history fallback")];
        let context = build_fork_context(
            "source",
            "finish fork",
            Some("Earlier compact summary"),
            &source_messages,
            &history,
        )
        .expect("fork context");
        assert!(context.contains("Fork focus: finish fork"));
        assert!(context.contains("Earlier compact summary"));
        assert!(context.contains("The toolbar now uses GUI actions for sleep."));
        assert!(!context.contains("history fallback"));
    }

    #[test]
    fn compact_visible_messages_preserve_history_and_hide_summary_body() {
        let existing = vec![
            json!({
                "id": "user-1",
                "sessionId": "session",
                "role": "user",
                "content": "keep this visible",
            }),
            json!({
                "id": "assistant-1",
                "sessionId": "session",
                "role": "assistant",
                "content": "keep this response",
            }),
        ];
        let summary = "private compact summary that must not be visible";

        let updated = compact_visible_messages_after_success("session", existing.clone(), summary);

        assert_eq!(updated.len(), 3);
        assert_eq!(updated[0], existing[0]);
        assert_eq!(updated[1], existing[1]);
        assert_eq!(updated[2]["sessionId"], "session");
        assert_eq!(updated[2]["role"], "system");
        let marker = updated[2]["content"].as_str().expect("marker content");
        assert!(marker.contains("Context compacted"));
        assert!(!marker.contains(summary));
    }

    #[test]
    fn code_web_timeline_message_persists_timeline_and_context() {
        let store_dir = temp_code_web_store_dir("timeline-context");
        let message = Message::user("hello from code web");

        save_code_web_timeline_message(&store_dir, "session", &message, 128_000, 0.85)
            .expect("save timeline message");
        save_code_web_timeline_message(
            &store_dir,
            "session",
            &Message::assistant("incremental reply"),
            128_000,
            0.85,
        )
        .expect("append timeline message incrementally");

        let timeline_store =
            crate::timeline::TimelineJsonlStore::for_session(&store_dir, "session");
        let events = timeline_store.load_all().expect("load timeline");
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].message.as_ref().unwrap().text(),
            "hello from code web"
        );

        let context_store = crate::compact::ContextJsonStore::for_session(&store_dir, "session");
        let context = context_store
            .load()
            .expect("load context")
            .expect("context");
        assert_eq!(context.source_message_count, 2);
        assert_eq!(context.source_event_count, 2);
        assert_eq!(
            context.source_file_bytes,
            timeline_store.file_len().unwrap()
        );
        assert_eq!(context.messages.len(), 2);
        assert_eq!(context.messages[0].text(), "hello from code web");
        assert_eq!(context.messages[1].text(), "incremental reply");
    }

    #[test]
    fn code_web_compact_summary_persists_hidden_summary_and_context_marker() {
        let store_dir = temp_code_web_store_dir("compact-summary");

        persist_code_web_compact_summary(&store_dir, "session", "compact summary", 128_000, 0.85)
            .expect("persist compact summary");

        let timeline_store =
            crate::timeline::TimelineJsonlStore::for_session(&store_dir, "session");
        let events = timeline_store.load_all().expect("load timeline");
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].event_kind,
            crate::timeline::TranscriptEventKind::ContextSummary
        );
        assert!(!events[0].display.visible);
        assert_eq!(
            events[1].event_kind,
            crate::timeline::TranscriptEventKind::CompactMarker
        );
        assert!(events[1].display.visible);

        let context_store = crate::compact::ContextJsonStore::for_session(&store_dir, "session");
        let context = context_store
            .load()
            .expect("load context")
            .expect("context");
        assert_eq!(context.compact_generation, 1);
        assert_eq!(context.messages.len(), 1);
        assert_eq!(context.messages[0].role, "user");
        assert_eq!(context.messages[0].text(), "compact summary");
        assert!(context
            .messages
            .iter()
            .all(|message| message.role != crate::compact::A3S_COMPACT_ROLE));
    }

    #[test]
    fn settings_patch_pins_model_and_execution_mode() {
        let mut settings = CodeWebSessionSettings::default();
        let changed = apply_settings_patch(
            &mut settings,
            &json!({
                "model": "openai/gpt-5.5",
                "followDefaultModel": false,
                "permissionMode": "plan",
                "planningMode": "enabled",
                "goalTracking": true,
            }),
            Some("openai/default".to_string()),
        )
        .expect("settings patch");
        assert!(changed);
        assert_eq!(settings.model.as_deref(), Some("openai/gpt-5.5"));
        assert!(!settings.follow_default_model);
        assert_eq!(settings.permission_mode, "plan");
        assert_eq!(settings.planning_mode.as_deref(), Some("enabled"));
        assert_eq!(settings.goal_tracking, Some(true));
    }

    #[test]
    fn settings_patch_returns_to_default_model() {
        let mut settings = CodeWebSessionSettings {
            model: Some("openai/gpt-5.5".to_string()),
            follow_default_model: false,
            ..CodeWebSessionSettings::default()
        };
        let changed = apply_settings_patch(
            &mut settings,
            &json!({ "followDefaultModel": true }),
            Some("openai/default".to_string()),
        )
        .expect("settings patch");
        assert!(changed);
        assert!(settings.model.is_none());
        assert!(settings.follow_default_model);
    }

    #[test]
    fn settings_patch_rejects_unknown_execution_mode() {
        let mut settings = CodeWebSessionSettings::default();
        let error =
            apply_settings_patch(&mut settings, &json!({ "permissionMode": "danger" }), None)
                .expect_err("unsupported permission mode should fail");
        assert!(error.to_string().contains("unsupported permissionMode"));
    }

    #[test]
    fn session_output_pairs_tool_use_and_result_blocks() {
        let messages = vec![json!({
            "id": "assistant-1",
            "role": "assistant",
            "createdAt": "2026-07-07T00:00:00Z",
            "contentBlocks": [
                {
                    "type": "tool_use",
                    "id": "call-1",
                    "name": "shell_command",
                    "input": { "command": "just web" }
                },
                {
                    "type": "tool_result",
                    "toolUseId": "call-1",
                    "content": "server started",
                    "isError": false,
                    "exitCode": 0,
                    "durationMs": 1200
                }
            ]
        })];

        let output = session_output_json("session-1", &messages);
        assert_eq!(output["sessionId"], "session-1");
        assert_eq!(output["total"], 1);
        assert_eq!(output["items"][0]["toolUseId"], "call-1");
        assert_eq!(output["items"][0]["toolName"], "shell_command");
        assert_eq!(
            output["items"][0]["input"],
            "{\n  \"command\": \"just web\"\n}"
        );
        assert_eq!(output["items"][0]["output"], "server started");
        assert_eq!(output["items"][0]["exitCode"], 0);
        assert_eq!(output["items"][0]["durationMs"], 1200);
        assert_eq!(output["items"][0]["sourceMessageId"], "assistant-1");
    }

    #[test]
    fn session_output_reads_legacy_content_array_aliases() {
        let messages = vec![json!({
            "id": "assistant-2",
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "tool_use_id": "call-2",
                    "tool_name": "read_file",
                    "tool_input": { "path": "README.md" }
                },
                {
                    "type": "tool_result",
                    "tool_use_id": "call-2",
                    "tool_output": [
                        { "type": "text", "text": "# A3S" }
                    ],
                    "is_error": "true",
                    "file_path": "README.md"
                }
            ]
        })];

        let output = session_output_json("session-2", &messages);
        assert_eq!(output["total"], 1);
        assert_eq!(output["items"][0]["toolName"], "read_file");
        assert_eq!(output["items"][0]["output"], "# A3S");
        assert_eq!(output["items"][0]["isError"], true);
        assert_eq!(output["items"][0]["filePath"], "README.md");
    }

    #[test]
    fn session_output_keeps_result_without_matching_use_visible() {
        let messages = vec![json!({
            "id": "assistant-3",
            "role": "assistant",
            "content_blocks": [
                {
                    "type": "tool_result",
                    "toolCallId": "orphan-result",
                    "name": "result",
                    "result": { "ok": true }
                }
            ]
        })];

        let output = session_output_json("session-3", &messages);
        assert_eq!(output["total"], 1);
        assert_eq!(output["items"][0]["toolUseId"], "orphan-result");
        assert_eq!(output["items"][0]["toolName"], "result");
        assert_eq!(output["items"][0]["output"], "{\n  \"ok\": true\n}");
    }

    #[test]
    fn shell_output_record_is_visible_to_output_page() {
        let record = shell_output_record(ShellOutputRecordInput {
            session_id: "session-shell",
            command: "printf hello",
            cwd: "/workspace",
            stdout: "hello",
            stderr: "",
            output: "hello",
            exit_code: Some(0),
            is_error: false,
            timed_out: false,
            duration_ms: 25,
            started_at: "2026-07-07T00:00:00Z",
            completed_at: "2026-07-07T00:00:00Z",
        });
        let messages = vec![json!({
            "id": "shell-message",
            "role": "assistant",
            "contentBlocks": [
                {
                    "type": "tool_use",
                    "id": record["toolUseId"],
                    "name": "shell_command",
                    "input": {
                        "command": record["input"],
                        "cwd": record["cwd"],
                    }
                },
                {
                    "type": "tool_result",
                    "toolUseId": record["toolUseId"],
                    "content": record["output"],
                    "isError": record["isError"],
                    "exitCode": record["exitCode"],
                    "durationMs": record["durationMs"],
                }
            ]
        })];

        let output = session_output_json("session-shell", &messages);
        assert_eq!(output["total"], 1);
        assert_eq!(output["items"][0]["toolName"], "shell_command");
        assert_eq!(
            output["items"][0]["input"],
            "{\n  \"command\": \"printf hello\",\n  \"cwd\": \"/workspace\"\n}"
        );
        assert_eq!(output["items"][0]["output"], "hello");
        assert_eq!(output["items"][0]["exitCode"], 0);
    }
}
