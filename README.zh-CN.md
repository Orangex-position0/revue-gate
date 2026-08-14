# revue-gate

本地 LLM API 网关桌面应用。把多个上游供应商渠道（OpenAI / Claude / Gemini / DeepSeek / 自定义 OpenAI 兼容）聚合到统一入口后面，对外暴露 OpenAI 兼容接口——下游任何 AI 客户端只需指向 `http://127.0.0.1:3000`，认证、渠道选择、模型映射、转发、记账与日志全部由网关完成。

> **状态：** 早期开发（v0.1.0 MVP），功能按阶段落地。详见 [CHANGELOG.md](CHANGELOG.md) 与 [docs/roadmap.md](docs/roadmap.md)。

**[English](README.md)**

## 为什么

同时使用多个 AI 工具通常意味着要管理 N 个供应商密钥、N 套 API 格式，而且对实际发送的内容毫无可见性。revue-gate 在供应商前面放一个本地网关：

- **统一 OpenAI 兼容接口** —— `/v1/chat/completions`（流式 + 非流式）、`/v1/models`、`/health`。ChatBox、NextChat、OpenAI SDK、CLI 全部指向本地地址即可工作
- **上游密钥永不触达客户端** —— 下游只看到本地密钥（`sk-revue-…`），真实供应商密钥只存在网关里
- **渠道管理** —— OpenAI、DeepSeek、自定义 OpenAI 兼容端点直通转发；Claude / Gemini 走协议转换适配器。可配置优先级、权重、模型映射
- **配额 + 审计** —— 每密钥配额上限、全量请求日志（模型 / Token / 延迟 / 错误 / trace id），仪表盘展示用量与渠道健康度
- **本地且私有** —— 单机单用户，数据存本地 SQLite，无账号体系，无遥测

## 功能（v0.1.0 MVP）

- **数据面** —— 非流式与 SSE 流式 chat completions，失败时按候选渠道顺序重试
- **渠道** —— CRUD / 启停 / 排序 / 连通性测试 / 模型映射（5 类内置渠道）
- **密钥** —— `sk-revue-<hex>` 生成、Bearer 认证、配额上限（超出返回 `429`）
- **请求日志** —— 全量记录、分页、按关键词 / 密钥 / 渠道 / 模型 / 日期筛选、详情查看
- **仪表盘** —— 请求数 / Token 数、平均延迟、渠道可用率、7 天趋势
- **设置中心** —— host / port、浅色 / 深色 / 跟随系统主题、托盘 + 开机自启、重试策略

## 快速开始

前置依赖：Node.js + pnpm、stable Rust 工具链（由 `src-tauri/rust-toolchain.toml` 固定版本）、平台对应的 [Tauri v2 前置条件](https://v2.tauri.app/start/prerequisites/)（Windows 上为 WebView2）。

```bash
pnpm install     # 安装前端依赖
pnpm tauri dev   # 启动桌面应用（数据面随之启动）
```

网关默认监听 `127.0.0.1:3000`（可在设置中心修改；`port: 0` 表示随机端口）。

## 使用

先在界面里创建一个渠道（填入你的供应商密钥）和一个 API Key，然后让任意 OpenAI 兼容客户端指向网关：

```bash
curl http://127.0.0.1:3000/v1/chat/completions \
  -H "Authorization: Bearer sk-revue-<your-local-key>" \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o-mini", "messages": [{"role": "user", "content": "Hello!"}]}'
```

或任意 OpenAI SDK：

```ts
const client = new OpenAI({
  baseURL: "http://127.0.0.1:3000/v1",
  apiKey: "sk-revue-<your-local-key>",
});
```

另有：`GET /health` → `{"status":"ok"}`、`GET /v1/models`（所有启用渠道的模型合并去重）。

## 文档

- [需求文档](docs/Requirements.md) —— 产品定位、MVP 范围、架构总览
- [后端架构](docs/Architecture-backend.md) · [前端架构](docs/Architecture-frontend.md)
- [路线图](docs/roadmap.md) —— v0.2.0（安全审计中心）与 v0.3.0（RAG / MCP / 更多对外接口）规划

## 开发

质量检查通过 `prek`（配置见 `prek.toml`）接入 git hooks——先执行 `prek install`。覆盖 `cargo fmt`、`cargo check`、`cargo clippy -D warnings`、`cargo nextest run`、`tsc` 与前端构建。也可直接运行：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo nextest run --manifest-path src-tauri/Cargo.toml
pnpm build   # tsc + vite build
```

## 安全

网关设计上不把上游密钥暴露给下游——客户端只使用本地密钥认证。如发现安全问题，请私下联系维护者，不要直接开公开 issue。

## 贡献

项目仍处于私有开发阶段；欢迎通过 issue 提交 bug 报告与功能建议。贡献指南将在项目开源时发布。

## License

本项目采用 Apache License 2.0 授权；完整协议文本见 [LICENSE](LICENSE)。
