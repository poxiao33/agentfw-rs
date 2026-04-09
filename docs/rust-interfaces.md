# Rust 接口说明（agentfw-rs）

本文档基于 `agentfw-rs` 当前代码实现整理，只描述**已经存在**的接口和默认实现，不引入新设计。

## 文档边界

当前内核只提供原子能力：

- `Agent` 统一对象模型
- `Message` 统一通信模型
- `Tool` 统一能力模型
- `Resolver` 统一运行时解析入口
- `Kernel / Runtime` 统一执行一轮 Agent 的能力
- `History / Archive / AudienceState` 状态与存储接口
- `RuntimeEffect` 状态变更效果

当前**不提供**：

- 调度器
- mailbox（邮箱）消费语义
- 串行/并行编排策略
- 固定角色模板
- 固定工作流模板

开发者系统需要自己决定：

- 哪个 Agent 何时运行
- 消息如何排队
- 是串行还是并行
- 历史如何裁剪、压缩、归档和召回

---

## 1. AgentSpec

路径：[`crates/agentfw-core/src/agent.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/agent.rs)

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

### 说明

- `id`：Agent 唯一标识。
- `name`：可读名称。
- `driver`：驱动器 key，用于在 `DriverRegistry` 中查找实际 Driver 实现。
- `prompt_ref`：Prompt 资源键，由 `PromptResolver` 在运行时解析。
- `model_ref`：模型资源键，由 `ModelResolver` 在运行时解析。
- `metadata`：开发者自定义附加信息，内核不解释其语义。

### 关键边界

当前框架不内置主代理、子代理、用户代理等角色概念。`AgentSpec` 只描述一个可运行 Agent 的最小配置。

---

## 2. DriverRegistry / AgentDriver

路径：

- [`crates/agentfw-core/src/driver.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/driver.rs)
- [`crates/agentfw-core/src/runtime.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

### DriverRegistry

```rust
pub trait DriverRegistry: Send + Sync {
    fn register(&mut self, key: String, driver: Box<dyn AgentDriver>)
        -> Result<(), FrameworkError>;
    fn get(&self, key: &str) -> Option<&dyn AgentDriver>;
}
```

### AgentDriver

```rust
#[async_trait::async_trait]
pub trait AgentDriver: Send + Sync {
    async fn run_turn(
        &self,
        env: RunEnv<'_>,
        agent: &AgentSpec,
        incoming: &[Message],
    ) -> Result<AgentTurnResult, FrameworkError>;
}
```

### 说明

- `DriverRegistry` 只负责按 key 注册和解析 Driver。
- `AgentDriver` 定义“运行某个 Agent 一轮”的接口。
- 当前框架已经包含两个默认 Driver：
  - `LlmDriver`
  - `ExternalDriver`
- 这些默认 Driver 是便捷实现，不是框架规则。

### 关键边界

默认 Driver 是 convenience default（便捷默认实现），不是框架规则。开发者完全可以注册自己的 Driver。

---

## 3. Message / MessageDraft / ContentBlock

路径：[`crates/agentfw-core/src/message.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/message.rs)

### 标识类型

- `AgentId`
- `MessageId`
- `SessionId`
- `Timestamp`

这些类型都使用透明新类型包装，避免在接口层直接传裸字符串。

### Message

```rust
pub struct Message {
    pub id: MessageId,
    pub session_id: SessionId,
    pub kind: MessageKind,
    pub from: AgentId,
    pub to: AgentId,
    pub content: Vec<ContentBlock>,
    pub meta: MessageMeta,
    pub created_at: Option<Timestamp>,
}
```

### MessageDraft

```rust
pub struct MessageDraft {
    pub kind: MessageKind,
    pub from: AgentId,
    pub to: AgentId,
    pub content: Vec<ContentBlock>,
    pub meta: MessageMeta,
}
```

### ContentBlock

当前支持：

- `Text`
- `ToolCall`
- `ToolResult`
- `Image`
- `Reference`
- `System`

### 关键边界

框架统一用 `Message` 表示通信内容，但**不决定消息如何消费、如何调度**。这些属于开发者系统的职责。

补充：

