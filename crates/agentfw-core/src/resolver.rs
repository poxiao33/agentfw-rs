use crate::agent::AgentSpec;
use crate::error::FrameworkError;
use crate::message::Message;
use crate::model::{ModelAdapter, ModelRequest, ModelToolDefinition, SharedModelAdapter};
use crate::state::SessionState;
use crate::tool::ToolDefinition;
use std::collections::HashMap;

pub struct PromptPayload {
    pub system: String,
    pub metadata: serde_json::Value,
}

#[async_trait::async_trait]
pub trait ModelResolver: Send + Sync {
    async fn resolve_model(
        &self,
        session: &SessionState,
        agent: &AgentSpec,
    ) -> Result<SharedModelAdapter, FrameworkError>;
}

#[async_trait::async_trait]
pub trait PromptResolver: Send + Sync {
    async fn resolve_prompt(
        &self,
        session: &SessionState,
        agent: &AgentSpec,
        model: &dyn ModelAdapter,
    ) -> Result<PromptPayload, FrameworkError>;
}

#[async_trait::async_trait]
pub trait ToolResolver: Send + Sync {
    async fn resolve_tools(
        &self,
        session: &SessionState,
        agent: &AgentSpec,
    ) -> Result<Vec<ToolDefinition>, FrameworkError>;
}

#[async_trait::async_trait]
pub trait RouteResolver: Send + Sync {
    // RouteResolver is the runtime authority for delivery decisions.
    // Static route config (e.g. SessionSpec.routes) should be treated as
    // developer-provided input data and consumed by a resolver implementation.
    async fn can_deliver(
        &self,
        session: &SessionState,
        from: &str,
        to: &str,
    ) -> Result<bool, FrameworkError>;
}

#[derive(Debug, Clone, Default)]
pub struct StaticRouteTable {
    rules: HashMap<(String, String), bool>,
}

impl StaticRouteTable {
    pub fn from_rules(
        rules: impl IntoIterator<Item = (String, String, bool)>,
    ) -> Result<Self, FrameworkError> {
        let mut table = HashMap::new();
        for (from, to, allow) in rules {
            let from = from.trim();
            let to = to.trim();
            if from.is_empty() || to.is_empty() {
                return Err(FrameworkError::Runtime(
                    "route rule requires non-empty from/to".to_string(),
                ));
            }
            let key = (from.to_string(), to.to_string());
            if let Some(existing) = table.get(&key) {
                if *existing != allow {
                    return Err(FrameworkError::Runtime(format!(
                        "conflicting route rules for {} -> {}",
                        from, to
                    )));
                }
            } else {
                table.insert(key, allow);
            }
        }
        Ok(Self { rules: table })
    }

