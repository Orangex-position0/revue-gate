# 路由调度器

> 领域服务 `ChannelSelector`，回答「这一请求该打给哪个渠道，失败后依次换谁」。

## 为什么需要路由调度器模块

revue-gate 把多个上游渠道（OpenAI / Claude / Gemini / DeepSeek / 自定义 OpenAI-compatible）聚合到统一的 OpenAI 兼容端点。一个模型请求可能命中多个渠道，需要一套规则决定：

1. **先用谁**（主备）——用户可能优先用某类渠道；
2. **失败后换谁**（failover）——上游限流 / 5xx / 网络错时应切换到下一个候选；
3. **同级别之间怎么分**（分摊）——多个同一模型的 api key 想按比例摊流量。

把「选渠道」独立成领域服务，而不是塞进 proxy 编排，是因为：

- **可测试**：纯函数、无 DB/HTTP，注入 RNG 即可确定性验证（Seam A）。
- **职责单一**：proxy 只负责「按队列逐个尝试 + 记账 + 重试」，选谁交给 `ChannelSelector`。
- **无状态**：桌面应用不维护长连接计数 / 运行态指标，选择是纯函数，每次请求洗一次牌即可复现。

## 分析场景

常见的负载均衡策略有多种，根据本项目的场景：

- **主备需要**：用户可能会优先选用某类模型，所以需要优先级。
- **分摊需求**：用户可能有多个对于同一模型的不同 api key，想按一定比例来分摊，所以需要权重。
- **无状态**：桌面应用不维护长连接计数，调度应该是无状态纯函数，所以权重随机而不用轮询（轮询需维护指针状态）。

最终决定采用 **「优先级分组 + 组内权重随机」** 的混合策略。

## 设计思路

- **优先级 = 主备**：高优先级组先试，组内耗尽 / 失败才落到下一组。
- **权重 = 分摊**：同优先级多渠道按权重比例随机分流。
- **无状态洗牌**：每次请求 shuffle 一次，重试只是消费同一队列，不重选。

调度器的产出是一个**有序 failover 队列**，proxy 顺序消费，只在 retryable 失败时换下一个候选。

## Dispatcher 领域

- 位置：`src-tauri/src/domain/dispatcher.rs`（`ChannelSelector`，领域服务，零技术依赖）。
- 相关领域类型：`src-tauri/src/domain/channel.rs`（`Channel`：`priority` / `weight` / `models` / `model_mappings` / `enabled`）。

### 核心方法 `select_channels()`

```rust
pub fn select_channels<R: Rng>(channels: &[Channel], model: &str, rng: &mut R) -> Vec<Channel>
```

- 输入：全量渠道 + 请求模型 + 随机源。
- 输出：有序候选队列（failover 顺序）；候选为空返回空 vec。
- RNG 注入：生产 `rand::rng()`，测试 `SmallRng::seed_from_u64(0)`（确定性）。

### 负载均衡流程

1. **Filter（过滤）**：只保留 `enabled` 且支持该模型的渠道。
2. **Group（分组）**：按 `priority` 升序稳定排序，同优先级聚成一组，组间硬排序（值小在前）。
3. **Draw（组内抽取）**：每组内按 `weight` 加权随机**不放回**，权重大者优先被选中。
4. **Concat（拼接）**：各组按序拼成一个队列返回。

### 模型支持判定（`supports`）

渠道支持某模型，当且仅当：

- `models` 为空（视为不限模型）；或
- `models` 含请求模型；或
- `model_mappings` 命中 `client_model`（映射名是面向下游的模型名）。

### 权重语义

| weight | 含义 |
| --- | --- |
| `> 0` | 组内按比例分摊，越大越常被选中 |
| `= 0` | 不参与随机，恒排组内最后作兜底 |
| `< 0` | 非法：命令层 create/update 拒绝（`usecases/channel.rs` normalize）；`select_channels` 内仍 `max(0)` 钳制防脏数据 |

### 加权随机不放回（轮盘赌）

组内 `total = Σ max(weight, 0)`：

- `total > 0`：在 `[0, total)` 取随机点，按累积权重找落点（`point -= weight`，`< 0` 命中即选中），抽出并移除，重复直到组空。
- `total == 0`（全 0 权重）：退化为输入顺序（每轮取第一个）。

## 与 proxy 编排的关系

- `usecases/proxy.rs` 消费队列：按顺序尝试，成功即返回。
- retryable 失败（传输错误 / 429 / 5xx）换下一个候选；`4xx` 属客户端错误不重试（原样透传）。
- 尝试上限由 `settings.retry` 控制：`enabled=false` 只试第一个；`max_retries=n` 表示首次后最多再试 `n` 次。
- 流式：打开失败同样重试；打开成功后的中途失败不跨渠道续接（下游协议不成立）。
- 无状态：每请求洗一次牌，重试只是消费同一队列。

## 边界与演进

- 当前**无 cooldown**：坏渠道每次请求会浪费一次尝试，单机单用户可接受（retry 已兜住）。见 `docs/Tech-debt.md` §10。
- 演进路线：失败 metrics → cooldown → latency / usage based routing；具体待办目前收敛在 `docs/Tech-debt.md`。
