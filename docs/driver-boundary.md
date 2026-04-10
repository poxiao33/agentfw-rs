# Driver Boundary（Driver 层接口边界）

本文档用于说明 `agentfw-rs` 当前 Driver（驱动器）层的边界：

- 哪些接口应视为稳定抽象
- 哪些对象只是默认实现
- 当前默认 Driver 的行为到底是什么
- 模型层流式能力与 Driver 层流式消费之间的关系

---

## 1. 总体结论

当前 Driver 层可以分成两个层次：

### 建议视为稳定抽象的

- `AgentDriver`
- `DriverRegistry`
- `BasicAgentEngine`
- `AgentTurnResult`

这些是框架的核心执行抽象，宿主系统可以依赖。

### 建议视为默认实现 / 参考实现的

- `LlmDriver`
- `ToolLoopLlmDriver`
- `StreamingLlmDriver`
- `ExternalDriver`
- `DefaultLlmDriver`
- `DefaultStreamingLlmDriver`
- `DefaultExternalDriver`

这些对象是当前项目提供的默认 Driver，不应被视为框架规则本身。

---

## 2. 稳定抽象

### 2.1 `AgentDriver`

路径：
[runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

核心接口：

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

建议理由：

- 它定义了“如何运行一个 Agent 一轮”的最小统一接口
- 宿主系统可以完全替换默认 Driver，而不需要改内核其他部分

建议稳定承诺：

- 继续保留“单轮运行”接口形态
- 不把调度、编排、自动下一轮推进塞进这个 trait

---

### 2.2 `DriverRegistry`

路径：
[driver.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/driver.rs)

核心接口：

```rust
pub trait DriverRegistry: Send + Sync {
    fn register(&mut self, key: String, driver: Box<dyn AgentDriver>)
        -> Result<(), FrameworkError>;
    fn get(&self, key: &str) -> Option<&dyn AgentDriver>;
}
```

建议理由：

- 它定义了 Driver 的查找和替换入口
- 是宿主系统接入自定义 Driver 的关键点

建议稳定承诺：

- 继续只做“注册和查找”
- 不在这里引入调度、优先级、角色语义等概念

---

### 2.3 `BasicAgentEngine`

路径：
[runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

说明：

`BasicAgentEngine` 当前只做一件事：

- 按 `agent.driver` 从 `DriverRegistry` 取出 Driver
- 调用该 Driver 跑一轮

它没有内置：

- mailbox（邮箱）
- 下一轮自动推进
- 主从 Agent 关系
- 工作流模板

建议理由：

- 这是当前内核“只跑一轮”的核心执行器

---

### 2.4 `AgentTurnResult`

路径：
[runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

当前定义：

```rust
pub struct AgentTurnResult {
    pub outbound_content: Vec<ContentBlock>,
    pub effects: Vec<RuntimeEffect>,
    pub meta: serde_json::Value,
}
```

建议理由：

- 它是 Driver 层和 Runtime 层之间的统一结果对象
- 现在已经很好地表达了：
  - 正文输出
  - 状态效果
  - 调试/原始元信息

建议稳定承诺：

- `outbound_content`
- `effects`
- `meta`

这三类信息应继续保留。

---

## 3. 默认实现

### 3.1 `LlmDriver`

路径：
[default_drivers.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/default_drivers.rs)

当前语义：

- 通过 `ResolverBundle::build_request(...)` 解析：
  - 模型
  - prompt
  - 历史
  - 工具
- 发起一次模型请求
- 返回该次请求产生的：
  - 正文内容
  - 或工具调用内容块

也就是说，当前默认 `LlmDriver` 是：

> 一个“单轮原子执行”的默认实现

它不会在 Driver 内部继续自动跑工具循环。

---

### 3.2 `ToolLoopLlmDriver`

路径：
[default_drivers.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/default_drivers.rs)

当前语义：

- 单轮内执行“模型返回工具调用 -> 执行工具 -> 工具结果回填 -> 再次请求模型”的循环
- 默认最多 20 轮
- 开发者可显式配置覆盖最大轮数

这说明：

> `ToolLoopLlmDriver` 是一个便捷实现，而不是框架内核默认行为

特别说明：

- 模型层现在已经支持 `stream()`
- 但当前默认 `LlmDriver` 仍然主要消费 `send()`
- 也就是说，“流式模型适配能力”和“流式 Driver 实现”目前还是分开的

---

### 3.3 `StreamingLlmDriver`

路径：
[default_drivers.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/default_drivers.rs)

当前语义：

- 优先检查 `ModelAdapter::stream()`
- 如果模型支持流式且当前请求不带工具，则消费 `stream()`
- 当前只做“最小可用文本流式”：
  - 收集文本块
  - 记录停止信号
  - 记录最后一条原始事件
- 如果模型不支持流式，或当前请求带工具，则自动回退到 `ToolLoopLlmDriver` 路径
- 如果流中出现 `ToolCall`，当前实现直接报协议错误，不做流式工具往返

这说明：

> `StreamingLlmDriver` 是一个“文本优先”的默认流式 Driver，而不是一个完整的流式工具代理驱动。

因此它更适合视为：

- 默认实现
- 参考实现
- 演进中的 Driver

而不是稳定规则。

---

### 3.3 `ExternalDriver`

路径：
[default_drivers.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/default_drivers.rs)

当前语义：

- 取最后一条输入消息
- 将其 `content` 直接透传为 `outbound_content`

这是最小默认实现，适合：

- 被动 Agent
- 外部输入型 Agent
- 示例和测试

它不应被视为“外部驱动只能这样工作”的规范。

---

### 3.4 `DefaultLlmDriver` / `DefaultStreamingLlmDriver` / `DefaultExternalDriver`

当前只是兼容别名：

- `DefaultLlmDriver = LlmDriver`
- `DefaultStreamingLlmDriver = StreamingLlmDriver`
- `DefaultExternalDriver = ExternalDriver`

建议：

- 可继续保留导出
- 但不要在文档中把它们包装成比真实实现更强的概念

---

## 4. 当前 Driver 层和模型层的边界

### 模型层负责

- 统一请求/响应对象
- 支持 `send`
- 支持可选 `stream`
- 细化模型相关错误

### Driver 层负责

- 决定这一轮如何消费模型能力
- 决定是否在一轮内进行工具往返
- 决定是否只走 `send`
- 将最终结果整理为 `AgentTurnResult`

### 当前状态

- 模型层：已支持 `send + optional stream`
- Driver 层：默认仍以 `send` 为主

这意味着：

> 模型层已经具备流式能力，但 Driver 层还没有把流式消费当成默认行为。

这是当前设计里的一个明确边界，而不是缺陷。

---

## 5. 宿主系统应依赖什么

如果开发者要基于 `agentfw-rs` 扩展 Driver 层，建议依赖：

- `AgentDriver`
- `DriverRegistry`
- `BasicAgentEngine`
- `AgentTurnResult`

不建议依赖：

- `LlmDriver` 的内部步骤
- `ExternalDriver` 的具体行为
- `LlmTurnRunner` 这种内部辅助结构

---

## 6. 最终建议

当前最合适的对外表述是：

> Driver 层的稳定部分是“单轮执行抽象”，不是默认 Driver 的具体行为。  
> `LlmDriver` 和 `ExternalDriver` 是框架附带的便捷实现，宿主系统可以直接使用，也可以完全替换。

如果未来要继续推进 Driver 层，我建议优先考虑：

- 是否新增 `StreamingLlmDriver`
- 是否把默认 `LlmDriver` 的“单轮工具往返”行为进一步参数化

但这些都属于默认实现演进，不影响当前公开抽象边界。
