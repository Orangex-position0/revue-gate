# revue-gate 项目导学

## 1. 前置知识（面试高频标注）

| 知识点 | 为何需要 | 在本项目中的位置 | 高频度 |
| --- | --- | --- | --- |
| API Gateway / Data Plane / Control Plane | 理解真实请求转发链路与桌面管理界面的职责拆分 | `docs/Architecture-backend.md`、`src-tauri/src/interface/http/*`、`src-tauri/src/interface/commands/*` | 高 |
| Rust Clean Architecture | 解释为什么 domain 不依赖 sqlx、reqwest、axum、tauri，以及如何做可测试编排 | `src-tauri/src/domain/*`、`src-tauri/src/usecases/*`、`src-tauri/src/infrastructure/*` | 高 |
| Tauri v2 + React 桌面应用 | 理解前端管理界面如何通过 Tauri Commands 操作本地后端 | `src/app/App.tsx`、`src/lib/api.ts`、`src-tauri/src/interface/commands/*` | 高 |
| Axum HTTP 与 SSE | 理解 `/v1/chat/completions`、`/v1/models`、流式响应如何对外提供 | `src-tauri/src/interface/http/handlers.rs`、`src-tauri/src/interface/http/router.rs` | 高 |
| Provider Adapter | 理解 OpenAI-compatible、Claude、Gemini、DeepSeek 等上游差异如何被隔离 | `src-tauri/src/domain/provider.rs`、`src-tauri/src/infrastructure/providers/*` | 高 |
| 鉴权与配额 | 理解本地 API Key、Bearer 认证、quota limit 和 usage 累计如何闭环 | `src-tauri/src/domain/api_key.rs`、`src-tauri/src/domain/quota.rs`、`src-tauri/src/usecases/auth.rs` | 高 |
| 渠道调度与失败重试 | 理解模型筛选、优先级、权重随机、失败切换如何组成候选队列 | `src-tauri/src/domain/dispatcher.rs`、`src-tauri/src/usecases/proxy.rs` | 高 |
| SQLite 本地持久化 | 理解渠道、密钥、日志、知识库状态如何存储和迁移 | `src-tauri/migrations/*`、`src-tauri/src/infrastructure/sqlite/*` | 中 |
| 安全审计与脱敏 | 理解请求转发前的风险扫描、阻断、日志脱敏和证据保存 | `src-tauri/src/domain/security_audit.rs`、`docs/design/backend/Security-Audit-Engine.md` | 中高 |
| 多协议转换 | 理解 Anthropic Messages / OpenAI Responses 如何转换为内部 canonical chat | `src-tauri/src/protocol/*`、`docs/adr/0003-multi-protocol-conversion-engine.md` | 中 |
| RAG / Knowledge Service | 理解知识库作为本地服务模块如何复用网关代理与治理链路 | `src-tauri/src/usecases/knowledge*`、`docs/design/backend/Knowledge.md` | 中 |

## 2. 重点亮点与学习顺序（先看这个）

