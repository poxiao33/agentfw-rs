use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{FrameworkError, ModelAdapterError};
use crate::http_client::build_bearer_client;
use crate::message::{ContentBlock, Message};
use crate::model::{
    ModelAdapter, ModelCapabilities, ModelRequest, ModelResponse, ModelToolDefinition, ModelUsage,
};

#[derive(Debug, Clone)]
pub struct OpenAICompatibleConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct OpenAIChatCompletionsAdapter {
    client: reqwest::Client,
    config: OpenAICompatibleConfig,
}

pub type OpenAICompatibleAdapter = OpenAIChatCompletionsAdapter;

impl OpenAIChatCompletionsAdapter {
    pub fn new(config: OpenAICompatibleConfig) -> Result<Self, FrameworkError> {
        let client = build_bearer_client(config.api_key.as_deref())?;
        Ok(Self { client, config })
    }
}

#[async_trait::async_trait]
impl ModelAdapter for OpenAIChatCompletionsAdapter {
    fn name(&self) -> &str {
        "openai-chat-completions"
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            supports_tools: true,
            supports_streaming: false,
            supports_images: false,
        }
    }

    async fn send(&self, request: ModelRequest) -> Result<ModelResponse, FrameworkError> {
        let body = ChatCompletionsRequest {
            model: self.config.model.clone(),
            messages: to_chat_messages(&request),
            tools: to_chat_tools(&request.tools),
        };

        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );

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
                "openai chat completions returned status {}: {}",
                status, raw
            ))));
        }

        from_chat_response(raw)
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionsRequest {
    model: String,
    messages: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Value>>,
}

fn to_chat_messages(request: &ModelRequest) -> Vec<Value> {
    let mut messages = Vec::new();

    if !request.system.trim().is_empty() {
        messages.push(json!({
            "role": "system",
            "content": request.system,
        }));
    }

    for message in &request.messages {
        // Determine role: tool results are "tool", assistant messages are "assistant",
        // everything else is "user". Alternate user/assistant to satisfy API requirements.
        let role = match message.kind {
            crate::message::MessageKind::Tool => {
                // Tool results go as "tool" role messages
                let tool_messages: Vec<Value> = message.content.iter().filter_map(|block| {
                    if let ContentBlock::ToolResult { tool_name: _, content, call_id, status } = block {
                        let is_error = matches!(status, crate::message::ToolResultStatus::Error);
                        Some(json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": content.to_string(),
                            "is_error": is_error,
                        }))
                    } else {
                        None
                    }
                }).collect();
                if !tool_messages.is_empty() {
                    messages.extend(tool_messages);
                    continue;
                }
                "user"
            }
            _ => {
                // Check if this message contains tool calls (assistant turn)
                let has_tool_calls = message.content.iter().any(|b| matches!(b, ContentBlock::ToolCall { .. }));
                if has_tool_calls { "assistant" } else { "user" }
            }
        };

        let content_parts = flatten_message_content(message);
        let tool_calls: Vec<Value> = message.content.iter().filter_map(|block| {
            if let ContentBlock::ToolCall { tool_name, arguments, call_id } = block {
                Some(json!({
                    "id": call_id.as_deref().filter(|s| !s.trim().is_empty()).unwrap_or(&format!("call-{}", tool_name)),
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": arguments.to_string(),
                    }
                }))
            } else {
                None
            }
        }).collect();

        let mut msg = json!({ "role": role });
        if !tool_calls.is_empty() {
            msg["tool_calls"] = json!(tool_calls);
            if !content_parts.is_empty() {
                msg["content"] = json!(content_parts);
            }
        } else if !content_parts.is_empty() {
            msg["content"] = json!(content_parts);
        } else {
            continue;
        }
        messages.push(msg);
    }

    messages
}

fn flatten_message_content(message: &Message) -> String {
    let mut parts = Vec::new();
    for block in &message.content {
        match block {
            ContentBlock::Text { text } | ContentBlock::System { text } => {
                if !text.trim().is_empty() {
                    parts.push(text.trim().to_string());
                }
            }
            ContentBlock::Reference { reference } => {
                if !reference.trim().is_empty() {
                    parts.push(format!("ref: {}", reference.trim()));
                }
            }
            ContentBlock::ToolResult {
                tool_name, content, ..
            } => {
                parts.push(format!("tool_result {}: {}", tool_name, content));
            }
            ContentBlock::ToolCall {
                tool_name,
                arguments,
                ..
            } => {
                parts.push(format!("tool_call {}: {}", tool_name, arguments));
            }
            ContentBlock::Image { url } => {
                parts.push(format!("image: {}", url));
            }
        }
    }
    parts.join("\n")
}

fn to_chat_tools(tools: &[ModelToolDefinition]) -> Option<Vec<Value>> {
    if tools.is_empty() {
        return None;
    }
    Some(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect(),
    )
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsResponse {
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatAssistantMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatAssistantMessage {
    content: Option<String>,
    tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Debug, Deserialize)]
struct ChatToolCall {
    id: Option<String>,
    function: ChatToolFunction,
}

#[derive(Debug, Deserialize)]
struct ChatToolFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

fn from_chat_response(raw: Value) -> Result<ModelResponse, FrameworkError> {
    let parsed: ChatCompletionsResponse = serde_json::from_value(raw.clone())
        .map_err(|err| FrameworkError::Protocol(format!("invalid chat completion shape: {err}")))?;

    let Some(choice) = parsed.choices.into_iter().next() else {
        return Err(FrameworkError::Protocol(
            "chat completion returned no choices".to_string(),
        ));
    };

    let mut content = Vec::new();
    if let Some(text) = choice.message.content {
        if !text.trim().is_empty() {
            content.push(ContentBlock::Text { text });
        }
    }
    if let Some(tool_calls) = choice.message.tool_calls {
        for call in tool_calls {
            let arguments = serde_json::from_str(&call.function.arguments)
                .unwrap_or_else(|_| json!({ "_raw": call.function.arguments }));
            content.push(ContentBlock::ToolCall {
                tool_name: call.function.name,
                arguments,
                call_id: call.id,
            });
        }
    }

    Ok(ModelResponse {
        content,
        stop_reason: choice.finish_reason,
        usage: parsed.usage.map(|usage| ModelUsage {
            input_tokens: usage.prompt_tokens.unwrap_or_default(),
            output_tokens: usage.completion_tokens.unwrap_or_default(),
        }),
        raw,
    })
}
