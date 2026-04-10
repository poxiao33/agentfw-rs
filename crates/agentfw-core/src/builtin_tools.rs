use crate::error::FrameworkError;
use crate::state::{AudienceState, RuntimeEffect};
use crate::tool::{
    InMemoryToolCatalog, ToolCall, ToolCatalog, ToolDefinition, ToolExecutor, ToolResult,
    ToolResultStatus, ToolSchema,
};
use serde_json::{json, Value};

pub const SET_VISIBLE_TO_TOOL_ID: &str = "builtin.set_visible_to";

pub struct SetVisibleToExecutor;

#[async_trait::async_trait]
impl ToolExecutor for SetVisibleToExecutor {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult, FrameworkError> {
        let input = call
            .arguments
            .as_object()
            .ok_or_else(|| FrameworkError::Tool("set_visible_to expects an object".to_string()))?;

        if !input.contains_key("visible_to") {
            return Err(FrameworkError::Tool(
                "set_visible_to requires the visible_to field".to_string(),
            ));
        }

        let visible_to_raw = input
            .get("visible_to")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                FrameworkError::Tool("set_visible_to missing visible_to array".to_string())
            })?;

        if visible_to_raw.is_empty() {
            return Err(FrameworkError::Tool(
                "set_visible_to requires at least one target".to_string(),
            ));
        }

        let mut visible_to = Vec::with_capacity(visible_to_raw.len());
        for value in visible_to_raw {
            let Some(target) = value.as_str() else {
                return Err(FrameworkError::Tool(
                    "set_visible_to visible_to items must all be strings".to_string(),
                ));
            };
            let target = target.trim();
            if target.is_empty() {
                return Err(FrameworkError::Tool(
                    "set_visible_to targets must not be empty".to_string(),
                ));
            }
            visible_to.push(target.to_string());
        }

        let AudienceState {
            visible_to: normalized,
        } = AudienceState::normalize(visible_to);

        // If the caller injected a known_agents list, validate all targets against it.
        if let Some(known) = call.meta.get("known_agents").and_then(Value::as_array) {
            let known_set: std::collections::HashSet<&str> =
                known.iter().filter_map(Value::as_str).collect();
            if !known_set.is_empty() {
                for target in &normalized {
                    if !known_set.contains(target.as_str()) {
                        return Err(FrameworkError::Tool(format!(
                            "set_visible_to target '{}' is not a known agent in this session",
                            target
                        )));
                    }
                }
            }
        }

        Ok(ToolResult {
            status: ToolResultStatus::Success,
            summary: format!("audience updated for {}", call.requested_by),
            structured: json!({ "visible_to": normalized }),
            raw_text: String::new(),
            effects: vec![RuntimeEffect::SetAudience {
                visible_to: normalized.clone(),
            }],
            meta: json!({ "call_id": call.call_id }),
        })
    }
}

pub fn set_visible_to_tool() -> ToolDefinition {
    ToolDefinition::new(
        SET_VISIBLE_TO_TOOL_ID,
        "set_visible_to",
        "Set which agents can see the following outbound content.",
        ToolSchema {
            input_schema: json!({
                "type": "object",
                "required": ["visible_to"],
                "additionalProperties": false,
                "properties": {
                    "visible_to": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string" },
                        "description": "List of agent IDs that can see subsequent outbound content."
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "visible_to": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                }
            }),
        },
        Box::new(SetVisibleToExecutor),
    )
}

pub fn register_builtin_tools(catalog: &mut dyn ToolCatalog) -> Result<(), FrameworkError> {
    catalog.register(set_visible_to_tool())?;
    Ok(())
}

pub(crate) fn default_builtin_catalog() -> Result<InMemoryToolCatalog, FrameworkError> {
    let mut catalog = InMemoryToolCatalog::new();
    register_builtin_tools(&mut catalog)?;
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RuntimeEffect;

    #[tokio::test]
    async fn set_visible_to_returns_set_audience_effect() {
        let result = SetVisibleToExecutor
            .execute(ToolCall {
                call_id: "call-1".to_string(),
                tool_id: SET_VISIBLE_TO_TOOL_ID.to_string(),
                arguments: json!({"visible_to":["agent:user"]}),
                requested_by: "agent:main".to_string(),
                meta: Value::Null,
            })
            .await
            .expect("execute");

        assert!(result.status == ToolResultStatus::Success);
        assert_eq!(
            result.effects,
            vec![RuntimeEffect::SetAudience {
                visible_to: vec!["agent:user".to_string()],
            }]
        );
    }

    #[tokio::test]
    async fn set_visible_to_removes_duplicates_and_sorts() {
        let result = SetVisibleToExecutor
            .execute(ToolCall {
                call_id: "call-dupe".to_string(),
                tool_id: SET_VISIBLE_TO_TOOL_ID.to_string(),
                arguments: json!({"visible_to":["agent:z", "agent:a", "agent:a"]}),
                requested_by: "agent:main".to_string(),
                meta: Value::Null,
            })
            .await
            .expect("execute");

        assert!(result.status == ToolResultStatus::Success);
        assert_eq!(
            result.effects,
            vec![RuntimeEffect::SetAudience {
                visible_to: vec!["agent:a".to_string(), "agent:z".to_string()],
            }]
        );
    }
}
