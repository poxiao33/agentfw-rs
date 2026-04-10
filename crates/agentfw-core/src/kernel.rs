use crate::agent::AgentSpec;
use crate::anthropic_messages::{AnthropicMessagesAdapter, AnthropicMessagesConfig};
use crate::config::{StaticConfig, StaticHistoryBinding, StaticModelBinding, StaticToolBinding};
use crate::driver::{DriverRegistry, InMemoryDriverRegistry};
use crate::error::FrameworkError;
use crate::message::{ContentBlock, Message};
use crate::model::SharedModelAdapter;
use crate::openai_compatible::{OpenAICompatibleAdapter, OpenAICompatibleConfig};
use crate::openai_responses::{OpenAIResponsesAdapter, OpenAIResponsesConfig};
use crate::resolver::{
    MemoryResolver, ModelResolver, NoopHistoryTransform, PromptPayload, PromptResolver,
    ResolverBundle, RouteResolver, ToolResolver,
};
use crate::runtime::{AgentTurnResult, InMemoryRuntime, RunEnv, Runtime};
use crate::state::{AudienceState, SessionState};
use crate::storage::{
    AudienceStateStore, InMemoryArchiveStore, InMemoryAudienceStateStore, InMemoryHistoryStore,
};
use crate::tool::ToolDefinition;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;

/// A single-threaded agent execution kernel.
///
/// `Kernel` is not `Send` because it holds `InMemoryRuntime` which uses
/// `#[async_trait(?Send)]`. Use separate `Kernel` instances per thread
/// or wrap in a `tokio::task::LocalSet` for async contexts.
pub struct Kernel {
    pub(crate) drivers: InMemoryDriverRegistry,
    runtime:
        InMemoryRuntime<InMemoryHistoryStore, InMemoryArchiveStore, InMemoryAudienceStateStore>,
}

impl Kernel {
    pub fn new() -> Self {
        Self {
            drivers: InMemoryDriverRegistry::new(),
            runtime: default_runtime(),
        }
    }

    pub fn builder() -> KernelBuilder {
        KernelBuilder::new()
    }

    pub fn from_static_config(
        config: &StaticConfig,
        builtin_tools: &[ToolDefinition],
    ) -> Result<(Self, ResolverBundle), FrameworkError> {
        let kernel = Self::new();
        let resolvers = kernel.build_static_resolvers(config, builtin_tools)?;
        Ok((kernel, resolvers))
    }

    pub fn set_audience_state(
        &mut self,
        session_id: &str,
        agent_id: &str,
        state: AudienceState,
    ) -> Result<(), FrameworkError> {
        self.runtime
            .audience_store_mut()
            .set(session_id, agent_id, state)
    }

    pub(crate) fn audience_store_mut(&mut self) -> &mut InMemoryAudienceStateStore {
        self.runtime.audience_store_mut()
    }

    /// Register an agent driver by key.
    pub fn register_driver(
        &mut self,
        key: impl Into<String>,
        driver: Box<dyn crate::runtime::AgentDriver>,
    ) -> Result<(), FrameworkError> {
        self.drivers.register(key.into(), driver)
    }

    pub async fn execute_agent_turn(
        &mut self,
        session: &SessionState,
        resolvers: &ResolverBundle,
        agent: &AgentSpec,
        incoming: &[Message],
    ) -> Result<AgentTurnResult, FrameworkError> {
        let driver = self.drivers.get(&agent.driver).ok_or_else(|| {
            FrameworkError::DriverNotFound(agent.driver.clone())
        })?;
        driver
            .run_turn(RunEnv { session, resolvers }, agent, incoming)
            .await
    }

    pub fn apply_turn_effects(
        &mut self,
        session: &SessionState,
        agent: &AgentSpec,
        result: &AgentTurnResult,
    ) -> Result<(), FrameworkError> {
        self.runtime
            .apply_effects(&session.session_id.0, &agent.id, &result.effects)
    }

