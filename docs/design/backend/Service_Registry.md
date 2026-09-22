# Service Module Registry

## Introduction

本文档描述 revue-gate 的 Service Module Registry 设计。它是一个本地单进程的扩展模块注册框架，用来为后续 Knowledge、MCP 等能力提供统一的 HTTP 路由挂载、模块声明校验、启用配置和 Tauri 控制面状态查询。

当前实现只落地框架，不实现真实 Knowledge / MCP 业务模块。生产环境下 `ServiceRegistry::new()` 返回空 registry，因此不会暴露 `/api/kb`、`/mcp` 等未来路由。

本文重点回答：

1. 为什么需要 Service Module Registry。
2. 模块由哪些部分组成，各自放在哪一层。
3. 关键接口、结构体、字段和容易出错的设计点。
4. 后续实现具体服务模块时应如何组织代码。

## 为什么需要本模块

revue-gate 当前核心能力是 OpenAI-compatible / Anthropic / Responses 等 `/v1/*` 网关路由。后续新增 Knowledge、MCP 等本地扩展能力时，它们会有自己的 HTTP namespace、状态展示、启用开关和内部依赖。如果每个能力都直接改根 router、直接读 settings、直接暴露 command，代码会很快变成分散的条件判断。

Service Module Registry 解决的是“扩展模块如何接入网关”这个装配问题：

- 统一声明模块身份、名称、描述和 path namespace。
- 在根 router 构建时，只合并已启用模块的路由。
- 在 Tauri 控制面中，按稳定顺序返回所有已注册模块的状态。
- 在开发期校验重复 id、保留路径、非法路径前缀和路径冲突。
- 为未来前端服务管理 UI 预留稳定的数据结构。

它不是分布式服务注册中心，也不管理独立进程生命周期。`ServiceModuleStatus.running` 表示模块当前健康且可服务，不表示有独立 server/process 正在运行。

## 设计思路

### 组成部分

| 组成部分 | 代码位置 | 职责 | 关键设计 |
| --- | --- | --- | --- |
| `ServiceModule` | `src-tauri/src/interface/http/service_modules.rs` | 定义可挂载 HTTP 扩展模块的 interface | 模块不自行判断 enabled；只声明元数据、path namespace、routes 和 status |
| `ServiceRegistry` | `src-tauri/src/interface/http/service_modules.rs` | 保存模块列表、校验模块声明、合并路由、聚合状态 | `Vec<Box<dyn ServiceModule>>` 保持注册顺序并允许不同具体模块放入同一列表 |
| `ServiceModuleStatus` | `src-tauri/src/interface/http/service_modules.rs` | 返回给 Tauri 控制面的模块状态 DTO | 使用 camelCase 序列化；`stats` 保持 JSON object，避免框架依赖模块私有统计类型 |
| `ServiceRegistryError` | `src-tauri/src/interface/http/service_modules.rs` | 表达 registry 装配校验错误 | 面向开发期 wiring bug；`build_router` 中直接 `expect` |
| `ServiceModuleSettings` | `src-tauri/src/domain/settings.rs` | 保存模块启用配置 | 强类型字段 `knowledge` / `mcp`，旧 settings 通过 serde default 兼容 |
| `build_router` 接入点 | `src-tauri/src/interface/http/router.rs` | 根 HTTP router 合并 Service Module routes | 接收 `GatewaySettings` 启动快照，模块启停第一版需要重启 HTTP server 生效 |
| `get_service_statuses` | `src-tauri/src/interface/commands/services.rs` | Tauri command 查询模块状态 | 使用显式 `tauri::State<'_, AppState>` 和 settings state 注入，不新增 HTTP 管理端点 |
| Frontend types / wrapper | `src/types/index.ts`, `src/lib/api.ts` | 保持前端与 command/settings 结构兼容 | 只补类型和 API wrapper，不实现 UI |

### `ServiceModule`

`ServiceModule` 是扩展模块挂载到 HTTP interface 层的最小 interface。它只表达模块作为 HTTP capability 的外壳，不承载业务流程本身。

当前代码形状：

