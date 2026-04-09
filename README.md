# agentfw-rs

`agentfw-rs` 是一个实验性的 **Agent Runtime Core**，用 Rust 实现的多智能体系统底层内核。

它只提供原子能力，不内置调度与编排。框架不预设"主代理 / 子代理 / 工作流"语义——谁先运行、谁后运行、串行还是并行，由上层应用自行决定。

## 设计理念

| 原则 | 说明 |
|------|------|
| 一切参与者统一建模为 `Agent` | 无论是 LLM、外部系统还是人类输入，都是 Agent |
| 一切通信统一建模为 `Message` | 消息是唯一的信息载体 |
| 一切能力统一建模为 `Tool` | 工具是 Agent 与外部世界交互的唯一方式 |
| 运行时策略统一由 `Resolver` 提供 | 模型选择、提示词、路由、历史——全部可替换 |
| 框架只负责原子执行 | 执行一轮 + 应用效果 + 消息分发，不负责推进调度 |

## 核心抽象

```
AgentSpec          — 代理定义（id、driver、prompt_ref、model_ref）
AgentDriver        — 代理执行策略（LlmDriver / StreamingLlmDriver / ExternalDriver）
Message            — 消息（含 ContentBlock：Text / ToolCall / ToolResult / Image）
ToolDefinition     — 工具定义（schema + executor）
ResolverBundle     — 运行时解析器集合（model / prompt / tools / routes / memory）
Runtime / Kernel   — 执行引擎（apply_effects + dispatch_content）
HistoryStore       — 历史存储
AudienceState      — 消息可见性状态（控制消息路由目标）
```

## 模型支持

| Provider 标识 | 说明 |
|---|---|
| `anthropic` / `anthropic-messages` | Anthropic Messages API |
| `openai` / `openai-chat-completions` | OpenAI Chat Completions |
| `openai-responses` | OpenAI Responses API |
| `openai-compatible` | 任意 OpenAI 兼容接口 |

流式能力（`ModelAdapter::stream()`）已在 Anthropic 和 OpenAI Responses 适配器上实现。`StreamingLlmDriver` 支持纯文本流式场景；带工具调用的流式需开发者自行实现 Driver。

## 内置 Driver

- **`LlmDriver`** — 单轮内工具往返直到拿到正文，支持完整工具调用循环（最多 20 轮）
- **`StreamingLlmDriver`** — 优先消费 `stream()`，不含工具时走流式，含工具时自动 fallback
- **`ExternalDriver`** — 透传最后一条入站消息，用于外部输入注入

## 快速开始

```toml
# Cargo.toml
[dependencies]
agentfw-core = { path = "crates/agentfw-core" }
```

```rust
use agentfw_core::{Kernel, LlmDriver, ResolverBundle};

// 1. 构建 Kernel 并注册 Driver
let mut kernel = Kernel::new();
kernel.register_driver("llm", Box::new(LlmDriver))?;

// 2. 构建 ResolverBundle（model / prompt / tools / routes / memory）
let resolvers = ResolverBundle::builder()
    .model(my_model_resolver)
    .prompt(my_prompt_resolver)
    .tools(my_tool_resolver)
    .routes(my_route_resolver)
    .memory(my_memory_resolver)
    .build()?;

// 3. 执行一轮
let messages = kernel.run_agent_turn(&session, &resolvers, &agent, &incoming).await?;
```

也可以通过静态配置文件（TOML / JSON）驱动：

```rust
let config = StaticConfig::from_path("agent-lab.toml")?;
config.validate()?;
let (mut kernel, resolvers) = Kernel::from_static_config(&config, &builtin_tools)?;
```

## 目录结构

```
agentfw-rs/
├── crates/
│   └── agentfw-core/       # 核心库
│       └── src/
│           ├── kernel.rs           # 执行内核
│           ├── runtime.rs          # 运行时（apply_effects / dispatch_content）
│           ├── resolver.rs         # Resolver trait 与 ResolverBundle
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
    ├── minimal/                    # 最小示例：自定义 ModelAdapter
    ├── multi_agent_static/         # 多代理静态配置
    ├── history_transform/          # 历史变换钩子
    └── three_agent_visibility/     # 三代理可见性控制
```

## 示例

```bash
# 最小示例
cargo run -p minimal

# 多代理
cargo run -p multi_agent_static

# 历史变换
cargo run -p history_transform

# 三代理可见性
cargo run -p three_agent_visibility
```

## 验证

```bash
cargo check -q
cargo test -q -p agentfw-core
```

## 扩展指南

### 自定义 Driver

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
        // ... 自定义处理逻辑
        Ok(AgentTurnResult { outbound_content: vec![], effects: vec![], meta: Default::default() })
    }
}
```

### 自定义 ModelAdapter

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
        // 调用你的模型 API
        todo!()
    }
}
```

## 文档

- [API Overview](docs/api-overview.md)
- [Public API](docs/public-api.md)
- [Driver Boundary](docs/driver-boundary.md)
- [Config Boundary](docs/config-boundary.md)
- [Runtime Boundary](docs/runtime-boundary.md)
- [State & Storage Boundary](docs/state-storage-boundary.md)

## License

MIT
