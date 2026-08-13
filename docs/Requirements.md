# revue-gate 需求文档

## 产品定位

本地运行的 LLM API 网关桌面应用：把多个供应商渠道（OpenAI / Claude / Gemini / DeepSeek / 自定义）聚合在一个统一入口后面，向下游暴露 OpenAI 兼容接口。其他程序（AI 客户端、CLI、OpenAI SDK）只需指向本地网关地址，由网关完成认证、渠道选择、模型映射、转发、记账与日志。

单机、单用户，数据存本地 SQLite，无账号体系。网关**永不暴露上游密钥**，下游只用网关自生成的本地密钥。

功能对齐参考项目 WaLiAPI，架构独立设计。

## 版本规划

| 版本 | 范围 |
| --- | --- |
| v0.1.0 (MVP) | 核心网关：对外接口、渠道管理、密钥管理、请求转发、请求日志、仪表盘、设置中心 |
| v0.2.0 | 安全审计中心 + 仪表盘完整时间粒度（自定义范围、周/月对比） |
| v0.3.0 | 扩展能力：RAG/知识库、MCP、导入导出、本地 AI 工具自动配置、密钥模型/渠道白名单、日志自动保留策略 |

详见 `docs/roadmap.md`。

## v0.1.0 MVP 功能需求

### 对外接口

- `POST /v1/chat/completions` — OpenAI 兼容聊天接口
- `GET /v1/models` — 模型列表（所有启用渠道的模型合并去重，不按密钥过滤；禁用渠道的模型剔除）
- `GET /health` — 服务健康检查
- 完整 SSE 流式转发（`stream: true`），兼容 ChatBox / NextChat / OpenAI SDK 等下游客户端

### 渠道管理

- 渠道 CRUD、启停、调整顺序
- 内置 5 类渠道：OpenAI / DeepSeek / Custom 走 OpenAI-compatible 直通；Claude / Gemini 走协议转换适配器
- 字段：名称、类型、Base URL、API Key、支持模型列表、优先级、权重、模型映射、状态、最近测试时间/结果
- 渠道连通性测试：`GET {base_url}/models` 探测（OpenAI 兼容端点），记录成功/失败、延迟、测试时间；Claude/Gemini 用各自模型列表端点，由 `ProviderAdaptor::test()` 实现
- 模型映射：客户端统一模型名 ↔ 上游实际模型名，未映射时直传

### 密钥管理

- 本地密钥 CRUD、启停
- 密钥格式：`sk-revue-<16 位随机 hex>`（26 字符，8 字节熵）
- HTTP 请求以 `Authorization: Bearer <key>` 认证，失败返回 `401`
- 配额上限：`quota_used >= quota_limit` 时返回 `429`

### 请求转发

- 非流式 + 流式代理
- 流程：认证 → 选择候选渠道（按模型筛选 → 优先级排序）→ 应用模型映射 → 转发上游 → 解析 usage → 写日志 → 累加配额
- 失败重试：按候选渠道顺序尝试下一个（最多不超过候选渠道数），每次失败写日志
- 上游密钥不落库明文暴露给下游

### 请求日志

- 全量记录：API Key、渠道、请求模型、实际上游模型、状态码、prompt/completion/total tokens、耗时、错误消息、是否流式、是否重试、trace id、请求体
- 分页查询 + 按关键词 / 密钥 / 渠道 / 模型 / 日期范围筛选
- 日志详情：对话构成、工具标签、请求参数、网关路由、原始 JSON
- 手动删除：按日期前删除 / 清空全部（自动保留策略留 v0.3.0）

### 仪表盘

- 卡片区：今日请求数、今日 Token、总请求数、总 Token、活跃渠道、密钥数量、平均延迟、渠道可用率
- 图表区：7 天请求数 / Token 趋势折线（按天聚合）
- 完整时间粒度（自定义范围、周/月对比）留 v0.2.0

### 设置中心

- 服务端口 / host 配置（端口 0 = 随机）
- 深色 / 浅色 / 跟随系统主题
- 最小化到托盘、关闭到托盘、开机自启
- 失败重试策略（开关 + 次数）

## 技术栈

### 前端

- React 19 + TypeScript 7 + Vite 7 + Tailwind CSS 4
- UI：shadcn/ui + Lucide Icons + React Router 7
- 状态管理：Zustand
- 服务状态：Tauri 事件 + Zustand 桥接（`server-started` / `server-stopped`）
- 数据通道：Tauri Command（控制面）；Axum HTTP 只管对外转发（数据面）
- 包管理器：pnpm

### 后端

- Rust + Tauri v2 + Axum + SQLite + Reqwest
- 数据库访问：sqlx（`query!` 宏编译期校验 + 内嵌迁移）
- 错误处理：domain / usecases 层 thiserror 定义错误枚举；interface / main 入口 anyhow 收尾
- 配置存储：tauri-plugin-store（settings.json）；SQLite 只存业务数据
- 运行日志：tracing + tracing-subscriber + axum TraceLayer（结构化日志，span 贯穿 trace id）
- ID 生成：uuid crate v7（`Uuid::now_v7()`，时间有序主键）
- 时间：chrono + `DateTime<Utc>`（审计时间一律 UTC 存储）

### 技术选型结论

