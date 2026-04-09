use crate::error::FrameworkError;
use crate::message::ContentBlock;
use crate::model::ModelResponse;
use crate::tool::ToolCall;
use serde_json::{json, Value};

#[derive(Debug, Clone, Default)]
pub struct AgentStep {
    pub outbound_content: Vec<ContentBlock>,
    pub tool_calls: Vec<ToolCall>,
    pub need_retry: bool,
    pub retry_hint: Option<String>,
    pub meta: serde_json::Value,
}

pub trait ProtocolNormalizer: Send + Sync {
    fn normalize(
        &self,
        response: ModelResponse,
        requested_by: &str,
    ) -> Result<AgentStep, FrameworkError>;
}

#[derive(Debug, Default)]
pub struct DefaultProtocolNormalizer;

impl ProtocolNormalizer for DefaultProtocolNormalizer {
    fn normalize(
        &self,
        response: ModelResponse,
        requested_by: &str,
    ) -> Result<AgentStep, FrameworkError> {
        let mut outbound_content = Vec::new();
        let mut tool_calls = Vec::new();

        for (idx, block) in response.content.into_iter().enumerate() {
            match block {
                ContentBlock::ToolCall {
                    tool_name,
                    arguments,
                    call_id,
                } => {
                    let resolved_call_id = match call_id {
                        Some(id) if !id.trim().is_empty() => id,
                        _ => format!("auto-{}-{}", requested_by, idx),
                    };
                    let normalized_arguments = normalize_tool_arguments(arguments);
                    tool_calls.push(ToolCall {
                        call_id: resolved_call_id,
                        tool_id: tool_name,
                        arguments: normalized_arguments,
                        requested_by: requested_by.to_string(),
                        meta: serde_json::Value::Null,
                    });
                }
                other => outbound_content.push(other),
            }
        }

        Ok(AgentStep {
            outbound_content,
            tool_calls,
            need_retry: false,
            retry_hint: None,
            meta: response.raw,
        })
    }
}

fn normalize_tool_arguments(arguments: Value) -> Value {
    match arguments {
        Value::String(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(parsed) => parsed,
            Err(_) => json!({ "_raw": raw }),
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ContentBlock;
    use crate::model::ModelResponse;
    use serde_json::json;

    #[test]
    fn default_normalizer_separates_tool_calls_and_text() {
        let response = ModelResponse {
            content: vec![
                ContentBlock::Text {
                    text: "hello".to_string(),
                },
                ContentBlock::ToolCall {
                    tool_name: "set_visible_to".to_string(),
                    arguments: json!({"visible_to":["agent:user"]}),
                    call_id: Some("call-1".to_string()),
                },
            ],
            stop_reason: Some("tool_calls".to_string()),
            usage: None,
            raw: json!({"raw":true}),
        };

        let step = DefaultProtocolNormalizer
            .normalize(response, "agent:main")
            .expect("normalize");

        assert_eq!(step.outbound_content.len(), 1);
        assert_eq!(step.tool_calls.len(), 1);
        assert_eq!(step.tool_calls[0].tool_id, "set_visible_to");
        assert_eq!(step.tool_calls[0].requested_by, "agent:main");
    }

    #[test]
    fn default_normalizer_decodes_stringified_tool_arguments() {
        let response = ModelResponse {
            content: vec![ContentBlock::ToolCall {
                tool_name: "set_visible_to".to_string(),
                arguments: Value::String("{\"visible_to\":[\"agent:user\"]}".to_string()),
                call_id: Some("call-1".to_string()),
            }],
            stop_reason: Some("tool_calls".to_string()),
            usage: None,
            raw: json!({"raw":true}),
        };

        let step = DefaultProtocolNormalizer
            .normalize(response, "agent:main")
            .expect("normalize");

        assert_eq!(
            step.tool_calls[0].arguments,
            json!({"visible_to":["agent:user"]})
        );
    }
}
