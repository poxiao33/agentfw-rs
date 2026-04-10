use serde::Serialize;
use serde_json::{json, Value};

use crate::error::{FrameworkError, ModelAdapterError};
use crate::http_client::build_bearer_client;
use crate::message::ContentBlock;
use crate::model::{
    ModelAdapter, ModelCapabilities, ModelRequest, ModelResponse, ModelStream, ModelStreamChunk,
    ModelUsage,
};
use futures::StreamExt;

const SSE_DONE_SENTINEL: &str = "[DONE]";

#[derive(Debug, Clone)]
pub struct OpenAIResponsesConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct OpenAIResponsesAdapter {
    client: reqwest::Client,
    config: OpenAIResponsesConfig,
}

impl OpenAIResponsesAdapter {
    pub fn new(config: OpenAIResponsesConfig) -> Result<Self, FrameworkError> {
        let client = build_bearer_client(config.api_key.as_deref())?;
        Ok(Self { client, config })
    }
}

#[async_trait::async_trait]
impl ModelAdapter for OpenAIResponsesAdapter {
    fn name(&self) -> &str {
        "openai-responses"
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            supports_tools: true,
            supports_streaming: true,
            supports_images: false,
        }
    }

    async fn send(&self, request: ModelRequest) -> Result<ModelResponse, FrameworkError> {
        let url = format!("{}/responses", self.config.base_url.trim_end_matches('/'));
        let body = ResponsesRequest {
            model: self.config.model.clone(),
            input: flatten_responses_input(&request),
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
                "openai responses returned status {}: {}",
                status, body
            ))));
        }

        let raw: Value = response
            .json()
            .await
            .map_err(|err| FrameworkError::from(ModelAdapterError::Request(format!("invalid json response: {err}"))))?;

        from_responses(raw)
    }

    fn stream(&self, request: ModelRequest) -> Option<ModelStream> {
        let client = self.client.clone();
        let config = self.config.clone();
        let body = ResponsesRequest {
            model: config.model.clone(),
            input: flatten_responses_input(&request),
            metadata: request.metadata,
        };

        let stream = async_stream::try_stream! {
            let url = format!("{}/responses", config.base_url.trim_end_matches('/'));
            let response = client
                .post(url)
                .json(&json!({
                    "model": body.model,
                    "input": body.input,
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
                    "openai responses stream returned status {}: {}",
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

                    if let Some(output) = raw.get("output").and_then(Value::as_array) {
                        for item in output {
                            if let Some(contents) = item.get("content").and_then(Value::as_array) {
                                for block in contents {
                                    if block.get("type").and_then(Value::as_str) == Some("output_text") {
                                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                                            if !text.trim().is_empty() {
                                                content.push(ContentBlock::Text { text: text.to_string() });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let Some(s) = raw.get("status").and_then(Value::as_str) {
                        stop_reason = Some(s.to_string());
                        if s == "completed" || s == "failed" || s == "cancelled" {
                            done = true;
                        }
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
struct ResponsesRequest {
    model: String,
    input: Vec<Value>,
    metadata: Value,
}

fn flatten_responses_input(request: &ModelRequest) -> Vec<Value> {
    let mut items = Vec::new();
    if !request.system.trim().is_empty() {
        items.push(json!({
            "type": "message",
            "role": "system",
            "content": [{"type": "input_text", "text": request.system}],
        }));
    }
    for msg in &request.messages {
        match msg.kind {
            crate::message::MessageKind::Tool => {
                // Tool results must be submitted as function_call_output items.
                for block in &msg.content {
                    if let ContentBlock::ToolResult { content, call_id, .. } = block {
                        items.push(json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": content.to_string(),
                        }));
                    }
                }
            }
            _ => {
                // Determine role: messages containing ToolCall blocks are assistant turns.
                let has_tool_calls = msg.content.iter().any(|b| matches!(b, ContentBlock::ToolCall { .. }));
                let role = if has_tool_calls { "assistant" } else { "user" };

                let mut content_parts: Vec<Value> = Vec::new();
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } | ContentBlock::System { text } => {
                            if !text.trim().is_empty() {
                                content_parts.push(json!({"type": "input_text", "text": text}));
                            }
                        }
                        ContentBlock::ToolCall { tool_name, arguments, call_id } => {
                            // Assistant tool call — emit as function_call item.
                            items.push(json!({
                                "type": "function_call",
                                "call_id": call_id.as_deref().unwrap_or(""),
                                "name": tool_name,
                                "arguments": arguments.to_string(),
                            }));
                        }
                        ContentBlock::Reference { reference } => {
                            content_parts.push(json!({"type": "input_text", "text": format!("ref: {reference}")}));
                        }
                        ContentBlock::Image { url } => {
                            content_parts.push(json!({"type": "input_text", "text": format!("image: {url}")}));
                        }
                        ContentBlock::ToolResult { .. } => {}
                    }
                }
                if !content_parts.is_empty() {
                    items.push(json!({
                        "type": "message",
                        "role": role,
                        "content": content_parts,
                    }));
                }
            }
        }
    }
    items
}

fn from_responses(raw: Value) -> Result<ModelResponse, FrameworkError> {
    let mut content = Vec::new();
    if let Some(output) = raw.get("output").and_then(Value::as_array) {
        for item in output {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    if let Some(contents) = item.get("content").and_then(Value::as_array) {
                        for block in contents {
                            match block.get("type").and_then(Value::as_str) {
                                Some("output_text") => {
                                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                                        if !text.trim().is_empty() {
                                            content.push(ContentBlock::Text {
                                                text: text.to_string(),
                                            });
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Some("function_call") => {
                    let tool_name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let call_id = item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    let arguments_str = item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}");
                    let arguments = serde_json::from_str(arguments_str)
                        .unwrap_or_else(|_| serde_json::json!({ "_raw": arguments_str }));
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
            .get("status")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        usage,
        raw,
    })
}