- 包管理器：**pnpm**（参考项目已用 pnpm 验证，Windows 下更稳）
- TypeScript：**锁定 TS 7.0.x**（2026-08-13 搭建冒烟通过：`pnpm build` + `tsc` 全绿，Vite 7.3.6 + React 19 兼容）；若后续引入 shadcn 时出现不兼容，则降级 TS6.x 并记录原因
- 密钥格式：`sk-revue-<16 位随机 hex>`

## 架构

### 总体形态

```
AI Client / CLI ──HTTP──► Axum 网关（数据面：/v1/* 转发）
React UI ──Tauri Commands──► 控制面（渠道/密钥/日志/设置管理）
Axum / Commands ──► 用例层 ──► 领域层 ──► SQLite
```

- **数据面**：Axum HTTP 服务，对外提供 OpenAI 兼容接口，负责认证、调度、转发
- **控制面**：Tauri Commands，供前端管理界面操作
- 依赖方向：`interface → usecases → domain`，`infrastructure → domain`（实现仓储接口），全部向内指向 domain，不反向

### 后端分层（Clean Architecture + DDD）

```
src-tauri/src/
├── main.rs                  # 二进制入口
├── lib.rs                   # 库入口：Tauri Builder + 依赖组装 + 启动 HTTP
├── domain.rs                #   domain 模块入口（pub mod channel / api_key / ...）
├── domain/                  # 领域层：纯业务规则，零技术依赖（sqlx/reqwest/axum 均不出现）
│   ├── channel.rs           #   Channel 聚合根：实体 + 模型映射/优先级值对象 + ChannelRepository trait
│   ├── api_key.rs           #   ApiKey 聚合根：实体 + 配额值对象 + ApiKeyRepository trait
│   ├── request_log.rs       #   RequestLog 聚合根：实体 + RequestLogRepository trait
│   ├── provider.rs          #   ProviderAdaptor trait（供应商适配器接口）
│   ├── dispatcher.rs        #   领域服务：渠道选择策略（按模型筛选 → 优先级排序）
│   └── quota.rs             #   领域服务：配额策略（QuotaPolicy）
├── usecases.rs              #   usecases 模块入口
├── usecases/                # 用例层：编排 domain + 调仓储，组织数据流
│   ├── proxy.rs             #   转发用例（认证→选渠道→映射→转发→记账→日志）
│   ├── auth.rs              #   认证用例
│   ├── channel.rs           #   渠道 CRUD/测试用例
│   ├── api_key.rs           #   密钥 CRUD/配额用例
│   ├── log.rs               #   日志查询用例
│   ├── stats.rs             #   统计用例
│   └── settings.rs          #   设置用例
├── infrastructure.rs        #   infrastructure 模块入口
├── infrastructure/          # 基础设施层：技术实现
│   ├── sqlite.rs            #   sqlx 连接池 + 内嵌迁移（仓储实现落地后升级为 sqlite.rs + sqlite/）
│   ├── providers.rs         #   providers 模块入口
│   └── providers/           #   供应商适配器实现（openai / claude / gemini / deepseek / custom）
├── interface.rs             #   interface 模块入口（pub mod http / commands）
├── interface/               # 入口层
│   ├── http.rs              #   http 模块入口（pub mod router / server）
│   ├── http/                #   Axum 网关（数据面）
│   │   ├── router.rs        #     路由树 + /health
│   │   ├── server.rs        #     服务生命周期（start/stop + 优雅停机）
│   │   └── handlers.rs      #     /v1/* 请求处理器（阶段 05/08/09）
│   ├── commands.rs          #   commands 模块入口（pub mod server）
│   └── commands/            #   Tauri 控制面
│       ├── server.rs        #     服务生命周期命令 + 状态事件
│       └── ...              #     渠道/密钥/日志/统计命令（阶段 03+）
├── utils.rs                 #   utils 模块入口
└── utils/                   # 通用小工具（如 hex 编码；ID 用 uuid crate，时间用 chrono，不自研）
```

设计要点：

- **一聚合一文件**：领域规模小（3 个聚合根），每个聚合根一个文件，仓储 trait 随聚合根放置，不拆 model/repository/service 子文件夹。单文件超过 ~400 行或出现多个独立子类型时，升级为 `channel/{model,repository,service}` 文件夹
- **2024 Edition 模块组织**：全部使用 `foo.rs` 命名，不使用 `mod.rs`；子模块以 `foo/bar.rs` + `crate::foo::bar` 引用
- **ProviderAdaptor trait 隔离供应商差异**：核心代理只依赖 trait，不关心供应商协议；协议转换限制在 `infrastructure/providers/`
- **领域服务 vs 用例**：domain 里的 dispatcher/quota 只做纯业务判断（选哪个渠道、是否超配额），不碰 DB 和 HTTP；usecases 负责编排

### 前端

- 页面：Dashboard / Channels / API Keys / Logs / Settings
- React Router 7 组织路由，shadcn/ui 组件 + Tailwind CSS 4，Zustand 管理跨页共享状态

## 命名约定

- **领域服务**（domain）：行为命名，如 `ChannelSelector`、`QuotaPolicy`，不用 `_service` 后缀（所在目录已暗示其角色）
- **用例**（usecases）：动词 + `Usecase` 后缀，如 `CreateChannelUsecase`、`ProxyRequestUsecase`；避免 `ChannelService` 这类含糊命名（防止与同名实体撞名）
- **仓储**：`<Aggregate>Repository`，如 `ChannelRepository`
- **适配器**：`<Provider>Adaptor`，如 `OpenAiAdaptor`
- **每个源文件开头必须有注释**，描述该文件的作用