    pub async fn dispatch_turn_content(
        &mut self,
        session: &SessionState,
        resolvers: &ResolverBundle,
        agent: &AgentSpec,
        result: &AgentTurnResult,
    ) -> Result<Vec<Message>, FrameworkError> {
        self.runtime
            .dispatch_content(
                session,
                resolvers.routes.as_ref(),
                &agent.id,
                &result.outbound_content,
            )
            .await
    }

    /// Run a full agent turn: execute → apply audience effects → dispatch → apply remaining effects.
    ///
    /// # Ordering
    ///
    /// `SetAudience` effects are applied before dispatch so that the audience
    /// updated by a tool call (e.g. `set_visible_to`) is visible to the router.
    /// All other effects (`AppendHistory`, `ArchivePayload`) are applied only
    /// after dispatch succeeds, so that a dispatch failure leaves history and
    /// archive in a consistent state.
    ///
    /// Callers requiring finer-grained control can use
    /// [`Kernel::execute_agent_turn`], [`Kernel::dispatch_turn_content`], and
    /// [`Kernel::apply_turn_effects`] separately.
    pub async fn run_agent_turn(
        &mut self,
        session: &SessionState,
        resolvers: &ResolverBundle,
        agent: &AgentSpec,
        incoming: &[Message],
    ) -> Result<Vec<Message>, FrameworkError> {
        let result = self
            .execute_agent_turn(session, resolvers, agent, incoming)
            .await?;

        // Split effects: audience changes must precede dispatch; everything else
        // is committed only after dispatch succeeds.
        let (pre_dispatch, post_dispatch): (Vec<_>, Vec<_>) =
            result.effects.iter().cloned().partition(|e| {
                matches!(e, crate::state::RuntimeEffect::SetAudience { .. })
            });

        let pre_result = AgentTurnResult {
            outbound_content: Vec::new(),
            effects: pre_dispatch,
            meta: serde_json::Value::Null,
        };
        self.apply_turn_effects(session, agent, &pre_result)?;

        // Dispatch using the updated audience.
        let dispatched = self
            .dispatch_turn_content(session, resolvers, agent, &result)
            .await?;

        // Commit remaining effects only after successful dispatch.
        let post_result = AgentTurnResult {
            outbound_content: Vec::new(),
            effects: post_dispatch,
            meta: serde_json::Value::Null,
        };
        self.apply_turn_effects(session, agent, &post_result)?;

        Ok(dispatched)
    }

    pub fn build_static_resolvers(
        &self,
        config: &StaticConfig,
        builtin_tools: &[ToolDefinition],
    ) -> Result<ResolverBundle, FrameworkError> {
        let prompts = config.prompts.clone();
        let models = build_static_models(&config.models)?;
        let toolsets = build_static_toolsets(&config.tool_bindings, builtin_tools)?;
        let rules = build_static_routes(&config.session.agents, &config.session.routes);
        let histories =
            build_static_histories(config.session.id.as_str(), &config.history_bindings);

        Ok(ResolverBundle {
            model: Box::new(StaticModelResolver::new(models)),
            prompt: Box::new(StaticPromptResolver::new(prompts)),
            tools: Box::new(StaticToolResolver::new(toolsets)),
            routes: Box::new(StaticRouteResolver::new(rules)),
            memory: Box::new(StaticMemoryResolver::new(histories)),
            history_transform: Box::new(NoopHistoryTransform),
        })
    }
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

pub struct KernelBuilder {
    drivers: InMemoryDriverRegistry,
    runtime: Option<
        InMemoryRuntime<InMemoryHistoryStore, InMemoryArchiveStore, InMemoryAudienceStateStore>,
    >,
    static_config: Option<StaticConfig>,
    builtin_tools: Vec<ToolDefinition>,
}

impl KernelBuilder {
    pub fn new() -> Self {
        Self {
            drivers: InMemoryDriverRegistry::new(),
            runtime: None,
            static_config: None,
            builtin_tools: Vec::new(),
        }
    }

    pub fn with_default_runtime(mut self) -> Self {
        self.runtime = Some(default_runtime());
        self
    }

