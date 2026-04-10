use agentfw_core::{
    builtin_tools::set_visible_to_tool, AgentSpec, DeveloperConfig, FrameworkError, Kernel, ModelAdapter, ModelCapabilities, ModelRequest,
    ModelResolver, ModelResponse, SessionId, SessionState,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

struct EchoModel;

#[async_trait]
impl ModelAdapter for EchoModel {
    fn name(&self) -> &str {
        "echo"
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            supports_tools: true,
            supports_streaming: false,
            supports_images: false,
        }
    }

    async fn send(&self, request: ModelRequest) -> Result<ModelResponse, FrameworkError> {
        let text = request
            .messages
            .last()
            .and_then(|msg| msg.content.first())
            .and_then(|block| match block {
                agentfw_core::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "empty".to_string());

        Ok(ModelResponse {
            content: vec![agentfw_core::ContentBlock::Text { text }],
            stop_reason: Some("stop".to_string()),
            usage: None,
            raw: json!({"provider":"echo"}),
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
        Ok(Arc::new(EchoModel))
    }
}

fn main() {
    // This example shows developer-configured static graph input.
    // The framework runs atomic capabilities only; orchestration is owned by caller.
    let config = DeveloperConfig::from_toml_str(
        r#"
[session]
id = "demo"

[[session.agents]]
id = "agent:source"
name = "source"
driver = "llm"
prompt_ref = "source.prompt"
model_ref = "echo.model"

[[session.agents]]
id = "agent:receiver"
name = "receiver"
driver = "llm"
prompt_ref = "receiver.prompt"
model_ref = "echo.model"

[[session.routes]]
from = "agent:source"
to = "agent:receiver"
allow = true

[[session.routes]]
from = "agent:receiver"
to = "agent:source"
allow = true

[prompts]
"source.prompt" = "you are source"
"receiver.prompt" = "you are receiver"

[[models]]
key = "echo.model"
provider = "openai-compatible"
model = "echo"
base_url = "http://localhost:0"

[bindings]

[[bindings.tools]]
agent_id = "agent:source"
tool_ids = ["builtin.set_visible_to"]

[[bindings.tools]]
agent_id = "agent:receiver"
tool_ids = ["builtin.set_visible_to"]
"#,
    )
    .expect("parse developer config");

    config.validate().expect("validate");
    let static_config = config.into_static();

    let (mut kernel, mut resolvers) =
        Kernel::from_static_config(&static_config, &[set_visible_to_tool()]).expect("build kernel");
    kernel
        .register_driver("llm".to_string(), Box::new(agentfw_core::LlmDriver))
        .expect("register driver");
    resolvers.model = Box::new(DemoModelResolver);
    kernel
        .set_audience_state(
            "demo",
            "agent:source",
            agentfw_core::AudienceState {
                visible_to: vec!["agent:receiver".to_string()],
            },
        )
        .expect("set audience");

    let session = SessionState {
        session_id: SessionId::from("demo"),
        metadata: serde_json::Value::Null,
    };
    let source_agent = static_config
        .session
        .agents
        .iter()
        .find(|agent| agent.id == "agent:source")
        .cloned()
        .expect("source agent");
    let messages =
        futures::executor::block_on(kernel.run_agent_turn(&session, &resolvers, &source_agent, &[]))
            .expect("run source");

    println!(
        "multi-agent static example: {} configured agents, {} dispatched message(s)",
        static_config.session.agents.len(),
        messages.len()
    );
    println!(
        "note: routes are developer config input; effective delivery is decided by RouteResolver at runtime"
    );
}
