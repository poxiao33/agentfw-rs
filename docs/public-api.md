# Public API Draft（公开 API 草案）

本文档描述 `agentfw-rs` 当前建议对外稳定暴露的接口边界。

目标不是定义“最终 1.0 已冻结 API”，而是明确：

- 哪些类型和 trait（特征）是框架核心抽象，适合对外承诺稳定性
- 哪些对象只是默认实现、示例实现或便捷实现
- 宿主系统应该依赖哪些接口，而不应该依赖哪些内部细节

---

## 1. 总原则

### 1.1 建议对外稳定的内容

对外稳定的应该是：

- 核心对象模型
- 核心 trait（特征）接口
- 最小内核行为约束

也就是：

- `AgentSpec`
- `Message`
- `ContentBlock`
- `ToolDefinition`
- `ToolCall`
- `ToolResult`
- `RuntimeEffect`
- `SessionState`
- `ModelAdapter`
- `AgentDriver`
- `DriverRegistry`
- `ToolCatalog`
- `ResolverBundle` 及其子 Resolver trait
- `HistoryStore`
- `ArchiveStore`
- `AudienceStateStore`
- `Runtime`
- `Kernel` 的最小执行入口
- `FrameworkError`

### 1.2 不建议承诺稳定的内容

以下内容目前更适合视为默认实现或便捷实现，而不是“对外 API 契约”：

- `LlmDriver`
- `StreamingLlmDriver`
- `ExternalDriver`
- `InMemoryRuntime`
- `InMemoryHistoryStore`
- `InMemoryArchiveStore`
- `InMemoryAudienceStateStore`
- `BasicAgentEngine`
- `StaticPromptResolver`
- `StaticModelResolver`
- `StaticToolResolver`
- `StaticRouteResolver`
- `StaticMemoryResolver`
- `NoopHistoryTransform`
- `Kernel::from_static_config`
- `Kernel::build_static_resolvers`
- `StaticConfig`
- `DeveloperConfig`
- `StaticModelBinding`
- `StaticToolBinding`
- `StaticHistoryBinding`
- 示例中的具体 agent 命名和装配方式

这些对象当然可以公开导出，但不建议在语义层面承诺“长期不变”。

---

## 2. 建议对外稳定的核心对象

### 2.1 `AgentSpec`

路径：
[agent.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/agent.rs)

建议稳定原因：

- 它定义了 Agent 的最小配置输入
- 宿主系统一定会构造它

当前关键字段：

- `id`
- `name`
- `driver`
- `prompt_ref`
- `model_ref`
- `metadata`

建议稳定承诺：

- 保持“最小配置对象”定位
- 允许未来增字段，但尽量不破坏现有字段语义

---

### 2.2 `Message` / `MessageDraft` / `ContentBlock`

路径：
[message.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/message.rs)

建议稳定原因：

- 它们是宿主系统与内核之间最基础的通信对象
- 一旦变化，几乎所有接入都会受影响

建议稳定承诺：

- `Message` 是统一消息对象
- `MessageDraft` 是推荐的消息构造入口
- `ContentBlock` 是统一内容块表示

注意：

当前 `MessageDraft::commit` / `commit_auto` 仍然允许绕过官方 `dispatch_content` 主路径，宿主系统应自行约束是否允许直接构造消息。

---

### 2.3 `ToolDefinition` / `ToolCall` / `ToolResult`

路径：
[tool.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/tool.rs)

建议稳定原因：

- 工具系统是框架内核的一等能力
- 宿主系统会直接定义工具和消费工具结果

建议稳定承诺：

- 工具定义模型保持稳定
- 工具调用模型保持稳定
- `ToolResult.effects` 继续作为运行时状态变更通道

---

### 2.4 `RuntimeEffect`

路径：
[state.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/state.rs)

建议稳定原因：

- 它是“工具 -> runtime（运行时）状态变化”的统一桥梁
- 宿主系统和工具实现都会依赖它

当前包含：

- `SetAudience`
- `AppendHistory`
- `ArchivePayload`
- `Custom`

建议稳定承诺：

- `RuntimeEffect` 继续作为效果模型存在
- 允许未来新增 effect 变体
- 不轻易移除已有 effect 语义

---

### 2.5 `AudienceState`

路径：
[state.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/state.rs)

建议稳定原因：

- 它承载当前 Agent 的“可见范围”状态
- 通信通过“可见范围 + 系统分发”这条主链依赖它

建议稳定承诺：

- `visible_to: Vec<String>` 继续作为最小模型
- 是否扩展消息级可见性，不应影响当前最小结构可用性

---

### 2.6 `SessionState`

路径：
[state.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/state.rs)

建议稳定原因：

- 它是 Runtime / Resolver 的共同输入

当前字段较少：

- `session_id`
- `metadata`

建议稳定承诺：

- 它继续作为会话运行状态的最小对象
- 允许未来扩展，不轻易改变已有字段语义

---

## 3. 建议对外稳定的核心 trait

### 3.1 `ModelAdapter`

路径：
[model.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/model.rs)

建议稳定原因：

- 不同模型接入一定依赖它

建议稳定承诺：

- `name`
- `capabilities`
- `send`
- `stream`（可选）
- `ModelRequest`
- `ModelResponse`
- `ModelStreamChunk`
- `ModelAdapterError`

