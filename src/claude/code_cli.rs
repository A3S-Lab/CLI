use super::host_tools::{host_tool_instructions, parse_host_tool_calls, HostToolParseResult};
use super::model::canonical_model_name;
use super::protocol::{parse_claude_cli_stream_event, AnthropicEventMapper, StreamMeta};
use a3s_code_core::llm::{
    ContentBlock, LlmResponse, LlmResponseMeta, Message, StreamEvent, TokenUsage, ToolDefinition,
};
use anyhow::{Context, Result};
use serde_json::json;
use std::fmt::Write as _;
use std::process::Stdio;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(crate) struct ClaudeCodeCliAdapter {
    model: String,
}

impl ClaudeCodeCliAdapter {
    pub(crate) fn new(model: &str) -> Self {
        Self {
            model: canonical_model_name(model),
        }
    }

    pub(crate) async fn complete_streaming(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
        cancel_token: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamEvent>> {
        let request_started_at = Instant::now();
        let prompt = claude_cli_prompt(messages);
        let appended_system = claude_cli_system_prompt(system, tools);
        let args = claude_cli_args(&self.model, appended_system.as_deref());
        let mut child = Command::new("claude")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("start Claude Code CLI adapter (`claude`)")?;

        let mut stdin = child
            .stdin
            .take()
            .context("open Claude Code CLI adapter stdin")?;
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
        });

        let stdout = child
            .stdout
            .take()
            .context("capture Claude Code CLI adapter stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("capture Claude Code CLI adapter stderr")?;

        let (tx, rx) = mpsc::channel(100);
        let request_model = self.model.clone();
        let request_url = claude_cli_request_label(&args);
        let host_tools = tools.to_vec();

        tokio::spawn(async move {
            let mut stderr_reader = BufReader::new(stderr);
            let stderr_task = tokio::spawn(async move {
                let mut stderr = String::new();
                let _ = stderr_reader.read_to_string(&mut stderr).await;
                stderr
            });

            let meta = StreamMeta {
                provider: "claude-code-cli",
                request_model,
                request_url,
                started_at: request_started_at,
            };
            let mut lines = BufReader::new(stdout).lines();
            let mut stream_mapper = host_tools
                .is_empty()
                .then(|| AnthropicEventMapper::new(meta.clone()));
            let mut host_tool_mapper =
                (!host_tools.is_empty()).then(|| ClaudeCliHostToolMapper::new(meta, host_tools));

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        let _ = child.kill().await;
                        return;
                    }
                    line = lines.next_line() => {
                        let line = match line {
                            Ok(Some(line)) => line,
                            Ok(None) | Err(_) => break,
                        };
                        let Some(event) = parse_claude_cli_stream_event(&line) else {
                            continue;
                        };
                        let done = if let Some(mapper) = stream_mapper.as_mut() {
                            mapper.handle(event, &tx).await
                        } else if let Some(mapper) = host_tool_mapper.as_mut() {
                            mapper.handle(event, &tx).await
                        } else {
                            false
                        };
                        if done {
                            let _ = child.wait().await;
                            let _ = stderr_task.await;
                            return;
                        }
                    }
                }
            }

            let status = child.wait().await;
            let stderr = stderr_task.await.unwrap_or_default();
            if let Ok(status) = status {
                if !status.success() && !stderr.trim().is_empty() {
                    let _ = tx
                        .send(StreamEvent::TextDelta(format!(
                            "Claude Code CLI adapter failed: {}",
                            stderr.trim()
                        )))
                        .await;
                }
            }
        });

        Ok(rx)
    }
}

fn claude_cli_args(model: &str, appended_system: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "-p".into(),
        "--safe-mode".into(),
        "--model".into(),
        canonical_model_name(model),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
        "--tools".into(),
        String::new(),
        "--no-session-persistence".into(),
    ];
    if let Some(appended_system) =
        appended_system.filter(|appended_system| !appended_system.trim().is_empty())
    {
        args.push("--append-system-prompt".into());
        args.push(appended_system.to_string());
    }
    args
}

fn claude_cli_request_label(args: &[String]) -> String {
    let mut redacted = Vec::with_capacity(args.len());
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            redacted.push("[system prompt redacted]".to_string());
            skip_next = false;
            continue;
        }
        redacted.push(arg.clone());
        if arg == "--append-system-prompt" || arg == "--system-prompt" {
            skip_next = true;
        }
    }
    format!("claude {}", redacted.join(" "))
}

