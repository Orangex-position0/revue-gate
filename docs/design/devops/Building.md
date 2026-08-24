# 软件打包

> 本文档描述 revue-gate 桌面应用的本地构建与打包设计。当前实现以 Tauri v2 的 bundle 能力为主，配置来源是 `package.json`、`src-tauri/tauri.conf.json` 和 `src-tauri/Cargo.toml`。

## 设计目标

revue-gate 是一个 Tauri 桌面应用，前端由 Vite 构建成静态资源，后端由 Rust/Tauri 打进桌面应用。打包流程需要保证：

- 前端 TypeScript 与 Vite 构建先通过；
- Tauri 读取 `dist/` 作为 webview 前端资源；
- Rust 后端、Tauri 插件、图标和安装包元数据统一由 Tauri bundler 处理；
- 不在设计文档中承诺尚未配置的签名、公证或自动更新能力；GitHub Actions release workflow 目前只负责构建 bundle 并创建 draft release。

## 构建链路

当前配置：

| 环节 | 配置位置 | 说明 |
| --- | --- | --- |
| 前端构建 | `package.json` 的 `build` | `tsc && vite build`，输出到 `dist/` |
| Tauri build 前置命令 | `src-tauri/tauri.conf.json` 的 `beforeBuildCommand` | `pnpm build` |
| Tauri 前端资源目录 | `src-tauri/tauri.conf.json` 的 `frontendDist` | `../dist` |
| Rust/Tauri crate | `src-tauri/Cargo.toml` | `revue_gate_lib`，Tauri v2 + Axum + sqlx + reqwest |
| bundle 开关 | `src-tauri/tauri.conf.json` 的 `bundle.active` | `true` |
| bundle target | `src-tauri/tauri.conf.json` 的 `bundle.targets` | `all` |

本地打包入口：

```powershell
pnpm tauri build
```

Tauri 会先执行 `pnpm build`，再编译 Rust 后端并生成当前平台支持的安装包/应用包。

## 产物策略

`bundle.targets = "all"` 表示让 Tauri 在当前操作系统上生成它能生成的全部目标。常见产物包括：

| 平台 | 常见产物 |
| --- | --- |
| Windows | `.msi` / `.exe` |
| macOS | `.app` / `.dmg` |
| Linux | `.deb` / `.rpm` / AppImage（取决于本机环境与 Tauri 支持） |

需要注意：Tauri 打包通常是按当前 OS 构建对应平台产物，不应把 `targets = "all"` 理解为“一台机器跨平台生成所有系统安装包”。跨平台发布需要分别在 Windows / macOS / Linux 环境构建。

## 应用元数据

| 字段 | 当前值 | 配置位置 |
| --- | --- | --- |
| productName | `revue-gate` | `src-tauri/tauri.conf.json` |
| version | `0.1.0` | `src-tauri/tauri.conf.json` / `package.json` / `Cargo.toml` |
| identifier | `com.revuegate.app` | `src-tauri/tauri.conf.json` |
| window title | `revue-gate` | `src-tauri/tauri.conf.json` |

图标由 `src-tauri/icons/*` 提供，包括 `.png`、`.icns`、`.ico`。安装包展示图标和系统快捷方式图标都依赖这些资源。

## 验证建议

打包前建议先跑最小验证：

```powershell
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

如果只验证前端类型和资源构建，`pnpm build` 已覆盖 `tsc` 和 Vite build。

## 当前未覆盖

当前仓库尚未覆盖以下发布能力：

- 代码签名 / macOS notarization；
- 自动更新 artifacts；
- release artifacts 校验和与人工验收清单；
- 按版本生成 changelog / release notes 的自动步骤。

这些属于发布工程，不是当前本地打包链路的一部分；后续引入时应补充到 `deployment.md` 或单独的 release 设计文档。
