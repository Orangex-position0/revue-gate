// Library entry: Tauri Builder + command registration + startup assembly (SQLite pool, settings, HTTP service autostart and status events, tray).
// Layering: see docs/Architecture-backend.md. Modules declared pub so seam B integration tests reference them via the lib public API.
pub mod domain;
pub mod infrastructure;
pub mod interface;
pub mod protocol;
#[cfg(test)]
mod test_support;
pub mod usecases;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use infrastructure::providers::adaptor_for;
use infrastructure::sqlite::api_key::SqliteApiKeyRepository;
use infrastructure::sqlite::channel::SqliteChannelRepository;
use infrastructure::sqlite::knowledge::SqliteKnowledgeRepository;
use infrastructure::sqlite::request_log::SqliteRequestLogRepository;
use infrastructure::store::StoreSettingsRepository;
use interface::commands::api_key::{
    create_api_key, delete_api_key, list_api_keys, set_api_key_enabled, update_api_key,
};
use interface::commands::channel::{
    create_channel, delete_channel, fetch_channel_models, list_channels, set_channel_enabled,
    test_channel, update_channel,
};
use interface::commands::knowledge::{
    create_knowledge_base, create_knowledge_source, delete_knowledge_base, get_knowledge_base,
    list_knowledge_bases, list_knowledge_documents, list_knowledge_sources, update_knowledge_base,
    upload_knowledge_document,
};
use interface::commands::log::{clear_logs, delete_logs_before, get_log_detail, list_logs};
use interface::commands::server::{
    get_server_status, resolve_host_port, start_gateway, start_server, stop_gateway, stop_server,
};
use interface::commands::services::get_service_statuses;
use interface::commands::settings::{apply_autostart, get_settings, save_settings};
use interface::commands::stats::{get_stats, usage_stats};
use interface::http::handlers::AppState;
use interface::http::server::ServerManager;
#[cfg(target_os = "macos")]
use tauri::RunEvent;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};

use crate::domain::api_key::ApiKeyRepository;
use crate::domain::channel::{Channel, ChannelRepository};
use crate::domain::request_log::RequestLogRepository;
use crate::domain::settings::{GatewaySettings, SettingsRepository};
use crate::usecases::proxy::ProxyRequestUsecase;

/// Tray "Quit" flag: closing-to-tray intercepts CloseRequested, so the quit path must set this first.
static FORCE_QUIT: AtomicBool = AtomicBool::new(false);