fn claude_cli_system_prompt(system: Option<&str>, tools: &[ToolDefinition]) -> Option<String> {
    let mut prompt = String::new();
    if let Some(system) = system.filter(|system| !system.trim().is_empty()) {
        prompt.push_str("# A3S System\n\n");
        prompt.push_str(system.trim());
        prompt.push_str("\n\n");
    }

    if !tools.is_empty() {
        if let Some(instructions) = host_tool_instructions(tools) {
            prompt.push_str(&instructions);
        }
    }

    (!prompt.trim().is_empty()).then_some(prompt)
}

fn claude_cli_prompt(messages: &[Message]) -> String {
    let mut prompt = String::new();
    prompt.push_str("# Conversation\n");
    for message in messages {
        prompt.push('\n');
        prompt.push_str(match message.role.as_str() {
            "assistant" => "Assistant",
            "user" => "User",
            "system" => "System",
            role => role,
        });
        prompt.push_str(":\n");
        for block in &message.content {
            match block {
                ContentBlock::Text { text } => {
                    prompt.push_str(text);
                    prompt.push('\n');
                }
                ContentBlock::Image { source } => {
                    prompt.push_str(&format!("[image omitted: {}]\n", source.media_type));
                }
                ContentBlock::ToolUse { id, name, input } => {
                    let block = json!({
                        "id": id,
                        "name": name,
                        "input": input,
                    });
                    prompt.push_str("<A3S_ASSISTANT_TOOL_CALL>\n");
                    let _ = writeln!(prompt, "{block}");
                    prompt.push_str("</A3S_ASSISTANT_TOOL_CALL>\n");
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    let status = if is_error.unwrap_or(false) {
                        "error"
                    } else {
                        "ok"
                    };
                    let block = json!({
                        "tool_use_id": tool_use_id,
                        "status": status,
                        "content": content.as_text(),
                    });
                    prompt.push_str("<A3S_TOOL_RESULT>\n");
                    let _ = writeln!(prompt, "{block}");
                    prompt.push_str("</A3S_TOOL_RESULT>\n");
                }
            }
        }
    }
    prompt
}

struct ClaudeCliHostToolMapper {
    meta: StreamMeta,
    tools: Vec<ToolDefinition>,
    text: String,
    usage: TokenUsage,
    stop_reason: Option<String>,
    response_id: Option<String>,
    response_model: Option<String>,
    response_object: Option<String>,
    first_token_ms: Option<u64>,
}

impl ClaudeCliHostToolMapper {
    fn new(meta: StreamMeta, tools: Vec<ToolDefinition>) -> Self {
        Self {
            meta,
            tools,
            text: String::new(),
            usage: TokenUsage::default(),
            stop_reason: None,
            response_id: None,
            response_model: None,
            response_object: Some("message".into()),
            first_token_ms: None,
        }
    }

    async fn handle(
        &mut self,
        event: super::protocol::AnthropicStreamEvent,
        tx: &mpsc::Sender<StreamEvent>,
    ) -> bool {
        match event {
            super::protocol::AnthropicStreamEvent::MessageStart { message } => {
                self.response_id = message.id;
                self.response_model = message.model;
                self.response_object = message.message_type;
                self.usage.prompt_tokens = message.usage.input_tokens;
                self.usage.cache_read_tokens = message.usage.cache_read_input_tokens;
                self.usage.cache_write_tokens = message.usage.cache_creation_input_tokens;
            }
            super::protocol::AnthropicStreamEvent::ContentBlockDelta {
                delta: super::protocol::AnthropicDelta::TextDelta { text },
                ..
            } => {
                self.mark_first_token();
                self.text.push_str(&text);
            }
            super::protocol::AnthropicStreamEvent::ContentBlockDelta { .. } => {}
            super::protocol::AnthropicStreamEvent::MessageDelta { delta, usage } => {
                self.stop_reason = Some(delta.stop_reason);
                self.usage.completion_tokens = usage.output_tokens;
                self.usage.total_tokens = self.usage.prompt_tokens + self.usage.completion_tokens;
            }
            super::protocol::AnthropicStreamEvent::MessageStop => {
                self.finish(tx).await;
                return true;
            }
            super::protocol::AnthropicStreamEvent::Error => return true,
            _ => {}
        }
        false
    }