    pub fn with_default_drivers(self) -> Result<Self, FrameworkError> {
        self.with_driver("llm".to_string(), Box::new(crate::LlmDriver))?
            .with_driver("llm_tool_loop".to_string(), Box::new(crate::ToolLoopLlmDriver))?
            .with_driver("external".to_string(), Box::new(crate::ExternalDriver))
    }

    pub fn with_default_builtins(self) -> Result<Self, FrameworkError> {
        self.with_builtin_catalog(vec![crate::set_visible_to_tool()])
    }

    pub fn with_builtin_catalog(
        mut self,
        tools: Vec<ToolDefinition>,
    ) -> Result<Self, FrameworkError> {
        self.builtin_tools = tools;
        Ok(self)
    }

    pub fn with_static_config(mut self, config: StaticConfig) -> Self {
        self.static_config = Some(config);
        self
    }

    pub fn with_driver(
        mut self,
        key: impl Into<String>,
        driver: Box<dyn crate::runtime::AgentDriver>,
    ) -> Result<Self, FrameworkError> {
        self.drivers.register(key.into(), driver)?;
        Ok(self)
    }

    pub fn build(self) -> Kernel {
        Kernel {
            drivers: self.drivers,
            runtime: self.runtime.unwrap_or_else(default_runtime),
        }
    }

