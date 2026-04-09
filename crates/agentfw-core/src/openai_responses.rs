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
        let raw: Value = response
            .json()
            .await
            .map_err(|err| FrameworkError::from(ModelAdapterError::Request(format!("invalid json response: {err}"))))?;

        if !status.is_success() {
            return Err(FrameworkError::from(ModelAdapterError::Request(format!(
                "openai responses returned status {}: {}",
                status, raw
            ))));
        }

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
        let text = msg
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } | ContentBlock::System { text } => Some(text.clone()),
                ContentBlock::Reference { reference } => Some(format!("ref: {reference}")),
                ContentBlock::ToolResult { tool_name, content, .. } => {
                    Some(format!("tool_result {tool_name}: {content}"))
                }
                ContentBlock::ToolCall { tool_name, arguments, .. } => {
                    Some(format!("tool_call {tool_name}: {arguments}"))
                }
                ContentBlock::Image { url } => Some(format!("image: {url}")),
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !text.trim().is_empty() {
            items.push(json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": text}],
            }));
        }
    }
    items
}

fn from_responses(raw: Value) -> Result<ModelResponse, FrameworkError> {
    let mut content = Vec::new();
    if let Some(output) = raw.get("output").and_then(Value::as_array) {
        for item in output {
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