| 亮点标题 | 为什么重要 | 通用技术关键词 | 先看哪些文件 | 建议学习顺序 |
| --- | --- | --- | --- | --- |
| 数据面与控制面分离 | 真实 LLM 请求走 Axum HTTP，配置管理走 Tauri Commands，边界清楚，适合解释桌面网关架构 | API Gateway、Control Plane、Data Plane、本地 IPC | `docs/Architecture-backend.md`、`docs/Architecture-frontend.md`、`src-tauri/src/interface/http/handlers.rs`、`src/lib/api.ts` | 1 |
| 请求治理闭环 | 一次请求串起认证、配额、调度、审计、转发、记账、日志，是项目最核心链路 | 认证、quota、路由、审计、日志、usage billing | `src-tauri/src/usecases/proxy.rs`、`src-tauri/src/usecases/auth.rs`、`src-tauri/src/domain/quota.rs` | 2 |
| 可扩展渠道调度 | 渠道选择从业务规则抽到 domain service，支持模型匹配、优先级、权重和失败队列 | 策略模式、领域服务、加权随机、failover | `src-tauri/src/domain/dispatcher.rs`、`src-tauri/src/domain/channel.rs`、`src-tauri/src/usecases/channel.rs` | 3 |
| Provider 协议隔离 | 上游差异被压到 adapter，核心代理只依赖统一 trait，方便接入新 provider | Adapter、trait、协议转换、边界隔离 | `src-tauri/src/domain/provider.rs`、`src-tauri/src/infrastructure/providers.rs`、`src-tauri/src/infrastructure/providers/openai.rs`、`claude.rs`、`gemini.rs` | 4 |
| 安全审计与可观测日志 | 请求在转发前做风险扫描，日志保留 trace、usage、审计报告和脱敏请求体，能支撑排障和安全复核 | 安全扫描、脱敏、trace id、结构化日志 | `docs/design/backend/Security-Audit-Engine.md`、`src-tauri/src/domain/security_audit.rs`、`src-tauri/src/usecases/log.rs`、`src/pages/logs/LogsPage.tsx` | 5 |
| 本地知识库服务模块 | Knowledge Service 把文档解析、分块、FTS、向量检索、RAG 生成放进同一进程，并复用网关治理 | RAG、FTS5、向量检索、RRF、Service Module | `docs/design/backend/Knowledge.md`、`src-tauri/src/usecases/knowledge.rs`、`src-tauri/src/usecases/knowledge/rag.rs` | 6 |

## 3. 必备知识点

- [ ] 能说清 revue-gate 是“本地单用户 LLM API Gateway 桌面应用”，不是云端多租户服务。
- [ ] 能画出请求链路：HTTP handler -> protocol conversion -> proxy usecase -> provider adapter -> request log。
- [ ] 能区分 Local API Key 与上游 Channel API Key。
- [ ] 能解释 quota 的判断边界：`used >= limit` 返回 429，usage 在上游成功或流式结束后累计。
- [ ] 能解释渠道选择：启用状态、模型支持、优先级、同优先级按权重随机、失败时沿候选队列重试。
- [ ] 能解释为什么 provider adapter 不能和 downstream protocol conversion 混在一起。
- [ ] 能解释 SSE 的边界：打开流之前可重试，流开始后中途失败无法再改 HTTP status，只能记录并结束。
- [ ] 能解释 request log 记录哪些信息：模型、上游模型、渠道、状态码、token、耗时、trace id、审计报告。
- [ ] 能解释前端为什么不直接访问 `/v1/*`，而是通过 Tauri Command 管理控制面。
- [ ] 能解释知识库模块为什么复用 Gateway proxy，而不是维护第二套模型凭据和路由。

## 4. 推荐阅读（结合仓库）

