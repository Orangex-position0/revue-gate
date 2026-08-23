# 代理转发模块

> 网关核心闭环：认证 → 选渠道 → 模型映射 → 安全审计 → 转发 → 记账 → 写日志 → 失败重试。

## 为什么需要

revue-gate 的对外价值是把多个上游供应商（OpenAI / Claude / Gemini / DeepSeek / 自定义）聚合到一个统一的 OpenAI 兼容端点。代理转发模块（`ProxyRequestUsecase`）就是实现这个聚合的引擎——它不自己碰 HTTP，只编排「认证、调度、安全审计、转发、记账、日志」这条链：协议差异丢给 `ProviderAdaptor`，选谁丢给 `ChannelSelector`。

## 设计思路

### 分层与依赖

- 位置：`src-tauri/src/usecases/proxy.rs`（用例层，**不是领域**）。
- 依赖全部向内：三个仓储 `Arc<dyn Trait>` + 适配器 resolver 闭包 + 共享 `GatewaySettings`。
- 不碰真实 HTTP / DB：适配器经 `AdaptorResolver`（`Box<dyn Fn(&Channel) -> Box<dyn ProviderAdaptor>>`）注入，Seam A 可完全 mock。

### 请求处理全链路

1. **认证**：`AuthenticateRequestUsecase`（缺 / 无效 key → 401；配额超 → 429）。
2. **选渠道**：`ChannelSelector::select_channels`（启用 → 模型匹配 → 优先级分组 → 组内权重随机），产出有序 failover 队列。
3. **模型映射**：`apply_mapping` 命中 `client_model → upstream_model` 时改写 `body["model"]`。
4. **安全审计**：从共享 settings 构造 `AuditPolicy`，生成 `AuditReport`；enforce + critical block candidate 时短路返回 403。
5. **转发**：逐个候选尝试，成功即返回；retryable 失败写日志后换下一个。
6. **记账**：成功时按 usage total 累加配额（best-effort）。
7. **写日志**：每次尝试各写一条 `RequestLog`（成功 / 失败 / 安全阻断）。

### 重试与 failover

- **retryable 失败**：传输错误 / 429 / 5xx → 换下一个候选。
- **4xx 客户端错误**：不重试，原样透传（换 provider 可能掩盖 bug）。
- **尝试上限**：`settings.retry` 控制——`enabled=false` 只试第一个；`max_retries=n` 表示首次后最多再试 `n` 次，且不超过候选数。
- **流式**：打开失败同样重试；打开成功后的**中途失败不跨渠道续接**（下游协议语义不成立，会把两个上游响应拼成一个下游响应）。

### 流式 vs 非流式

- **非流式**：`forward` 返回 `ProviderResponse`（status + body + usage），成功直接返回。
- **流式**：`forward_stream` 返回 SSE 帧流，`wrap_stream_bookkeeping` 包装，在流结束（正常 `None` 或出错 `Err`）时聚合 usage、记账、写日志。中途错误时已下发的帧不受影响，但记账/日志只覆盖已聚合的增量 usage。

### 红线与 best-effort

- **上游密钥不下游暴露**：OpenAI 的 401 错误体会回显提交的 key（`Incorrect API key provided: sk-...`），适配器把上游错误体收敛成通用错误体（`upstream_error_body`），只保留状态码、丢弃原文。
- **记账 / 写日志 best-effort**：上游已经处理了请求后，记账失败不能回 5xx（会导致下游重试 → 双重计费）；失败只 `tracing::warn!`，响应照常返回。

## 基础容器

| 类型 | 位置 | 说明 |
| --- | --- | --- |
| `ProxyRequest` | `usecases/proxy.rs` | 数据面解析出的转发输入：bearer / model / stream / body / trace_id |
| `ProxySuccess` | `usecases/proxy.rs` | `NonStream(ProviderResponse)` 或 `Stream(BoxStream<...>)` |
| `ProxyError` | `usecases/proxy.rs` | 用例错误，变体一对一映射 HTTP 状态码 |
| `ChatRequest` / `ProviderResponse` / `StreamEvent` / `TokenUsage` | `domain/provider.rs` | 适配器边界类型，隔离协议差异 |

## 错误映射（`interface/http/handlers.rs`）

| `ProxyError` 变体 | HTTP |
| --- | --- |
| `Unauthorized` | 401 |
| `QuotaExceeded` | 429 |
| `NoCandidateChannel(_)` | 404 |
| `NoChannelAvailable { .. }` / `Provider(_)` | 502 |
| `Repository(_)` | 500 |
| `InvalidRequest(_)` | 400 |
| `SecurityPolicyBlocked` | 403 |

## 测试

- **Seam A**（主）：`test_support.rs` 的内存 mock 仓储，覆盖 usecase 编排的每一条分支（成功 / 5xx / 传输错 / 4xx 不重试 / 流式打开失败 / 流式中断 / 配额 / 认证）。
- **Seam B**（补充）：`tower::oneshot` + 真实路由 + 内存仓储，验证路由存在性、Bearer 401、配额 429、`/v1/models` 合并去重、**SSE 流式含 `[DONE]` 收尾**——SSE 是唯一必须真实 HTTP 验证的点。
