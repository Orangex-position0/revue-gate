// 库入口：Tauri Builder + 命令注册 + 启动装配（SQLite 连接池、HTTP 服务自启与状态事件）。
// 分层见 docs/Architecture-backend.md。模块声明为 pub：seam B 集成测试经 lib 公共 API 引用。
pub mod domain;
pub mod infrastructure;
pub mod interface;
#[cfg(test)]
mod test_support;

use interface::commands::server::{
    DEFAULT_HOST, DEFAULT_PORT, ServerStatus, get_server_status, start_server, stop_server,
};
use interface::http::server::ServerManager;
use tauri::{Emitter, Manager};

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            get_server_status,
            start_server,
            stop_server
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
            app.manage(pool);

            // 2) 服务管理器入 state（命令层经 tauri::State 访问）。
            let server = ServerManager::new();
            app.manage(server);

            // 3) 自动启动 HTTP 服务并广播 server-started（事件是唯一权威源）。
            //    端口被占用等启动失败不致命：记录日志，用户可经控制面改端口后重试。
            match tauri::async_runtime::block_on(
                app.state::<ServerManager>()
                    .start(DEFAULT_HOST, DEFAULT_PORT),
            ) {
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
