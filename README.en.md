# agentfw-rs

[中文](README.md) | **English**

`agentfw-rs` is an **Agent Runtime Core** written in Rust — a low-level execution kernel for building multi-agent systems.

## What You Can Build With It

- **Multi-LLM collaboration systems** — wire multiple LLM agents together to pass messages and complete tasks cooperatively
- **Tool-calling pipelines** — build agents that automatically execute tools and feed results back to the model in a loop
- **Hybrid agent systems** — model LLMs, external APIs, and human input uniformly as agents and compose them freely
- **Streaming conversation apps** — use the built-in streaming driver to build real-time responsive dialogue systems
- **Configurable agent labs** — rapidly prototype and debug multi-agent topologies via TOML/JSON static config
- **Custom execution strategies** — implement your own Driver or ModelAdapter to plug in any model or execution logic

## Design Philosophy

| Principle | Description |
|-----------|-------------|
| Every participant is an `Agent` | LLMs, external systems, and human input are all modeled uniformly |
| Every communication is a `Message` | Messages are the sole information carrier |
| Every capability is a `Tool` | Tools are the only way agents interact with the outside world |
| Runtime policy is provided by `Resolver` | Model selection, prompts, routing, history — all replaceable |
| The framework handles atomic execution | One turn + apply effects + dispatch content; scheduling and orchestration belong to the application layer |

## Core Abstractions

```
AgentSpec          — Agent definition (id, driver, prompt_ref, model_ref)
AgentDriver        — Execution strategy (LlmDriver / ToolLoopLlmDriver / StreamingLlmDriver / ExternalDriver)
Message            — Message (with ContentBlock: Text / ToolCall / ToolResult / Image)
ToolDefinition     — Tool definition (schema + executor)
ResolverBundle     — Runtime resolver set (model / prompt / tools / routes / memory)
Runtime / Kernel   — Execution engine (apply_effects + dispatch_content)
HistoryStore       — History storage
AudienceState      — Message visibility state (controls routing targets)
```

## Model Support

| Provider ID | Description |
|---|---|
| `anthropic` / `anthropic-messages` | Anthropic Messages API |
| `openai` / `openai-chat-completions` | OpenAI Chat Completions |
| `openai-responses` | OpenAI Responses API |
| `openai-compatible` | Any OpenAI-compatible endpoint |

Streaming (`ModelAdapter::stream()`) is implemented for both Anthropic and OpenAI Responses adapters. `StreamingLlmDriver` handles text-only streaming; streaming with tool calls can be implemented via a custom Driver.

## Built-in Drivers (Reference Implementations)

The framework defines execution strategy via the `AgentDriver` trait — developers can replace it entirely. The following are default implementations shipped with the framework, usable as-is or as a reference for custom Drivers:

- **`LlmDriver`** — Single-step execution: one model request, surfacing outbound text and tool calls as-is
- **`ToolLoopLlmDriver`** — Executes a tool loop inside one turn (default cap of 20 rounds, configurable)
- **`StreamingLlmDriver`** — Prefers `stream()` when no tools are present; falls back to `send()` otherwise
- **`ExternalDriver`** — Passes through the last inbound message; useful for injecting external input

## Quick Start

```toml
# Cargo.toml
[dependencies]
agentfw-core = { path = "crates/agentfw-core" }
```

```rust
use agentfw_core::{Kernel, LlmDriver, ResolverBundle};

// 1. Build a Kernel and register a Driver
let mut kernel = Kernel::new();
kernel.register_driver("llm", Box::new(LlmDriver))?;

// 2. Build a ResolverBundle (model / prompt / tools / routes / memory)
let resolvers = ResolverBundle::builder()
    .model(my_model_resolver)
    .prompt(my_prompt_resolver)
    .tools(my_tool_resolver)
    .routes(my_route_resolver)
    .memory(my_memory_resolver)
    .build()?;

// 3. Run one agent turn
let messages = kernel.run_agent_turn(&session, &resolvers, &agent, &incoming).await?;
```

You can also drive the kernel from a static config file (TOML / JSON):

```rust
let config = StaticConfig::from_path("agent-lab.toml")?;
config.validate()?;
let (mut kernel, resolvers) = Kernel::from_static_config(&config, &builtin_tools)?;
```

## Directory Structure

```
agentfw-rs/
├── crates/
│   └── agentfw-core/       # Core library
│       └── src/
│           ├── kernel.rs           # Execution kernel
│           ├── runtime.rs          # Runtime (apply_effects / dispatch_content)
│           ├── resolver.rs         # Resolver traits and ResolverBundle
│           ├── default_drivers.rs  # LlmDriver / StreamingLlmDriver / ExternalDriver
│           ├── model.rs            # ModelAdapter trait
│           ├── anthropic_messages.rs
│           ├── openai_compatible.rs
│           ├── openai_responses.rs
│           ├── message.rs          # Message / ContentBlock
│           ├── state.rs            # AudienceState / RuntimeEffect
│           ├── storage.rs          # HistoryStore / ArchiveStore
│           ├── tool.rs             # ToolDefinition / ToolExecutor
│           ├── config.rs           # StaticConfig / DeveloperConfig
│           └── ...
└── examples/
    ├── minimal/                    # Minimal example: custom ModelAdapter
    ├── multi_agent_static/         # Multi-agent static config
    ├── history_transform/          # History transform hook
    └── three_agent_visibility/     # Three-agent visibility control
```

## Examples

```bash
# Minimal
cargo run -p minimal

# Multi-agent
cargo run -p multi_agent_static

# History transform
cargo run -p history_transform

# Three-agent visibility
cargo run -p three_agent_visibility
```

## Verification

```bash
cargo check -q
cargo test -q -p agentfw-core
```

## Extension Guide

### Custom Driver

```rust
use agentfw_core::{AgentDriver, AgentTurnResult, RunEnv, AgentSpec, Message, FrameworkError};

pub struct MyDriver;

#[async_trait::async_trait]
impl AgentDriver for MyDriver {
    async fn run_turn(
        &self,
        env: RunEnv<'_>,
        agent: &AgentSpec,
        incoming: &[Message],
    ) -> Result<AgentTurnResult, FrameworkError> {
        let (model, request, _tools) = env.resolvers.build_request(env.session, agent).await?;
        let response = model.send(request).await?;
        // ... custom logic
        Ok(AgentTurnResult { outbound_content: vec![], effects: vec![], meta: Default::default() })
    }
}
```

### Custom ModelAdapter

```rust
use agentfw_core::{ModelAdapter, ModelCapabilities, ModelRequest, ModelResponse, FrameworkError};

pub struct MyAdapter;

#[async_trait::async_trait]
impl ModelAdapter for MyAdapter {
    fn name(&self) -> &str { "my-model" }
    fn capabilities(&self) -> ModelCapabilities {
        ModelCapabilities { supports_tools: true, supports_streaming: false, supports_images: false }
    }
    async fn send(&self, request: ModelRequest) -> Result<ModelResponse, FrameworkError> {
        // Call your model API
        todo!()
    }
}
```

## Documentation

- [API Overview](docs/api-overview.md)
- [Public API](docs/public-api.md)
- [Driver Boundary](docs/driver-boundary.md)
- [Config Boundary](docs/config-boundary.md)
- [Runtime Boundary](docs/runtime-boundary.md)
- [State & Storage Boundary](docs/state-storage-boundary.md)

## License

MIT
