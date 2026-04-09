use crate::agent::AgentSpec;
use crate::error::FrameworkError;
use crate::message::{AgentId, ContentBlock, Message, MessageDraft, SessionId, ToolResultStatus};
use crate::model::{ModelRequest, SharedModelAdapter};
use crate::protocol::{AgentStep, DefaultProtocolNormalizer, ProtocolNormalizer};
use crate::runtime::{AgentDriver, AgentTurnResult, RunEnv};
use crate::state::RuntimeEffect;
use crate::tool::{ToolCall, ToolDefinition, ToolResultStatus as ToolStatus};
use std::collections::HashMap;
use futures::StreamExt;

/// Default external-input driver implementation.
///
/// This is a convenience default, not a framework-level behavior rule.
pub struct ExternalDriver;
pub type DefaultExternalDriver = ExternalDriver;
fn tool_result_content(result: &crate::tool::ToolResult) -> serde_json::Value {
    if !result.structured.is_null() {
        result.structured.clone()
    } else if !result.raw_text.is_empty() {
        serde_json::json!({ "text": result.raw_text })
    } else if !result.summary.is_empty() {
        serde_json::json!({ "summary": result.summary })
    } else {
        serde_json::json!({ "empty": true })
    }
}

#[async_trait::async_trait]
impl AgentDriver for ExternalDriver {
    async fn run_turn(
        &self,
        _env: RunEnv<'_>,
        _agent: &AgentSpec,
        incoming: &[Message],
    ) -> Result<AgentTurnResult, FrameworkError> {
        let mut outbound = Vec::new();
        if let Some(last) = incoming.last() {
            outbound.extend(last.content.clone());
        }
        Ok(AgentTurnResult {
            outbound_content: outbound,
            effects: Vec::new(),
            meta: serde_json::Value::Null,
        })
    }
}

/// Default LLM driver implementation.
///
/// Host applications can replace this driver; framework semantics are defined
/// by traits and resolvers, not by this default implementation.
pub struct LlmDriver;
pub type DefaultLlmDriver = LlmDriver;

/// Default streaming-first LLM driver implementation.
///
/// This driver prefers `ModelAdapter::stream()` when the adapter supports
/// streaming and the request does not include tools. Otherwise it falls back to
/// the default `LlmDriver` behavior.
pub struct StreamingLlmDriver;
pub type DefaultStreamingLlmDriver = StreamingLlmDriver;

struct LlmTurnRunner<'a> {
    session_id: &'a str,
    agent_id: &'a str,
    model: SharedModelAdapter,
    request: ModelRequest,
    tools: HashMap<String, ToolDefinition>,
    effects: Vec<RuntimeEffect>,
    tool_round: u32,
}

const MAX_TOOL_ROUNDS: u32 = 20;

enum StepDisposition {
    Finished(AgentTurnResult),
    ContinueWithTools(Vec<ToolCall>),
}

impl<'a> LlmTurnRunner<'a> {
    async fn create(env: RunEnv<'a>, agent: &'a AgentSpec) -> Result<Self, FrameworkError> {
        let (model, request, tools) = env.resolvers.build_request(env.session, agent).await?;

        Ok(Self {
            session_id: &env.session.session_id.0,
            agent_id: &agent.id,
            model,
            request,
            tools,
            effects: Vec::new(),
            tool_round: 0,
        })
    }

    async fn run(mut self) -> Result<AgentTurnResult, FrameworkError> {
        loop {
            if self.tool_round >= MAX_TOOL_ROUNDS {
                return Err(FrameworkError::Runtime(format!(
                    "agent {} exceeded maximum tool rounds ({})",
                    self.agent_id, MAX_TOOL_ROUNDS
                )));
            }
            let step = self.request_step().await?;
            match self.classify_step(step)? {
                StepDisposition::Finished(result) => return Ok(result),
                StepDisposition::ContinueWithTools(tool_calls) => {
                    self.tool_round += 1;
                    let tool_result_blocks = self.execute_tool_calls(tool_calls).await?;
                    self.request.messages.push(tool_result_message(
                        self.session_id,
                        self.agent_id,
                        tool_result_blocks,
                    ));
                }
            }
        }
    }

    async fn request_step(&self) -> Result<AgentStep, FrameworkError> {
        let response = self.model.send(self.request.clone()).await?;
        DefaultProtocolNormalizer.normalize(response, self.agent_id)
    }

    fn classify_step(&self, step: AgentStep) -> Result<StepDisposition, FrameworkError> {
        if step.tool_calls.is_empty() {
            return Ok(StepDisposition::Finished(finalize_turn(
                step,
                &self.effects,
            )?));
        }

        Ok(StepDisposition::ContinueWithTools(step.tool_calls))
    }

    async fn execute_tool_calls(
        &mut self,
        tool_calls: Vec<ToolCall>,
    ) -> Result<Vec<ContentBlock>, FrameworkError> {
        let mut tool_result_blocks = Vec::new();
        // Snapshot effects before this round so we can roll back on failure.
        let effects_snapshot = self.effects.clone();

        for tool_call in tool_calls {
            let tool = self.resolve_tool(&tool_call)?.clone();
            match tool.executor.execute(repair_tool_call(tool_call)).await {
                Ok(result) => {
                    self.effects.extend(result.effects.clone());
                    tool_result_blocks.push(tool_result_block(&tool, &result));
                }
                Err(err) => {
                    // Roll back any effects accumulated in this round.
                    self.effects = effects_snapshot;
                    return Err(err);
                }
            }
        }

        Ok(tool_result_blocks)
    }

