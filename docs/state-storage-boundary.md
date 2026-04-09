# State & Storage Boundary（状态与存储边界）

本文档用于说明 `agentfw-rs` 当前状态层与存储层的边界：

- 哪些对象适合视为稳定抽象
- 哪些实现只是默认内存实现
- `RuntimeEffect` 在体系里的职责是什么
- 历史与归档能力在框架中到底处于什么层次

---

## 1. 总体结论

当前状态与存储层可以分成两类：

### 建议视为稳定抽象的

- `SessionState`
- `AudienceState`
- `AudienceOnMissing`
- `RuntimeEffect`
- `HistoryStore`
- `ArchiveStore`
- `AudienceStateStore`

### 建议视为默认实现 / 便捷实现的

- `InMemoryHistoryStore`
- `InMemoryArchiveStore`
- `InMemoryAudienceStateStore`

也就是说：

> 状态与存储的**接口层**已经比较适合作为框架稳定面；  
> 内存实现仍更适合作为默认实现，而不是框架规则。

---

## 2. 状态对象边界

### 2.1 `SessionState`

路径：
[state.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/state.rs)

当前结构：

```rust
pub struct SessionState {
    pub session_id: SessionId,
    pub metadata: Value,
}
```

建议理由：

- 它是 Runtime、Resolver、Driver 共享的最小会话状态对象

建议稳定承诺：

- 保持“最小会话状态”定位
- 允许未来增字段，但尽量不破坏现有语义

---

### 2.2 `AudienceState`

路径：
[state.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/state.rs)

当前结构：

```rust
pub struct AudienceState {
    pub visible_to: Vec<String>,
}
```

说明：

- 它承载当前 Agent 的“后续正文可见对象列表”
- 系统在 `dispatch_content(...)` 时读取它并完成分发

建议稳定承诺：

- `visible_to: Vec<String>` 继续作为最小模型
- `normalize(...)` 继续只做排序和去重，不扩张成复杂分发策略

---

### 2.3 `AudienceOnMissing`

路径：
[state.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/state.rs)

当前变体：

- `Error`
- `UseState { state: AudienceState }`

说明：

- 它控制：如果某个 Agent 当前没有初始化 audience（可见范围），运行时该如何处理

建议稳定承诺：

- 继续保留“错误”与“使用默认状态”这两类最小策略

---

## 3. `RuntimeEffect` 的边界

路径：
[state.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/state.rs)

当前结构：

```rust
pub enum RuntimeEffect {
    SetAudience { visible_to: Vec<String> },
    AppendHistory { messages: Vec<Message> },
    ArchivePayload { reference: String, payload: String },
    Custom { name: String, payload: Value },
}
```

### 当前职责

`RuntimeEffect` 的职责是：

- 承载某一轮执行后产生的运行时状态变化
- 由 Runtime 统一应用

当前已经明确落地的 effect：

- `SetAudience`
- `AppendHistory`
- `ArchivePayload`

### 关键边界

`RuntimeEffect` 是：

- **状态变化的推荐主路径**

但当前不是：

- **唯一合法状态写入口**

原因是：

- `Kernel::set_audience_state(...)` 仍然允许宿主做受控初始化
- InMemory store（内存存储）仍然是可变对象

所以当前最准确的表述应该是：

> `RuntimeEffect` 已经是框架主路径中的统一效果模型，但还没有从类型层完全封死一切旁路。

### 建议稳定承诺

- `RuntimeEffect` 继续作为效果模型保留
- 允许未来扩展更多 effect 变体
- 不轻易改变已有变体语义

---

## 4. 历史与归档接口边界

路径：
[storage.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/storage.rs)

### 4.1 `HistoryStore`

当前接口：

```rust
pub trait HistoryStore: Send + Sync {
    fn load(&self, session_id: &str, agent_id: &str) -> Result<Vec<Message>, FrameworkError>;
    fn append(&mut self, session_id: &str, agent_id: &str, msgs: Vec<Message>) -> Result<(), FrameworkError>;
    fn replace(&mut self, session_id: &str, agent_id: &str, msgs: Vec<Message>) -> Result<(), FrameworkError>;
}
```

建议理由：

- 它定义了最小历史读写能力
- 当前已明确按 `session_id + agent_id` 维度组织历史，边界比较清楚

建议稳定承诺：

- 继续按 `session_id + agent_id` 维度索引
- 继续只提供 `load / append / replace`
- 不在这里内置裁剪、压缩、摘要策略

---

### 4.2 `ArchiveStore`

当前接口：

```rust
pub trait ArchiveStore: Send + Sync {
    fn save(&mut self, reference: &str, payload: &str) -> Result<(), FrameworkError>;
    fn load(&self, reference: &str) -> Result<Option<String>, FrameworkError>;
}
```

建议理由：

- 它提供最小的“归档存取”能力
- 当前边界非常清楚：只管存取，不管摘要策略

建议稳定承诺：

- 继续保持最小 key-value（键值）风格接口

---

### 4.3 `AudienceStateStore`

当前接口：

```rust
pub trait AudienceStateStore: Send + Sync {
    fn get(&self, session_id: &str, agent_id: &str) -> Result<AudienceState, FrameworkError>;
    fn set(&mut self, session_id: &str, agent_id: &str, state: AudienceState) -> Result<(), FrameworkError>;
    fn set_on_missing_policy(&mut self, policy: AudienceOnMissing) -> Result<(), FrameworkError>;
}
```

建议理由：

- 它是“可见范围 + 系统分发”模型的核心存储接口

建议稳定承诺：

- 保持按 `session_id + agent_id` 维度读写 audience（可见范围）
- 保持 `on_missing` 策略配置能力

---

## 5. 默认内存实现的定位

### 5.1 `InMemoryHistoryStore`
### 5.2 `InMemoryArchiveStore`
### 5.3 `InMemoryAudienceStateStore`

这些实现都在：
[storage.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/storage.rs)

当前定位应为：

- 默认内存实现
- 示例和测试友好
- 方便开发者快速接入

但不建议现在承诺它们的内部行为和性能语义长期不变。

换句话说：

> 接口可以稳定，内存实现不一定要被视为正式规范。

---

## 6. 与压缩/摘要策略的关系

当前框架里：

- `HistoryStore`
- `ArchiveStore`

已经提供了足够的基础能力，使开发者在框架外部实现：

- 历史压缩
- 摘要替换
- 分层记忆
- 外部归档

因此：

- 状态层 / 存储层只提供基础设施
- 不内置压缩策略
- 不内置摘要规则

这和整个项目“只提供原子能力，不提供策略”的原则是一致的。

---

## 7. 宿主系统应该依赖什么

宿主系统如果要长期依赖状态与存储层，建议依赖：

- `SessionState`
- `AudienceState`
- `AudienceOnMissing`
- `RuntimeEffect`
- `HistoryStore`
- `ArchiveStore`
- `AudienceStateStore`

不建议强依赖：

- `InMemoryHistoryStore`
- `InMemoryArchiveStore`
- `InMemoryAudienceStateStore`

除非只是测试、示例或快速接入。

---

## 8. 一句话总结

当前 `agentfw-rs` 的状态与存储层最适合被理解为：

> **“稳定的状态/存储接口 + 默认的内存实现”**

其中：

- 接口层已经比较适合视为框架稳定面
- 默认实现仍然更适合作为便捷实现，而不是规则本身
