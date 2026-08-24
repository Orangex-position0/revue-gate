# 发布与部署

> revue-gate 是本地桌面应用，不是服务端 SaaS。这里的“部署”主要指把 Tauri 安装包交付给用户，并让应用在用户本机运行。

## 当前状态

当前仓库已经具备本地打包基础：

- `pnpm tauri build` 触发 Tauri 打包；
- `beforeBuildCommand = "pnpm build"` 保证前端先完成类型检查和 Vite 构建；
- `frontendDist = "../dist"` 让 Tauri 使用前端静态产物；
- `bundle.active = true` 且 `targets = "all"`。

当前仓库包含 GitHub Actions 发布流水线草案：推送 `v*` tag 后在 Windows / macOS / Linux 分别构建 Tauri bundle，并创建 draft GitHub Release。当前尚未配置签名、公证、自动更新或校验和发布，因此本文只描述源码、CI 草案与本地交付模型。

## 交付模型

```text
source tree
  -> pnpm build
  -> dist/
  -> pnpm tauri build
  -> platform bundle
  -> user installs and runs locally
```

CI 发布草案：

```text
push v* tag
  -> GitHub Actions matrix build
  -> upload platform bundle artifacts
  -> create draft GitHub Release
  -> maintainer reviews assets and release notes
```

运行后，应用在用户本机提供两类能力：

- 控制面：Tauri webview + Tauri Commands，用于管理渠道、密钥、日志、设置；
- 数据面：用户手动或配置启动后，在本机监听 `host:port`，提供 OpenAI-compatible `/v1/*`。

## 发布前检查

建议最小检查：

```powershell
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

若发布面向真实用户，还应手工验证：

- 首次启动是否能打开主窗口；
- 设置页启动/停止服务是否能正确显示实际监听地址；
- `/health` 是否返回 200；
- 创建 Channel 和 API Key 后，`/v1/chat/completions` 能走通；
- 关闭到托盘、最小化到托盘、开机自启等平台相关设置是否符合预期。

## 后续发布能力

后续如果需要正式发布，应新增设计并落地：

- release workflow 产物命名与人工验收流程；
- Windows 代码签名、macOS signing + notarization；
- release artifacts 命名与校验和；
- 自动更新策略；
- 版本号同步策略，避免 `package.json`、`Cargo.toml`、`tauri.conf.json` 漂移。
