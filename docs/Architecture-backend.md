# 后端架构

> 本文档描述 revue-gate 后端（src-tauri）的架构：分层、目录结构、依赖方向与关键流程。
> 技术选型与功能需求见 `docs/Requirements.md`，版本规划见 `docs/roadmap.md`。

## 架构总览

后端是 Tauri 桌面应用内置的本地 LLM API 网关，分两个面：

- **数据面（Data Plane）**：Axum HTTP 服务，对外暴露 OpenAI 兼容接口（`/v1/*`、`/health`），负责认证、渠道调度、转发、记账与请求日志。
- **控制面（Control Plane）**：Tauri Commands，供前端 React 管理界面调用，负责渠道/密钥/日志/设置等管理操作。

两条面共用同一套 `usecases → domain → repository` 业务核心，数据面与控制面在 `interface` 层汇合。

## 分层结构与依赖方向

```
interface (入口层)
    ├── http/       Axum 数据面：router + handlers
    └── commands/   Tauri 控制面
        │
        ▼
usecases (用例层)   编排 domain + 调仓储接口，组织数据流
        │
        ▼
domain (领域层)     纯业务规则，零技术依赖
        │
        ▼
infrastructure (基础设施层)   实现仓储接口 + 供应商适配器
    ├── sqlite/     sqlx 仓储实现
    └── providers/  供应商适配器实现
```

**依赖规则**：

- `interface → usecases → domain`，`infrastructure → domain`
- 所有依赖向内指向 domain，**不反向**
- domain 层不出现 `sqlx` / `reqwest` / `axum` / `tauri` 任何技术依赖
- 仓储依赖倒置：domain 定义仓储 trait，infrastructure 提供 sqlx 实现，usecases 只依赖 trait（可 mock 测试）

## 目录结构

> 2024 Edition 模块组织：全部使用 `foo.rs` 命名，不使用 `mod.rs`；子模块以 `foo/bar.rs` 引用为 `crate::foo::bar`。

```
src-tauri/src/
├── main.rs                  # 二进制入口
├── lib.rs                   # 库入口：Tauri Builder + 依赖组装 + 启动 HTTP
├── domain/                  # 领域层：纯业务规则，零技术依赖
│   ├── channel.rs           #   Channel 聚合根：实体 + 模型映射/优先级值对象 + ChannelRepository trait
│   ├── api_key.rs           #   ApiKey 聚合根：实体 + 配额值对象 + ApiKeyRepository trait
│   ├── request_log.rs       #   RequestLog 聚合根：实体 + RequestLogRepository trait
│   ├── provider.rs          #   ProviderAdaptor trait（供应商适配器接口）
│   ├── dispatcher.rs        #   领域服务：渠道选择策略（按模型筛选 → 优先级排序）
│   └── quota.rs             #   领域服务：配额策略（QuotaPolicy）
├── usecases/                # 用例层：编排 domain + 调仓储，组织数据流
│   ├── proxy.rs             #   转发用例（认证→选渠道→映射→转发→记账→日志）
│   ├── auth.rs              #   认证用例
│   ├── channel.rs           #   渠道 CRUD/测试用例
│   ├── api_key.rs           #   密钥 CRUD/配额用例
│   ├── log.rs               #   日志查询用例
│   ├── stats.rs             #   统计用例
│   └── settings.rs          #   设置用例
├── infrastructure/          # 基础设施层：技术实现
│   ├── sqlite/              #   sqlx 仓储实现（ChannelRepository 等）
│   └── providers/           #   供应商适配器实现（openai / claude / gemini / deepseek / custom）
├── interface/               # 入口层
│   ├── http/                #   Axum 网关（数据面）：router + handlers
│   └── commands/            #   Tauri 控制面
├── utils/                   # 通用小工具（如 hex 编码；ID 用 uuid crate，时间用 chrono，不自研）
└── migrations/              # sqlx 内嵌迁移
    ├── 001_init.sql         #   建表：channels / api_keys / request_logs / settings 相关
    ├── 002_add_model_mapping.sql
    └── ...
```

## 分层职责

### domain（领域层）

承载全部业务规则，是架构的核心。**一聚合一文件**：领域规模小（3 个聚合根），仓储 trait 随聚合根放置，不拆 model/repository/service 子文件夹。

| 文件 | 内容 |
| --- | --- |
| `channel.rs` | `Channel` 实体（名称/类型/Base URL/API Key/模型列表/优先级/权重/模型映射/启停状态）+ `ModelMapping` 值对象 + `ChannelRepository` trait |
| `api_key.rs` | `ApiKey` 实体（名称/密钥/启停/配额上限/已用额度）+ `Quota` 值对象 + `ApiKeyRepository` trait |
| `request_log.rs` | `RequestLog` 实体（API Key/渠道/模型/usage/耗时/trace id/请求体/状态码）+ `RequestLogRepository` trait |
| `provider.rs` | `ProviderAdaptor` trait：`channel_type` / `default_models` / `default_base_url` / `test` / `forward` / `forward_stream` |
| `dispatcher.rs` | 领域服务 `ChannelSelector`：按模型筛选候选渠道 → 按优先级排序 |
| `quota.rs` | 领域服务 `QuotaPolicy`：校验配额是否超限 |