    async fn finish(&mut self, tx: &mpsc::Sender<StreamEvent>) {
        let mut content = Vec::new();
        let mut stop_reason = self.stop_reason.clone();

        match parse_host_tool_calls(&self.text, &self.tools) {
            HostToolParseResult::Calls(calls) => {
                stop_reason = Some("tool_use".into());
                for call in calls {
                    let input_delta = call.input.to_string();
                    let _ = tx
                        .send(StreamEvent::ToolUseStart {
                            id: call.id.clone(),
                            name: call.name.clone(),
                        })
                        .await;
                    let _ = tx.send(StreamEvent::ToolUseInputDelta(input_delta)).await;
                    content.push(call.into_content_block());
                }
            }
            HostToolParseResult::Invalid(reason) => {
                stop_reason = Some("host_tool_protocol_error".into());
                content.push(ContentBlock::Text {
                    text: format!(
                        "I need to retry the a3s host tool call because {reason}. I should output exactly one valid Claude Code <function_calls> block next."
                    ),
                });
            }
            HostToolParseResult::NoCall if !self.text.is_empty() => {
                let text = std::mem::take(&mut self.text);
                let _ = tx.send(StreamEvent::TextDelta(text.clone())).await;
                content.push(ContentBlock::Text { text });
            }
            HostToolParseResult::NoCall => {}
        }

        let response = LlmResponse {
            message: Message {
                role: "assistant".into(),
                content,
                reasoning_content: None,
            },
            usage: self.usage.clone(),
            stop_reason,
            token_logprobs: Vec::new(),
            meta: Some(LlmResponseMeta {
                provider: Some(self.meta.provider.into()),
                request_model: Some(self.meta.request_model.clone()),
                request_url: Some(self.meta.request_url.clone()),
                response_id: self.response_id.clone(),
                response_model: self.response_model.clone(),
                response_object: self.response_object.clone(),
                first_token_ms: self.first_token_ms,
                duration_ms: Some(self.meta.started_at.elapsed().as_millis() as u64),
            }),
        };
        let _ = tx.send(StreamEvent::Done(response)).await;
    }

    fn mark_first_token(&mut self) {
        if self.first_token_ms.is_none() {
            self.first_token_ms = Some(self.meta.started_at.elapsed().as_millis() as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_cli_args_use_safe_text_only_streaming_mode() {
        let args = claude_cli_args(" claude-opus-4-8[1m] ", Some("secret system prompt"));

        assert!(args.contains(&"--safe-mode".to_string()));
        assert!(args.contains(&"--no-session-persistence".to_string()));
        assert!(args.contains(&"--include-partial-messages".to_string()));
        assert_eq!(
            args.windows(2)
                .find(|window| window[0] == "--model")
                .map(|window| window[1].as_str()),
            Some("claude-opus-4-8")
        );
        assert_eq!(
            args.windows(2)
                .find(|window| window[0] == "--tools")
                .map(|window| window[1].as_str()),
            Some("")
        );
        assert_eq!(
            args.windows(2)
                .find(|window| window[0] == "--append-system-prompt")
                .map(|window| window[1].as_str()),
            Some("secret system prompt")
        );
        assert_eq!(
            claude_cli_request_label(&args),
            "claude -p --safe-mode --model claude-opus-4-8 --output-format stream-json --verbose --include-partial-messages --tools  --no-session-persistence --append-system-prompt [system prompt redacted]"
        );
    }

    #[test]
    fn claude_cli_system_prompt_injects_host_tool_protocol() {
        let prompt = claude_cli_system_prompt(
            Some("Be concise."),
            &[ToolDefinition {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: json!({"type":"object"}),
            }],
        )
        .unwrap();

        assert!(prompt.contains("# A3S System"));
        assert!(prompt.contains("Be concise."));
        assert!(prompt.contains("# A3S Host Tools"));
        assert!(prompt.contains("<function_calls>"));
        assert!(!prompt.contains("<A3S_TOOL_CALLS>"));
    }

    #[test]
    fn claude_cli_prompt_flattens_history_as_structured_tool_blocks() {
        let prompt = claude_cli_prompt(&[
            Message::user("hello"),
            Message {
                role: "assistant".into(),
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_1".into(),
                    name: "read_file".into(),
                    input: json!({"file_path":"README.md"}),
                }],
                reasoning_content: None,
            },
            Message::tool_result("toolu_1", "contents", false),
        ]);

        assert!(!prompt.contains("# A3S Host Tools"));
        assert!(prompt.contains("User:\nhello"));
        assert!(prompt.contains("<A3S_ASSISTANT_TOOL_CALL>"));
        assert!(prompt.contains("\"id\":\"toolu_1\""));
        assert!(prompt.contains("<A3S_TOOL_RESULT>"));
        assert!(prompt.contains("\"status\":\"ok\""));
    }

    #[test]
    fn claude_cli_prompt_keeps_system_out_of_user_prompt() {
        let prompt = claude_cli_prompt(&[
            Message::user("hello"),
            Message {
                role: "assistant".into(),
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_1".into(),
                    name: "read_file".into(),
                    input: json!({"path":"README.md"}),
                }],
                reasoning_content: None,
            },
            Message::tool_result("toolu_1", "contents", false),
        ]);

