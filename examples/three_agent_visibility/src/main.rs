use agentfw_core::{
    set_visible_to_tool, AgentSpec, AudienceState, ContentBlock,
    DeveloperConfig, FrameworkError, Kernel, MessageId, ModelAdapter,
    ModelCapabilities, ModelRequest, ModelResolver, ModelResponse, SessionId, SessionState,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

struct TwoStepModel;

#[async_trait]
impl ModelAdapter for TwoStepModel {
    fn name(&self) -> &str {
        "two-step-model"
    }

    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities {
            supports_tools: true,
            supports_streaming: false,
            supports_images: false,
        }
    }

    async fn send(&self, request: ModelRequest) -> Result<ModelResponse, FrameworkError> {
        let has_tool_result = request.messages.iter().any(|msg| {
            msg.content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolResult { .. }))
        });

        if !has_tool_result {
            return Ok(ModelResponse {
                content: vec![ContentBlock::ToolCall {
                    tool_name: "builtin.set_visible_to".to_string(),
                    arguments: json!({
                        "visible_to": ["agent:observer_b", "agent:observer_a"]
                    }),
                    call_id: Some("call-set-audience".to_string()),
                }],
                stop_reason: Some("tool_calls".to_string()),
                usage: None,
                raw: json!({"provider":"two-step","phase":"set_visible_to"}),
            });
        }

        Ok(ModelResponse {
            content: vec![ContentBlock::Text {
                text: "execution report: breakpoint hit and variables collected".to_string(),
            }],
            stop_reason: Some("stop".to_string()),
            usage: None,
            raw: json!({"provider":"two-step","phase":"final-text"}),
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
        Ok(Arc::new(TwoStepModel))
    }
}

fn main() {
    let config = DeveloperConfig::from_toml_str(
        r#"
[session]
id = "demo-3agent"

[[session.agents]]
id = "agent:source"
name = "source"
driver = "llm"
prompt_ref = "source.prompt"
model_ref = "demo.model"

[[session.agents]]
id = "agent:observer_a"
name = "observer-a"
driver = "external"
prompt_ref = "observer-a.prompt"
model_ref = "demo.model"

[[session.agents]]
id = "agent:observer_b"
name = "observer-b"
driver = "external"
prompt_ref = "observer-b.prompt"
model_ref = "demo.model"

[[session.routes]]
from = "agent:source"
to = "agent:observer_a"
allow = true

[[session.routes]]
from = "agent:source"
to = "agent:observer_b"
allow = true

[prompts]
"source.prompt" = "you are source agent"
"observer-a.prompt" = "you are observer a agent"
"observer-b.prompt" = "you are observer b agent"

[[models]]
key = "demo.model"
provider = "openai-compatible"
model = "demo"
base_url = "http://localhost:0"

[bindings]

[[bindings.tools]]
agent_id = "agent:source"
tool_ids = ["builtin.set_visible_to"]
"#,
    )
    .expect("parse config");

    config.validate().expect("validate");
    let static_config = config.into_static();

    let (mut kernel, mut resolvers) =
        Kernel::from_static_config(&static_config, &[set_visible_to_tool()]).expect("build kernel");
    kernel
        .register_driver("llm".to_string(), Box::new(agentfw_core::LlmDriver))
        .expect("register llm driver");
    kernel
        .register_driver(
            "external".to_string(),
            Box::new(agentfw_core::ExternalDriver),
        )
        .expect("register external driver");
    resolvers.model = Box::new(DemoModelResolver);

    let session = SessionState {
        session_id: SessionId::from("demo-3agent"),
        metadata: serde_json::Value::Null,
    };
    let source = static_config
        .session
        .agents
        .iter()
        .find(|a| a.id == "agent:source")
        .cloned()
        .expect("source agent");

    // Initial audience is single-target, then the agent overwrites it via set_visible_to.
    kernel
        .set_audience_state(
            "demo-3agent",
            "agent:source",
            AudienceState {
                visible_to: vec!["agent:observer_a".to_string()],
            },
        )
        .expect("set initial audience");

    let dispatched =
        futures::executor::block_on(kernel.run_agent_turn(&session, &resolvers, &source, &[]))
            .expect("run execution turn");

    let mut targets: Vec<String> = dispatched.iter().map(|m| m.to.0.clone()).collect();
    targets.sort();

    println!(
        "dispatched {} message(s); targets={:?}",
        dispatched.len(),
        targets
    );

    // Minimal proof that one outbound text was distributed to two targets.
    assert_eq!(dispatched.len(), 2);
    assert_eq!(
        targets,
        vec![
            "agent:observer_a".to_string(),
            "agent:observer_b".to_string()
        ]
    );
    assert!(dispatched.iter().all(|m| m.id != MessageId::from("")));
    assert!(dispatched.iter().all(|m| {
        m.content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { .. }))
    }));
}
