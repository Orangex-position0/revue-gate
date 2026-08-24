# revue-gate v0.1.0 实现规格（Spec）

> 本文档把已敲定的架构决策落成可执行规格：实现顺序、每个阶段的产出与验证点、测试 seam、已锁定不返工的决策。
> 前置文档：需求见 `Requirements.md`，版本规划见 `roadmap.md`，架构见 `Architecture-backend.md` 与 `Architecture-frontend.md`。
> 本仓库无 issue tracker，规格直接落 `docs/`；语义上标记 `ready-for-agent`，实现 agent 可无歧义照此推进。

## Problem Statement

revue-gate 的设计文档已齐备（需求、路线图、前后端架构），但代码从零开始。缺少一份连接"架构"与"代码"的规格：

- 不知道 v0.1.0 按什么顺序实现，先写什么、后写什么
- 不知道每个阶段用什么验证，TDD 的靶位在哪里
- 不知道哪些决策已经锁定、实现时不许返工

没有这份规格，实现时会重新决策、验证点漂移、顺序混乱——这正是本次要消除的。

## Solution

把 v0.1.0 MVP 拆成 7 个有序实现阶段，每阶段有明确产出、可运行验证点、按 TDD 推进；锁定 **2 个测试 seam**（usecases mock 编排测试为主 + http oneshot 集成测试补充）；固化全部已定技术决策，实现时照抄不返工。domain 层纯逻辑经由 usecases 测试覆盖，不单独建 seam。

## User Stories

### 网关接口

1. 作为 AI 客户端（ChatBox / NextChat / OpenAI SDK / CLI），我想调用 `POST /v1/chat/completions`，以便对话请求被转发到任意已配置的上游渠道。
2. 作为 AI 客户端，我想在请求中带 `stream: true`，以便实时收到 SSE 流式增量响应。
3. 作为 AI 客户端，我想 `GET /v1/models` 返回所有启用渠道的模型（合并去重），以便选择可用模型。
4. 作为网关管理员，我想 `GET /health` 检查服务健康，以便确认网关在线。

### 认证与安全

5. 作为下游程序，我想用 `sk-revue-*` 本地密钥通过 Bearer 认证，以便访问网关而不接触上游密钥。
6. 作为网关管理员，我想让无有效密钥的请求返回 `401`，以便只放行授权客户端。
7. 作为网关管理员，我想让超过配额上限的密钥返回 `429`，以便控制成本。
8. 作为网关管理员，我想网关永远不把上游密钥暴露给下游，以便上游凭据安全。

### 渠道管理

9. 作为网关管理员，我想创建/编辑 OpenAI、DeepSeek、Custom 渠道，以便直通 OpenAI-compatible 端点。
10. 作为网关管理员，我想创建 Claude、Gemini 渠道，以便协议转换适配器自动完成协议转换。
11. 作为网关管理员，我想启停渠道，以便维护时下线指定渠道。
12. 作为网关管理员，我想测试渠道连通性（记录成功/失败、延迟、测试时间），以便快速发现失效渠道。
13. 作为网关管理员，我想配置模型映射（客户端统一模型名 ↔ 上游实际模型名），以便统一下游接口面，未映射时直传。
14. 作为网关管理员，我想配置优先级与权重，以便控制渠道调度顺序。
15. 作为网关管理员，我想删除渠道，以便清理不再使用的配置。

### 请求转发

16. 作为网关管理员，我想网关按"启用 → 按模型筛选 → 按优先级排序"选择候选渠道，以便请求落到合适的上游。
17. 作为网关管理员，我想转发失败时按候选渠道顺序重试下一个（不超过候选渠道数），以便提高成功率，且每次失败写日志。
18. 作为网关管理员，我想非流式与流式都正确代理，以便各类客户端可用。
19. 作为网关管理员，我想每次请求解析 usage 并累加密钥配额，以便掌握消耗。

### 请求日志

20. 作为网关管理员，我想每次请求全量记日志（密钥、渠道、请求/实际上游模型、状态码、usage、耗时、错误消息、是否流式、是否重试、trace id、请求体），以便审计与排障。
21. 作为网关管理员，我想分页查询并按关键词 / 密钥 / 渠道 / 模型 / 日期范围筛选，以便快速定位。
22. 作为网关管理员，我想查看日志详情（对话构成、工具标签、请求参数、网关路由、原始 JSON），以便复现问题。
23. 作为网关管理员，我想按日期前删除 / 清空日志，以便控制存储（自动保留策略留 v0.3.0）。