    pub fn lookup(&self, from: &str, to: &str) -> Option<bool> {
        self.rules.get(&(from.to_string(), to.to_string())).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

#[async_trait::async_trait]
pub trait MemoryResolver: Send + Sync {
    async fn resolve_history(
        &self,
        session: &SessionState,
        agent: &AgentSpec,
    ) -> Result<Vec<Message>, FrameworkError>;
}

#[async_trait::async_trait]
pub trait HistoryTransform: Send + Sync {
    async fn transform_history(
        &self,
        session: &SessionState,
        agent: &AgentSpec,
        model: &dyn ModelAdapter,
        history: Vec<Message>,
    ) -> Result<Vec<Message>, FrameworkError>;
}

pub struct ResolverBundle {
    pub model: Box<dyn ModelResolver>,
    pub prompt: Box<dyn PromptResolver>,
    pub tools: Box<dyn ToolResolver>,
    pub routes: Box<dyn RouteResolver>,
    pub memory: Box<dyn MemoryResolver>,
    pub history_transform: Box<dyn HistoryTransform>,
}

impl ResolverBundle {
    pub async fn build_request(
        &self,
        session: &SessionState,
        agent: &AgentSpec,
    ) -> Result<
        (
            SharedModelAdapter,
            ModelRequest,
            HashMap<String, ToolDefinition>,
        ),
        FrameworkError,
    > {
        let model = self.model.resolve_model(session, agent).await?;
        let prompt = self
            .prompt
            .resolve_prompt(session, agent, model.as_ref())
            .await?;
        let history = self
            .history_transform
            .transform_history(
                session,
                agent,
                model.as_ref(),
                self.memory.resolve_history(session, agent).await?,
            )
            .await?;
        let tools = self
            .tools
            .resolve_tools(session, agent)
            .await?;
        let mut tool_map = HashMap::new();
        let mut tool_schemas = Vec::new();
        for tool in tools {
            if tool_map.contains_key(&tool.name) {
                return Err(FrameworkError::Protocol(format!(
                    "duplicate tool name resolved for agent {}: {}",
                    agent.id, tool.name
                )));
            }
            tool_schemas.push(ModelToolDefinition {
                name: tool.name.clone(),
                description: tool.description.clone(),
                input_schema: tool.schema.input_schema.clone(),
            });
            tool_map.insert(tool.name.clone(), tool);
        }

        Ok((
            model,
            ModelRequest {
                system: prompt.system,
                messages: history,
                tools: tool_schemas,
                metadata: prompt.metadata,
            },
            tool_map,
        ))
    }
}

/// Builder for [`ResolverBundle`].
///
/// Obtain one via [`ResolverBundle::builder()`].
pub struct ResolverBundleBuilder {
    model: Option<Box<dyn ModelResolver>>,
    prompt: Option<Box<dyn PromptResolver>>,
    tools: Option<Box<dyn ToolResolver>>,
    routes: Option<Box<dyn RouteResolver>>,
    memory: Option<Box<dyn MemoryResolver>>,
    history_transform: Option<Box<dyn HistoryTransform>>,
}

impl ResolverBundleBuilder {
    pub fn new() -> Self {
        Self {
            model: None,
            prompt: None,
            tools: None,
            routes: None,
            memory: None,
            history_transform: None,
        }
    }

    pub fn model(mut self, resolver: impl ModelResolver + 'static) -> Self {
        self.model = Some(Box::new(resolver));
        self
    }

    pub fn prompt(mut self, resolver: impl PromptResolver + 'static) -> Self {
        self.prompt = Some(Box::new(resolver));
        self
    }

    pub fn tools(mut self, resolver: impl ToolResolver + 'static) -> Self {
        self.tools = Some(Box::new(resolver));
        self
    }

    pub fn routes(mut self, resolver: impl RouteResolver + 'static) -> Self {
        self.routes = Some(Box::new(resolver));
        self
    }

    pub fn memory(mut self, resolver: impl MemoryResolver + 'static) -> Self {
        self.memory = Some(Box::new(resolver));
        self
    }

    pub fn history_transform(mut self, transform: impl HistoryTransform + 'static) -> Self {
        self.history_transform = Some(Box::new(transform));
        self
    }

    /// Build the [`ResolverBundle`].
    ///
    /// All fields except `history_transform` are required. If `history_transform`
    /// is not provided, [`NoopHistoryTransform`] is used as the default.
    pub fn build(self) -> Result<ResolverBundle, FrameworkError> {
        Ok(ResolverBundle {
            model: self
                .model
                .ok_or_else(|| FrameworkError::Config("model resolver required".into()))?,
            prompt: self
                .prompt
                .ok_or_else(|| FrameworkError::Config("prompt resolver required".into()))?,
            tools: self
                .tools
                .ok_or_else(|| FrameworkError::Config("tools resolver required".into()))?,
            routes: self
                .routes
                .ok_or_else(|| FrameworkError::Config("routes resolver required".into()))?,
            memory: self
                .memory
                .ok_or_else(|| FrameworkError::Config("memory resolver required".into()))?,
            history_transform: self
                .history_transform
                .unwrap_or_else(|| Box::new(NoopHistoryTransform)),
        })
    }
}

impl Default for ResolverBundleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ResolverBundle {
    pub fn builder() -> ResolverBundleBuilder {
        ResolverBundleBuilder::new()
    }
}

pub struct NoopHistoryTransform;

#[async_trait::async_trait]
impl HistoryTransform for NoopHistoryTransform {
    async fn transform_history(
        &self,
        _session: &SessionState,
        _agent: &AgentSpec,
        _model: &dyn ModelAdapter,
        history: Vec<Message>,
    ) -> Result<Vec<Message>, FrameworkError> {
        Ok(history)
    }
}
