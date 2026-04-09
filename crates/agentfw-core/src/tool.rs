use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::FrameworkError;
use crate::state::RuntimeEffect;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolSchema {
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub tool_id: String,
    pub arguments: Value,
    pub requested_by: String,
    pub meta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    Success,
    Error,
    Partial,
    Cancelled,
}

impl Default for ToolResultStatus {
    fn default() -> Self {
        Self::Success
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolResult {
    #[serde(default)]
    pub status: ToolResultStatus,
    pub summary: String,
    pub structured: Value,
    pub raw_text: String,
    pub effects: Vec<RuntimeEffect>,
    pub meta: Value,
}

#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult, FrameworkError>;
}

pub struct ToolDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub schema: ToolSchema,
    pub executor: Arc<dyn ToolExecutor>,
    pub metadata: Value,
}

impl core::fmt::Debug for ToolDefinition {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolDefinition")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("description", &self.description)
            .field("schema", &self.schema)
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl ToolDefinition {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        schema: ToolSchema,
        executor: Box<dyn ToolExecutor>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            schema,
            executor: executor.into(),
            metadata: Value::Null,
        }
    }
}

impl Clone for ToolDefinition {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            schema: self.schema.clone(),
            executor: Arc::clone(&self.executor),
            metadata: self.metadata.clone(),
        }
    }
}

pub trait ToolCatalog {
    fn register(&mut self, definition: ToolDefinition) -> Result<(), FrameworkError>;
    fn get(&self, tool_id: &str) -> Option<&ToolDefinition>;
    fn list(&self) -> Vec<&ToolDefinition>;
}

#[derive(Default)]
pub(crate) struct InMemoryToolCatalog {
    definitions: HashMap<String, ToolDefinition>,
}

impl InMemoryToolCatalog {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

impl ToolCatalog for InMemoryToolCatalog {
    fn register(&mut self, definition: ToolDefinition) -> Result<(), FrameworkError> {
        if self.definitions.contains_key(&definition.id) {
            return Err(FrameworkError::Config(format!(
                "duplicate tool id: {}",
                definition.id
            )));
        }
        self.definitions.insert(definition.id.clone(), definition);
        Ok(())
    }

    fn get(&self, tool_id: &str) -> Option<&ToolDefinition> {
        self.definitions.get(tool_id)
    }

    fn list(&self) -> Vec<&ToolDefinition> {
        self.definitions.values().collect()
    }
}
