# API Overview (Developer View)

本文档只描述 `agentfw-rs` 当前已实现接口，不引入新设计。

## Scope

当前说明范围：

- `AgentSpec`
- `DriverRegistry / AgentDriver`
- `Message / MessageDraft`
- `ToolDefinition / ToolCatalog`
- `ResolverBundle`
- `Runtime / Kernel`
- `AudienceState / RuntimeEffect`
- `HistoryStore / ArchiveStore`
- `HistoryTransform`

## Design Boundary

- 框架提供原子能力，不提供调度和编排。
- 框架不定义“主代理/子代理”角色语义。
- 框架不内置 workflow 推进规则。

开发者系统负责：

- 哪个 Agent 何时运行
- 消息如何排队/并发
- 串行或并行策略
- 是否以及如何做历史压缩/摘要/裁剪（框架只提供 HistoryTransform 钩子和存储能力）

补充：

- 默认 `LlmDriver` 是单轮原子执行实现：只做一次模型请求，不在内核里自动执行工具循环。
- 如果开发者需要“模型返回工具调用 -> 执行工具 -> 回填工具结果 -> 再请求模型”的便捷路径，可显式选择 `ToolLoopLlmDriver`。

---

## AgentSpec

路径：[agent.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/agent.rs)

```rust
pub struct AgentSpec {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub prompt_ref: String,
    pub model_ref: String,
    pub metadata: Value,
}
```

说明：

- `driver` 用于在 `DriverRegistry` 中解析驱动实现。
- `prompt_ref` / `model_ref` 由 resolver 解释。
- 不包含角色语义字段。

---

## DriverRegistry / AgentDriver

路径：