### 仪表盘

24. 作为网关管理员，我想看今日请求数、今日 Token、总请求数、总 Token，以便掌握用量。
25. 作为网关管理员，我想看平均延迟与渠道可用率，以便评估服务质量。
26. 作为网关管理员，我想看 7 天请求数 / Token 趋势折线（按天聚合），以便发现增长与异常。

### 设置中心

27. 作为网关管理员，我想配置服务端口与 host（端口 0 = 随机），以便适应本机网络环境。
28. 作为网关管理员，我想切换浅色 / 深色 / 跟随系统主题，以便界面舒适。
29. 作为网关管理员，我想最小化到托盘 / 关闭到托盘，以便常驻后台。
30. 作为网关管理员，我想配置开机自启，以便免手动启动。
31. 作为网关管理员，我想配置失败重试开关与次数，以便平衡成功率与延迟。

## Implementation Decisions

已锁定决策，实现时照此执行，不返工：

1. **技术栈**（详见 Requirements）：后端 Rust + Tauri v2 + Axum + SQLite(sqlx) + Reqwest + thiserror/anyhow + tracing + uuid v7 + chrono；前端 React 19 + TS + Vite 7 + Tailwind CSS 4 + shadcn/ui + Lucide + React Router 7 + Zustand + pnpm。
2. **分层与依赖方向**：`interface → usecases → domain`，`infrastructure → domain`，全部向内指向 domain，不反向。domain 零技术依赖（无 sqlx / reqwest / axum / tauri）。仓储依赖倒置：trait 定义在 domain，sqlx 实现在 infrastructure，usecases 只依赖 trait。
3. **模块形态**：一聚合一文件——Channel、ApiKey、RequestLog 三个聚合根，各自携带 Repository trait；值对象 ModelMapping、Quota 随聚合根；领域服务 ChannelSelector（选渠道）、QuotaPolicy（配额校验）只做纯业务判断；ProviderAdaptor trait 隔离供应商协议差异。单文件超过 ~400 行升级为文件夹结构（演进触发线）。
4. **数据面契约**：`POST /v1/chat/completions`、`GET /v1/models`（启用渠道模型合并去重，不按密钥过滤，禁用渠道剔除）、`GET /health`；Bearer 认证；SSE 流式透传。认证失败 `401`，配额超限 `429`。
5. **调度与重试**：按模型筛选候选 → 优先级排序 → 应用模型映射 → 转发 → 解析 Token Usage → 记账 → 写日志；失败按候选渠道顺序重试（≤ 候选渠道数）。
6. **供应商适配器**：OpenAI / DeepSeek / Custom 走 OpenAI-compatible 直通；Claude / Gemini 协议转换，转换逻辑限定在 infrastructure 的适配器内。`test()` 用各自模型列表端点，`fetch_models()` 用于用户手动获取 provider-reported 模型列表，不支持时显式返回 unsupported。
7. **密钥格式**：`sk-revue-<16 位随机 hex>`（25 字符，8 字节熵）；上游密钥永不落库明文暴露给下游。
8. **ID 与时间**：uuid v7（时间有序主键）；chrono `DateTime<Utc>`，审计时间一律 UTC 存储。
9. **错误处理**：domain / usecases 用 thiserror 错误枚举；interface / main 用 anyhow 收尾。
10. **存储**：SQLite 只存业务数据，`sqlx::migrate!` 内嵌迁移；应用设置存 tauri-plugin-store 的 settings.json。
11. **前端形态**（详见 Architecture-frontend）：MemoryRouter + 集中 Routes + Layout 薄壳；唯一全局 store `useServerStore` 只由 server 事件写入，事件是唯一权威源，前端不自改；业务数据页面本地 `useState` + `load()`，不引 TanStack Query；`lib/api.ts` 按域封装 `invoke`；主题三态 `data-theme` + CSS 变量，`system` 由 `@media (prefers-color-scheme: dark)` 响应；Tailwind 4 `@custom-variant dark`。
12. **TS7 冒烟锁定**：搭建时跑 `pnpm dev` + `pnpm build` + `tsc`；若 TS7 与 Vite 7 / shadcn 不兼容，锁定 TS6.x。