```rust
#[async_trait::async_trait]
pub trait ServiceModule: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn path_prefixes(&self) -> &'static [&'static str];
    async fn get_status(&self, state: &AppState, enabled: bool) -> ServiceModuleStatus;
    fn routes(&self) -> Router<AppState>;
}
```

关键设计：

- `id()` 是稳定机器标识，用于 settings 映射、状态列表和冲突排查。合法格式是 `[a-z][a-z0-9_-]*`。
- `name()` / `description()` 面向控制面展示，不参与业务判断。
- `path_prefixes()` 声明模块拥有的 canonical HTTP namespace，比如 `/api/kb`、`/mcp`。它用于校验和展示，不自动重写 route。
- `get_status()` 接收 `enabled: bool`，但不接收完整 `GatewaySettings`。启用判断由 registry 统一完成，避免模块各自读取配置。
- `routes()` 返回 `Router<AppState>`，不直接接收 `AppState`。根 router 最后统一 `.with_state(state)`。

容易出错的点：

- 不要给 trait 增加 `enabled()`。enabled 是用户配置，不是模块内生属性。
- 不要在 `routes()` 里创建数据库连接、读 settings 或启动后台任务。这里应只返回 HTTP route tree。
- `get_status()` 不应因为模块内部依赖查询失败而让整个 command 失败。具体模块应返回 `running = false`，并在 `stats` 中放入错误摘要。
- `path_prefixes()` 要声明对外可见 namespace。若实际 router 同时支持 `/mcp` 和 `/mcp/`，声明仍应只写 canonical 形式 `/mcp`。

### `ServiceModuleStatus`

`ServiceModuleStatus` 是 Tauri command 返回给前端的控制面 DTO。

当前代码形状：

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceModuleStatus {
    /// Stable module id.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Human-readable summary.
    pub description: String,
    /// Canonical HTTP path prefixes owned by the module.
    pub path_prefixes: Vec<String>,
    /// Whether routes are enabled by settings.
    pub enabled: bool,
    /// Whether the module is healthy and serviceable.
    pub running: bool,
    /// Module-specific status counters or diagnostics.
    pub stats: Value,
}
```

字段语义：

| 字段 | 语义 |
| --- | --- |
| `id` | 稳定模块 id，例如 `knowledge`、`mcp` |
| `name` | 人类可读名称，用于 UI 展示 |
| `description` | 人类可读简介，用于 UI 展示 |
| `path_prefixes` | 模块声明拥有的 canonical HTTP path prefixes |
| `enabled` | settings 中解析出的启用状态 |
| `running` | 模块当前是否健康、可服务 |
| `stats` | 模块私有统计或诊断信息，顶层应为 JSON object |

前端对应字段使用 camelCase：

```ts
export interface ServiceModuleStatus {
  id: string;
  name: string;
  description: string;
  pathPrefixes: string[];
  enabled: boolean;
  running: boolean;
  stats: Record<string, unknown>;
}
```

### `ServiceRegistry`

`ServiceRegistry` 是框架的核心装配 Module。它用很小的 interface 隐藏四类行为：注册、校验、路由合并、状态聚合。

当前代码形状：

```rust
pub struct ServiceRegistry {
    /// Registered service modules
    modules: Vec<Box<dyn ServiceModule>>,
}

