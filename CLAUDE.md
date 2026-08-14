# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# 项目概述

revue-gate 是一个本地 LLM API 网关桌面应用（Tauri v2）。它把多个上游供应商（OpenAI、Claude、Gemini、DeepSeek、自定义 OpenAI-compatible 端点）聚合到一个统一的 OpenAI 兼容端点 `http://127.0.0.1:3000`（默认），并负责认证、渠道调度、模型映射、转发、配额记账与请求日志。单用户、本地 SQLite、无账号体系。需求/路线图/规格见 `docs/`（Requirements、Architecture-backend、Architecture-frontend、Spec-implementation、roadmap）。

## 常用命令

前端在仓库根（pnpm）；Rust crate 在 `src-tauri/`，所有 cargo 命令需带 `--manifest-path src-tauri/Cargo.toml`。

```bash
pnpm install                     # 安装前端依赖
pnpm tauri dev                   # 启动桌面应用（数据面随应用启动；Vite dev 端口 1420）

# 前端校验（前端无单测框架，验证靠这三样 + 手工冒烟）
pnpm build                       # tsc + vite build
pnpm exec tsc                    # 仅 TS 类型检查

# 后端校验
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml --all-targets --all-features
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo nextest run --manifest-path src-tauri/Cargo.toml      # 项目测试跑器是 nextest
```

- 跑单个测试：`cargo nextest run --manifest-path src-tauri/Cargo.toml <test_name>`（标准 `cargo test` 同参也可用）。
- 测试用例内联在源文件 `#[cfg(test)] mod tests`（无独立 `tests/` 目录）；`src-tauri/src/test_support.rs` 提供 seam A 的内存 mock 仓储。
- git hooks 由 `prek` 托管（配置在 `prek.toml`，安装：`prek install`）：pre-commit = fmt/check/tsc，pre-push = clippy `-D warnings` + nextest + `pnpm build`。改动后可 `prek run` 手动触发全套。
- `Cargo.toml [lints.clippy]`：`todo` / `unimplemented` 为 `deny`，禁止占位提交。

## 架构

双面结构，共用同一 `usecases → domain ← infrastructure` 业务核心：

- **数据面（Data Plane）**：`src-tauri/src/interface/http/`，Axum 服务，对外暴露 `/v1/chat/completions`（流式 + 非流式）、`/v1/models`、`/health`。负责 Bearer 认证、转发、SSE 透传、trace id 贯穿（`x-request-id`）。
- **控制面（Control Plane）**：`src-tauri/src/interface/commands/`，Tauri Commands，React 前端经 `invoke` 调用。**前端不直连数据面 HTTP**，两条面在 `interface` 层汇合。

依赖方向全部向内指向 domain、不反向；**domain 层零技术依赖**（不出现 sqlx / reqwest / axum / tauri）。仓储依赖倒置：trait 定义在 domain，sqlx 实现在 `infrastructure/sqlite/`。供应商协议差异隔离在 `infrastructure/providers/`（OpenAI/DeepSeek/Custom 走 OpenAI-compatible 直通，Claude/Gemini 做协议转换）。

### 关键文件

- `src-tauri/src/lib.rs` — Tauri Builder + 依赖装配（SQLite 池、共享设置、HTTP 启动）+ 托盘/窗口事件。setup 里的装配顺序就是运行时依赖关系。
- `src-tauri/src/usecases/proxy.rs` — 网关核心闭环：认证 → 选渠道 → 模型映射 → 转发 → 记账 → 写日志 → 失败按候选渠道顺序重试。
- `src-tauri/src/domain/dispatcher.rs`（`ChannelSelector`）、`quota.rs`（`QuotaPolicy`）— 纯业务领域服务，只做判断不碰 DB/HTTP；编排在 usecases。
- `src-tauri/migrations/001_init.sql` — sqlx 内嵌迁移（`sqlx::migrate!`），新表改动必须加新迁移文件。
- `src/lib/api.ts` — 全项目唯一 `invoke` 调用点，按域封装（`channelApi` / `logApi` / …）；后端加命令只改这一个文件 + `src/types/`。

### 前端要点

- React 19 + Vite 7 + Tailwind 4 + React Router 7（`MemoryRouter`）+ Zustand。业务数据**不进 store**：各页面本地 `useState` + `load()`，本地 IPC 重新拉取零成本。
- 唯一全局 store `useServerStore`（`{running, host, port}`）**只由 `server-started` / `server-stopped` 事件写入**；事件是唯一权威源，前端按钮不自改状态（首挂载经 `get_server_status` 校准一次是唯一例外）。
- 主题三态 `light` / `dark` / `system`，`data-theme` 属性 + CSS 变量；`system` 由 `@media (prefers-color-scheme: dark)` 响应，零 JS 监听。Tailwind 4 用 `@custom-variant dark` 让 `dark:` 跟随 `data-theme`。
- 路径别名 `@/` → `src/`（vite.config.ts + tsconfig 各配一条）。

### 约定

- 命名：用例 `<Verb>Usecase`；仓储 `<Aggregate>Repository`；适配器 `<Provider>Adaptor`；领域服务行为命名（`ChannelSelector` / `QuotaPolicy`），不用 `_service` 后缀。
- 2024 Edition 模块布局：全部 `foo.rs`，不用 `mod.rs`；子模块 `foo/bar.rs` 引用为 `crate::foo::bar`。单文件超 ~400 行升级为 `foo/{...}` 文件夹（演进触发线）。
- **每个源文件开头必须有注释**描述该文件作用；**代码注释一律使用英文**（红线，Rust 与 TS/TSX 同样适用）。
- ID 用 uuid v7（时间有序主键）；审计时间一律 `DateTime<Utc>`（chrono）；错误处理 domain/usecases 用 thiserror，interface/main 用 anyhow。
- 应用设置存 tauri-plugin-store 的 `settings.json`；SQLite 只存业务数据。
- 提交信息遵循 Conventional Commits（英文，scope 用 kebab-case）；CHANGELOG 遵循 Keep a Changelog。内部文档用中文。

## 测试策略

- 只测外部行为（公开接口输入 → 输出），不 mock 业务本身。
- **Seam A**（主）：`test_support.rs` 的内存 mock 仓储，覆盖 usecases 编排的每一条分支。
- **Seam B**（补充）：`tower::ServiceExt::oneshot` + 内存仓储 + 真实路由，验证 HTTP 路由存在性、Bearer `401`、配额 `429`、`/v1/models` 合并去重、**SSE 流式含 `[DONE]` 收尾**——SSE 是唯一必须真实 HTTP 验证的点。
- domain 纯逻辑（`ChannelSelector` / `QuotaPolicy`）经 usecases 测试覆盖，不单独建 seam。
