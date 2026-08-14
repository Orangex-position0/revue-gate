//! 控制面：服务生命周期命令（start/stop/status）+ 状态事件广播。
//!
//! 命令只做薄胶水：调用 `ServerManager` 生命周期，并把结果以
//! `server-started` / `server-stopped` 事件广播给前端。事件是运行状态的唯一权威源。
//! `start_gateway` / `stop_gateway` 为命令（start_server / stop_server）与托盘菜单共用的
//! 启停入口：监听 host/port 来自共享设置（`Arc<RwLock<GatewaySettings>>`），保存设置后即时生效。

use std::sync::{Arc, RwLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::domain::settings::GatewaySettings;
use crate::interface::http::handlers::AppState;
use crate::interface::http::router::build_router;
use crate::interface::http::server::ServerManager;

/// 服务运行状态载荷：命令返回值与 `server-started` / `server-stopped` 事件共用同一结构，
/// 保证前端 store 只消费一种形状。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub running: bool,
    pub host: Option<String>,
    pub port: Option<u16>,
}

impl ServerStatus {
    pub(crate) fn from_addr(addr: std::net::SocketAddr) -> Self {
        Self {
            running: true,
            host: Some(addr.ip().to_string()),
            port: Some(addr.port()),
        }
    }

    pub(crate) fn stopped() -> Self {
        Self {
            running: false,
            host: None,
            port: None,
        }
    }
}

/// 查询服务当前运行状态（前端首次挂载可用来校准初始 UI，常规刷新仍依赖事件）。
#[tauri::command]
pub fn get_server_status(server: tauri::State<'_, ServerManager>) -> ServerStatus {
    match server.addr() {
        Some(addr) => ServerStatus::from_addr(addr),
        None => ServerStatus::stopped(),
    }
}

/// 解析 host/port：显式入参优先 → 共享设置 → 默认设置（共享设置由设置页保存后即时更新）。
/// 命令（start_server）与托盘启停 / setup 自启共用，避免监听地址取值逻辑分散。
pub(crate) fn resolve_host_port(
    app: &AppHandle,
    host: Option<String>,
    port: Option<u16>,
) -> (String, u16) {
    let defaults = GatewaySettings::default();
    let (shared_host, shared_port) = app
        .try_state::<Arc<RwLock<GatewaySettings>>>()
        .map(|shared| {
            let s = shared.read().expect("settings lock poisoned");
            (s.host.clone(), s.port)
        })
        .unwrap_or((defaults.host, defaults.port));
    (host.unwrap_or(shared_host), port.unwrap_or(shared_port))
}

/// 启动 HTTP 服务（命令与托盘菜单共用入口）。
/// 启动成功后先广播 `server-started`（携带实际监听地址），再返回同一状态。
/// 路由树在启动时从数据面 AppState（managed state）现构建，保证每次启动用最新仓储装配。
/// 广播失败即回滚：事件是状态同步的唯一通道，不允许“服务已启动但无人知晓”。
pub(crate) async fn start_gateway(
    app: &AppHandle,
    host: &str,
    port: u16,
) -> Result<ServerStatus, String> {
    let router = build_router(app.state::<AppState>().inner().clone());
    let addr = app
        .state::<ServerManager>()
        .start(host, port, router)
        .await
        .map_err(|e| e.to_string())?;
    let status = ServerStatus::from_addr(addr);
    if let Err(err) = app.emit("server-started", status.clone()) {
        let _ = app.state::<ServerManager>().stop().await;
        return Err(err.to_string());
    }
    Ok(status)
}

/// 停止 HTTP 服务（命令与托盘菜单共用入口）：优雅停机完成后广播 `server-stopped`。
pub(crate) async fn stop_gateway(app: &AppHandle) -> Result<ServerStatus, String> {
    app.state::<ServerManager>()
        .stop()
        .await
        .map_err(|e| e.to_string())?;
    let status = ServerStatus::stopped();
    app.emit("server-stopped", status.clone())
        .map_err(|e| e.to_string())?;
    Ok(status)
}

/// 启动 HTTP 服务。host/port 缺省时取共享设置（保存设置后即时生效；端口 0 = 随机）。
#[tauri::command]
pub async fn start_server(
    app: tauri::AppHandle,
    host: Option<String>,
    port: Option<u16>,
) -> Result<ServerStatus, String> {
    let (host, port) = resolve_host_port(&app, host, port);
    start_gateway(&app, &host, port).await
}

/// 停止 HTTP 服务（优雅停机完成后返回），并广播 `server-stopped`。
#[tauri::command]
pub async fn stop_server(app: tauri::AppHandle) -> Result<ServerStatus, String> {
    stop_gateway(&app).await
}