/// Restore the main window from explicit user actions (tray click/menu or macOS Dock reopen).
fn restore_main_window(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.show();
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Build the tray icon and menu (Show window / Start service / Stop service / Quit); left click shows the main window.
/// Start/stop reuse the command-layer `start_gateway` / `stop_gateway` (host/port read from shared settings, effective immediately after save).
fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", "启动服务", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "停止服务", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &start, &stop, &quit])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundle icon is configured in tauri.conf.json");

    TrayIconBuilder::with_id("main")
        .menu(&menu)
        .icon(icon)
        .tooltip("revue-gate")
        // On Windows a left click does not pop the menu; on_tray_icon_event shows the main window instead.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                restore_main_window(app);
            }
            "start" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let (host, port) = resolve_host_port(&app, None, None);
                    if let Err(err) = start_gateway(&app, &host, port).await {
                        tracing::error!(error = %err, "tray: failed to start http server");
                    }
                });
            }
            "stop" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = stop_gateway(&app).await {
                        tracing::error!(error = %err, "tray: failed to stop http server");
                    }
                });
            }
            "quit" => {
                FORCE_QUIT.store(true, Ordering::Relaxed);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                restore_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Avoid re-initializing an existing subscriber (e.g. injected by a debugger/host): use try_init instead of init to avoid a panic.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            greet,
            get_server_status,
            start_server,
            stop_server,
            get_service_statuses,
            get_settings,
            save_settings,
            list_channels,
            create_channel,
            update_channel,
            delete_channel,
            set_channel_enabled,
            test_channel,
            fetch_channel_models,
            list_api_keys,
            create_api_key,
            update_api_key,
            delete_api_key,
            set_api_key_enabled,
            list_logs,
            get_log_detail,
            delete_logs_before,
            clear_logs,
            get_stats,
            usage_stats,
            list_knowledge_bases,
            get_knowledge_base,
            create_knowledge_base,
            update_knowledge_base,
            delete_knowledge_base,
            upload_knowledge_document,
            list_knowledge_documents,
            create_knowledge_source,
            list_knowledge_sources,
        ])
        // Window events: close-to-tray / minimize-to-tray. Events fire only after setup completes,
        // when the shared settings state is registered (try_state as a fallback to avoid a pre-setup panic).
        .on_window_event(|window, event| {
            let app = window.app_handle();
            let Some(shared) = app.try_state::<Arc<RwLock<GatewaySettings>>>() else {
                return;
            };
            let (close_to_tray, minimize_to_tray) = {
                let s = shared.read().expect("settings lock poisoned");
                (s.close_to_tray, s.minimize_to_tray)
            };
            match event {
                // Close-to-tray: intercept the default close and hide the window (allow a real exit once the tray "Quit" flag is set).
                WindowEvent::CloseRequested { api, .. }
                    if close_to_tray && !FORCE_QUIT.load(Ordering::Relaxed) =>
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // Minimize-to-tray: hide to the tray once minimized (a tray click can restore it).
                WindowEvent::Resized(_)
                    if minimize_to_tray && window.is_minimized().unwrap_or(false) =>
                {
                    let _ = window.hide();
                }
                _ => {}
            }
        })
        .setup(|app| {
            // 1) SQLite pool + embedded migrations: failure means startup failure (running without the data layer is pointless).
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("revue-gate.db");
            let db_path = db_path
                .to_str()
                .ok_or("app data dir path is not valid utf8")?;
            let pool = tauri::async_runtime::block_on(infrastructure::sqlite::init_pool(db_path))?;
            app.manage(pool.clone());

            // 1b) Channel/API key repositories into state (command layer accesses via tauri::State).
            app.manage(SqliteChannelRepository::new(pool.clone()));
            app.manage(SqliteApiKeyRepository::new(pool.clone()));
            app.manage(SqliteRequestLogRepository::new(pool.clone()));
            app.manage(SqliteKnowledgeRepository::new(pool.clone()));

            // 1c) Settings: Store repository (settings.json) → load → shared settings into state.
            //     A load failure is not fatal: log it and fall back to defaults (startup must not abort on bad config).
            let store = tauri_plugin_store::StoreBuilder::new(app, "settings.json").build()?;
            let settings_repo: Arc<dyn SettingsRepository> =
                Arc::new(StoreSettingsRepository::new(store));
            let settings = match tauri::async_runtime::block_on(settings_repo.load()) {
                Ok(settings) => settings,
                Err(err) => {
                    tracing::warn!(error = %err, "failed to load settings, using defaults");
                    GatewaySettings::default()
                }
            };
            let shared = Arc::new(RwLock::new(settings));
            app.manage(shared.clone());
            app.manage(settings_repo);

            // 1d) Data-plane AppState: Arc repositories sharing the same pool + the forwarding usecase (shared settings injected so
            //     the proxy reads the retry policy on execute; effective immediately after save).
            let channel_repo: Arc<dyn ChannelRepository> =
                Arc::new(SqliteChannelRepository::new(pool.clone()));
            let api_key_repo: Arc<dyn ApiKeyRepository> =
                Arc::new(SqliteApiKeyRepository::new(pool.clone()));
            let log_repo: Arc<dyn RequestLogRepository> =
                Arc::new(SqliteRequestLogRepository::new(pool.clone()));
            let proxy = Arc::new(ProxyRequestUsecase::new(
                api_key_repo,
                Arc::clone(&channel_repo),
                log_repo,
                Box::new(|channel: &Channel| adaptor_for(channel.channel_type)),
                Arc::clone(&shared),
            ));
            app.manage(AppState {
                proxy,
                channel_repo,
                knowledge_repo: Some(Arc::new(SqliteKnowledgeRepository::new(pool.clone()))),
            });

            // 2) Server manager into state (command layer accesses via tauri::State).
            let server = ServerManager::new();
            app.manage(server);

            // 3) Tray (menu start/stop + left click shows the main window).
            setup_tray(app.handle())?;

            // 4) Apply autostart per settings (no-op when it already matches the OS state).
            let autostart = shared.read().expect("settings lock poisoned").autostart;
            if let Err(err) = apply_autostart(app.handle(), autostart) {
                tracing::warn!(error = %err, "failed to apply autostart on startup");
            }

            // 5) Autostart the HTTP service per settings; host/port come from shared settings (0 = random).
            //    start_gateway already broadcasts server-started internally (the event is the single source of truth), so no duplicate broadcast here.
            //    Startup failure such as a busy port is not fatal: log it; the user can change the port via the control surface / tray and retry.
            let (host, port) = resolve_host_port(app.handle(), None, None);
            if let Err(err) =
                tauri::async_runtime::block_on(start_gateway(app.handle(), &host, port))
            {
                tracing::warn!(error = %err, "http server failed to start at {host}:{port}");
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            {
                if let RunEvent::Reopen { .. } = event {
                    restore_main_window(app);
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (app, &event);
            }
        });
}
