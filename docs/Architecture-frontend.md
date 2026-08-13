# 前端架构

> 本文档描述 revue-gate 前端（React 管理界面）的架构：目录结构、分层职责、数据通道与关键流程。
> 技术选型与功能需求见 `docs/Requirements.md`，后端架构见 `docs/Architecture-backend.md`。

## 架构总览

前端是 Tauri 桌面应用内置的 React 管理界面（webview），单页应用。它**不直接访问网关数据面**（Axum HTTP，`/v1/*`），所有数据通过 **Tauri Command 控制面**走本地 IPC。

```
React UI ──invoke──► Tauri Command ──► usecases ──► domain ──► SQLite
      ◄──事件/返回──┘        （控制面，见 Architecture-backend.md）
```

结构参考 WaLiAPI（`pages/` + `components/` + `lib/` + `types/`，无独立 store 层），业务代码按页面就近组织，**不照搬后端分层**——前端只表达调用、展示与交互边界。

## 目录结构

```
src/
├── main.tsx                  # 入口：StrictMode + 挂载 + 启动 server 事件监听
├── app/
│   ├── App.tsx               # 组装 Layout + 集中定义 Routes（薄壳）
│   └── providers.tsx         # 全局 providers（如需）
├── pages/                    # 路由入口；业务逻辑就近放页面目录内
│   ├── dashboard/            #   仪表盘（统计卡片 + 7 天趋势）
│   ├── channels/             #   渠道管理
│   ├── api-keys/             #   密钥管理
│   ├── logs/                 #   请求日志
│   └── settings/             #   设置中心
├── components/               # 跨页复用组件
│   ├── ui/                   #   shadcn/ui 生成的基础组件（Button/Dialog/...）
│   └── layout/               #   侧边栏、顶栏、应用外壳
├── stores/                   # Zustand store（仅 useServerStore）
├── hooks/                    # 跨页复用 hooks（如 useServerStatus）
├── lib/                      # 基础设施封装：tauri api / 事件 / 主题 / 常量
│   ├── api.ts                #   Command 按域封装（channelApi / logApi / ...）
│   ├── server-events.ts      #   server-started/stopped 事件桥接
│   ├── theme.ts              #   主题三态应用
│   └── constants.ts          #   渠道类型 label、格式化函数等
├── types/                    # 跨页共享类型（index.ts 集中导出）
├── config/                   # 环境常量配置
├── styles/                   # 全局样式、Tailwind 入口、CSS 变量
└── assets/                   # 静态资源
```

## 分层职责

### app / main.tsx（应用组装）

- `main.tsx`：`ReactDOM.createRoot` + `<StrictMode>`，调用 `setupServerEvents()` 启动事件桥接
- `App.tsx`：只做组装——`Layout` 包 `Routes`，集中定义 5 个页面路由；不写业务逻辑、不塞启动副作用

### pages（路由入口 + 业务载体）

每个页面一个目录，对应一个后端 usecase 域：

| 页面 | 职责 | 对应 Command 封装 |
| --- | --- | --- |
| `dashboard/` | 统计卡片 + 7 天趋势折线 | `statsApi` |
| `channels/` | 渠道 CRUD / 启停 / 测试 / 模型映射 / 排序 | `channelApi` |
| `api-keys/` | 密钥 CRUD / 配额管理 | `apiKeyApi` |
| `logs/` | 日志分页 / 筛选 / 详情 / 删除 | `logApi` |
| `settings/` | 端口 / 主题 / 托盘 / 自启 / 重试策略 | `settingsApi` |

业务代码**就近放页面目录内**，只有真正被多处复用的才提升到 `components/` / `hooks/`。不建空目录、不为对称性建目录。

### components（复用组件）

- `ui/`：仅 shadcn/ui 生成物，不含业务
- `layout/`：侧边栏、顶栏、应用外壳
- 业务组件复用超过一个页面时放入 `components/`（如 `ChannelForm.tsx`），否则留在页面目录

### stores / hooks（跨页状态）

- **唯一全局 store `useServerStore`**：`{ running, host, port }`，**只被 server 事件写入**，全局读取（顶栏状态灯、设置页、状态展示）
- 业务数据**不进 store**：各页面本地 `useState` + `load()`，CRUD 后就地刷新
- 跨页复用 hooks 才放 `hooks/`，未复用前留在使用处

### lib（基础设施层）

