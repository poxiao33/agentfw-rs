# Runtime Boundary（运行时边界）

本文档用于说明 `agentfw-rs` 当前 Runtime / Kernel（运行时 / 内核）层的边界：

- 哪些接口应视为稳定抽象
- 哪些对象只是默认实现
- 哪些能力属于内核原子能力
- 哪些事情明确不属于内核职责

---

## 1. 总体结论

当前 Runtime 层可以分成三层：

### 建议视为稳定抽象的

- `RunEnv`
- `AgentTurnResult`
- `AgentEngine`
- `Runtime`

### 建议视为高层便捷入口，但仍应谨慎承诺的

- `Kernel`

### 建议视为默认实现 / 参考实现的

- `BasicAgentEngine`
- `InMemoryRuntime`

也就是说：

> Runtime 层真正稳定的，是“单轮执行 + 应用效果 + 分发内容”这组原子能力抽象；  
> `Kernel` 和 `InMemoryRuntime` 更适合视为当前项目的默认落地实现。

---

## 2. 核心抽象

### 2.1 `RunEnv`

路径：
[runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

当前定义：

```rust
pub struct RunEnv<'a> {
    pub session: &'a SessionState,
    pub resolvers: &'a ResolverBundle,
}
```

建议理由：

- 它是 Driver / Engine 的统一运行时输入
- 它只携带：
  - 当前 Session 状态
  - 当前 Resolver 集合

建议稳定承诺：

- 继续保持“最小运行环境对象”定位
- 不在这里注入调度器、队列、工作流控制器等内容

---

### 2.2 `AgentTurnResult`

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

- 它是“单轮 Agent 执行结果”的统一承载对象
- 它天然把“正文内容”和“状态效果”分开

建议稳定承诺：

- `outbound_content`
- `effects`
- `meta`

继续作为最小结果模型。

---

### 2.3 `AgentEngine`

路径：
[runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

当前接口：

```rust
#[async_trait::async_trait]
pub trait AgentEngine: Send + Sync {
    async fn run_turn(
        &self,
        env: RunEnv<'_>,
        agent: &AgentSpec,
        incoming: &[Message],
    ) -> Result<AgentTurnResult, FrameworkError>;
}
```

建议理由：

- 它定义了“如何执行一轮 Agent”的统一抽象
- 它不带调度语义，只执行一轮

建议稳定承诺：

- 保持“单轮执行”定位
- 不扩张成自动工作流推进器

---

### 2.4 `Runtime`

路径：
[runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

当前接口：

```rust
#[async_trait]
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
```

建议理由：

- `apply_effects` 明确表示：把这一轮产生的状态效果落地
- `dispatch_content` 明确表示：把当前正文按可见范围和路由规则实体化成消息

建议稳定承诺：

- `Runtime` 继续只负责：
  - 应用效果
  - 分发内容
- 不承担：
  - 调度
  - 编排
  - 队列消费
  - 自动触发下游 Agent

---

## 3. 默认实现

### 3.1 `BasicAgentEngine`

路径：
[runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

当前行为：

- 读取 `agent.driver`
- 从 `DriverRegistry` 取出对应 Driver
- 调用该 Driver 的 `run_turn`

它只做“Driver 分派”，没有额外工作流语义。

建议定位：

- 可以公开使用
- 但更适合视为默认 Engine 实现，而不是框架语义本身

---

### 3.2 `InMemoryRuntime`

路径：
[runtime.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/runtime.rs)

当前行为：

- 应用 `RuntimeEffect`
- 读取 `AudienceState`
- 调用 `RouteResolver`
- 将正文分发成真正 `Message`

需要明确的边界：

- 它不是调度器
- 它不会自动运行下一个 Agent
- 它只是把当前这轮的结果落盘并分发出去

建议定位：

- 公开可用
- 但应视为默认落地实现，而不是框架唯一实现

---

## 4. `Kernel` 的定位

路径：
[kernel.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/kernel.rs)

当前 `Kernel` 提供：

- `execute_agent_turn`
- `apply_turn_effects`
- `dispatch_turn_content`
- `run_agent_turn`
- `set_audience_state`
- 静态配置装配辅助能力

### 4.1 建议保留为稳定便捷入口的

- `Kernel::new`
- `Kernel::builder`
- `Kernel::execute_agent_turn`
- `Kernel::apply_turn_effects`
- `Kernel::dispatch_turn_content`
- `Kernel::run_agent_turn`
- `Kernel::set_audience_state`

这些接口都与“单轮执行内核”直接相关，适合继续暴露。

### 4.2 不建议现在承诺稳定的

- `Kernel::from_static_config`
- `Kernel::build_static_resolvers`
- 依赖 `Static*Resolver` 的静态引导路径

这些更像 bootstrap（引导）路径，不应被视为内核长期契约。

---

## 5. `run_agent_turn` 的边界

这是当前最容易被误解的接口。

当前行为：

1. 执行一轮 Agent
2. 应用这一轮产生的 `RuntimeEffect`
3. 按当前 `AudienceState + RouteResolver` 分发正文
4. 返回当前这轮落成的 `Vec<Message>`

它**不会**：

- 自动触发任何下游 Agent
- 自动形成主从工作流
- 自动消费消息队列
- 自动等待多个 Agent 的结果

所以正确理解应是：

> `run_agent_turn(...)` 是“单轮执行 + 应用效果 + 分发正文”的便捷入口，不是工作流执行器。

---

## 6. 当前 Runtime 层和宿主系统的责任分界

### Runtime / Kernel 负责

- 运行某个 Agent 的一轮
- 应用效果
- 生成分发后的消息

### 宿主系统负责

- 哪个 Agent 何时运行
- 多条消息如何消费
- 是否串行、并行
- 是否重试
- 是否等待其他 Agent
- 是否形成工作流

这条分界是当前项目最重要的边界之一。

---

## 7. 对宿主系统的建议

宿主系统如果要稳定依赖 Runtime 层，建议只依赖：

- `AgentEngine`
- `Runtime`
- `Kernel` 的最小执行入口
- `AgentTurnResult`

不建议依赖：

- `InMemoryRuntime` 的内部字段结构
- `Kernel` 的静态配置构造细节

---

## 8. 一句话总结

当前 `agentfw-rs` 的 Runtime / Kernel 层最适合被理解为：

> **“单轮执行内核 + 状态效果落地 + 正文分发助手”**

而不是：

> “工作流引擎”或“调度器”。

这也是当前最适合对外公开的边界定义。