| 主题 | 通用技术点 | 建议阅读位置 | 预计时间 | 读完能回答什么 |
| --- | --- | --- | --- | --- |
| 产品定位与功能范围 | 本地网关、MVP 范围、单机单用户边界 | `README.zh-CN.md`、`docs/Requirements.md` | 20 分钟 | 这个项目解决什么问题，哪些能力已经在 MVP 内 |
| 后端总体架构 | Clean Architecture、DDD 轻量建模、Data Plane / Control Plane | `docs/Architecture-backend.md` | 30 分钟 | 为什么分 domain/usecases/interface/infrastructure |
| 前端总体架构 | Tauri Command 客户端、页面切片、轻量全局状态 | `docs/Architecture-frontend.md`、`src/lib/api.ts` | 25 分钟 | React 管理界面如何和后端控制面协作 |
| HTTP 数据面入口 | Axum handler、trace id、协议转换、错误映射、SSE response | `src-tauri/src/interface/http/handlers.rs`、`src-tauri/src/interface/http/router.rs` | 45 分钟 | 一个客户端请求如何进入网关并变成统一代理请求 |
| 代理编排主链路 | 认证、渠道选择、审计、重试、记账、日志 | `src-tauri/src/usecases/proxy.rs` | 60 分钟 | 项目最核心的请求闭环如何工作 |
| 鉴权与配额 | Bearer token、Local API Key、quota policy | `src-tauri/src/usecases/auth.rs`、`src-tauri/src/domain/api_key.rs`、`src-tauri/src/domain/quota.rs` | 30 分钟 | 为什么无效 key 是 401，超配额是 429 |
| 渠道与模型映射 | Channel entity、model mappings、priority/weight | `src-tauri/src/domain/channel.rs`、`src-tauri/src/domain/dispatcher.rs`、`src-tauri/src/usecases/channel.rs` | 40 分钟 | 请求如何选择上游渠道，如何重写模型名 |
| Provider 适配 | trait boundary、OpenAI-compatible passthrough、Claude/Gemini conversion | `src-tauri/src/domain/provider.rs`、`src-tauri/src/infrastructure/providers/*` | 50 分钟 | 新增一个 provider 要动哪些边界 |
| 请求日志与统计 | 日志持久化、分页筛选、dashboard 聚合、usage 排行 | `src-tauri/src/domain/request_log.rs`、`src-tauri/src/usecases/log.rs`、`src-tauri/src/usecases/stats.rs`、`src-tauri/src/domain/stats.rs` | 45 分钟 | 如何从日志构建可观测闭环 |
| 安全审计 | scope builder、detector、aggregator、阻断、脱敏 | `docs/design/backend/Security-Audit-Engine.md`、`src-tauri/src/domain/security_audit.rs` | 60 分钟 | 为什么审计发生在转发前，怎样避免日志二次泄露 |
| 多协议转换 | Canonical Chat Protocol、CodecRegistry、转换报告 | `docs/adr/0003-multi-protocol-conversion-engine.md`、`src-tauri/src/protocol/*` | 45 分钟 | 为什么 downstream protocol conversion 独立于 provider adapter |
| Knowledge Service | 文档解析、chunk、FTS5、向量检索、RAG、Service Module | `docs/design/backend/Knowledge.md`、`src-tauri/src/usecases/knowledge.rs`、`src-tauri/src/domain/knowledge.rs` | 60 分钟 | 知识库如何复用网关治理链路 |
| 前端页面实现 | 页面状态、表单、列表、日志详情、设置 | `src/pages/channels/*`、`src/pages/api-keys/*`、`src/pages/logs/LogsPage.tsx`、`src/pages/settings/SettingsPage.tsx` | 50 分钟 | 控制面管理任务在 UI 上如何落地 |
| 数据库迁移 | SQLite schema、request logs、knowledge tables | `src-tauri/migrations/*`、`docs/Database.md` | 35 分钟 | 本地状态如何持久化，哪些表是业务核心 |

## 5. 自学提醒

若某文件或原理看不懂，请继续追问 AI；本技能负责给学习路径与题目，不提供逐行讲解。建议每读完一组文件，就用自己的话复述“输入是什么、输出是什么、失败边界是什么、日志在哪里”，不要只背函数名。

## 6. 项目技术定位

这是一个 **后端 / AI 工程 / 桌面客户端交叉项目**。依据是：核心价值在 Rust 后端网关的请求治理、协议适配、渠道调度和本地可观测；React + Tauri 前端承担控制面管理；Knowledge Service 又把 RAG 检索与模型调用纳入同一治理链路。

## 7. 核心原理解析

### 7.1 数据面与控制面分离

**问题 ->** 如果桌面 UI、配置管理和真实 LLM 请求混在一条链路里，权限、错误处理和性能边界会变得不清楚。

**机制 ->** 项目把 Axum HTTP 服务作为 Data Plane，对外暴露 `/v1/*` 与 `/health`；把 React + Tauri Commands 作为 Control Plane，负责渠道、密钥、日志、统计、设置和服务生命周期管理。

**在本项目中的落点 ->** Data Plane 在 `src-tauri/src/interface/http/*`，Control Plane 在 `src-tauri/src/interface/commands/*` 和 `src/lib/api.ts`。两者共用 `usecases -> domain -> repository`，避免重复业务规则。

### 7.2 请求治理闭环

**问题 ->** 一个本地 LLM 网关不能只做 HTTP 转发，还要回答“谁在调用、能不能调用、走哪个渠道、出了什么错、消耗了多少 token”。

**机制 ->** `ProxyRequestUsecase` 统一编排认证、quota 检查、渠道选择、安全审计、模型映射、provider 转发、usage 累计和 request log 写入。非流式请求在响应后记录，流式请求在 stream wrapper 结束时聚合 usage 并写日志。

