//! 控制面：服务生命周期命令（start/stop/status）+ 状态事件广播。
//!
//! 命令只做薄胶水：调用 `ServerManager` 生命周期，并把结果以
//! `server-started` / `server-stopped` 事件广播给前端。事件是运行状态的唯一权威源。

use serde::Serialize;
use tauri::Emitter;

use crate::interface::http::server::ServerManager;

/// 数据面 HTTP 服务默认启动参数（设置中心落地前先固定默认值）。
/// 启动自启（lib.rs）与命令缺省参数（start_server）共用，避免两处漂移。
pub(crate) const DEFAULT_HOST: &str = "127.0.0.1";
pub(crate) const DEFAULT_PORT: u16 = 3000;

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

    fn stopped() -> Self {
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

/// 启动 HTTP 服务。host/port 缺省时使用默认值（127.0.0.1:3000）。
/// 启动成功后先广播 `server-started`（携带实际监听地址），再返回同一状态。
#[tauri::command]
pub async fn start_server(
    app: tauri::AppHandle,
    server: tauri::State<'_, ServerManager>,
    host: Option<String>,
    port: Option<u16>,
) -> Result<ServerStatus, String> {
    let host = host.as_deref().unwrap_or(DEFAULT_HOST);
    let port = port.unwrap_or(DEFAULT_PORT);
    let addr = server.start(host, port).await.map_err(|e| e.to_string())?;
    let status = ServerStatus::from_addr(addr);
    if let Err(err) = app.emit("server-started", status.clone()) {
        // 广播失败即回滚：事件是状态同步的唯一通道，不允许“服务已启动但无人知晓”。
        let _ = server.stop().await;
        return Err(err.to_string());
    }
    Ok(status)
}

/// 停止 HTTP 服务（优雅停机完成后返回），并广播 `server-stopped`。
#[tauri::command]
pub async fn stop_server(
    app: tauri::AppHandle,
    server: tauri::State<'_, ServerManager>,
) -> Result<ServerStatus, String> {
    server.stop().await.map_err(|e| e.to_string())?;
    let status = ServerStatus::stopped();
    app.emit("server-stopped", status.clone())
        .map_err(|e| e.to_string())?;
    Ok(status)
}
