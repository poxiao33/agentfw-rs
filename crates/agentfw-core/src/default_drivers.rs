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
            for block in &last.content {
                match block {
                    // Only forward safe, user-visible content types.
                    // System, ToolCall, ToolResult, and Reference blocks are stripped to
                    // prevent prompt injection. Reference blocks are stripped because their
                    // resolved content is opaque at this layer and could carry injected
                    // instructions.
                    ContentBlock::Text { .. } | ContentBlock::Image { .. } => {
                        outbound.push(block.clone());
                    }
                    ContentBlock::ToolResult { .. }
                    | ContentBlock::ToolCall { .. }
                    | ContentBlock::System { .. }
                    | ContentBlock::Reference { .. } => {}
                }
            }
        }
        Ok(AgentTurnResult {
            outbound_content: outbound,
            effects: Vec::new(),
            meta: serde_json::Value::Null,
        })
    }
}

/// Default single-step LLM driver implementation.
///
/// This driver performs exactly one model request for the turn and returns the
/// normalized result. It does not execute tool loops internally.
///
/// Host applications can replace this driver; framework semantics are defined
/// by traits and resolvers, not by this default implementation.
pub struct LlmDriver;
pub type DefaultLlmDriver = LlmDriver;

/// Optional convenience LLM driver that executes tool loops inside a single
/// turn.
///
/// This preserves the previous batteries-included behavior for host
/// applications that want a self-contained tool-using agent loop, but it is
/// not the most minimal kernel behavior.
pub struct ToolLoopLlmDriver;
pub type DefaultToolLoopLlmDriver = ToolLoopLlmDriver;

#[derive(Debug, Clone, Copy)]
pub struct ToolLoopDriverConfig {
    pub max_tool_rounds: u32,
}

impl Default for ToolLoopDriverConfig {
    fn default() -> Self {
        Self {
            max_tool_rounds: 20,
        }
    }
}

pub struct ConfigurableToolLoopLlmDriver {
    config: ToolLoopDriverConfig,
}

impl ConfigurableToolLoopLlmDriver {
    pub fn new(config: ToolLoopDriverConfig) -> Self {
        Self { config }
    }
}

/// Default streaming-first LLM driver implementation.
///
/// This driver prefers `ModelAdapter::stream()` when the adapter supports
/// streaming and the request does not include tools. Otherwise it falls back to
/// the full `ToolLoopLlmDriver` behavior (including tool execution and
/// effects).
///
/// # Effects
///
/// The streaming path does not support tool calls and therefore produces no
/// `RuntimeEffect`s. If the model or adapter does not support streaming, or if
/// the request includes tools, this driver delegates to `ToolLoopLlmDriver`
/// which does accumulate effects.
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
    /// Accumulated text/non-tool content from mixed responses across all tool rounds.
    accumulated_content: Vec<ContentBlock>,
    /// Known agent IDs for this session, used to validate set_visible_to targets.
    /// Sourced from session.metadata["known_agents"] if present.
    known_agent_ids: Vec<String>,
    max_tool_rounds: u32,
}

enum StepDisposition {
    Finished(AgentTurnResult),
    ContinueWithTools(Vec<ToolCall>),
}

impl<'a> LlmTurnRunner<'a> {
    async fn create(env: RunEnv<'a>, agent: &'a AgentSpec) -> Result<Self, FrameworkError> {
        Self::create_with_config(env, agent, ToolLoopDriverConfig::default()).await
    }

    async fn create_with_config(
        env: RunEnv<'a>,
        agent: &'a AgentSpec,
        config: ToolLoopDriverConfig,
    ) -> Result<Self, FrameworkError> {
        let (model, request, tools) = env.resolvers.build_request(env.session, agent).await?;

        let known_agent_ids: Vec<String> = env
            .session
            .metadata
            .get("known_agents")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();

        Ok(Self {
            session_id: &env.session.session_id.0,
            agent_id: &agent.id,
            model,
            request,
            tools,
            effects: Vec::new(),
            tool_round: 0,
            accumulated_content: Vec::new(),
            known_agent_ids,
            max_tool_rounds: config.max_tool_rounds.max(1),
        })
    }

