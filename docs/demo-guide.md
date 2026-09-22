# revue-gate 面试演示文档

> 目标：在 8–12 分钟内完成一次可复现的本地 LLM Gateway 演示。演示重点不是“页面很多”，而是说明统一入口、密钥隔离、渠道调度和可观测性如何闭环。

## 1. 当前可运行性结论

截至本次检查：

| 项目 | 结果 |
| --- | --- |
| 前端 TypeScript + Vite 构建 | ✅ `pnpm build` 通过 |
| Tauri 环境检查 | ⚠️ WebView2、MSVC、Node、pnpm 正常；未安装 Rust/Cargo |
| `pnpm tauri dev` | ❌ 当前机器无法启动，失败于 `cargo metadata`，原因是找不到 `cargo` |
| Rust 格式化、编译、测试、Clippy | ⏸️ 尚未执行，原因是当前环境没有 Rust/Cargo |

因此，**项目代码的前端构建是正常的，但当前机器不能直接启动完整桌面应用**。安装 Rust stable（项目要求 Rust 1.85）后，再执行下面的启动流程即可验证完整运行链路。

## 2. 面试前一次性准备

### 2.1 安装与验证环境

Windows 需要：

- Node.js 与 pnpm
- Rust stable / Cargo；项目已通过 `src-tauri/rust-toolchain.toml` 固定工具链
- Visual Studio C++ Build Tools
- WebView2

验证：

```powershell
node --version
pnpm --version
rustc --version
cargo --version
pnpm tauri info
```

如果依赖已安装：

```powershell
pnpm install
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo check --manifest-path src-tauri/Cargo.toml --all-targets --all-features
cargo nextest run --manifest-path src-tauri/Cargo.toml
pnpm tauri dev
```

首次启动会创建本地 SQLite 数据库和 `settings.json`。默认数据面地址是：

```text
http://127.0.0.1:3000
```

### 2.2 演示前准备上游账号

准备一个可用的 OpenAI-compatible 上游 API Key（OpenAI、DeepSeek 或公司内部兼容端点均可）。不要把真实 key 写进演示录屏、PPT、Git 或聊天窗口。

建议使用：

- 渠道名称：`demo-deepseek`
- 类型：`DeepSeek`
- Base URL：使用页面提示的默认值，或填入实际兼容端点
- 模型：`deepseek-chat`
- 优先级：`0`
- 权重：`1`

如果没有可用上游 Key，仍可演示页面、配置、服务生命周期、健康检查和认证失败路径；不要把“真实模型调用成功”描述成已验证。

## 3. 推荐现场演示流程

### 第一步：介绍产品定位（30 秒）

可以这样说：

> revue-gate 是一个本地 LLM API Gateway。它把不同供应商的 API 聚合到一个 OpenAI-compatible 地址后面，下游只需要配置一个本地 API Key。网关负责认证、模型映射、渠道选择、协议转换、失败切换、配额和请求日志。桌面端是 control plane，内置 Axum HTTP 服务是 data plane。

### 第二步：启动应用并展示服务状态（1 分钟）

1. 执行 `pnpm tauri dev`。
2. 打开“设置”。
3. 确认 Host 为 `127.0.0.1`，端口为 `3000`。
4. 点击“保存”，再点击“启动服务”。
5. 展示状态变为运行中，并说明服务状态由后端事件同步到前端。

在 PowerShell 另开窗口验证：

```powershell
curl.exe http://127.0.0.1:3000/health
```

预期结果：

```json
{"status":"ok"}
```

### 第三步：创建渠道（1–2 分钟）

进入“渠道” → “新建渠道”：

1. 填写名称、渠道类型、Base URL、上游 API Key。
2. 填写模型列表。
3. 保持启用，优先级填 `0`，权重填 `1`。
4. 保存后展示列表中的 API Key 已被掩码。
5. 如条件允许，点击“测试”或“获取”验证上游连通性。

讲解重点：

- OpenAI、DeepSeek、Custom 走 OpenAI-compatible 直通。
- Claude、Gemini 由 provider adapter 负责协议转换。
- 上游 Key 只保存在网关渠道配置中，不下发给客户端。

### 第四步：创建本地 API Key（1 分钟）

进入“密钥” → “新建密钥”：

1. 名称填写 `interview-demo`。
2. 配额可以先留空，表示不限额；也可以填写一个小数值用于演示 429。
3. 保存后**立即复制完整 key**。完整 key 只在创建结果中显示一次，列表只显示掩码。

将 key 暂存为 PowerShell 变量：

```powershell
$KEY = "sk-revue-替换成刚刚生成的完整密钥"
```

