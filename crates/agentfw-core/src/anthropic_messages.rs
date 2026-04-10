use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::Serialize;
use serde_json::{json, Value};

use crate::error::{FrameworkError, ModelAdapterError};
use crate::message::ContentBlock;
use crate::model::{
    ModelAdapter, ModelCapabilities, ModelRequest, ModelResponse, ModelStream, ModelStreamChunk,
    ModelUsage,
};
use futures::StreamExt;

const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";
const SSE_DONE_SENTINEL: &str = "[DONE]";

#[derive(Debug, Clone)]
pub struct AnthropicMessagesConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub anthropic_version: String,
    pub max_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct AnthropicMessagesAdapter {
    client: reqwest::Client,
    config: AnthropicMessagesConfig,
}

impl AnthropicMessagesAdapter {
    pub fn new(config: AnthropicMessagesConfig) -> Result<Self, FrameworkError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(api_key) = &config.api_key {
            let key = api_key.trim();
            if key.is_empty() {
                return Err(FrameworkError::from(ModelAdapterError::Request(
                    "anthropic api key must not be empty".to_string(),
                )));
            }
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(key).map_err(|err| {
                    ModelAdapterError::Request(format!("invalid anthropic api key header: {err}"))
                })?,
            );
        }
        headers.insert(
            ANTHROPIC_VERSION_HEADER,
            HeaderValue::from_str(&config.anthropic_version).map_err(|err| {
                ModelAdapterError::Request(format!("invalid anthropic-version header: {err}"))
            })?,
        );

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|err| ModelAdapterError::Request(format!("failed to build reqwest client: {err}")))?;

        Ok(Self { client, config })
    }
}

#[async_trait::async_trait]
impl ModelAdapter for AnthropicMessagesAdapter {
    fn name(&self) -> &str {
        "anthropic-messages"
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            supports_tools: true,
            supports_streaming: true,
            supports_images: false,
        }
    }

    async fn send(&self, request: ModelRequest) -> Result<ModelResponse, FrameworkError> {
        let url = format!("{}/messages", self.config.base_url.trim_end_matches('/'));
        let body = AnthropicMessagesRequest {
            model: self.config.model.clone(),
            max_tokens: self.config.max_tokens,
            system: request.system.clone(),
            messages: flatten_messages(&request),
            metadata: request.metadata,
        };

        let response = self
            .client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|err| FrameworkError::from(ModelAdapterError::Request(format!("request failed: {err}"))))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            return Err(FrameworkError::from(ModelAdapterError::Request(format!(
                "anthropic messages returned status {}: {}",
                status, body
            ))));
        }

        let raw: Value = response
            .json()
            .await
            .map_err(|err| FrameworkError::from(ModelAdapterError::Request(format!("invalid json response: {err}"))))?;

        from_anthropic_response(raw)
    }

    fn stream(&self, request: ModelRequest) -> Option<ModelStream> {
        let client = self.client.clone();
        let config = self.config.clone();
        let body = AnthropicMessagesRequest {
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            system: request.system.clone(),
            messages: flatten_messages(&request),
            metadata: request.metadata,
        };

        let stream = async_stream::try_stream! {
            let url = format!("{}/messages", config.base_url.trim_end_matches('/'));
            let response = client
                .post(url)
                .json(&json!({
                    "model": body.model,
                    "max_tokens": body.max_tokens,
                    "system": body.system,
                    "messages": body.messages,
                    "metadata": body.metadata,
                    "stream": true,
                }))
                .send()
                .await
                .map_err(|err| ModelAdapterError::Streaming(format!("request failed: {err}")))?;

            let status = response.status();
            if !status.is_success() {
                let raw_body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "<unreadable body>".to_string());
                Err(ModelAdapterError::Streaming(format!(
                    "anthropic messages stream returned status {}: {}",
                    status, raw_body
                )))?;
                return;
            }

            const MAX_BUFFER: usize = 4 * 1024 * 1024; // 4 MiB guard
            let mut buffer = String::new();
            let mut done = false;
            let mut byte_stream = response.bytes_stream();

            while !done {
                let Some(chunk) = byte_stream.next().await else { break };
                let chunk = chunk
                    .map_err(|err| ModelAdapterError::Streaming(format!("stream read failed: {err}")))?;
                let text = String::from_utf8_lossy(&chunk);
                buffer.push_str(&text);

                if buffer.len() > MAX_BUFFER {
                    Err(ModelAdapterError::Streaming(
                        "SSE buffer exceeded 4 MiB; possible malformed stream".to_string(),
                    ))?;
                    return;
                }

                while let Some(idx) = buffer.find("\n\n") {
                    let frame = buffer[..idx].to_string();
                    buffer.drain(..idx + 2);

                    let mut data_lines = Vec::new();
                    for line in frame.lines() {
                        if let Some(rest) = line.strip_prefix("data:") {
                            data_lines.push(rest.trim());
                        }
                    }
                    if data_lines.is_empty() {
                        continue;
                    }
                    let data = data_lines.join("\n");
                    if data == SSE_DONE_SENTINEL {
                        done = true;
                        break;
                    }

                    let raw: Value = serde_json::from_str(&data)
                        .map_err(|err| ModelAdapterError::Streaming(format!("invalid json event: {err}")))?;
                    let mut content = Vec::new();
                    let mut stop_reason = None;

                    match raw.get("type").and_then(Value::as_str) {
                        Some("content_block_delta") => {
                            if let Some(text) = raw
                                .get("delta")
                                .and_then(|v| v.get("text"))
                                .and_then(Value::as_str)
                            {
                                if !text.trim().is_empty() {
                                    content.push(ContentBlock::Text { text: text.to_string() });
                                }
                            }
                        }
                        Some("message_stop") => {
                            stop_reason = Some("message_stop".to_string());
                            done = true;
                        }
                        _ => {}
                    }

                    yield ModelStreamChunk {
                        content,
                        stop_reason,
                        raw,
                    };
                }
            }
        };

        Some(Box::pin(stream))
    }
}