    pub fn build_static(self) -> Result<(Kernel, ResolverBundle), FrameworkError> {
        let KernelBuilder {
            drivers,
            runtime,
            static_config,
            builtin_tools,
        } = self;
        let config = static_config
            .ok_or_else(|| FrameworkError::Config("static config not provided".to_string()))?;
        let kernel = Kernel {
            drivers,
            runtime: runtime.unwrap_or_else(default_runtime),
        };
        let resolvers = kernel.build_static_resolvers(&config, &builtin_tools)?;
        Ok((kernel, resolvers))
    }
}

macro_rules! impl_static_resolver {
    ($name:ident, $key:ty, $value:ty) => {
        pub struct $name {
            map: HashMap<$key, $value>,
        }
        impl $name {
            pub fn new(map: HashMap<$key, $value>) -> Self {
                Self { map }
            }
        }
    };
}

impl_static_resolver!(StaticPromptResolver, String, String);
impl_static_resolver!(StaticModelResolver, String, SharedModelAdapter);
impl_static_resolver!(StaticToolResolver, String, Vec<ToolDefinition>);

pub struct StaticRouteResolver {
    rules: HashMap<String, HashMap<String, bool>>,
}
impl StaticRouteResolver {
    pub fn new(rules: HashMap<String, HashMap<String, bool>>) -> Self {
        Self { rules }
    }
}

pub struct StaticMemoryResolver {
    histories: HashMap<String, HashMap<String, Vec<Message>>>,
}
impl StaticMemoryResolver {
    pub fn new(histories: HashMap<String, HashMap<String, Vec<Message>>>) -> Self {
        Self { histories }
    }
}

#[async_trait::async_trait]
impl PromptResolver for StaticPromptResolver {
    async fn resolve_prompt(
        &self,
        _session: &SessionState,
        agent: &AgentSpec,
        _model: &dyn crate::model::ModelAdapter,
    ) -> Result<PromptPayload, FrameworkError> {
        Ok(PromptPayload {
            system: self
                .map
                .get(&agent.prompt_ref)
                .cloned()
                .ok_or_else(|| {
                    FrameworkError::Config(format!("prompt ref not found: {}", agent.prompt_ref))
                })?,
            metadata: Value::Null,
        })
    }
}

#[async_trait::async_trait]
impl ModelResolver for StaticModelResolver {
    async fn resolve_model(
        &self,
        _session: &SessionState,
        agent: &AgentSpec,
    ) -> Result<SharedModelAdapter, FrameworkError> {
        self.map.get(&agent.model_ref).cloned().ok_or_else(|| {
            FrameworkError::Config(format!("model ref not found: {}", agent.model_ref))
        })
    }
}

#[async_trait::async_trait]
impl ToolResolver for StaticToolResolver {
    async fn resolve_tools(
        &self,
        _session: &SessionState,
        agent: &AgentSpec,
    ) -> Result<Vec<ToolDefinition>, FrameworkError> {
        Ok(self.map.get(&agent.id).cloned().unwrap_or_default())
    }
}

#[async_trait::async_trait]
impl RouteResolver for StaticRouteResolver {
    async fn can_deliver(
        &self,
        _session: &SessionState,
        from: &str,
        to: &str,
    ) -> Result<bool, FrameworkError> {
        Ok(self
            .rules
            .get(from)
            .and_then(|m| m.get(to))
            .copied()
            .unwrap_or(false))
    }
}

#[async_trait::async_trait]
impl MemoryResolver for StaticMemoryResolver {
    async fn resolve_history(
        &self,
        session: &SessionState,
        agent: &AgentSpec,
    ) -> Result<Vec<Message>, FrameworkError> {
        Ok(self
            .histories
            .get(session.session_id.0.as_str())
            .and_then(|m| m.get(agent.id.as_str()))
            .cloned()
            .unwrap_or_default())
    }
}

fn default_runtime(
) -> InMemoryRuntime<InMemoryHistoryStore, InMemoryArchiveStore, InMemoryAudienceStateStore> {
    InMemoryRuntime::new(
        InMemoryHistoryStore::default(),
        InMemoryArchiveStore::default(),
        InMemoryAudienceStateStore::default(),
    )
}

pub fn text_block(text: impl Into<String>) -> Vec<ContentBlock> {
    vec![ContentBlock::Text { text: text.into() }]
}

pub fn empty_metadata() -> Value {
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentSpec;
    use crate::builtin_tools::set_visible_to_tool;
    use crate::driver::DriverRegistry;
    use crate::message::ContentBlock;
    use crate::message::{AgentId, MessageId, MessageKind, MessageMeta, SessionId, Timestamp};
    use crate::model::{ModelCapabilities, ModelRequest, ModelResponse};
    use crate::storage::AudienceStateStore;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;

    struct DummyModel;

    #[async_trait]
    impl crate::model::ModelAdapter for DummyModel {
        fn name(&self) -> &str {
            "dummy"
        }

        fn capabilities(&self) -> ModelCapabilities {
            ModelCapabilities {
                supports_tools: false,
                supports_streaming: false,
                supports_images: false,
            }
        }

        async fn send(&self, request: ModelRequest) -> Result<ModelResponse, FrameworkError> {
            let last = request
                .messages
                .last()
                .and_then(|msg| msg.content.first())
                .cloned()
                .unwrap_or(ContentBlock::Text {
                    text: "empty".to_string(),
                });
            Ok(ModelResponse {
                content: vec![last],
                stop_reason: Some("stop".to_string()),
                usage: None,
                raw: json!({"provider":"dummy"}),
            })
        }
    }

    #[tokio::test]
    async fn kernel_run_agent_turn_dispatches_visible_content() {
        let agent = AgentSpec::new("agent:echo", "echo", "llm", "echo.prompt", "dummy.model");
        let session = SessionState {
            session_id: SessionId::from("demo"),
            metadata: Value::Null,
        };

        let mut prompts = HashMap::new();
        prompts.insert("echo.prompt".to_string(), "you are echo".to_string());

        let mut models = HashMap::new();
        models.insert(
            "dummy.model".to_string(),
            Arc::new(DummyModel) as SharedModelAdapter,
        );

        let mut toolsets = HashMap::new();
        toolsets.insert("agent:echo".to_string(), vec![set_visible_to_tool()]);

        let mut rules: HashMap<String, HashMap<String, bool>> = HashMap::new();
        rules.entry("agent:echo".to_string()).or_default().insert("agent:user".to_string(), true);

        let mut histories: HashMap<String, HashMap<String, Vec<Message>>> = HashMap::new();
        histories.entry("demo".to_string()).or_default().insert(
            "agent:echo".to_string(),
            vec![Message {
                id: MessageId::from("msg-1"),
                session_id: SessionId::from("demo"),
                kind: MessageKind::Standard,
                from: AgentId::from("agent:user"),
                to: AgentId::from("agent:echo"),
                content: vec![ContentBlock::Text {
                    text: "hello".to_string(),
                }],
                meta: MessageMeta::default(),
                created_at: Some(Timestamp::now_utc_rfc3339()),
            }],
        );

        let resolvers = ResolverBundle {
            model: Box::new(StaticModelResolver::new(models)),
            prompt: Box::new(StaticPromptResolver::new(prompts)),
            tools: Box::new(StaticToolResolver::new(toolsets)),
            routes: Box::new(StaticRouteResolver::new(rules)),
            memory: Box::new(StaticMemoryResolver::new(histories)),
            history_transform: Box::new(NoopHistoryTransform),
        };

        let mut kernel = Kernel::new();
        kernel
            .drivers
            .register("llm".to_string(), Box::new(crate::LlmDriver))
            .expect("register driver");
        kernel
            .audience_store_mut()
            .set(
                "demo",
                "agent:echo",
                crate::AudienceState {
                    visible_to: vec!["agent:user".to_string()],
                },
            )
            .expect("set audience");

        let messages = kernel
            .run_agent_turn(&session, &resolvers, &agent, &[])
            .await
            .expect("run turn");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].to, AgentId::from("agent:user"));
    }

    struct ToolThenTextModel;

    #[async_trait]
    impl crate::model::ModelAdapter for ToolThenTextModel {
        fn name(&self) -> &str {
            "tool-then-text"
        }

        fn capabilities(&self) -> ModelCapabilities {
            ModelCapabilities {
                supports_tools: true,
                supports_streaming: false,
                supports_images: false,
            }
        }

        async fn send(&self, request: ModelRequest) -> Result<ModelResponse, FrameworkError> {
            let saw_tool_result = request.messages.iter().any(|msg| {
                msg.content
                    .iter()
                    .any(|block| matches!(block, ContentBlock::ToolResult { .. }))
            });

            if !saw_tool_result {
                return Ok(ModelResponse {
                    content: vec![ContentBlock::ToolCall {
                        // The model sees tool.name ("set_visible_to"), not tool.id.
                        tool_name: "set_visible_to".to_string(),
                        arguments: json!({"visible_to":["agent:analysis"]}),
                        call_id: Some("call-1".to_string()),
                    }],
                    stop_reason: Some("tool_calls".to_string()),
                    usage: None,
                    raw: json!({"provider":"tool-then-text","stage":"tool"}),
                });
            }

            Ok(ModelResponse {
                content: vec![ContentBlock::Text {
                    text: "handoff report".to_string(),
                }],
                stop_reason: Some("stop".to_string()),
                usage: None,
                raw: json!({"provider":"tool-then-text","stage":"text"}),
            })
        }
    }

    #[tokio::test]
    async fn kernel_run_agent_turn_applies_audience_effect_before_dispatch() {
        let agent =
            AgentSpec::new("agent:exec", "exec", "llm_tool_loop", "exec.prompt", "tool.model");
        let session = SessionState {
            session_id: SessionId::from("demo"),
            metadata: Value::Null,
        };

        let mut prompts = HashMap::new();
        prompts.insert("exec.prompt".to_string(), "you are exec".to_string());

        let mut models = HashMap::new();
        models.insert(
            "tool.model".to_string(),
            Arc::new(ToolThenTextModel) as SharedModelAdapter,
        );

        let mut toolsets = HashMap::new();
        toolsets.insert("agent:exec".to_string(), vec![set_visible_to_tool()]);

        let mut rules: HashMap<String, HashMap<String, bool>> = HashMap::new();
        rules.entry("agent:exec".to_string()).or_default().insert("agent:analysis".to_string(), true);
        rules.entry("agent:exec".to_string()).or_default().insert("agent:user".to_string(), false);

        let mut histories: HashMap<String, HashMap<String, Vec<Message>>> = HashMap::new();
        histories.entry("demo".to_string()).or_default().insert(
            "agent:exec".to_string(),
            vec![Message {
                id: MessageId::from("msg-1"),
                session_id: SessionId::from("demo"),
                kind: MessageKind::Standard,
                from: AgentId::from("agent:user"),
                to: AgentId::from("agent:exec"),
                content: vec![ContentBlock::Text {
                    text: "do work".to_string(),
                }],
                meta: MessageMeta::default(),
                created_at: Some(Timestamp::now_utc_rfc3339()),
            }],
        );

        let resolvers = ResolverBundle {
            model: Box::new(StaticModelResolver::new(models)),
            prompt: Box::new(StaticPromptResolver::new(prompts)),
            tools: Box::new(StaticToolResolver::new(toolsets)),
            routes: Box::new(StaticRouteResolver::new(rules)),
            memory: Box::new(StaticMemoryResolver::new(histories)),
            history_transform: Box::new(NoopHistoryTransform),
        };

        let mut kernel = Kernel::new();
        kernel
            .drivers
            .register("llm_tool_loop".to_string(), Box::new(crate::ToolLoopLlmDriver))
            .expect("register driver");
        kernel
            .audience_store_mut()
            .set(
                "demo",
                "agent:exec",
                crate::AudienceState {
                    visible_to: vec!["agent:user".to_string()],
                },
            )
            .expect("set initial audience");

        let messages = kernel
            .run_agent_turn(&session, &resolvers, &agent, &[])
            .await
            .expect("run turn");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].to, AgentId::from("agent:analysis"));
        assert!(matches!(
            messages[0].content.as_slice(),
            [ContentBlock::Text { text }] if text == "handoff report"
        ));
    }

    #[tokio::test]
    async fn kernel_turn_steps_can_be_called_separately() {
        let agent =
            AgentSpec::new("agent:exec", "exec", "llm_tool_loop", "exec.prompt", "tool.model");
        let session = SessionState {
            session_id: SessionId::from("demo"),
            metadata: Value::Null,
        };

        let mut prompts = HashMap::new();
        prompts.insert("exec.prompt".to_string(), "you are exec".to_string());

        let mut models = HashMap::new();
        models.insert(
            "tool.model".to_string(),
            Arc::new(ToolThenTextModel) as SharedModelAdapter,
        );

        let mut toolsets = HashMap::new();
        toolsets.insert("agent:exec".to_string(), vec![set_visible_to_tool()]);

        let mut rules: HashMap<String, HashMap<String, bool>> = HashMap::new();
        rules.entry("agent:exec".to_string()).or_default().insert(
            "agent:analysis".to_string(),
            true,
        );

        let resolvers = ResolverBundle {
            model: Box::new(StaticModelResolver::new(models)),
            prompt: Box::new(StaticPromptResolver::new(prompts)),
            tools: Box::new(StaticToolResolver::new(toolsets)),
            routes: Box::new(StaticRouteResolver::new(rules)),
            memory: Box::new(StaticMemoryResolver::new(HashMap::new())),
            history_transform: Box::new(NoopHistoryTransform),
        };

        let mut kernel = Kernel::new();
        kernel
            .drivers
            .register("llm_tool_loop".to_string(), Box::new(crate::ToolLoopLlmDriver))
            .expect("register driver");
        kernel
            .audience_store_mut()
            .set(
                "demo",
                "agent:exec",
                crate::AudienceState {
                    visible_to: vec!["agent:user".to_string()],
                },
            )
            .expect("set initial audience");

        let result = kernel
            .execute_agent_turn(&session, &resolvers, &agent, &[])
            .await
            .expect("execute turn");

        assert!(matches!(
            result.outbound_content.as_slice(),
            [ContentBlock::Text { text }] if text == "handoff report"
        ));

        kernel
            .apply_turn_effects(&session, &agent, &result)
            .expect("apply effects");
        let messages = kernel
            .dispatch_turn_content(&session, &resolvers, &agent, &result)
            .await
            .expect("dispatch content");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].to, AgentId::from("agent:analysis"));
    }
}

