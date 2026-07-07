use a3s_code_core::llm::{
    ContentBlock, LlmResponse, LlmResponseMeta, Message, StreamEvent, TokenUsage,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;
use tokio::sync::mpsc;

#[derive(Clone)]
pub(crate) struct StreamMeta {
    pub provider: &'static str,
    pub request_model: String,
    pub request_url: String,
    pub started_at: Instant,
}

pub(crate) struct AnthropicEventMapper {
    meta: StreamMeta,
    content_blocks: Vec<ContentBlock>,
    text_content: String,
    current_tool_id: String,
    current_tool_name: String,
    current_tool_input: String,
    usage: TokenUsage,
    stop_reason: Option<String>,
    response_id: Option<String>,
    response_model: Option<String>,
    response_object: Option<String>,
    first_token_ms: Option<u64>,
}

impl AnthropicEventMapper {
    pub fn new(meta: StreamMeta) -> Self {
        Self {
            meta,
            content_blocks: Vec::new(),
            text_content: String::new(),
            current_tool_id: String::new(),
            current_tool_name: String::new(),
            current_tool_input: String::new(),
            usage: TokenUsage::default(),
            stop_reason: None,
            response_id: None,
            response_model: None,
            response_object: Some("message".to_string()),
            first_token_ms: None,
        }
    }

    pub async fn handle(
        &mut self,
        event: AnthropicStreamEvent,
        tx: &mpsc::Sender<StreamEvent>,
    ) -> bool {
        match event {
            AnthropicStreamEvent::MessageStart { message } => {
                self.response_id = message.id;
                self.response_model = message.model;
                self.response_object = message.message_type;
                self.usage.prompt_tokens = message.usage.input_tokens;
                self.usage.cache_read_tokens = message.usage.cache_read_input_tokens;
                self.usage.cache_write_tokens = message.usage.cache_creation_input_tokens;
            }
            AnthropicStreamEvent::ContentBlockStart { content_block, .. } => match content_block {
                AnthropicContentBlock::Text { text } => {
                    let _ = text;
                }
                AnthropicContentBlock::ToolUse { id, name, input } => {
                    if !self.text_content.is_empty() {
                        self.content_blocks.push(ContentBlock::Text {
                            text: std::mem::take(&mut self.text_content),
                        });
                    }
                    self.current_tool_id = id.clone();
                    self.current_tool_name = name.clone();
                    self.current_tool_input = initial_tool_input_json(&input).unwrap_or_default();
                    let _ = tx.send(StreamEvent::ToolUseStart { id, name }).await;
                    if !self.current_tool_input.is_empty() {
                        self.mark_first_token();
                        let _ = tx
                            .send(StreamEvent::ToolUseInputDelta(
                                self.current_tool_input.clone(),
                            ))
                            .await;
                    }
                }
            },
            AnthropicStreamEvent::ContentBlockDelta { delta, .. } => match delta {
                AnthropicDelta::TextDelta { text } => {
                    self.mark_first_token();
                    self.text_content.push_str(&text);
                    let _ = tx.send(StreamEvent::TextDelta(text)).await;
                }
                AnthropicDelta::InputJsonDelta { partial_json } => {
                    self.mark_first_token();
                    self.current_tool_input.push_str(&partial_json);
                    let _ = tx.send(StreamEvent::ToolUseInputDelta(partial_json)).await;
                }
            },
            AnthropicStreamEvent::ContentBlockStop if !self.current_tool_id.is_empty() => {
                let input = parse_tool_input(&self.current_tool_input);
                self.content_blocks.push(ContentBlock::ToolUse {
                    id: self.current_tool_id.clone(),
                    name: self.current_tool_name.clone(),
                    input,
                });
                self.current_tool_id.clear();
                self.current_tool_name.clear();
                self.current_tool_input.clear();
            }
            AnthropicStreamEvent::MessageDelta {
                delta,
                usage: msg_usage,
            } => {
                self.stop_reason = Some(delta.stop_reason);
                self.usage.completion_tokens = msg_usage.output_tokens;
                self.usage.total_tokens = self.usage.prompt_tokens + self.usage.completion_tokens;
            }
            AnthropicStreamEvent::MessageStop => {
                if !self.text_content.is_empty() {
                    self.content_blocks.push(ContentBlock::Text {
                        text: std::mem::take(&mut self.text_content),
                    });
                }
                let response = LlmResponse {
                    message: Message {
                        role: "assistant".to_string(),
                        content: std::mem::take(&mut self.content_blocks),
                        reasoning_content: None,
                    },
                    usage: self.usage.clone(),
                    stop_reason: self.stop_reason.clone(),
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
                return true;
            }
            AnthropicStreamEvent::ContentBlockStop | AnthropicStreamEvent::Ping => {}
            AnthropicStreamEvent::Error => return true,
        }
        false
    }

    fn mark_first_token(&mut self) {
        if self.first_token_ms.is_none() {
            self.first_token_ms = Some(self.meta.started_at.elapsed().as_millis() as u64);
        }
    }
}

pub(crate) fn parse_sse_data(data: &str) -> Option<AnthropicStreamEvent> {
    if data == "[DONE]" {
        return None;
    }
    serde_json::from_str(data).ok()
}

pub(crate) fn parse_claude_cli_stream_event(line: &str) -> Option<AnthropicStreamEvent> {
    let value = serde_json::from_str::<Value>(line).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("stream_event") {
        return None;
    }
    serde_json::from_value(value.get("event")?.clone()).ok()
}

fn initial_tool_input_json(input: &Value) -> Option<String> {
    match input {
        Value::Object(map) if map.is_empty() => None,
        Value::Null => None,
        value => serde_json::to_string(value).ok(),
    }
}

fn parse_tool_input(input: &str) -> Value {
    if input.trim().is_empty() {
        return json!({});
    }
    serde_json::from_str(input).unwrap_or_else(|error| {
        json!({
            "__parse_error": format!(
                "Malformed tool arguments: {error}. Raw input: {input}"
            )
        })
    })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicUsage {
    pub(crate) input_tokens: usize,
    #[serde(rename = "output_tokens")]
    _output_tokens: usize,
    #[serde(default)]
    pub(crate) cache_read_input_tokens: Option<usize>,
    #[serde(default)]
    pub(crate) cache_creation_input_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum AnthropicStreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: AnthropicMessageStart },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        content_block: AnthropicContentBlock,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { delta: AnthropicDelta },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop,
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: AnthropicMessageDeltaData,
        usage: AnthropicOutputUsage,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicMessageStart {
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(rename = "type", default)]
    pub(crate) message_type: Option<String>,
    pub(crate) usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum AnthropicDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicMessageDeltaData {
    pub(crate) stop_reason: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicOutputUsage {
    pub(crate) output_tokens: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_cli_stream_event_lines() {
        let event = parse_claude_cli_stream_event(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}}"#,
        )
        .unwrap();

        assert!(matches!(
            event,
            AnthropicStreamEvent::ContentBlockDelta {
                delta: AnthropicDelta::TextDelta { text }
            } if text == "hi"
        ));
        assert!(parse_claude_cli_stream_event(r#"{"type":"system"}"#).is_none());
    }
}
