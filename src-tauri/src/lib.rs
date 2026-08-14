// 库入口：Tauri Builder + 命令注册 + 启动装配（SQLite 连接池、设置、HTTP 服务自启与状态事件、托盘）。
// 分层见 docs/Architecture-backend.md。模块声明为 pub：seam B 集成测试经 lib 公共 API 引用。
pub mod domain;
pub mod infrastructure;
pub mod interface;
#[cfg(test)]
mod test_support;
pub mod usecases;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use infrastructure::providers::adaptor_for;
use infrastructure::sqlite::api_key::SqliteApiKeyRepository;
use infrastructure::sqlite::channel::SqliteChannelRepository;
use infrastructure::sqlite::request_log::SqliteRequestLogRepository;
use infrastructure::store::StoreSettingsRepository;
use interface::commands::api_key::{
    create_api_key, delete_api_key, list_api_keys, set_api_key_enabled, update_api_key,
};
use interface::commands::channel::{
    create_channel, delete_channel, list_channels, set_channel_enabled, test_channel,
    update_channel,
};
use interface::commands::log::{clear_logs, delete_logs_before, get_log_detail, list_logs};
use interface::commands::server::{
    get_server_status, resolve_host_port, start_gateway, start_server, stop_gateway, stop_server,
};
use interface::commands::settings::{apply_autostart, get_settings, save_settings};
use interface::commands::stats::get_stats;
use interface::http::handlers::AppState;
use interface::http::server::ServerManager;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};

use crate::domain::api_key::ApiKeyRepository;
use crate::domain::channel::{Channel, ChannelRepository};
use crate::domain::request_log::RequestLogRepository;
use crate::domain::settings::{GatewaySettings, SettingsRepository};
use crate::usecases::proxy::ProxyRequestUsecase;

/// 托盘「退出」置位：关闭到托盘会拦截 CloseRequested，退出路径需先置位再放行。
static FORCE_QUIT: AtomicBool = AtomicBool::new(false);

/// 构建托盘图标与菜单（启动服务 / 停止服务 / 退出），左键单击唤出主窗口。
/// 启停复用命令层 `start_gateway` / `stop_gateway`（host/port 读共享设置，保存后即时生效）。
fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let start = MenuItem::with_id(app, "start", "启动服务", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "停止服务", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&start, &stop, &quit])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundle icon is configured in tauri.conf.json");

    TrayIconBuilder::with_id("main")
        .menu(&menu)
        .icon(icon)
        .tooltip("revue-gate")
        // Windows 左键单击不弹菜单：交给 on_tray_icon_event 唤出主窗口。
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
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
                && let Some(window) = tray.app_handle().get_webview_window("main")
            {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
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
    // 已存在 subscriber（如调试器/宿主注入）时不重复初始化：try_init 而非 init，避免 panic。
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
            get_settings,
            save_settings,
            list_channels,
            create_channel,
            update_channel,
            delete_channel,
            set_channel_enabled,
            test_channel,
            list_api_keys,
            create_api_key,
            update_api_key,
            delete_api_key,
            set_api_key_enabled,
            list_logs,
            get_log_detail,
            delete_logs_before,
            clear_logs,
            get_stats
        ])
        // 窗口事件：关闭到托盘 / 最小化到托盘。事件在 setup 完成后才触发，
        // 届时共享设置 state 已注册（try_state 兜底，避免 setup 前事件 panic）。
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
                // 关闭到托盘：拦截默认关闭并隐藏窗口（托盘「退出」置位后放行真退出）。
                WindowEvent::CloseRequested { api, .. }
                    if close_to_tray && !FORCE_QUIT.load(Ordering::Relaxed) =>
                {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // 最小化到托盘：最小化完成即隐藏到托盘（托盘单击可唤回）。
                WindowEvent::Resized(_)
                    if minimize_to_tray && window.is_minimized().unwrap_or(false) =>
                {
                    let _ = window.hide();
                }
                _ => {}
            }
        })
        .setup(|app| {
            // 1) SQLite 连接池 + 内嵌迁移：失败即启动失败（业务数据层不可用则无意义运行）。
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("revue-gate.db");
            let db_path = db_path
                .to_str()
                .ok_or("app data dir path is not valid utf8")?;
            let pool = tauri::async_runtime::block_on(infrastructure::sqlite::init_pool(db_path))?;
            app.manage(pool.clone());

            // 1b) 渠道/密钥仓储入 state（命令层经 tauri::State 访问）。
            app.manage(SqliteChannelRepository::new(pool.clone()));
            app.manage(SqliteApiKeyRepository::new(pool.clone()));
            app.manage(SqliteRequestLogRepository::new(pool.clone()));

            // 1c) 设置：Store 仓储（settings.json）→ 加载 → 共享设置入 state。
            //     加载失败不致命：记录日志并回退默认设置（启动不因坏配置中断）。
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

            // 1d) 数据面 AppState：共享同一连接池的 Arc 仓储 + 转发用例（注入共享设置，
            //     代理 execute 时读取重试策略，保存设置后即时生效）。
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
            });

            // 2) 服务管理器入 state（命令层经 tauri::State 访问）。
            let server = ServerManager::new();
            app.manage(server);

            // 3) 托盘（菜单启停 + 左键唤出主窗口）。
            setup_tray(app.handle())?;

            // 4) 开机自启按设置应用（与 OS 当前状态一致时无操作）。
            let autostart = shared.read().expect("settings lock poisoned").autostart;
            if let Err(err) = apply_autostart(app.handle(), autostart) {
                tracing::warn!(error = %err, "failed to apply autostart on startup");
            }

            // 5) 按设置自动启动 HTTP 服务；host/port 来自共享设置（0 = 随机）。
            //    start_gateway 内部已广播 server-started（事件是唯一权威源），此处不再重复广播。
            //    端口被占用等启动失败不致命：记录日志，用户可经控制面 / 托盘改端口后重试。
            let (host, port) = resolve_host_port(app.handle(), None, None);
            if let Err(err) =
                tauri::async_runtime::block_on(start_gateway(app.handle(), &host, port))
            {
                tracing::warn!(error = %err, "http server failed to start at {host}:{port}");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