- [driver.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/driver.rs)
- [runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

核心接口：

```rust
pub trait DriverRegistry {
    fn register(&mut self, key: String, driver: Box<dyn AgentDriver>) -> Result<(), FrameworkError>;
    fn get(&self, key: &str) -> Option<&dyn AgentDriver>;
}
```

```rust
pub trait AgentDriver {
    async fn run_turn(
        &self,
        env: RunEnv<'_>,
        agent: &AgentSpec,
        incoming: &[Message],
    ) -> Result<AgentTurnResult, FrameworkError>;
}
```

说明：

- `DriverRegistry` 仅负责按 key 解析驱动。
- 不包含调度、编排、角色约束语义。

---

## Message

路径：[message.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/message.rs)

关键类型：

- `AgentId`
- `MessageId`
- `SessionId`
- `Timestamp`
- `Message`
- `MessageDraft`
- `ContentBlock`

`Message` 统一承载：

- `from`
- `to`
- `content`
- `meta`

框架不区分业务层“用户消息/代理消息”的角色语义，均为统一消息对象。

补充说明：
- `MessageDraft` 是推荐的消息构造入口。
- `dispatch_content()` 是当前框架中把 Agent 一轮正文按 AudienceState（可见范围）与 RouteResolver（路由解析器）落成真正 Message 的官方分发主路径。
- 如果开发者直接手动 `commit()` 任意消息，那么就可能绕开 AudienceState + RouteResolver；这种旁路能力目前仍然存在，属于宿主系统需要自律控制的部分。

---

## Model Capability

路径：

- [model.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/model.rs)
- [openai_compatible.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/openai_compatible.rs)
- [openai_responses.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/openai_responses.rs)
- [anthropic_messages.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/anthropic_messages.rs)

当前模型层已经包含：

- `ModelAdapter`
- `ModelRequest`
- `ModelResponse`
- `ModelStreamChunk`
- `ModelStream`
- `ModelCapabilities`
- `ModelAdapterError`

补充说明：

- `ModelAdapter::send(...)` 仍然是当前主路径。
- `ModelAdapter::stream(...)` 现已存在，但属于可选能力；默认实现可以返回 `None`。
- 当前已拆分出三个默认适配器实现：
  - OpenAI Chat Completions
  - OpenAI Responses API
  - Anthropic Messages
- `OpenAICompatibleAdapter` 当前作为兼容别名，等价于 OpenAI Chat Completions 适配器。
- `supports_streaming` 现在和 `stream()` 接口语义对齐，而不再只是预留字段。
- 模型相关错误已从粗粒度 `Api(String)` 细化为：
  - `ModelAdapterError::Request`
  - `ModelAdapterError::Streaming`
  并统一包进 `FrameworkError::Model(...)`。
- 当前默认流式实现只统一：
  - 文本 `ContentBlock`
  - 停止信号
  - 原始事件载荷
  不在这一层强行统一更复杂的 provider 专属流事件语义。
- 注意：当前默认 `LlmDriver` 仍然通过 `send` 路径运行 Agent，一轮内部不会自动切换到 `stream()`；也就是说，流式能力当前已经在模型层落地，但还没有在默认 Driver 层被消费。

---

## ToolDefinition / ToolCatalog

路径：[tool.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/tool.rs)

关键类型：

- `ToolSchema`
- `ToolCall`
- `ToolResult`
- `ToolDefinition`
- `ToolCatalog`
- `InMemoryToolCatalog`

核心接口：

```rust
pub trait ToolCatalog {
    fn register(&mut self, definition: ToolDefinition) -> Result<(), FrameworkError>;
    fn get(&self, tool_id: &str) -> Option<&ToolDefinition>;
    fn list(&self) -> Vec<&ToolDefinition>;
}
```

说明：

- `ToolResult` 可携带 `effects: Vec<RuntimeEffect>`。
- 工具执行产生效果，具体如何应用由 runtime 负责。

---

## ResolverBundle

路径：[resolver.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/resolver.rs)

当前包含：

- `ModelResolver`
- `PromptResolver`
- `ToolResolver`
- `RouteResolver`
- `MemoryResolver`
- `HistoryTransform`

组合对象：

```rust
pub struct ResolverBundle {
    pub model: Box<dyn ModelResolver>,
    pub prompt: Box<dyn PromptResolver>,
    pub tools: Box<dyn ToolResolver>,
    pub routes: Box<dyn RouteResolver>,
    pub memory: Box<dyn MemoryResolver>,
    pub history_transform: Box<dyn HistoryTransform>,
}
```

说明：

- 路由判定以 `RouteResolver` 为运行时准。
- 静态 route 配置是开发者输入，解析器是运行时判定入口。
- `HistoryTransform` 是开发者可选的历史变换钩子；框架不把它等同于内置压缩模块，也不内置固定压缩策略。

---

## Runtime / Kernel

路径：

- [runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)
- [kernel.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/kernel.rs)

`Runtime` 当前接口：

```rust
pub trait Runtime {
    fn apply_effects(&mut self, session_id: &str, agent_id: &str, effects: &[RuntimeEffect]) -> Result<(), FrameworkError>;
    async fn dispatch_content(
        &mut self,
        session: &SessionState,
        routes: &dyn RouteResolver,
        from_agent: &str,
        content: &[ContentBlock],
    ) -> Result<Vec<Message>, FrameworkError>;
}
```

说明：

- `apply_effects` 是运行时效果应用 helper。
- `dispatch_content` 是把当前 Agent 一轮 `outbound_content` 按 AudienceState + RouteResolver 落成真正消息的官方分发 helper。
- 它只负责生成可分发消息，不再隐式写入接收方历史；接收方是否、何时消费这些消息，完全由宿主系统决定。
- 这两个接口不负责调度推进，不触发下一轮执行。
- `dispatch_content` 已经是 `async`，因为运行时可能需要异步调用 `RouteResolver`。

`Kernel` 提供当前高层封装：

- `execute_agent_turn`
- `apply_turn_effects`
- `dispatch_turn_content`
- `run_agent_turn`

`run_agent_turn` 是 convenience 路径（执行一轮 + 应用效果 + 分发），不是工作流编排器。

补充：

- `run_agent_turn` 只返回本轮落成的消息集合，不会自动运行任何下游 Agent。

---

## AudienceState / RuntimeEffect

路径：[state.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/state.rs)

`AudienceState`：

```rust
pub struct AudienceState {
    pub visible_to: Vec<String>,
}
```

`RuntimeEffect` 当前包含：

- `SetAudience`
- `AppendHistory`
- `ArchivePayload`
- `Custom`

说明：

- 工具或驱动可产出 `RuntimeEffect`。
- runtime 将 effect 应用到状态/存储。
- 当前主路径推荐通过 `RuntimeEffect` 修改运行时状态；但框架并未从类型层彻底封死所有旁路写状态的可能。

---

## HistoryStore / ArchiveStore

路径：[storage.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/storage.rs)

接口：

```rust
pub trait HistoryStore {
    fn load(&self, session_id: &str, agent_id: &str) -> Result<Vec<Message>, FrameworkError>;
    fn append(&mut self, session_id: &str, agent_id: &str, msgs: Vec<Message>) -> Result<(), FrameworkError>;
    fn replace(&mut self, session_id: &str, agent_id: &str, msgs: Vec<Message>) -> Result<(), FrameworkError>;
}
```

```rust
pub trait ArchiveStore {
    fn save(&mut self, reference: &str, payload: &str) -> Result<(), FrameworkError>;
    fn load(&self, reference: &str) -> Result<Option<String>, FrameworkError>;
}
```

补充：

- `AudienceStateStore` 用于读写可见范围状态，支持 `on_missing` 策略配置（`Error` / `UseState`）。
- `HistoryStore` 当前按 `session_id + agent_id` 共同索引历史，避免跨会话串历史。

---

## Defaults vs Rules

默认实现（如 `LlmDriver`、`ExternalDriver`、`InMemoryRuntime`、`Static*Resolver`）是便捷实现，不是框架规则。

框架规则是接口边界与数据模型本身；调度与编排始终由开发者系统负责。
