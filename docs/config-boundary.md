# Config Boundary（配置层接口边界）

本文档用于说明 `agentfw-rs` 当前配置层的边界：

- 哪些配置结构更适合作为对外稳定的配置输入
- 哪些配置结构只是静态装配 / 引导辅助
- 哪些能力仍应视为默认实现，而不是框架规则

---

## 1. 总体结论

当前 `config.rs` 里的配置对象可以分成两层：

### 建议视为较稳定的配置模型

- `SessionSpec`
- `RouteRule`

这两者更像框架的“最小图输入”：
- 有哪些 Agent
- 它们的静态路由关系是什么

### 不建议现在就承诺长期稳定的配置模型

- `StaticConfig`
- `DeveloperConfig`
- `DeveloperBindings`
- `StaticPromptMap`
- `StaticToolBinding`
- `StaticHistoryBinding`
- `StaticModelBinding`

这些对象更接近：

- demo / bootstrap（引导）配置
- 静态装配辅助结构
- 默认解析器输入格式

它们当然可以继续使用，但从框架治理角度，不建议现在就把它们等同于“正式稳定配置 API”。

---

## 2. 建议较稳定的配置对象

### 2.1 `SessionSpec`

路径：
[config.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/config.rs)

当前结构：

```rust
pub struct SessionSpec {
    pub id: String,
    pub agents: Vec<AgentSpec>,
    pub routes: Vec<RouteRule>,
    pub metadata: Value,
}
```

建议理由：

- 它表达了一个 Session（会话）最基础的静态装配信息
- 与框架“只提供原子能力，不提供编排”的原则一致

建议稳定承诺：

- `id`
- `agents`
- `routes`
- `metadata`

这四类字段应尽量保持兼容。

---

### 2.2 `RouteRule`

路径：
[config.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/config.rs)

当前结构：

```rust
pub struct RouteRule {
    pub from: String,
    pub to: String,
    pub allow: bool,
}
```

建议理由：

- 它是最小静态路由输入
- 当前 `StaticRouteResolver` 就基于它工作

说明：

- 它只是静态输入
- 真正运行时是否允许投递，仍以 `RouteResolver` 为准

也就是说：

> `RouteRule` 更适合被看成开发者提供给 Resolver 的输入数据，而不是框架唯一的路由规则来源。

---

## 3. 不建议现在就承诺稳定的配置对象

### 3.1 `StaticConfig`

路径：
[config.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/config.rs)

当前用途：

- 作为一份完整静态配置输入
- 供 `Kernel::from_static_config(...)` 和 `build_static_resolvers(...)` 使用

不建议现在承诺稳定的原因：

- 它把 Prompt、Model、Tool、History 全部装进一个静态结构
- 更像“默认引导格式”，不一定适合未来所有宿主系统

建议表述：

> `StaticConfig` 目前是默认静态接入格式，不应被视为框架最终配置规范。

---

### 3.2 `DeveloperConfig`

路径：
[config.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/config.rs)

当前用途：

- 更贴近开发者书写的配置格式
- 通过 `into_static()` 转成 `StaticConfig`

不建议现在承诺稳定的原因：

- 它本质上还是默认静态装配配置
- 并没有体现更通用的 Resolver 驱动配置能力

建议表述：

> `DeveloperConfig` 是当前项目提供的开发者友好输入格式，不应等同于框架长期稳定配置 API。

---

### 3.3 `StaticModelBinding`

路径：
[config.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/config.rs)

当前字段：

- `key`
- `provider`
- `model`
- `base_url`
- `api_key_env`

当前状态：

- 已经能映射到：
  - OpenAI Chat Completions
  - OpenAI Responses API
  - Anthropic Messages

但仍不建议现在承诺长期稳定，因为：

- 它还是一个“统一最小壳”
- provider-specific（厂商专属）高级参数还未系统化
- 未来可能继续分层或细化

建议表述：

> `StaticModelBinding` 是当前静态配置下的模型绑定格式，适合作为默认输入，但仍应视为演进中的配置对象。

---

### 3.4 `StaticToolBinding` / `StaticHistoryBinding`

路径：
[config.rs](/Users/kang/Claude-works/agentCdp/agentfw-rs/crates/agentfw-core/src/config.rs)

它们当前的角色是：

- `StaticToolBinding`
  给某个 Agent 静态绑定工具集合
- `StaticHistoryBinding`
  给某个 Agent 提供初始历史

不建议视为长期稳定的原因：

- 它们本质上是静态 Resolver 输入
- 未来更复杂的宿主系统可能根本不会用静态绑定，而是自己实现 Resolver

建议表述：

> 它们更适合作为默认静态装配能力，而不是对外长期承诺的正式配置协议。

---

## 4. `validate()` 的定位

`StaticConfig::validate()` 和 `DeveloperConfig::validate()` 当前主要负责：

- 检查 session id 非空
- 检查 agent id 非空
- 检查 prompt/model 引用存在
- 检查 route 指向的 agent 存在
- 检查静态绑定引用的 agent 存在

这类校验是有价值的，但需要注意：

- 它校验的是“当前静态配置模型的自洽性”
- 不是框架所有可能配置方式的通用真理

所以建议文档里把它描述成：

> 当前默认静态配置路径的完整性校验逻辑

而不是“框架唯一配置校验标准”。

---

## 5. 当前配置层和 Resolver 的关系

当前代码里，静态配置层的真正作用是：

1. 读取开发者的静态输入
2. 转换成：
   - `StaticPromptResolver`
   - `StaticModelResolver`
   - `StaticToolResolver`
   - `StaticRouteResolver`
   - `StaticMemoryResolver`

也就是说：

> 配置层不是框架规则层，而是默认 Resolver 的输入数据层。

这是理解当前配置边界的关键。

---

## 6. 宿主系统应该怎么依赖配置层

### 建议依赖

宿主系统如果只是想快速用起来，可以依赖：

- `SessionSpec`
- `RouteRule`
- `DeveloperConfig`
- `StaticConfig`

### 不建议强依赖

如果宿主系统准备长期使用并深度定制，建议不要把以下内容视为稳定契约：

- `StaticModelBinding`
- `StaticToolBinding`
- `StaticHistoryBinding`
- `Kernel::from_static_config(...)`

原因是这些都更像：
- 默认接入路径
- 演示路径
- bootstrap（引导）路径

---

## 7. 建议的对外表述

当前最合适的公开表述是：

> 配置层中，`SessionSpec` 与 `RouteRule` 更接近框架稳定输入；  
> 其余 `Static*` / `Developer*` 配置对象主要服务于默认静态装配路径，适合快速接入，但不建议现在就视为长期冻结的公开配置协议。

---

## 8. 一句话总结

当前 `agentfw-rs` 的配置层应被理解为：

> **“默认静态装配输入”已经存在，但“长期稳定配置协议”尚未完全冻结。**

这和当前框架整体状态是一致的：

- 核心抽象已经基本清楚
- 默认实现已经可用
- 但默认配置装配路径仍然属于演进中的外围能力
