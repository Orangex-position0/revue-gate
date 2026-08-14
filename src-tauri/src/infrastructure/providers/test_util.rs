//! Test support for provider adapters: starts an axum mock server on a random 127.0.0.1 port for real reqwest to hit.
//! Compiles in tests only (`#[cfg(test)] mod test_util` in `providers.rs`).

use axum::Router;
use uuid::Uuid;

use crate::domain::channel::{Channel, ChannelType};

/// Starts an axum router on 127.0.0.1:0, returning the base URL and the server task handle.
/// Tests build a Channel from `base_url`; the adapter's real reqwest requests hit this mock.
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

/// Builds a Channel pointing at the mock server: `base_url` overrides the default; api_key uses a fixed test value.
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