### 第五步：验证统一模型入口（1 分钟）

```powershell
curl.exe http://127.0.0.1:3000/v1/models `
  -H "Authorization: Bearer $KEY"
```

展示启用渠道暴露的模型列表，并说明这里是网关的统一模型视图，而不是直接暴露某一个 provider 的 API。

### 第六步：发起一次非流式请求（1–2 分钟）

```powershell
$body = @'
{
  "model": "deepseek-chat",
  "messages": [
    {"role": "user", "content": "用一句话解释什么是 API Gateway。"}
  ],
  "stream": false
}
'@

curl.exe http://127.0.0.1:3000/v1/chat/completions `
  -H "Authorization: Bearer $KEY" `
  -H "Content-Type: application/json" `
  -d $body
```

讲解请求链路：

```text
HTTP handler
  → Bearer 本地密钥认证 / quota 检查
  → 安全审计
  → 按模型、priority、weight 选择渠道
  → model mapping
  → provider adapter 转发
  → usage 记账 + request log
```

### 第七步：演示流式 SSE（1 分钟）

```powershell
$streamBody = @'
{
  "model": "deepseek-chat",
  "messages": [
    {"role": "user", "content": "请分三点说明流式响应的优点。"}
  ],
  "stream": true
}
'@

curl.exe -N http://127.0.0.1:3000/v1/chat/completions `
  -H "Authorization: Bearer $KEY" `
  -H "Content-Type: application/json" `
  -d $streamBody
```

指出响应会以 SSE chunk 持续返回并以 `[DONE]` 收尾。流式请求会边转发边累计 usage；如果上游在响应已经开始后失败，HTTP status 无法再修改，系统会记录失败并结束流，这是 SSE 的正常错误边界。

### 第八步：回到日志和仪表盘（1 分钟）

1. 打开“日志”，展示模型、渠道、状态码、耗时、Token、trace id 等字段。
2. 打开“仪表盘”，展示请求数、Token、平均延迟和趋势。
3. 打开“用量”，说明统计维度可以支持后续配额和成本治理。

最后可点击“停止服务”，说明控制面负责服务生命周期，数据面可以独立对外提供统一 API。

## 4. 建议准备的异常演示

只选一个即可，避免现场风险：

### 方案 A：错误本地 Key

```powershell
curl.exe http://127.0.0.1:3000/v1/models `
  -H "Authorization: Bearer sk-revue-invalid"
```

预期：返回 `401`，说明客户端使用的是网关本地 Key。

### 方案 B：渠道故障切换

准备两个同模型渠道：同一 priority 下设置不同 weight，或把主渠道临时配置为不可用地址。请求失败后展示网关继续尝试候选渠道，并在日志中留下失败记录。

### 方案 C：配额限制

创建一个很小的 quota，完成一次请求后再次请求，展示超过额度时返回 `429`，且不会进入上游转发。

## 5. 面试时的架构说明

### 目录分层

```text
interface       HTTP routes / Tauri commands
usecases        认证、转发、渠道选择、统计等业务编排
domain          Channel、ApiKey、Quota、ProviderAdaptor 等核心模型和 trait
infrastructure  SQLite、Tauri Store、reqwest provider adapter
protocol        OpenAI / Anthropic / Gemini 相关协议模型与转换
```

依赖方向是：

```text
interface → usecases → domain
infrastructure → domain
```

### 三个最值得强调的设计点

1. **Data Plane / Control Plane 分离**：真实请求链路与配置管理解耦。
2. **Provider Adapter**：上层只依赖统一语义，Claude/Gemini 的差异被隔离在适配器内。
3. **可观测闭环**：认证、路由、转发、usage、quota、trace id、日志和统计串成一条链。

## 6. 现场风险与备用方案

- **没有 Rust/Cargo**：无法启动 Tauri；提前安装并执行 `pnpm tauri info`。
- **上游 Key 失效或网络不稳定**：优先演示 `/health`、配置、认证失败和日志；不要临时更换代码。
- **端口 3000 被占用**：在设置中改为可用端口并保存，再启动服务；演示命令同步替换端口。
- **首次数据库初始化慢**：提前启动一次，让迁移完成；不要删除应用数据目录。
- **录屏泄露 Key**：使用一次性演示 Key，结束后删除渠道和本地 API Key。

## 7. 收尾话术

> 这个项目的核心不是把几个供应商 API 拼在一起，而是把 AI 请求统一入口之后的治理问题集中解决：身份隔离、协议归一、渠道调度、失败处理、配额和可观测性。当前版本定位是单机单用户 MVP，下一步可以继续补齐 provider timeout、熔断、更加完整的成本统计和多用户鉴权。