impl ServiceRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, module: Box<dyn ServiceModule>);
    pub fn validate(&self) -> Result<(), ServiceRegistryError>;
    pub fn merge_routes(&self, settings: &ServiceModuleSettings) -> Router<AppState>;
    pub async fn list_statuses(
        &self,
        state: &AppState,
        settings: &ServiceModuleSettings,
    ) -> Vec<ServiceModuleStatus>;
}
```

`modules: Vec<Box<dyn ServiceModule>>` 的含义：

- `modules` 是已经注册的 in-process Service Module 实例列表。
- `Vec` 保留确定性顺序，影响 status 展示顺序和 route merge 顺序。
- `Box<dyn ServiceModule>` 允许不同 concrete 类型放入同一个列表，例如未来的 `KnowledgeServiceModule` 和 `McpServiceModule`。
- 字段保持 private，外部只能通过 `register()` 增加模块，避免调用方绕过 registry 的约束。

关键设计：

- `ServiceRegistry::new()` 必须纯粹：不读 settings、不访问数据库、不做 IO、不启动任务。
- 当前生产 `new()` 返回空 registry。这是刻意设计，避免用 fake/noop 模块冒充真实能力。
- registry 不作为 Tauri managed state 保存。它是轻量装配对象，需要时重新构造即可。
- `register()` 使用简单 mutable API，不引入 builder。当前模块数量少，builder 会让 interface 变浅。
- `merge_routes()` 只合并 `settings.is_enabled(module.id()) == true` 的模块路由。
- `list_statuses()` 返回所有已注册模块，包括 disabled 模块，顺序与注册顺序一致。
- 未知 module id 在 settings 中默认 disabled，防止未来新增模块时忘记补配置字段导致误启用。

推荐把字段注释改成更准确的版本：

```rust
/// Registered in-process Service Modules, kept in deterministic status and route-merge order.
modules: Vec<Box<dyn ServiceModule>>,
```

### Registry 校验

`ServiceRegistryError` 表达开发期装配错误。它不是模块业务错误，也不返回给前端作为常规状态。

当前错误类型：

```rust
#[derive(Debug, thiserror::Error)]
pub enum ServiceRegistryError {
    #[error("service module id is invalid: {0}")]
    InvalidId(String),
    #[error("duplicate service module id: {0}")]
    DuplicateId(String),
    #[error("service module {module_id} must declare at least one path prefix")]
    EmptyPathPrefixes { module_id: String },
    #[error("service module {module_id} has invalid path prefix: {path_prefix}")]
    InvalidPathPrefix {
        module_id: String,
        path_prefix: String,
    },
    #[error("service module {module_id} uses reserved path prefix: {path_prefix}")]
    ReservedPathPrefix {
        module_id: String,
        path_prefix: String,
    },
    #[error(
        "service module path prefix conflict: {left_module_id}:{left_path_prefix} conflicts with {right_module_id}:{right_path_prefix}"
    )]
    PathPrefixConflict {
        left_module_id: String,
        left_path_prefix: String,
        right_module_id: String,
        right_path_prefix: String,
    },
}
```

校验规则：

- module id 不得重复。
- module id 必须匹配 `[a-z][a-z0-9_-]*`。
- 每个模块必须至少声明一个 path prefix。
- path prefix 必须以 `/` 开头。
- path prefix 不能是 `/`。
- path prefix 不能以 `/` 结尾。
- path prefix 不能占用 `/health`、`/v1` 或 `/v1/*`。
- 不同模块的 path prefix 不能存在路径段级别的包含关系。

路径冲突必须按 segment boundary 判断：

```rust
fn path_prefixes_conflict(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
```

因此：

- `/api/kb` 与 `/api/kb` 冲突。
- `/api/kb` 与 `/api/kb/admin` 冲突。
- `/api/kb` 与 `/api/kbase` 不冲突。

`build_router` 不返回 `Result<Router, ServiceRegistryError>`。registry 校验失败说明开发者把模块接错了，应在启动时尽早 panic：

```rust
service_registry
    .validate()
    .expect("service module registry must be valid");
```

### 模块启用配置

`GatewaySettings` 增加 `service_modules` 字段，并通过 serde camelCase 暴露为前端的 `serviceModules`。

当前代码形状：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServiceModuleSettings {
    pub knowledge: bool,
    pub mcp: bool,
}

impl Default for ServiceModuleSettings {
    fn default() -> Self {
        Self {
            knowledge: true,
            mcp: true,
        }
    }
}

impl ServiceModuleSettings {
    pub fn is_enabled(&self, module_id: &str) -> bool {
        match module_id {
            "knowledge" => self.knowledge,
            "mcp" => self.mcp,
            _ => false,
        }
    }
}
```

关键设计：

- 使用强类型字段，不使用 `HashMap<String, bool>`。这样新增模块时必须显式修改 settings 类型、默认值、前端类型和测试。
- `knowledge` / `mcp` 默认值为 `true`。这表示未来实现真实模块后，默认随网关启动而可用。
- 旧 settings 文件缺少 `serviceModules` 时，应通过 `#[serde(default)]` 自动补齐。
- `is_enabled()` 是 module id 到 typed settings 的唯一映射点。未来新增模块时，需要在这里增加分支。
- 未知 module id 返回 `false`，避免模块忘记加入 settings 时被意外启用。

需要注意：第一版开关只影响 HTTP server 启动时的 route tree。运行中保存 settings 不会动态拆卸或挂载路由；动态启停已放入 roadmap 的后续项。

### Router 接入

根 router 接收 settings 快照：

```rust
pub fn build_router(state: AppState, settings: &GatewaySettings) -> Router {
    let service_registry = ServiceRegistry::new();
    service_registry
        .validate()
        .expect("service module registry must be valid");

    Router::new()
        .route("/health", get(health))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/messages", post(anthropic_messages))
        .route("/v1/responses", post(responses))
        .route("/v1/models", get(models))
        .merge(service_registry.merge_routes(&settings.service_modules))
        .with_state(state)
        // existing layers...
}
```

关键设计：

- Service Module routes 与核心 `/v1/*` route tree 同处一个 Axum router。
- Service Module routes 在 `.with_state(state)` 前 merge，因此各模块 router 共享根 `AppState`。
- 当前 registry 为空时，`merge_routes()` 返回空 `Router<AppState>`，不会改变现有路由行为。
- HTTP server 启动入口需要先从 shared settings 读取快照，再调用 `build_router(state, &settings_snapshot)`。

### Tauri Command 接入

新增 command：

```rust
#[tauri::command(rename_all = "camelCase")]
pub async fn get_service_statuses(
    state: tauri::State<'_, AppState>,
    settings: tauri::State<'_, Arc<RwLock<GatewaySettings>>>,
) -> Result<Vec<ServiceModuleStatus>, String> {
    let registry = ServiceRegistry::new();
    registry
        .validate()
        .expect("service module registry must be valid");
    let settings = settings.read().expect("settings lock poisoned").clone();
    Ok(registry
        .list_statuses(state.inner(), &settings.service_modules)
        .await)
}
```

这里选择显式 `State<'_, AppState>` 和 `State<'_, Arc<RwLock<GatewaySettings>>>` 注入，而不是通过 `AppHandle` 间接查询 state：

- command 依赖一眼可见。
- 测试和后续重构更容易定位输入。
- 不把 command 做成泛用 state lookup surface。
- 保持现有 `AppState` 管理方式，不为了这个 command 改成 `Arc<AppState>`。

该 command 只用于本地桌面控制面。不要额外增加 `GET /api/services/statuses` 之类的 HTTP 管理端点。

### 前端兼容

当前只补类型和 wrapper，不做 UI：

```ts
export interface ServiceModuleSettings {
  knowledge: boolean;
  mcp: boolean;
}

export interface ServiceModuleStatus {
  id: string;
  name: string;
  description: string;
  pathPrefixes: string[];
  enabled: boolean;
  running: boolean;
  stats: Record<string, unknown>;
}
```

API wrapper：

```ts
export const serviceApi = {
  /** Service Module statuses: currently empty until real modules are registered. */
  statuses: () => invoke<ServiceModuleStatus[]>("get_service_statuses"),
};
```

`GatewaySettings` 需要包含 `serviceModules`，设置页即使暂时没有 UI，也必须在保存时保留该字段，避免保存 settings 时丢失模块开关。

### 代码组织方式

当前框架代码位置：

```text
src-tauri/src/
└── interface/
    ├── http/
    │   └── service_modules.rs
    └── commands/
        └── services.rs