### 实施顺序（7 阶段，骨架先于血肉）

| 阶段 | 内容 | 产出 | 验证点 |
| --- | --- | --- | --- |
| 1 | 项目搭建 + 冒烟 | Tauri + React 骨架，TS 锁定 | `pnpm dev` / `pnpm build` / `tsc` 全绿 |
| 2 | domain 层 | 3 个聚合根 + 值对象 + Repository trait + ProviderAdaptor trait + 领域服务 | 编译通过，纯逻辑单测（经 usecases 触发） |
| 3 | infrastructure：sqlite + migrations | 各仓储实现 + 内嵌迁移 | `sqlx query!` 编译期校验通过 |
| 4 | usecases 编排 | proxy / auth / channel / api_key / log / stats / settings 用例 + 错误枚举成型；channel 用例含手动获取模型列表 | **seam A**：mock trait 编排测试全绿 |
| 5 | interface：http 数据面 | `/v1/*` 路由 + handlers + SSE 透传 + TraceLayer | **seam B**：oneshot 集成测试（含 SSE）全绿 |
| 6 | interface：tauri 控制面 | 各域命令封装 | 命令调用冒烟通过 |
| 7 | 前端骨架 + 各页面 | Dashboard / Channels / API Keys / Logs / Settings | `pnpm build` + `tsc` 全绿，页面冒烟 |

## Testing Decisions

- **原则**：只测外部行为（公开接口的输入 → 输出），不测实现细节、不 mock 业务本身、不 mock 被测对象内部协作之外的技术细节。
- **Seam A — usecases 编排测试（主）**：对 Repository trait 与 ProviderAdaptor trait 提供内存 mock 实现。覆盖：proxy 全闭环（认证 → 选渠道 → 映射 → 转发 → 记账 → 日志 → 重试的每一条分支）、auth（401 / 429）、channel / api_key CRUD、模型发现与配额、log 分页与筛选、stats 聚合。domain 纯逻辑（ChannelSelector / QuotaPolicy）经由这些用例被覆盖。
- **Seam B — http oneshot 集成测试（补充）**：`tower::ServiceExt::oneshot` + 内存 SQLite + 真实仓储实现。验证：路由存在性、Bearer 认证 `401`、配额 `429`、`/v1/models` 合并去重、**SSE 流式透传含 `[DONE]` 收尾**——SSE 是唯一必须真实 HTTP 验证的点，流式中断与收尾只有该 seam 能可靠覆盖。
- **TDD 流程**：每阶段 Red-Green-Refactor 循环，先写失败测试定义需求，再最小实现，最后重构；阶段切换时向用户声明当前阶段。
- **前端验证**：`pnpm dev` + `pnpm build` + `tsc` 冒烟 + 页面手工冒烟；不引入前端单测框架，页面逻辑保持薄壳，核心在 Command 封装与后端。
- **现有同类测试先例**：无（本仓库从零开始，seam A / B 即项目测试基线，后续版本沿同一 seam 扩展）。

## Out of Scope

- v0.2.0 安全审计中心：请求扫描、风险等级、处理策略（只审计/警告/脱敏/阻断）、规则与黑白名单管理、安全配置面板（见 roadmap）
- v0.2.0 仪表盘完整时间粒度：自定义范围、周 / 月对比
- v0.3.0 扩展能力：RAG / 知识库、MCP、导入导出、本地 AI 工具自动配置、密钥模型/渠道白名单、日志自动保留策略
- 高可用增强：健康探测、熔断、加权负载均衡、按天/成本统计
- 账号体系、多用户、远程部署
- 打包签名、自动更新、CI 流水线

## Further Notes

- 本仓库非 git 仓库、无 issue tracker，规格直接落 `docs/`；语义标记 `ready-for-agent`。
- TS7 冒烟失败时锁定 TS6.x，这是唯一可能触发技术栈调整的已知风险。
- struct / trait 的具体字段与错误枚举分支**不预先设计**，随 TDD 涌现（见架构文档"演进触发线"）——需求文档已枚举全部字段，实现是照抄，不是决策。
- 上游密钥安全是红线：任何路径不得将上游密钥下发给下游或写入请求日志明文。
