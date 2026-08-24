# 后端核心设计大纲

> 后端核心设计 / 核心功能的大纲式索引文档：本文件只做概览与导航，详细内容在各子文档中，避免重复。

## 定位

revue-gate 后端是一个本地 LLM API 网关（Tauri v2 + Axum）：对外暴露统一的 OpenAI 兼容端点（`/v1/*`、`/health`），对内聚合多家上游供应商，负责认证、渠道调度、模型映射、转发、配额记账与请求日志。单用户、本地 SQLite。

## 核心设计文档（本目录）

| 文档 | 主题 |
| --- | --- |
| [Domain-context-map.md](./Domain-context-map.md) | 领域层有界上下文划分：各上下文职责、边界与关系 |
| [HTTP_Server&SSE.md](./HTTP_Server&SSE.md) | Axum 数据面入口、路由树、SSE 流式链路与 server 生命周期 |
| [Proxy.md](./Proxy.md) | 代理转发闭环：认证、选路、审计、转发、记账、日志与重试 |
| [Routing.md](./Routing.md) | `ChannelSelector` 调度策略：优先级分组 + 组内权重随机 |
| [API-Keys-Quota.md](./API-Keys-Quota.md) | Virtual Key、配额判定、鉴权链路和安全红线 |
| [Provider-adapter.md](./Provider-adapter.md) | 供应商适配器架构：`ProviderAdaptor` trait 契约、实现与装配、安全红线、测试策略 |
| [Provider-protocols.md](./Provider-protocols.md) | 各供应商协议差异：认证方式 / 请求体结构 / 响应格式 / Token 计数字段映射 |
| [Security-Audit-Engine.md](./Security-Audit-Engine.md) | 安全审计引擎：风险类别、评分、处置策略与落地位置 |
| [Settings.md](./Settings.md) | 应用设置、Tauri Store、托盘行为、自启、主题和运行期生效 |
| [Usage-Stats.md](./Usage-Stats.md) | 基于 `request_logs` 的统计读模型 |

## 核心功能大纲

- **数据面（Data Plane）**：Axum HTTP 服务，`/v1/chat/completions`（流式 + 非流式）、`/v1/models`、`/health`。转发闭环：认证 → 选渠道 → 模型映射 → 转发 → 记账 → 写日志 → 失败按候选渠道顺序重试。
- **控制面（Control Plane）**：Tauri Commands，供前端管理界面调用：渠道 CRUD / 启停 / 连通性测试、密钥 CRUD / 配额、日志查询 / 删除、仪表盘统计、设置读写。
- **分层结构**：`interface → usecases → domain ← infrastructure`，依赖全部向内指向 domain，domain 层零技术依赖；仓储依赖倒置（trait 在 domain，sqlx 实现在 infrastructure）。
- **持久化**：SQLite（sqlx + 内嵌迁移）存业务数据；应用设置存 tauri-plugin-store 的 `settings.json`。
- **协议隔离**：OpenAI / DeepSeek / Custom 走 OpenAI-compatible 直通；Claude / Gemini 做协议转换，转换逻辑锁死在 `infrastructure/providers/`。

## 相关文档

- [后端架构](../../Architecture-backend.md) — 分层、目录结构、依赖方向与关键流程
- [实现规格](../../Spec-implementation.md) — 各阶段实现细节
- [数据库表结构](../../Database.md) — channels / api_keys / request_logs 三张表
- [待办 / 待优化清单](../../Tech-debt.md) — 代码审查与开发中发现的待解决点
