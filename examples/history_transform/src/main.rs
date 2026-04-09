use agentfw_core::{
    AgentId, AgentSpec, AudienceState, FrameworkError,
    HistoryTransform, Kernel, Message, MessageId, MessageKind, MessageMeta, ModelAdapter,
    ModelCapabilities, ModelRequest, ModelResponse, PromptPayload, PromptResolver, ResolverBundle,
    SessionId, SessionState, StaticMemoryResolver, StaticModelResolver, StaticRouteResolver,
    StaticToolResolver,
};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
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
        let summary = request
            .messages
            .iter()
            .filter_map(|msg| msg.content.first())
            .filter_map(|block| match block {
                agentfw_core::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" | ");

        Ok(ModelResponse {
            content: vec![agentfw_core::ContentBlock::Text {
                text: format!("history transformed into: {summary}"),
            }],
            stop_reason: Some("stop".to_string()),
            usage: None,
            raw: json!({"provider":"dummy"}),
        })
    }
}

struct DummyPromptResolver;

#[async_trait]
impl PromptResolver for DummyPromptResolver {
    async fn resolve_prompt(
        &self,
        _session: &SessionState,
        _agent: &AgentSpec,
        _model: &dyn ModelAdapter,
    ) -> Result<PromptPayload, FrameworkError> {
        Ok(PromptPayload {
            system: "you summarize transformed history".to_string(),
            metadata: serde_json::Value::Null,
        })
    }
}

struct SummarizingHistoryTransform;

#[async_trait]
impl HistoryTransform for SummarizingHistoryTransform {
    async fn transform_history(
        &self,
        session: &SessionState,
        agent: &AgentSpec,
        _model: &dyn ModelAdapter,
        history: Vec<Message>,
    ) -> Result<Vec<Message>, FrameworkError> {
        let texts = history
            .iter()
            .flat_map(|msg| msg.content.iter())
            .filter_map(|block| match block {
                agentfw_core::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();

        let summary = if texts.is_empty() {
            "no history".to_string()
        } else {
            format!("summary({}): {}", texts.len(), texts.join(" / "))
        };

        Ok(vec![Message {
            id: MessageId::from("summary-1"),
            session_id: session.session_id.clone(),
            kind: MessageKind::System,
            from: AgentId::from("agent:history-transform"),
            to: AgentId::from(agent.id.clone()),
            content: vec![agentfw_core::ContentBlock::Text { text: summary }],
            meta: MessageMeta::default(),
            created_at: None,
        }])
    }
}

fn main() {
    // History transformation is developer-defined.
    // The framework exposes storage/resolver hooks but does not own compression strategy.
    let session = SessionState {
        session_id: SessionId::from("history-demo"),
        metadata: serde_json::Value::Null,
    };

    let agent = AgentSpec::new(
        "agent:summary",
        "summary-agent",
        "llm",
        "summary.prompt",
        "dummy.model",
    );

    let mut models = HashMap::new();
    models.insert(
        "dummy.model".to_string(),
        Arc::new(DummyModel) as agentfw_core::SharedModelAdapter,
    );

    let mut prompts = HashMap::new();
    prompts.insert(
        "summary.prompt".to_string(),
        "you summarize history".to_string(),
    );

    let histories = HashMap::from([(
        "history-demo".to_string(),
        HashMap::from([(
            "agent:summary".to_string(),
            vec![
                Message {
                    id: MessageId::from("msg-1"),
                    session_id: SessionId::from("history-demo"),
                    kind: MessageKind::Standard,
                    from: AgentId::from("agent:user"),
                    to: AgentId::from("agent:summary"),
                    content: vec![agentfw_core::ContentBlock::Text {
                        text: "first fact".to_string(),
                    }],
                    meta: MessageMeta::default(),
                    created_at: None,
                },
                Message {
                    id: MessageId::from("msg-2"),
                    session_id: SessionId::from("history-demo"),
                    kind: MessageKind::Standard,
                    from: AgentId::from("agent:user"),
                    to: AgentId::from("agent:summary"),
                    content: vec![agentfw_core::ContentBlock::Text {
                        text: "second fact".to_string(),
                    }],
                    meta: MessageMeta::default(),
                    created_at: None,
                },
            ],
        )]),
    )]);

    let resolvers = ResolverBundle {
        model: Box::new(StaticModelResolver::new(models)),
        prompt: Box::new(DummyPromptResolver),
        tools: Box::new(StaticToolResolver::new(HashMap::new())),
        routes: Box::new(StaticRouteResolver::new(HashMap::from([(
            "agent:summary".to_string(),
            HashMap::from([("agent:user".to_string(), true)]),
        )]))),
        memory: Box::new(StaticMemoryResolver::new(histories)),
        history_transform: Box::new(SummarizingHistoryTransform),
    };

    let mut kernel = Kernel::new();
    kernel
        .register_driver("llm".to_string(), Box::new(agentfw_core::LlmDriver))
        .expect("register driver");

    kernel
        .set_audience_state(
            "history-demo",
            "agent:summary",
            AudienceState {
                visible_to: vec!["agent:user".to_string()],
            },
        )
        .expect("set audience");

    let messages =
        futures::executor::block_on(kernel.run_agent_turn(&session, &resolvers, &agent, &[]))
            .expect("run turn");

    println!(
        "history_transform example dispatched {} message(s) after custom history transform",
        messages.len()
    );
    for msg in messages {
        println!("{} -> {}: {:?}", msg.from, msg.to, msg.content);
    }
}