fn resolve_api_key(binding: &StaticModelBinding) -> Result<Option<String>, FrameworkError> {
    let env_name = binding.api_key_env.trim();
    if env_name.is_empty() {
        return Ok(None);
    }
    match env::var(env_name) {
        Ok(val) => Ok(Some(val)),
        Err(_) => Err(FrameworkError::Config(format!(
            "environment variable '{}' required for model '{}' is not set",
            env_name, binding.key
        ))),
    }
}

fn build_static_models(
    bindings: &[StaticModelBinding],
) -> Result<HashMap<String, SharedModelAdapter>, FrameworkError> {
    let mut models = HashMap::new();
    for binding in bindings {
        let api_key = resolve_api_key(binding)?;
        let adapter: SharedModelAdapter = match binding.provider.as_str() {
            "openai-compatible" | "openai-chat-completions" | "openai-compatible-chat" | "openai" => {
                Arc::new(OpenAICompatibleAdapter::new(OpenAICompatibleConfig {
                    base_url: binding.base_url.clone(),
                    api_key,
                    model: binding.model.clone(),
                })?)
            }
            "openai-responses" => Arc::new(OpenAIResponsesAdapter::new(OpenAIResponsesConfig {
                base_url: binding.base_url.clone(),
                api_key,
                model: binding.model.clone(),
            })?),
            "anthropic" | "anthropic-messages" => {
                Arc::new(AnthropicMessagesAdapter::new(AnthropicMessagesConfig {
                    base_url: binding.base_url.clone(),
                    api_key,
                    model: binding.model.clone(),
                    anthropic_version: "2023-06-01".to_string(),
                    max_tokens: 4096,
                })?)
            }
            other => {
                return Err(FrameworkError::Config(format!(
                    "unsupported static model provider: {other}"
                )));
            }
        };
        models.insert(binding.key.clone(), adapter);
    }
    Ok(models)
}