| 文件 | 职责 |
| --- | --- |
| `api.ts` | `invoke` 的按域薄封装：`xxxApi = { cmd: () => invoke<T>("cmd", args) }`；类型从 `types/` 导入；错误统一转换 |
| `server-events.ts` | `listen("server-started"/"server-stopped")` → 写 `useServerStore`；模块级幂等标志 |
| `theme.ts` | `applyTheme(theme)` 设置 `data-theme` 属性 |
| `constants.ts` | 渠道类型/协议 label、时间/数字格式化等纯函数 |

## 状态管理策略

| 状态类型 | 归属 | 依据 |
| --- | --- | --- |
| 服务运行状态 | `useServerStore` | 跨页全局、由后端事件驱动，唯一需要全局共享 |
| 主题/设置 | 设置页本地 + `applyTheme()` 立即生效 | 启动读一次，改动就地应用，无需全局缓存 |
| 业务列表（渠道/密钥/日志/统计） | 页面本地 `useState` + `load()` | 本地 IPC 毫秒级，重新拉取零成本，全局同步是多余复杂度 |
| 表单/弹窗/加载态 | 页面本地 `useState` | 单页面瞬态 |

**不引入 TanStack Query / SWR**：它们为 HTTP 网络缓存设计，本地 Command 场景是过度设计。

## 关键流程

### 数据通道（页面 → 后端）

```
页面 load() ──► lib/api.ts (channelApi.getAll) ──invoke──► Tauri Command ──► usecases ──► SQLite
      ◄── typed result（Channel[] 等）────────────────────┘
```

- `api.ts` 是全项目唯一的 `invoke` 调用点；后端加命令只改这一个文件 + `types/`
- 返回值类型化（`invoke<T>`），与后端 Command 签名对齐

### 服务状态桥接（后端 → 前端）

```
main.tsx setupServerEvents()
  ├── listen("server-started", e → useServerStore.setState({ running: true, host, port }))
  ├── listen("server-stopped",  → useServerStore.setState({ running: false }))
  └── 模块级标志位幂等（StrictMode 双挂载防重复注册）
```

- **事件是唯一权威源**：前端"启动/停止服务"按钮调 `serverApi.start()` → 按钮 `loading` 态 → 等事件回调最终同步，**不自改 store**，状态永远与后端一致
- 事件常驻监听，不随 App 生命周期清理

### 主题应用

```
启动: settingsApi.get() → applyTheme(theme)
设置页: 用户选主题 → settingsApi.save() → applyTheme(theme) 立即生效
```

- 三态 `light` / `dark` / `system`，`data-theme` 属性 + CSS 语义变量
- `system` 态不写暗色变量，由 CSS `@media (prefers-color-scheme: dark)` 响应，零 JS 监听
- Tailwind 4：`@custom-variant dark (&:where([data-theme="dark"]))` 让 `dark:` 变体跟随 `data-theme`

## 技术选型

| 项 | 选型 | 说明 |
| --- | --- | --- |
| 构建 | Vite 7 + TS + React 19 | 需求文档锁定，见 `docs/Requirements.md` |
| 路由 | React Router 7 + `MemoryRouter` | Tauri 场景不依赖 URL history API，刷新/文件协议下更稳 |
| 状态管理 | Zustand（仅 server store） | 唯一全局状态 |
| UI | shadcn/ui + Tailwind CSS 4 + Lucide | `components/ui/` 隔离生成物 |
| 数据通道 | Tauri Command（`invoke`） | 不直连网关 HTTP 数据面 |
| 事件 | `@tauri-apps/api` `listen` | server 状态桥接 |
| 主题 | CSS 变量 + `data-theme` | 三态，跟随系统零 JS |
| 路径别名 | `@/` → `src/` | vite + tsconfig 各配一条 |

## 命名约定

- **页面组件**：`XxxPage.tsx`（`DashboardPage`）
- **Command 封装**：`lib/api.ts` 中按域 `xxxApi`（`channelApi` / `logApi`）
- **Store**：`useServerStore`（hooks 风格）
- **复用组件**：PascalCase（`ChannelForm.tsx`）
- **类型**：`types/index.ts` 集中导出；`interface` 用于可扩展公共契约，`type` 用于 union/mapped
- **常量**：`lib/constants.ts`
- **每个源文件开头必须有注释**，描述该文件的作用（与后端约定一致）

## 与后端的分界

- 前端只走 **控制面**（Tauri Command）；数据面（Axum `/v1/*`）对前端透明，前端感知不到渠道/密钥/转发细节
- 类型以 Command 签名为准，`types/` 与 `usecases` 的入参/出参一一对应
- server 状态由后端 `emit` 事件驱动，前端不做轮询