#[derive(Debug, Serialize)]
struct AnthropicMessagesRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<Value>,
    metadata: Value,
}

fn flatten_messages(request: &ModelRequest) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();
    for msg in &request.messages {
        // Determine role from message kind / content structure.
        // ToolResult messages always go as "user" (Anthropic wraps them in user turns).
        // ToolCall messages are "assistant". Everything else alternates.
        let role = match msg.kind {
            crate::message::MessageKind::Tool => "user",
            _ => {
                let has_tool_calls = msg.content.iter().any(|b| matches!(b, ContentBlock::ToolCall { .. }));
                if has_tool_calls {
                    "assistant"
                } else {
                    // Alternate based on the last role actually pushed to result.
                    let last_role = result.last()
                        .and_then(|v| v.get("role"))
                        .and_then(Value::as_str)
                        .unwrap_or("assistant"); // default: make next one "user"
                    if last_role == "user" { "assistant" } else { "user" }
                }
            }
        };

        let mut content_blocks: Vec<Value> = Vec::new();
        for block in &msg.content {
            match block {
                ContentBlock::Text { text } | ContentBlock::System { text } => {
                    if !text.trim().is_empty() {
                        content_blocks.push(json!({"type": "text", "text": text}));
                    }
                }
                ContentBlock::ToolCall { tool_name, arguments, call_id } => {
                    content_blocks.push(json!({
                        "type": "tool_use",
                        "id": call_id.as_deref().unwrap_or(""),
                        "name": tool_name,
                        "input": arguments,
                    }));
                }
                ContentBlock::ToolResult { tool_name: _, content, call_id, status } => {
                    let is_error = matches!(status, crate::message::ToolResultStatus::Error);
                    content_blocks.push(json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": content.to_string(),
                        "is_error": is_error,
                    }));
                }
                ContentBlock::Reference { reference } => {
                    content_blocks.push(json!({"type": "text", "text": format!("ref: {reference}")}));
                }
                ContentBlock::Image { url } => {
                    content_blocks.push(json!({"type": "text", "text": format!("image: {url}")}));
                }
            }
        }

        if content_blocks.is_empty() {
            continue;
        }

        // Merge consecutive same-role messages to satisfy Anthropic's alternating requirement.
        if let Some(last) = result.last_mut() {
            if last.get("role").and_then(Value::as_str) == Some(role) {
                if let Some(arr) = last.get_mut("content").and_then(Value::as_array_mut) {
                    arr.extend(content_blocks);
                    continue;
                }
            }
        }

        result.push(json!({"role": role, "content": content_blocks}));
    }
    result
}

fn from_anthropic_response(raw: Value) -> Result<ModelResponse, FrameworkError> {
    let mut content = Vec::new();
    if let Some(blocks) = raw.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        if !text.trim().is_empty() {
                            content.push(ContentBlock::Text {
                                text: text.to_string(),
                            });
                        }
                    }
                }
                Some("tool_use") => {
                    let tool_name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let call_id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    let arguments = block
                        .get("input")
                        .cloned()
                        .unwrap_or(serde_json::Value::Object(Default::default()));
                    content.push(ContentBlock::ToolCall {
                        tool_name,
                        arguments,
                        call_id,
                    });
                }
                _ => {}
            }
        }
    }

    let usage = raw.get("usage").map(|usage| ModelUsage {
        input_tokens: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        output_tokens: usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    });

    Ok(ModelResponse {
        content,
        stop_reason: raw
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        usage,
        raw,
    })
}
