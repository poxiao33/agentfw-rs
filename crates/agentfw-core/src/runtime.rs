use crate::agent::AgentSpec;
use crate::error::FrameworkError;
use crate::message::{AgentId, ContentBlock, Message, MessageDraft, SessionId};
use crate::resolver::{ResolverBundle, RouteResolver};
use crate::state::{AudienceState, RuntimeEffect, SessionState};
use crate::storage::{ArchiveStore, AudienceStateStore, HistoryStore};
use crate::tool::ToolCall;
use async_trait::async_trait;

pub struct RunEnv<'a> {
    pub session: &'a SessionState,
    pub resolvers: &'a ResolverBundle,
}

#[derive(Debug, Clone, Default)]
pub struct AgentTurnResult {
    pub outbound_content: Vec<ContentBlock>,
    pub effects: Vec<RuntimeEffect>,
    pub meta: serde_json::Value,
}

#[async_trait::async_trait]
pub trait AgentDriver {
    async fn run_turn(
        &self,
        env: RunEnv<'_>,
        agent: &AgentSpec,
        incoming: &[Message],
    ) -> Result<AgentTurnResult, FrameworkError>;
}


/// Agent runtime trait. Implementations are not required to be `Send`.
/// The default `InMemoryRuntime` is single-threaded.
#[async_trait(?Send)]
pub trait Runtime {
    fn apply_effects(
        &mut self,
        session_id: &str,
        agent_id: &str,
        effects: &[RuntimeEffect],
    ) -> Result<(), FrameworkError>;

    async fn dispatch_content(
        &mut self,
        session: &SessionState,
        routes: &dyn RouteResolver,
        from_agent: &str,
        content: &[ContentBlock],
    ) -> Result<Vec<Message>, FrameworkError>;
}

pub(crate) struct InMemoryRuntime<H, A, R>
where
    H: HistoryStore,
    A: ArchiveStore,
    R: AudienceStateStore,
{
    history: H,
    archive: A,
    audiences: R,
}

impl<H, A, R> InMemoryRuntime<H, A, R>
where
    H: HistoryStore,
    A: ArchiveStore,
    R: AudienceStateStore,
{
    pub(crate) fn new(history: H, archive: A, audiences: R) -> Self {
        Self {
            history,
            archive,
            audiences,
        }
    }

    pub fn history_store(&self) -> &H {
        &self.history
    }

    pub fn history_store_mut(&mut self) -> &mut H {
        &mut self.history
    }

    pub fn archive_store(&self) -> &A {
        &self.archive
    }

    pub fn archive_store_mut(&mut self) -> &mut A {
        &mut self.archive
    }

    pub fn audience_store(&self) -> &R {
        &self.audiences
    }

    pub fn audience_store_mut(&mut self) -> &mut R {
        &mut self.audiences
    }
}

#[async_trait(?Send)]
impl<H, A, R> Runtime for InMemoryRuntime<H, A, R>
where
    H: HistoryStore,
    A: ArchiveStore,
    R: AudienceStateStore,
{
    fn apply_effects(
        &mut self,
        session_id: &str,
        agent_id: &str,
        effects: &[RuntimeEffect],
    ) -> Result<(), FrameworkError> {
        let mut next_audience: Option<AudienceState> = None;
        let mut archive_writes = Vec::new();
        let mut history_appends = Vec::new();

        for effect in effects {
            match effect {
                RuntimeEffect::SetAudience { visible_to } => {
                    if next_audience.is_some() {
                        return Err(FrameworkError::Runtime(
                            "multiple SetAudience effects in a single turn are not supported; \
                             emit exactly one SetAudience per turn"
                                .to_string(),
                        ));
                    }
                    next_audience = Some(crate::state::AudienceState::normalize(visible_to.clone()));
                }
                RuntimeEffect::ArchivePayload { reference, payload } => {
                    archive_writes.push((reference.clone(), payload.clone()));
                }
                RuntimeEffect::AppendHistory { messages } => {
                    history_appends.extend(messages.clone());
                }
                RuntimeEffect::Custom { name, .. } => {
                    return Err(FrameworkError::Runtime(format!(
                        "custom runtime effect is not supported by in-memory runtime: {}",
                        name
                    )));
                }
            }
        }

        for (reference, payload) in &archive_writes {
            self.archive.save(reference, payload)?;
        }
        if let Some(audience) = next_audience {
            self.audiences.set(session_id, agent_id, audience)?;
        }
        if !history_appends.is_empty() {
            self.history.append(session_id, agent_id, history_appends)?;
        }
        Ok(())
    }

    async fn dispatch_content(
        &mut self,
        session: &SessionState,
        routes: &dyn RouteResolver,
        from_agent: &str,
        content: &[ContentBlock],
    ) -> Result<Vec<Message>, FrameworkError> {
        let audience = self.audiences.get(&session.session_id.0, from_agent)?;
        let mut messages = Vec::new();
        for target in audience.visible_to {
            if !routes.can_deliver(session, from_agent, &target).await? {
                continue;
            }
            let msg = MessageDraft {
                kind: crate::message::MessageKind::Standard,
                from: AgentId::from(from_agent.to_string()),
                to: AgentId::from(target.clone()),
                content: content.to_vec(),
                meta: Default::default(),
            }
            .commit_auto(SessionId::from(session.session_id.0.clone()));

            // Write to recipient's history so they receive the message.
            // Sender outbox is the caller's responsibility (via AppendHistory effect).
            self.history
                .append(&session.session_id.0, &target, vec![msg.clone()])?;

            messages.push(msg);
        }
        Ok(messages)
    }
}

pub fn extract_tool_calls(content: &[ContentBlock], requested_by: &str) -> Vec<ToolCall> {
    content
        .iter()
        .enumerate()
        .filter_map(|(idx, block)| match block {
            ContentBlock::ToolCall {
                tool_name,
                arguments,
                call_id,
            } => {
                let resolved_call_id = match call_id {
                    Some(id) if !id.trim().is_empty() => id.clone(),
                    _ => format!("auto-{}-{}", requested_by, idx),
                };
                Some(ToolCall {
                    call_id: resolved_call_id,
                    tool_id: tool_name.clone(),
                    arguments: arguments.clone(),
                    requested_by: requested_by.to_string(),
                    meta: serde_json::Value::Null,
                })
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{InMemoryArchiveStore, InMemoryAudienceStateStore, InMemoryHistoryStore};

    struct DenyRoutes;

    #[async_trait::async_trait]
    impl RouteResolver for DenyRoutes {
        async fn can_deliver(
            &self,
            _session: &SessionState,
            _from: &str,
            _to: &str,
        ) -> Result<bool, FrameworkError> {
            Ok(false)
        }
    }

    #[tokio::test]
    async fn dispatch_content_respects_route_resolver() {
        let mut runtime = InMemoryRuntime::new(
            InMemoryHistoryStore::default(),
            InMemoryArchiveStore::default(),
            InMemoryAudienceStateStore::default(),
        );

        runtime
            .audience_store_mut()
            .set(
                "demo",
                "agent:main",
                AudienceState {
                    visible_to: vec!["agent:user".to_string()],
                },
            )
            .expect("set audience");

        let messages = runtime
            .dispatch_content(
                &SessionState {
                    session_id: SessionId::from("demo"),
                    metadata: serde_json::Value::Null,
                },
                &DenyRoutes,
                "agent:main",
                &[ContentBlock::Text {
                    text: "secret".to_string(),
                }],
            )
            .await
            .expect("dispatch");

        assert!(messages.is_empty());
    }
}