    fn resolve_tool(&self, tool_call: &ToolCall) -> Result<&ToolDefinition, FrameworkError> {
        if tool_call.tool_id.trim().is_empty() {
            return Err(FrameworkError::Protocol(format!(
                "agent {} emitted tool call with empty tool name",
                self.agent_id
            )));
        }

        self.tools.get(&tool_call.tool_id).ok_or_else(|| {
            FrameworkError::Protocol(format!(
                "tool not resolved for agent {}: {}",
                self.agent_id, tool_call.tool_id
            ))
        })
    }
}

struct StreamingLlmTurnRunner {
    model: SharedModelAdapter,
    request: ModelRequest,
}

impl StreamingLlmTurnRunner {
    async fn create(env: RunEnv<'_>, agent: &AgentSpec) -> Result<Self, FrameworkError> {
        let (model, request, _tools) = env.resolvers.build_request(env.session, agent).await?;
        Ok(Self { model, request })
    }

    async fn run(self) -> Result<AgentTurnResult, FrameworkError> {
        let model = self.model;
        let request = self.request;

        if !model.capabilities().supports_streaming || !request.tools.is_empty() {
            let response = model.send(request).await?;
            let step = DefaultProtocolNormalizer.normalize(response, "stream-fallback")?;
            return finalize_turn(step, &[]);
        }

        let Some(mut stream) = model.stream(request.clone()) else {
            let response = model.send(request).await?;
            let step = DefaultProtocolNormalizer.normalize(response, "stream-fallback")?;
            return finalize_turn(step, &[]);
        };

        let mut outbound_content = Vec::new();
        let mut stop_reason: Option<String> = None;
        let mut last_raw = serde_json::Value::Null;
        let mut chunk_count = 0u64;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(FrameworkError::from)?;
            chunk_count = chunk_count.saturating_add(1);
            if let Some(reason) = chunk.stop_reason.clone() {
                stop_reason = Some(reason);
            }
            last_raw = chunk.raw.clone();

            if chunk
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
            {
                return Err(FrameworkError::Protocol(
                    "streaming driver does not support tool calls in streamed chunks".to_string(),
                ));
            }

            outbound_content.extend(chunk.content);
        }

        if outbound_content.is_empty() {
            return Err(FrameworkError::Protocol(
                "stream returned no outbound content".to_string(),
            ));
        }

        Ok(AgentTurnResult {
            outbound_content,
            effects: Vec::new(),
            meta: serde_json::json!({
                "mode": "stream",
                "stop_reason": stop_reason,
                "chunks": chunk_count,
                "last_raw": last_raw,
            }),
        })
    }
}

fn finalize_turn(
    step: AgentStep,
    effects: &[RuntimeEffect],
) -> Result<AgentTurnResult, FrameworkError> {
    let outbound_content = step.outbound_content;

    if outbound_content.is_empty() {
        return Err(FrameworkError::Protocol(
            "model returned neither tool calls nor outbound content".to_string(),
        ));
    }

    Ok(AgentTurnResult {
        outbound_content,
        effects: effects.to_vec(),
        meta: step.meta,
    })
}

fn repair_tool_call(tool_call: ToolCall) -> ToolCall {
    if tool_call.arguments.is_null() {
        ToolCall {
            arguments: serde_json::json!({}),
            ..tool_call
        }
    } else {
        tool_call
    }
}

fn tool_result_block(tool: &ToolDefinition, result: &crate::tool::ToolResult) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_name: tool.id.clone(),
        content: tool_result_content(result),
        call_id: result
            .meta
            .get("call_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string(),
        status: match result.status {
            ToolStatus::Success => ToolResultStatus::Success,
            ToolStatus::Error => ToolResultStatus::Error,
            ToolStatus::Partial => ToolResultStatus::Partial,
            ToolStatus::Cancelled => ToolResultStatus::Cancelled,
        },
    }
}

fn tool_result_message(session_id: &str, agent_id: &str, content: Vec<ContentBlock>) -> Message {
    MessageDraft {
        kind: crate::message::MessageKind::Tool,
        from: AgentId::from(agent_id.to_string()),
        to: AgentId::from(agent_id.to_string()),
        content,
        meta: Default::default(),
    }
    .commit_auto(SessionId::from(session_id.to_string()))
}

#[async_trait::async_trait]
impl AgentDriver for LlmDriver {
    async fn run_turn(
        &self,
        env: RunEnv<'_>,
        agent: &AgentSpec,
        _incoming: &[Message],
    ) -> Result<AgentTurnResult, FrameworkError> {
        LlmTurnRunner::create(env, agent).await?.run().await
    }
}

#[async_trait::async_trait]
impl AgentDriver for StreamingLlmDriver {
    async fn run_turn(
        &self,
        env: RunEnv<'_>,
        agent: &AgentSpec,
        _incoming: &[Message],
    ) -> Result<AgentTurnResult, FrameworkError> {
        StreamingLlmTurnRunner::create(env, agent).await?.run().await
    }
}