        assert!(!prompt.contains("Be concise."));
        assert!(!prompt.contains("<A3S_TOOL_CALLS>"));
        assert!(prompt.contains("User:\nhello"));
    }

    #[tokio::test]
    async fn host_tool_mapper_converts_envelope_to_a3s_tool_use() {
        let tools = vec![ToolDefinition {
            name: "read".into(),
            description: "Read a file".into(),
            parameters: json!({
                "type":"object",
                "properties":{"file_path":{"type":"string"}},
                "required":["file_path"]
            }),
        }];
        let mut mapper = ClaudeCliHostToolMapper::new(
            StreamMeta {
                provider: "claude-code-cli",
                request_model: "claude-opus-4-8".into(),
                request_url: "claude -p".into(),
                started_at: Instant::now(),
            },
            tools,
        );
        let (tx, mut rx) = mpsc::channel(10);
        let lines = [
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_1","type":"message","model":"claude-opus-4-8","usage":{"input_tokens":3,"output_tokens":0}}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"<A3S_TOOL_CALLS>{\"calls\":[{\"name\":\"Read\",\"input\":{\"path\":\"README.md\"}}]}</A3S_TOOL_CALLS>"}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_stop"}}"#,
        ];

        for line in lines {
            let event = parse_claude_cli_stream_event(line).unwrap();
            if mapper.handle(event, &tx).await {
                break;
            }
        }

        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::ToolUseStart { name, .. }) if name == "read"
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::ToolUseInputDelta(delta)) if delta.contains("README.md")
        ));
        let Some(StreamEvent::Done(response)) = rx.recv().await else {
            panic!("expected done");
        };
        let calls = response.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read");
        assert_eq!(calls[0].args, json!({"file_path":"README.md"}));
    }

    #[tokio::test]
    async fn host_tool_mapper_converts_claude_function_calls_to_a3s_tool_use() {
        let tools = vec![ToolDefinition {
            name: "read".into(),
            description: "Read a file".into(),
            parameters: json!({
                "type":"object",
                "properties":{"file_path":{"type":"string"}},
                "required":["file_path"]
            }),
        }];
        let mut mapper = ClaudeCliHostToolMapper::new(
            StreamMeta {
                provider: "claude-code-cli",
                request_model: "claude-opus-4-8".into(),
                request_url: "claude -p".into(),
                started_at: Instant::now(),
            },
            tools,
        );
        let (tx, mut rx) = mpsc::channel(10);
        let lines = [
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_1","type":"message","model":"claude-opus-4-8","usage":{"input_tokens":3,"output_tokens":0}}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"<function_calls><invoke name=\"Read\"><parameter name=\"file_path\">README.md</parameter></invoke></function_calls>"}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_stop"}}"#,
        ];

        for line in lines {
            let event = parse_claude_cli_stream_event(line).unwrap();
            if mapper.handle(event, &tx).await {
                break;
            }
        }

        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::ToolUseStart { name, .. }) if name == "read"
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::ToolUseInputDelta(delta)) if delta.contains("README.md")
        ));
        let Some(StreamEvent::Done(response)) = rx.recv().await else {
            panic!("expected done");
        };
        let calls = response.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read");
        assert_eq!(calls[0].args, json!({"file_path":"README.md"}));
    }

    #[tokio::test]
    async fn host_tool_mapper_hides_invalid_protocol_text_and_requests_retry() {
        let tools = vec![ToolDefinition {
            name: "bash".into(),
            description: "Run a command".into(),
            parameters: json!({
                "type":"object",
                "properties":{"command":{"type":"string"}},
                "required":["command"]
            }),
        }];
        let mut mapper = ClaudeCliHostToolMapper::new(
            StreamMeta {
                provider: "claude-code-cli",
                request_model: "claude-opus-4-8".into(),
                request_url: "claude -p".into(),
                started_at: Instant::now(),
            },
            tools,
        );
        let (tx, mut rx) = mpsc::channel(10);
        let lines = [
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_1","type":"message","model":"claude-opus-4-8","usage":{"input_tokens":3,"output_tokens":0}}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"<A3S_TOOL_CALLS>{\"calls\":[{\"name\":\"bash\",\"input\":{}}]}</A3S_TOOL_CALLS>"}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_stop"}}"#,
        ];

        for line in lines {
            let event = parse_claude_cli_stream_event(line).unwrap();
            if mapper.handle(event, &tx).await {
                break;
            }
        }

        let Some(StreamEvent::Done(response)) = rx.recv().await else {
            panic!("expected done");
        };
        assert_eq!(response.tool_calls().len(), 0);
        assert_eq!(
            response.stop_reason.as_deref(),
            Some("host_tool_protocol_error")
        );
        assert!(response.text().contains("retry the a3s host tool call"));
        assert!(!response.text().contains("<A3S_TOOL_CALLS>"));
        drop(tx);
        assert!(rx.recv().await.is_none());
    }
}
