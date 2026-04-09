use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::pin::Pin;

use crate::error::{FrameworkError, ModelAdapterError};
use crate::message::ContentBlock;
use futures::Stream;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelCapabilities {
    pub supports_tools: bool,
    pub supports_streaming: bool,
    pub supports_images: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelRequest {
    pub system: String,
    pub messages: Vec<crate::message::Message>,
    pub tools: Vec<ModelToolDefinition>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelStreamChunk {
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub raw: Value,
}

pub type ModelStream =
    Pin<Box<dyn Stream<Item = Result<ModelStreamChunk, ModelAdapterError>> + Send>>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Option<ModelUsage>,
    pub raw: Value,
}

#[async_trait::async_trait]
pub trait ModelAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ModelCapabilities;

    async fn send(&self, request: ModelRequest) -> Result<ModelResponse, FrameworkError>;

    fn stream(&self, _request: ModelRequest) -> Option<ModelStream> {
        None
    }
}

pub type SharedModelAdapter = Arc<dyn ModelAdapter>;