**领域服务 vs 用例**：`dispatcher.rs`、`quota.rs` 只做纯业务判断（选哪个渠道、是否超配额），不碰 DB 和 HTTP；编排职责在 `usecases`。

**演进触发线**：单文件超过 ~400 行或出现多个独立子类型时，升级为 `channel/{model,repository,service}` 文件夹结构。

### usecases（用例层）

每个文件对应一个业务用例，编排 domain 实体/领域服务 + 调仓储接口：

- `proxy.rs` — 网关核心闭环：认证 → 选渠道 → 模型映射 → 转发 → 记账 → 写日志 → 失败重试
- `auth.rs` — Bearer 密钥认证 + 配额校验
- `channel.rs` — 渠道 CRUD / 启停 / 连通性测试
- `api_key.rs` — 密钥 CRUD / 配额管理
- `log.rs` — 日志分页 / 多条件查询 / 详情 / 删除
- `stats.rs` — 仪表盘统计（卡片指标 + 7 天聚合）
- `settings.rs` — 设置读写

用例不直接触碰技术细节；DB 通过仓储 trait，错误通过 domain/usecases 定义的 thiserror 错误枚举。

### infrastructure（基础设施层）

实现 domain 定义的接口：

- `sqlite/` — 用 sqlx 实现各仓储 trait；`query!` 宏编译期校验 SQL；迁移用 `sqlx::migrate!` 内嵌 `migrations/`
- `providers/` — 实现 `ProviderAdaptor`：OpenAI/DeepSeek/Custom 走 OpenAI-compatible 直通；Claude/Gemini 做协议转换

### interface（入口层）

- `http/` — Axum 数据面：`router.rs` 定义 `/v1/chat/completions`、`/v1/models`、`/health`；`handlers.rs` 解析请求、识别流式、读 Bearer、透传 SSE；挂 `TraceLayer` 结构化日志
- `commands/` — Tauri 控制面：channel / api_key / log / stats / settings 各命令，调用对应 usecases

## 关键流程

### 转发闭环（数据面）

```
HTTP 请求 /v1/chat/completions
  → http/handlers 解析 JSON、识别 stream、提取 Authorization Bearer
  → auth 用例：校验密钥（401）+ 配额（429）
  → proxy 用例：
      dispatcher.ChannelSelector 选候选渠道（启用 → 按模型筛选 → 按优先级排序）
      ProviderAdaptor 应用模型映射 → 转发上游
      解析 usage → 累加配额 → 写 RequestLog
  → 返回上游响应 / SSE 流式透传
  失败 → 按候选渠道顺序重试下一个（≤ 候选渠道数）
```

### 控制面操作

```
React UI → Tauri Command → usecases → repository trait → sqlx → SQLite
```

### 服务生命周期

```
lib.rs 启动：
  初始化 sqlx 连接池 + 内嵌迁移
  初始化 settings（tauri-plugin-store）
  构建 Axum 服务（host/port 来自设置）
  启动 HTTP：emit server-started 事件（前端 Zustand 订阅刷新）
  托盘菜单：启动/停止服务、退出
```

## 技术选型

| 项 | 选型 | 说明 |
| --- | --- | --- |
| Web 框架 | Axum | 数据面 HTTP 服务 |
| HTTP 客户端 | Reqwest | 转发上游请求 |
| 数据库 | SQLite + sqlx | `query!` 编译期校验 + 内嵌迁移 |
| 错误处理 | thiserror + anyhow | domain/usecases 用 thiserror 错误枚举；interface/main 用 anyhow 收尾 |
| 配置存储 | tauri-plugin-store | settings.json 存应用设置；SQLite 只存业务数据 |
| 运行日志 | tracing + tracing-subscriber | 结构化日志；axum TraceLayer 记录请求；span 贯穿 trace id |
| ID | uuid crate v7 | `Uuid::now_v7()` 时间有序主键 |
| 时间 | chrono | `DateTime<Utc>`，审计时间一律 UTC 存储 |

## 命名约定

- **领域服务**（domain）：行为命名，如 `ChannelSelector`、`QuotaPolicy`，不用 `_service` 后缀
- **用例**（usecases）：动词 + `Usecase` 后缀，如 `CreateChannelUsecase`、`ProxyRequestUsecase`；避免 `ChannelService` 这类含糊命名
- **仓储**：`<Aggregate>Repository`，如 `ChannelRepository`
- **适配器**：`<Provider>Adaptor`，如 `OpenAiAdaptor`
- **每个源文件开头必须有注释**，描述该文件的作用
