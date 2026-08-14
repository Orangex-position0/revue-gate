//! Control plane: service lifecycle commands (start/stop/status) + state event broadcast.
//!
//! Commands are thin glue: drive the `ServerManager` lifecycle and broadcast the result as
//! `server-started` / `server-stopped` events to the frontend. Events are the single source
//! of truth for runtime state. `start_gateway` / `stop_gateway` are the shared start/stop entry
//! used by both the commands (start_server / stop_server) and the tray menu: listen host/port
//! come from shared settings (`Arc<RwLock<GatewaySettings>>`), effective immediately after save.

use std::sync::{Arc, RwLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::domain::settings::GatewaySettings;
use crate::interface::http::handlers::AppState;
use crate::interface::http::router::build_router;
use crate::interface::http::server::ServerManager;

/// Server run-status payload: the command return value and the `server-started` / `server-stopped`
/// events share one structure, so the frontend store consumes a single shape.
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

/// Query the current server run status (the frontend can calibrate its initial UI on first mount;
/// regular refreshes still rely on events).
#[tauri::command]
pub fn get_server_status(server: tauri::State<'_, ServerManager>) -> ServerStatus {
    match server.addr() {
        Some(addr) => ServerStatus::from_addr(addr),
        None => ServerStatus::stopped(),
    }
}

/// Resolve host/port: explicit args → shared settings → defaults (shared settings update immediately
/// after save on the settings page). Shared by the command (start_server), tray start/stop, and
/// setup auto-start, keeping listen-address resolution in one place.
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

/// Start the HTTP service (shared entry for the command and the tray menu).
/// On success, first broadcast `server-started` (carrying the actual listen address), then return the same status.
/// The route tree is built at startup from the data-plane AppState (managed state), so each start uses fresh repository wiring.
/// A failed broadcast rolls back: events are the only channel for state sync, so a started-but-unknown service is not allowed.
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

/// Stop the HTTP service (shared entry for the command and the tray menu): after graceful shutdown completes, broadcast `server-stopped`.
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

/// Start the HTTP service. host/port default to shared settings when absent (effective immediately after save; port 0 = random).
#[tauri::command]
pub async fn start_server(
    app: tauri::AppHandle,
    host: Option<String>,
    port: Option<u16>,
) -> Result<ServerStatus, String> {
    let (host, port) = resolve_host_port(&app, host, port);
    start_gateway(&app, &host, port).await
}

/// Stop the HTTP service (returns after graceful shutdown completes) and broadcast `server-stopped`.
#[tauri::command]
pub async fn stop_server(app: tauri::AppHandle) -> Result<ServerStatus, String> {
    stop_gateway(&app).await
}
