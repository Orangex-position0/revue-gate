// 库入口：Tauri Builder + 命令注册 + 启动装配（SQLite 连接池、HTTP 服务自启与状态事件）。
// 分层见 docs/Architecture-backend.md。模块声明为 pub：seam B 集成测试经 lib 公共 API 引用。
pub mod domain;
pub mod infrastructure;
pub mod interface;
#[cfg(test)]
mod test_support;
pub mod usecases;

use std::sync::Arc;

use infrastructure::providers::adaptor_for;
use infrastructure::sqlite::api_key::SqliteApiKeyRepository;
use infrastructure::sqlite::channel::SqliteChannelRepository;
use infrastructure::sqlite::request_log::SqliteRequestLogRepository;
use interface::commands::api_key::{
    create_api_key, delete_api_key, list_api_keys, set_api_key_enabled, update_api_key,
};
use interface::commands::channel::{
    create_channel, delete_channel, list_channels, set_channel_enabled, test_channel,
    update_channel,
};
use interface::commands::log::{clear_logs, delete_logs_before, get_log_detail, list_logs};
use interface::commands::server::{
    DEFAULT_HOST, DEFAULT_PORT, ServerStatus, get_server_status, start_server, stop_server,
};
use interface::commands::stats::get_stats;
use interface::http::handlers::AppState;
use interface::http::router::build_router;
use interface::http::server::ServerManager;
use tauri::{Emitter, Manager};

use crate::domain::api_key::ApiKeyRepository;
use crate::domain::channel::{Channel, ChannelRepository};
use crate::domain::request_log::RequestLogRepository;
use crate::usecases::proxy::ProxyRequestUsecase;

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
        .invoke_handler(tauri::generate_handler![
            greet,
            get_server_status,
            start_server,
            stop_server,
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

            // 1c) 数据面 AppState：共享同一连接池的 Arc 仓储 + 转发用例。
            //     与命令层仓储是不同实例，但共用 pool → 数据一致；接口也便于 mock（seam A）。
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
            ));
            app.manage(AppState {
                proxy,
                channel_repo,
            });

            // 2) 服务管理器入 state（命令层经 tauri::State 访问）。
            let server = ServerManager::new();
            app.manage(server);

            // 3) 自动启动 HTTP 服务并广播 server-started（事件是唯一权威源）。
            //    端口被占用等启动失败不致命：记录日志，用户可经控制面改端口后重试。
            let router = build_router(app.state::<AppState>().inner().clone());
            match tauri::async_runtime::block_on(app.state::<ServerManager>().start(
                DEFAULT_HOST,
                DEFAULT_PORT,
                router,
            )) {
                Ok(addr) => {
                    app.emit("server-started", ServerStatus::from_addr(addr))?;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "http server failed to start on default port")
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