**在本项目中的落点 ->** `src-tauri/src/usecases/proxy.rs` 是主链路，`src-tauri/src/usecases/auth.rs` 负责认证，`src-tauri/src/domain/quota.rs` 定义配额边界，`src-tauri/src/domain/request_log.rs` 定义日志实体。

### 7.3 渠道选择与失败切换

**问题 ->** 多个上游渠道同时存在时，不能把请求随意转发；需要按模型能力、启用状态、优先级和权重生成一个可解释的候选队列。

**机制 ->** `ChannelSelector` 先过滤禁用渠道，再判断模型列表或模型映射是否支持请求模型，然后按 priority 升序分组，同优先级组内按 weight 做无放回随机，输出一次请求内稳定的 failover 队列。

**在本项目中的落点 ->** 纯选择规则在 `src-tauri/src/domain/dispatcher.rs`，代理编排在 `src-tauri/src/usecases/proxy.rs` 按候选队列执行，失败时只对 429 和 5xx 这类可重试状态走下一个候选。

### 7.4 Provider 适配与协议边界

**问题 ->** OpenAI-compatible、Claude、Gemini 等 provider 的请求格式、响应格式、usage 字段和流式语法不同，如果核心代理直接处理差异，会不断膨胀。

**机制 ->** domain 层只定义 `ProviderAdaptor` trait 和统一的 `ChatRequest`、`ProviderResponse`、`StreamEvent`、`TokenUsage`。OpenAI / DeepSeek / Custom 复用 OpenAI-compatible passthrough，Claude / Gemini 在 adapter 内做协议转换。

**在本项目中的落点 ->** trait 在 `src-tauri/src/domain/provider.rs`，适配器入口在 `src-tauri/src/infrastructure/providers.rs`，具体实现位于 `src-tauri/src/infrastructure/providers/openai.rs`、`claude.rs`、`gemini.rs`。

### 7.5 安全审计与日志脱敏

**问题 ->** AI 请求可能携带 API key、私钥、数据库连接串、敏感路径或风险动作；如果直接转发并完整写日志，可能造成二次泄露。

**机制 ->** 安全审计先从 canonical request body 构建 scan scope，再由确定性 detector 生成 findings，聚合为 request-level report。Enforce 模式下 critical finding 可以阻断；无论是否阻断，存储 payload 时都会做 redaction，转发体本身不被改写。

**在本项目中的落点 ->** 审计模型和规则在 `src-tauri/src/domain/security_audit.rs`，设计说明在 `docs/design/backend/Security-Audit-Engine.md`，代理调用点在 `src-tauri/src/usecases/proxy.rs`，日志展示在 `src/pages/logs/LogsPage.tsx`。

### 7.6 Knowledge Service 复用网关治理

**问题 ->** 如果知识库模块单独维护模型 key、路由和日志，会形成第二套网关，带来配置重复和可观测断裂。

**机制 ->** Knowledge Service 负责文档生命周期、解析、分块、FTS、embedding、hybrid retrieval、citation 和 RAG 上下文；模型生成仍复用 Gateway proxy，从而继承本地 key、quota、request log 和 security audit。

**在本项目中的落点 ->** 设计在 `docs/design/backend/Knowledge.md`，用例在 `src-tauri/src/usecases/knowledge.rs` 与子模块，领域模型在 `src-tauri/src/domain/knowledge.rs`，HTTP service module 在 `src-tauri/src/interface/http/knowledge.rs`。

## 8. 关键设计决策