fn build_static_toolsets(
    bindings: &[StaticToolBinding],
    builtin_tools: &[ToolDefinition],
) -> Result<HashMap<String, Vec<ToolDefinition>>, FrameworkError> {
    let tool_index = builtin_tools
        .iter()
        .map(|tool| (tool.id.clone(), tool.clone()))
        .collect::<HashMap<_, _>>();

    let mut toolsets: HashMap<String, Vec<ToolDefinition>> = HashMap::new();
    for binding in bindings {
        let mut resolved = Vec::new();
        for tool_id in &binding.tool_ids {
            match tool_index.get(tool_id) {
                Some(tool) => resolved.push(tool.clone()),
                None => {
                    return Err(FrameworkError::Config(format!(
                        "tool_id '{}' referenced by agent '{}' not found in builtin_tools",
                        tool_id, binding.agent_id
                    )));
                }
            }
        }
        toolsets.insert(binding.agent_id.clone(), resolved);
    }
    Ok(toolsets)
}

fn build_static_routes(
    agents: &[AgentSpec],
    rules: &[crate::config::RouteRule],
) -> HashMap<String, HashMap<String, bool>> {
    let mut route_map: HashMap<String, HashMap<String, bool>> = HashMap::new();
    for rule in rules {
        route_map
            .entry(rule.from.clone())
            .or_default()
            .insert(rule.to.clone(), rule.allow);
    }
    for agent in agents {
        route_map
            .entry(agent.id.clone())
            .or_default()
            .entry(agent.id.clone())
            .or_insert(false);
    }
    route_map
}

fn build_static_histories(
    default_session_id: &str,
    bindings: &[StaticHistoryBinding],
) -> HashMap<String, HashMap<String, Vec<Message>>> {
    let mut histories: HashMap<String, HashMap<String, Vec<Message>>> = HashMap::new();
    for binding in bindings {
        let session_id = if binding.session_id.trim().is_empty() {
            if !default_session_id.is_empty() {
                eprintln!(
                    "[agentfw] warning: history binding for agent '{}' has no session_id; \
                     falling back to session '{}'",
                    binding.agent_id, default_session_id
                );
            }
            default_session_id.to_string()
        } else {
            binding.session_id.clone()
        };
        histories
            .entry(session_id)
            .or_default()
            .insert(binding.agent_id.clone(), binding.messages.clone());
    }
    histories
}