- `MessageDraft` 是推荐的消息构造入口。
- 直接使用 `MessageDraft::commit` / `commit_auto` 可以手工构造消息，因此宿主系统仍然可以绕开 `dispatch_content(...)` 这条官方分发主路径；当前版本没有从类型层完全封死这种旁路。

---

## 4. ModelAdapter

路径：[`crates/agentfw-core/src/model.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/model.rs)

### ModelCapabilities

```rust
pub struct ModelCapabilities {
    pub supports_tools: bool,
    pub supports_streaming: bool,
    pub supports_images: bool,
}
```

### ModelRequest

```rust
pub struct ModelRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ModelToolDefinition>,
    pub metadata: Value,
}
```

### ModelResponse

```rust
pub struct ModelResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Option<ModelUsage>,
    pub raw: Value,
}
```

### ModelStreamChunk / ModelStream

当前模型层额外包含：

- `ModelStreamChunk`
- `ModelStream`

用于表达可选的流式输出能力。

### ModelAdapter

```rust
#[async_trait::async_trait]
pub trait ModelAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ModelCapabilities;
    async fn send(&self, request: ModelRequest) -> Result<ModelResponse, FrameworkError>;
    fn stream(&self, request: ModelRequest) -> Option<ModelStream>;
}
```

### 说明

- 模型接口已经被统一抽象。
- `send` 仍然是当前主路径。
- `stream()` 是可选能力；适配器可以选择返回 `None`。
- `SharedModelAdapter = Arc<dyn ModelAdapter>`，方便在 Resolver 和 Driver 中共享。
- 当前已拆分出三个默认适配器实现：
  - OpenAI Chat Completions
  - OpenAI Responses API
  - Anthropic Messages
- `OpenAICompatibleAdapter` 当前是 OpenAI Chat Completions 的兼容别名。
- 当前默认流式实现只统一文本块、停止信号和原始事件载荷，不在这一层强行统一 provider 专属流事件模型。
- 当前默认 `LlmDriver` 仍然走 `send` 主路径，因此“模型层支持 stream”和“默认 Agent 驱动实际消费 stream”是两回事；目前只完成了前者。

---

## 5. ToolDefinition / ToolCatalog / ToolExecutor

路径：[`crates/agentfw-core/src/tool.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/tool.rs)

### ToolSchema

```rust
pub struct ToolSchema {
    pub input_schema: Value,
    pub output_schema: Value,
}
```

### ToolCall

```rust
pub struct ToolCall {
    pub call_id: String,
    pub tool_id: String,
    pub arguments: Value,
    pub requested_by: String,
    pub meta: Value,
}
```

### ToolResult

```rust
pub struct ToolResult {
    pub success: bool,
    pub status: ToolResultStatus,
    pub summary: String,
    pub structured: Value,
    pub raw_text: String,
    pub effects: Vec<RuntimeEffect>,
    pub meta: Value,
}
```

### ToolExecutor

```rust
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, call: ToolCall) -> Result<ToolResult, FrameworkError>;
}
```

### ToolDefinition

```rust
pub struct ToolDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub schema: ToolSchema,
    pub executor: Arc<dyn ToolExecutor>,
    pub metadata: Value,
}
```

### ToolCatalog

```rust
pub trait ToolCatalog: Send + Sync {
    fn register(&mut self, definition: ToolDefinition) -> Result<(), FrameworkError>;
    fn get(&self, tool_id: &str) -> Option<&ToolDefinition>;
    fn list(&self) -> Vec<&ToolDefinition>;
}
```

### 说明

- `ToolCatalog` 只维护工具定义。
- 当前 Agent 能看到哪些工具，不由 `ToolCatalog` 决定，而由 `ToolResolver` 决定。
- `ToolResult.effects` 是工具对运行时状态的影响通道，例如 `SetAudience`。
- 当前主路径推荐通过 `RuntimeEffect` 修改运行时状态，但框架还没有从 API 层彻底禁止宿主直接改底层 store。

---

## 6. ResolverBundle