    async fn run(mut self) -> Result<AgentTurnResult, FrameworkError> {
        loop {
            if self.tool_round >= self.max_tool_rounds {
                return Err(FrameworkError::Runtime(format!(
                    "agent {} exceeded maximum tool rounds ({})",
                    self.agent_id, self.max_tool_rounds
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
        // Include tool_round in the requester ID so auto-generated call IDs are
        // unique across rounds (prevents collisions when idx resets to 0 each round).
        let requester = format!("{}-r{}", self.agent_id, self.tool_round);
        DefaultProtocolNormalizer.normalize(response, &requester)
    }

    fn classify_step(&mut self, step: AgentStep) -> Result<StepDisposition, FrameworkError> {
        if step.tool_calls.is_empty() {
            // Merge any content accumulated from prior mixed rounds with this final round.
            let mut final_content = std::mem::take(&mut self.accumulated_content);
            final_content.extend(step.outbound_content);
            let merged_step = AgentStep {
                outbound_content: final_content,
                tool_calls: Vec::new(),
                need_retry: step.need_retry,
                retry_hint: step.retry_hint,
                meta: step.meta,
            };
            return Ok(StepDisposition::Finished(finalize_turn(
                merged_step,
                &self.effects,
            )?));
        }

        // Mixed response: preserve any non-tool content before continuing with tool calls.
        self.accumulated_content.extend(step.outbound_content);
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
            let enriched = self.enrich_tool_call(repair_tool_call(tool_call.clone()));
            match tool.executor.execute(enriched).await {
                Ok(result) => {
                    self.effects.extend(result.effects.clone());
                    tool_result_blocks.push(tool_result_block(&tool_call, &tool, &result));
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

        // tool_map is keyed by tool.name (what the model sees), not tool.id.
        self.tools.get(&tool_call.tool_id).ok_or_else(|| {
            FrameworkError::Protocol(format!(
                "tool not resolved for agent {}: '{}' (check that tool name matches a registered tool)",
                self.agent_id, tool_call.tool_id
            ))
        })
    }

    /// Inject session context into the tool call's meta so executors can validate inputs.
    /// Currently injects `known_agents` for audience validation.
    fn enrich_tool_call(&self, mut call: ToolCall) -> ToolCall {
        if !self.known_agent_ids.is_empty() {
            if let Some(obj) = call.meta.as_object_mut() {
                obj.insert(
                    "known_agents".to_string(),
                    serde_json::Value::Array(
                        self.known_agent_ids
                            .iter()
                            .map(|id| serde_json::Value::String(id.clone()))
                            .collect(),
                    ),
                );
            } else {
                call.meta = serde_json::json!({ "known_agents": self.known_agent_ids });
            }
        }
        call
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

        // Fall back to the full LlmTurnRunner (with tool support and effects) when
        // streaming is not available or tools are present.
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

async fn build_single_step_request(
    env: RunEnv<'_>,
    agent: &AgentSpec,
    incoming: &[Message],
) -> Result<(SharedModelAdapter, ModelRequest), FrameworkError> {
    let (model, mut request, _tools) = env.resolvers.build_request(env.session, agent).await?;
    if !incoming.is_empty() {
        request.messages.extend_from_slice(incoming);
    }
    Ok((model, request))
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

fn tool_result_block(call: &ToolCall, tool: &ToolDefinition, result: &crate::tool::ToolResult) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_name: tool.name.clone(),
        content: tool_result_content(result),
        // Use the originating call_id as the authoritative source, not result.meta.
        call_id: call.call_id.clone(),
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
        incoming: &[Message],
    ) -> Result<AgentTurnResult, FrameworkError> {
        let (model, request) = build_single_step_request(env, agent, incoming).await?;
        let response = model.send(request).await?;
        let step = DefaultProtocolNormalizer.normalize(response, &agent.id)?;

        let mut outbound_content = step.outbound_content;
        for tc in step.tool_calls {
            outbound_content.push(ContentBlock::ToolCall {
                tool_name: tc.tool_id,
                arguments: tc.arguments,
                call_id: Some(tc.call_id),
            });
        }

        if outbound_content.is_empty() {
            return Err(FrameworkError::Protocol(
                "model returned neither outbound content nor tool calls".to_string(),
            ));
        }

        Ok(AgentTurnResult {
            outbound_content,
            effects: Vec::new(),
            meta: step.meta,
        })
    }
}

#[async_trait::async_trait]
impl AgentDriver for ToolLoopLlmDriver {
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
impl AgentDriver for ConfigurableToolLoopLlmDriver {
    async fn run_turn(
        &self,
        env: RunEnv<'_>,
        agent: &AgentSpec,
        _incoming: &[Message],
    ) -> Result<AgentTurnResult, FrameworkError> {
        LlmTurnRunner::create_with_config(env, agent, self.config)
            .await?
            .run()
            .await
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