```

当前不创建顶层 `src-tauri/src/services/`。原因是 `services/` 容易变成把 domain、usecase、HTTP adapter、数据库实现混在一起的大包。后续具体能力应按 revue-gate 现有分层组织：

```text
src-tauri/src/
├── domain/
│   └── knowledge.rs
├── usecases/
│   └── knowledge.rs
├── infrastructure/
│   └── sqlite/
│       └── knowledge.rs
└── interface/
    ├── http/
    │   └── service_modules/
    │       ├── knowledge.rs
    │       └── mcp.rs
    └── commands/
        └── services.rs
```

分层职责：

- `domain`：具体能力的领域模型、值对象、不变量、repository trait。不得依赖 Axum、Tauri、SQLite 具体实现。
- `usecases`：应用流程和编排，例如知识库导入、索引、搜索、MCP tool 列表生成、状态查询。
- `infrastructure`：SQLite、文件系统、向量索引、embedding provider、MCP session store 等技术实现。
- `interface/http/service_modules/*`：HTTP 挂载壳，负责 module metadata、routes 和 status DTO 装配。
- `interface/commands/services.rs`：Tauri 控制面 command，只做状态查询入口和 registry 调用。

### 后续模块实现示例

未来 Knowledge 模块可以长这样：

```rust
pub struct KnowledgeServiceModule;

#[async_trait::async_trait]
impl ServiceModule for KnowledgeServiceModule {
    fn id(&self) -> &'static str {
        "knowledge"
    }

    fn name(&self) -> &'static str {
        "Knowledge"
    }

    fn description(&self) -> &'static str {
        "Local knowledge base"
    }

    fn path_prefixes(&self) -> &'static [&'static str] {
        &["/api/kb"]
    }

    async fn get_status(&self, state: &AppState, enabled: bool) -> ServiceModuleStatus {
        // Query real status through usecases and repositories.
        todo!()
    }

    fn routes(&self) -> Router<AppState> {
        knowledge_routes()
    }
}
```

未来 MCP 模块可以长这样：

```rust
pub struct McpServiceModule;

#[async_trait::async_trait]
impl ServiceModule for McpServiceModule {
    fn id(&self) -> &'static str {
        "mcp"
    }

    fn name(&self) -> &'static str {
        "MCP Server"
    }

    fn description(&self) -> &'static str {
        "Model Context Protocol server"
    }

    fn path_prefixes(&self) -> &'static [&'static str] {
        &["/mcp"]
    }

    async fn get_status(&self, state: &AppState, enabled: bool) -> ServiceModuleStatus {
        // Query MCP availability and dependency status through usecases.
        todo!()
    }

    fn routes(&self) -> Router<AppState> {
        mcp_routes()
    }
}
```

这些代码块是后续实现参考，不是当前框架已经注册的生产模块。

### 测试策略

测试应覆盖 registry 的外部行为，而不是越过 interface 测内部细节：

- `ServiceRegistry::new()` 在生产第一版为空。
- 测试内使用 `FakeServiceModule` 通过 `register()` 注册模块。
- enabled module 的 routes 会被 merge 并可访问。
- disabled module 的 routes 不会被 merge，访问返回 404。
- `list_statuses()` 返回 disabled modules，且顺序与注册顺序一致。
- `validate()` 拒绝重复 id、非法 id、空 path prefix、非法 path prefix、reserved path prefix 和 path namespace 冲突。
- `validate()` 允许 `/api/kb` 与 `/api/kbase` 这种字符串相似但路径段不冲突的 prefix。
- 旧 settings JSON 缺少 `serviceModules` 时，默认 `knowledge = true`、`mcp = true`。
- `get_service_statuses` 在当前生产 registry 为空时返回 `[]`。

手动验证当前框架：

```bash
npm run tauri dev
```

在前端调用：

```ts
const statuses = await invoke("get_service_statuses");
console.log(statuses);
```

当前期望结果：

```json
[]
```

因为真实 Knowledge / MCP 模块尚未实现，下面这些未来验收项当前应返回 404：

```bash
curl -i http://localhost:3456/api/kb
curl -i http://localhost:3456/mcp/sse
curl -i -X POST http://localhost:3456/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
```

### 后续演进

后续实现 Knowledge / MCP 时，开发者需要：

1. 在对应 layer 中实现真实 domain / usecase / infrastructure / HTTP routes。
2. 在 `interface/http/service_modules/*` 中实现具体 `ServiceModule`。
3. 在 `ServiceRegistry::new()` 中注册真实模块。
4. 在 `ServiceModuleSettings` 中确认 module id 到 typed setting 字段的映射。
5. 补充模块自己的状态统计与错误降级逻辑。
6. 补充前端 Service Module UI。

运行中动态启停 Service Module 属于后续能力：需要按最新 settings 重建 Axum route tree 并重启 HTTP server，同时处理端口占用、重启失败回滚、前端状态事件和进行中 SSE 请求语义。