这几个核心接口应尽量保持稳定。

补充：

- 当前项目已经拆分出多个默认适配器实现：
  - OpenAI Chat Completions
  - OpenAI Responses API
  - Anthropic Messages
- `OpenAICompatibleAdapter` 目前是兼容别名，而不是独立第四套语义。
- 模型层已经具备可选 `stream()` 能力，但默认 `LlmDriver` 目前仍主要消费 `send()`；因此“流式模型适配能力”和“流式 Agent 驱动能力”应视为两个不同层次。

---

### 3.2 `AgentDriver`

路径：
[runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

建议稳定原因：

- Driver 是“如何运行一个 Agent 一轮”的核心扩展点

建议稳定承诺：

- `run_turn(env, agent, incoming) -> AgentTurnResult`

说明：

当前默认 Driver（如 `LlmDriver`）的具体执行策略不应被视为框架规则，但 `AgentDriver` 这个抽象本身应保持稳定。

---

### 3.3 `DriverRegistry`

路径：
[driver.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/driver.rs)

建议稳定原因：

- 所有 Driver 的注册与解析入口都依赖它

建议稳定承诺：

- `register`
- `get`

---

### 3.4 `ToolCatalog`

路径：
[tool.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/tool.rs)

建议稳定原因：

- 它是工具定义目录的基础接口

建议稳定承诺：

- `register`
- `get`
- `list`

---

### 3.5 Resolver traits

路径：
[resolver.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/resolver.rs)

建议稳定原因：

- Resolver 是策略注入主入口

建议稳定的 trait：

- `ModelResolver`
- `PromptResolver`
- `ToolResolver`
- `RouteResolver`
- `MemoryResolver`
- `HistoryTransform`

以及：

- `ResolverBundle`

建议稳定承诺：

- 这些 trait 会继续存在
- 入参与返回值形状尽量保持兼容

---

### 3.6 `HistoryStore` / `ArchiveStore` / `AudienceStateStore`

路径：
[storage.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/storage.rs)

建议稳定原因：

- 存储是宿主系统最容易替换的部分
- 这些 trait 是扩展点

建议稳定承诺：

- `HistoryStore` 继续按 `session_id + agent_id` 维度组织历史
- `ArchiveStore` 继续提供简单的按 reference（引用键）读写
- `AudienceStateStore` 继续提供当前可见范围状态读写能力

---

### 3.7 `Runtime`

路径：
[runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

建议稳定原因：

- 它定义了 Runtime 最小职责

建议稳定承诺：

- `apply_effects`
- `dispatch_content`

说明：

`Runtime` 的职责应继续保持在：
- 应用效果
- 分发内容

而不扩张成调度器或工作流编排器。

---

## 4. `Kernel` 应该如何看待

路径：
[kernel.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/kernel.rs)

`Kernel` 当前是高层 convenience（便捷）入口。

建议区分：

### 可作为对外稳定入口的部分

- `Kernel::new`
- `Kernel::builder`
- `Kernel::execute_agent_turn`
- `Kernel::apply_turn_effects`
- `Kernel::dispatch_turn_content`
- `Kernel::run_agent_turn`
- `Kernel::set_audience_state`

### 不建议作为长期稳定承诺的部分

- `from_static_config`
- `build_static_resolvers`
- 各种 `Static*Resolver`
- `with_default_*`

这些更像 demo / bootstrap（引导）能力，而不是长期稳定 API。

---

## 5. 默认实现的定位

当前项目已经内置一些默认实现，它们很有用，但不建议等同于框架规则。

包括：

- `LlmDriver`
- `ExternalDriver`
- `InMemoryRuntime`
- `InMemoryHistoryStore`
- `InMemoryArchiveStore`
- `InMemoryAudienceStateStore`
- `Static*Resolver`

建议对外表述统一为：

> 这些是参考实现和便捷默认实现，不是框架语义本身。

---

## 6. 宿主系统应该依赖什么，不应该依赖什么

### 建议依赖

- trait（特征）接口
- 核心对象模型
- `Kernel` 的最小执行入口

### 不建议依赖

- 示例中的 agent 命名
- 文档里的静态配置形状作为唯一方案
- 默认 Driver 的具体内部步骤
- InMemory 实现的内部结构

---

## 7. 当前状态下的公开 API 建议

如果现在就要对外发布 alpha（早期）版本，我建议：

### 标注为“建议稳定”

- `agent.rs`
- `message.rs`
- `tool.rs`
- `model.rs`
- `resolver.rs`
- `runtime.rs`
- `state.rs`
- `storage.rs`
- `error.rs`

### 标注为“默认实现 / 实验性”

- `default_drivers.rs`
- `kernel.rs` 中的 static config 帮助方法
- `config.rs`
- `openai_compatible.rs`
- `builtin_tools.rs`

---

## 8. 一句话结论

如果按“框架内核”和“默认实现”来区分，当前 `agentfw-rs` 最适合对外公开的方式是：

> **稳定暴露核心抽象与 trait，明确默认实现只是便捷实现，不等同于框架规则。**

这样既不把接口藏起来，也不会把当前尚在演进的默认实现误包装成最终规范。