路径：[`crates/agentfw-core/src/resolver.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/resolver.rs)

### PromptPayload

```rust
pub struct PromptPayload {
    pub system: String,
    pub metadata: serde_json::Value,
}
```

### 各类 Resolver

当前包含：

- `ModelResolver`
- `PromptResolver`
- `ToolResolver`
- `RouteResolver`
- `MemoryResolver`
- `HistoryTransform`

### ResolverBundle

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

### `build_request(...)`

`ResolverBundle::build_request(...)` 当前会统一完成：

1. 解析模型
2. 解析 prompt
3. 解析历史
4. 执行历史变换
5. 解析工具集合
6. 组装 `ModelRequest`
7. 产出工具映射 `HashMap<String, ToolDefinition>`

### 关键边界

Resolver 是运行时入口，但框架不规定这些解析器如何实现。开发者可替换任意一项解析逻辑。

---

## 7. AudienceState / RuntimeEffect

路径：[`crates/agentfw-core/src/state.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/state.rs)

### AudienceState

```rust
pub struct AudienceState {
    pub visible_to: Vec<String>,
}
```

含义：

- 当前 Agent 后续正文允许被哪些 Agent ID 看到。
- 系统在 `dispatch_content(...)` 中读取该状态并进行分发。

### RuntimeEffect

```rust
pub enum RuntimeEffect {
    SetAudience { visible_to: Vec<String> },
    AppendHistory { messages: Vec<Message> },
    ArchivePayload { reference: String, payload: String },
    Custom { name: String, payload: Value },
}
```

说明：

- `SetAudience`：更新当前 Agent 的 AudienceState。
- `AppendHistory`：把消息追加到指定 Agent 历史。
- `ArchivePayload`：写归档。
- `Custom`：保留给开发者扩展。

### 关键边界

框架只提供 `RuntimeEffect` 机制，不解释业务层的工作流语义。

---

## 8. HistoryStore / ArchiveStore / AudienceStateStore

路径：[`crates/agentfw-core/src/storage.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/storage.rs)

### HistoryStore

```rust
pub trait HistoryStore: Send + Sync {
    fn load(&self, agent_id: &str) -> Result<Vec<Message>, FrameworkError>;
    fn append(&mut self, agent_id: &str, msgs: Vec<Message>) -> Result<(), FrameworkError>;
    fn replace(&mut self, agent_id: &str, msgs: Vec<Message>) -> Result<(), FrameworkError>;
}
```

### ArchiveStore

```rust
pub trait ArchiveStore: Send + Sync {
    fn save(&mut self, reference: &str, payload: &str) -> Result<(), FrameworkError>;
    fn load(&self, reference: &str) -> Result<Option<String>, FrameworkError>;
}
```

### AudienceStateStore

```rust
pub trait AudienceStateStore: Send + Sync {
    fn get(&self, session_id: &str, agent_id: &str) -> Result<AudienceState, FrameworkError>;
    fn set(&mut self, session_id: &str, agent_id: &str, state: AudienceState) -> Result<(), FrameworkError>;
    fn set_on_missing_policy(&mut self, policy: AudienceOnMissing);
}
```

### 说明

- `HistoryStore` 只管理历史读写，不负责裁剪或压缩策略。
- `ArchiveStore` 只管理归档存取，不负责摘要策略。
- `AudienceStateStore` 只管理可见范围状态。

### AudienceOnMissing

当前 `AudienceStateStore` 的“未初始化默认行为”是可配置的：

- `Error`
- `UseState { state: AudienceState }`

这使得默认行为不再由存储层写死。

---

## 9. Runtime / InMemoryRuntime / Kernel

路径：

- [`crates/agentfw-core/src/runtime.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)
- [`crates/agentfw-core/src/kernel.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/kernel.rs)

### RunEnv

```rust
pub struct RunEnv<'a> {
    pub session: &'a SessionState,
    pub resolvers: &'a ResolverBundle,
}
```

### AgentTurnResult

```rust
pub struct AgentTurnResult {
    pub outbound_content: Vec<ContentBlock>,
    pub effects: Vec<RuntimeEffect>,
    pub meta: serde_json::Value,
}
```

### Runtime trait

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

### 说明

- `apply_effects(...)` 是原子状态 helper，不承载调度/编排语义。
- `dispatch_content(...)` 是底层分发 helper，只把“当前 Agent 输出内容”实体化成可投递消息，不触发下一轮运行。
- `dispatch_content(...)` 当前已经是 `async`，因为分发时可能依赖异步的 `RouteResolver`。

### InMemoryRuntime

当前提供了默认内存实现：

- `InMemoryHistoryStore`
- `InMemoryArchiveStore`
- `InMemoryAudienceStateStore`
- `InMemoryRuntime`

### Kernel

`Kernel` 是当前高层 convenience 入口，提供：

- `execute_agent_turn(...)`
- `apply_turn_effects(...)`
- `dispatch_turn_content(...)`
- `run_agent_turn(...)`

其中：

- `run_agent_turn(...)` = 执行一轮 + 应用效果 + 分发正文  
  这是一个 convenience path（便捷路径），**不是工作流编排器**。
- `run_agent_turn(...)` 只返回当前这轮生成的 `Message` 列表，不会自动触发任何下游 Agent。

### Static 配置路径

`Kernel::from_static_config(...)` 当前已经能从静态配置组装：

- 默认 PromptResolver
- 默认 ModelResolver
- 默认 ToolResolver
- 默认 RouteResolver
- 默认 MemoryResolver
- 默认 NoopHistoryTransform

这是一条开发者可直接使用的接入路径，但仍不代表框架内置工作流。

---

## 10. 默认 Driver 实现

路径：[`crates/agentfw-core/src/default_drivers.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/default_drivers.rs)