| 决策 | 备选 | 取舍 | 风险 | 验证 |
| --- | --- | --- | --- | --- |
| 使用本地桌面应用承载控制面 | 做成 Web 管理后台或纯 CLI | Tauri 适合单机单用户、本地私有数据和托盘生命周期管理 | 桌面环境依赖 Rust/Tauri/WebView，分发复杂度高于纯服务 | `pnpm tauri dev`、`pnpm tauri info`、手动演示 `docs/demo-guide.md` |
| Data Plane / Control Plane 分离 | 前端直接操作 HTTP 数据面 | 数据面保持 OpenAI-compatible；控制面通过 IPC 管理本地配置 | 两套入口都要维护错误语义和状态同步 | 读 `interface/http/*` 与 `interface/commands/*`，运行 `/health` 和 UI 设置页 |
| Clean Architecture + 轻量 DDD | 在 handler 中直接写 SQL 和 reqwest | domain 规则可单测，usecase 可注入 mock 仓储和 mock provider | 层数增加，简单 CRUD 文件数量较多 | `cargo test` 中 domain/usecase/infrastructure 测试；阅读 `test_support.rs` |
| ProviderAdaptor trait 隔离供应商差异 | 在 proxy usecase 中按 provider 类型分支 | 新 provider 可放进 infrastructure adapter，不污染核心代理 | trait 需要持续抽象公共能力；复杂 provider 特性可能难以统一 | provider adapter 单测、连通性测试、实际 `/v1/chat/completions` 请求 |
| Downstream protocol conversion 独立于 provider adapter | 把 Anthropic/Responses 转换塞进 provider adapter | 客户端协议与上游 provider 协议边界清楚，避免“协议转换”大泥球 | 初期只支持受控子集，部分官方能力会 400 | `docs/adr/0003-multi-protocol-conversion-engine.md`、`src-tauri/tests/protocol_conversion.rs` |
| 安全审计使用内置确定性规则 | 引入 LLM 判别或第三方 secret scanner | 可预测、离线、本地、易测试，适合 MVP | 规则覆盖有限，误报/漏报需要持续迭代 | 安全审计单测、日志详情 UI、`security_policy_blocked` 路径 |
| 统计聚合在 Rust 内存层完成 | SQL group by + timezone 函数 | sqlx 与 in-memory repo 共用聚合逻辑，测试一致性强 | 日志量增大时全量轻量扫描可能需要优化 | `src-tauri/src/usecases/stats.rs` 单测、usage 页面数据 |
| Knowledge Service 与 Gateway 同进程 | 独立 RAG 服务 | 复用渠道、key、quota、日志和审计，不引入第二套运维面 | 进程内模块复杂度增加，长任务调度需谨慎演进 | `docs/design/backend/Knowledge.md`、知识库上传/检索/RAG 接口测试 |

## 9. 量化与验证（含待测，建议）

| 验证目标 | 建议方法 | 当前状态 |
| --- | --- | --- |
| 前端构建可用 | 执行 `pnpm build`，确认 TypeScript + Vite 通过 | 已在本地多次通过；作为项目健康检查可继续保留 |
| 后端核心链路可测 | 执行 `cargo test --manifest-path src-tauri/Cargo.toml` 或按模块跑 `cargo test proxy` | 待按完整 CI 口径复测 |
| 本地服务可启动 | 执行 `pnpm tauri dev`，在 UI 保存设置并启动服务，再 `curl /health` | 待测；依赖本机 Tauri/Rust 环境 |
| 认证语义正确 | 用缺失/错误 Local API Key 请求 `/v1/models`，预期 401 | 待测 |
| 配额语义正确 | 创建 quota=100 的 key，完成一次请求后再次请求，预期超限时 429 | 待测；演示步骤已写入 `docs/demo-guide.md` |
| 渠道路由正确 | 配置支持同一模型的多个渠道，观察 priority、weight、retry 日志 | 待测；可通过 request log 的 channel / is_retry 验证 |
| 上游 key 不泄露 | 故意配置错误上游 key，确认下游错误体不回显真实 key | 待测；关键逻辑在 `src-tauri/src/infrastructure/providers.rs` |
| SSE 记账与日志 | 发起流式请求，结束后查看 request log 的 token、status、response choices | 待测 |
| 安全审计阻断 | 打开 enforce + block critical，构造包含明显 secret 的请求，预期 403 和日志报告 | 待测 |
| Dashboard / Usage 一致性 | 发起多次请求后对比日志列表、dashboard、usage 页面 | 待测 |
| Knowledge 检索可用 | 上传小型 Markdown，重建索引，执行 keyword / hybrid / RAG 查询 | 待测 |

