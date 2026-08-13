//! 供应商适配器测试支撑：在本机 127.0.0.1 随机端口起 axum mock 服务器，供真实 reqwest 打桩。
//! 仅测试编译（`providers.rs` 中 `#[cfg(test)] mod test_util`）。

use axum::Router;
use uuid::Uuid;

use crate::domain::channel::{Channel, ChannelType};

/// 在 127.0.0.1:0 上启动 axum 路由器，返回基础 URL 与服务器任务句柄。
/// 用例用 `base_url` 构造 Channel，适配器的真实 reqwest 请求打到本 mock。
pub(crate) async fn spawn(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve test server");
    });
    (format!("http://{addr}"), handle)
}

/// 构造指向 mock 服务器的 Channel：`base_url` 覆盖默认值，api_key 用固定测试值。
pub(crate) fn test_channel(channel_type: ChannelType, base_url: &str) -> Channel {
    Channel {
        id: Uuid::now_v7(),
        name: "test".to_string(),
        channel_type,
        base_url: Some(base_url.to_string()),
        api_key: Some("sk-test-upstream".to_string()),
        models: vec!["test-model".to_string()],
        priority: 0,
        weight: 1,
        model_mappings: Vec::new(),
        enabled: true,
        last_test_at: None,
        last_test_ok: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}