当前提供两个默认 Driver：

- `LlmDriver`
- `ExternalDriver`

并提供兼容别名：

- `DefaultLlmDriver`
- `DefaultExternalDriver`

### LlmDriver

- 通过 `ResolverBundle::build_request(...)` 构造请求
- 调模型
- 使用 `DefaultProtocolNormalizer`
- 如果模型返回 tool call，则执行工具并把 `ToolResult` 回填成一条 `Tool` 消息，再次请求模型
- 当前默认语义是：**单轮内工具往返直到拿到正文输出或报错**
- 最终输出 `AgentTurnResult`

### ExternalDriver

- 当前为最小实现
- 直接把最后一条输入消息内容透传为 `outbound_content`

### 关键边界

这两个 Driver 是默认实现，不是框架规则。开发者可以注册自己的 Driver 并完全替换它们。

---

## 11. 配置模型

路径：[`crates/agentfw-core/src/config.rs`](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/config.rs)

当前提供两层配置：

### StaticConfig

更接近内核装配对象，包含：

- `session`
- `prompts`
- `models`
- `tool_bindings`
- `history_bindings`

### DeveloperConfig

更贴近开发者输入，包含：

- `session`
- `prompts`
- `models`
- `bindings.tools`
- `bindings.history`

`DeveloperConfig::into_static()` 会转换成 `StaticConfig`。

### RouteRule / normalized_routes

`SessionSpec.routes` 作为开发者静态输入存在。  
运行时真正生效依赖 `RouteResolver`。  
当前 `SessionSpec::normalized_routes()` 和 `StaticRouteTable` 已经把：

- 非空校验
- 去重
- 冲突检测

这些静态输入清洗做好了。

---

## 12. 当前边界总结

### 框架当前提供

- 统一 Agent 对象模型
- 统一 Message 对象模型
- 统一 Tool 对象模型
- 统一模型适配接口
- Resolver-first 运行时解析入口
- Driver 抽象与默认实现
- RuntimeEffect 状态变更通道
- 历史 / 归档 / 可见范围状态存储接口
- 便捷 Kernel 入口

### 框架当前不提供

- 调度器
- mailbox（邮箱）消费语义
- 串行/并行编排策略
- 固定角色模板
- 固定业务工作流
- 固定压缩/摘要策略

### 对开发者的含义

如果你基于当前 `agentfw-rs` 开发系统，需要自己负责：

- 哪个 Agent 何时被调用
- 消息如何排队
- 串行还是并行
- 历史如何裁剪/压缩/归档/召回

但你可以直接复用当前内核来：

- 跑 Agent 的一轮
- 调模型
- 调工具
- 应用运行时效果
- 生成并分发正文消息

---

## 13. 当前最适合的使用方式

如果你现在要接入 `agentfw-rs`，建议最小路径如下：

1. 定义 `AgentSpec`
2. 提供 `DeveloperConfig` 或自定义 Resolver
3. 创建 `Kernel`
4. 注册 Driver
5. 运行 `Kernel::run_agent_turn(...)`
6. 在开发者自己的系统里决定下一轮轮到谁

这条路径当前已经能跑通最小闭环。
