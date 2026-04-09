use agentfw_core::{
    builtin_tools::SET_VISIBLE_TO_TOOL_ID, AgentId, AgentSpec, ContentBlock,
    DeveloperBindings, DeveloperConfig, FrameworkError, Kernel, Message, MessageId,
    MessageKind, MessageMeta, ModelAdapter, ModelCapabilities, ModelRequest, ModelResolver,
    ModelResponse, SessionId, SessionState, StaticHistoryBinding, StaticModelBinding,
    StaticToolBinding,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

struct DummyModel;

#[async_trait]
impl ModelAdapter for DummyModel {
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

struct DemoModelResolver;

#[async_trait]
impl ModelResolver for DemoModelResolver {
    async fn resolve_model(
        &self,
        _session: &SessionState,
        _agent: &AgentSpec,
    ) -> Result<agentfw_core::SharedModelAdapter, FrameworkError> {
        Ok(Arc::new(DummyModel))
    }
}

fn main() {
    // "agent:user" is just an Agent ID provided by developer config.
    // The framework does not treat user as a special built-in role.
    let agent = AgentSpec::new(
        "agent:echo",
        "echo-agent",
        "llm",
        "echo.prompt",
        "dummy.model",
    );

    let session = SessionState {
        session_id: SessionId::from("demo"),
        metadata: serde_json::Value::Null,
    };

    let user_agent = AgentSpec::new(
        "agent:user",
        "user-agent",
        "external",
        "user.prompt",
        "dummy.model",
    );

    let developer_config = DeveloperConfig {
        session: agentfw_core::SessionSpec {
            id: "demo".to_string(),
            agents: vec![agent.clone(), user_agent.clone()],
            routes: vec![agentfw_core::RouteRule {
                from: "agent:echo".to_string(),
                to: "agent:user".to_string(),
                allow: true,
            }],
            metadata: serde_json::Value::Null,
        },
        prompts: std::collections::HashMap::from([
            ("echo.prompt".to_string(), "you are echo-agent".to_string()),
            (
                "user.prompt".to_string(),
                "user placeholder prompt".to_string(),
            ),
        ]),
        models: vec![StaticModelBinding {
            key: "dummy.model".to_string(),
            provider: "openai-compatible".to_string(),
            model: "dummy".to_string(),
            base_url: "http://localhost:0".to_string(),
            api_key_env: String::new(),
        }],
        bindings: DeveloperBindings {
            tools: vec![StaticToolBinding {
                agent_id: "agent:echo".to_string(),
                tool_ids: vec![SET_VISIBLE_TO_TOOL_ID.to_string()],
            }],
            history: vec![StaticHistoryBinding {
                session_id: "demo".to_string(),
                agent_id: "agent:echo".to_string(),
                messages: vec![Message {
                    id: MessageId::from("msg-1"),
                    session_id: SessionId::from("demo"),
                    kind: MessageKind::Standard,
                    from: AgentId::from("agent:user"),
                    to: AgentId::from("agent:echo"),
                    content: vec![agentfw_core::ContentBlock::Text {
                        text: "hello world".to_string(),
                    }],
                    meta: MessageMeta::default(),
                    created_at: None,
                }],
            }],
        },
    };

    developer_config.validate().expect("validate config");
    let static_config = developer_config.into_static();

    let (mut kernel, mut resolvers) =
        Kernel::from_static_config(&static_config, &[agentfw_core::set_visible_to_tool()])
            .expect("build kernel");
    kernel
        .register_driver("llm".to_string(), Box::new(agentfw_core::LlmDriver))
        .expect("register driver");

    resolvers.model = Box::new(DemoModelResolver {});
    kernel
        .set_audience_state(
            "demo",
            "agent:echo",
            agentfw_core::AudienceState {
                visible_to: vec!["agent:user".to_string()],
            },
        )
        .expect("set audience");

    let dispatched =
        futures::executor::block_on(kernel.run_agent_turn(&session, &resolvers, &agent, &[]))
            .expect("run turn");

    println!(
        "minimal example dispatched {} message(s) from configured audience/routing",
        dispatched.len()
    );

    if let Some(message) = dispatched.first() {
        let _ = (&message.from, &message.to);
    }
}
