use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};

use axum::body::Body;
use axum::extract::Json;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use axum::{Router, serve};
use chrono::Utc;
use revue_gate_lib::domain::api_key::{ApiKey, ApiKeyRepository, Quota, generate_local_key};
use revue_gate_lib::domain::channel::{Channel, ChannelRepository, ChannelType};
use revue_gate_lib::domain::request_log::RequestLogRepository;
use revue_gate_lib::domain::settings::GatewaySettings;
use revue_gate_lib::infrastructure::providers::adaptor_for;
use revue_gate_lib::infrastructure::sqlite::api_key::SqliteApiKeyRepository;
use revue_gate_lib::infrastructure::sqlite::channel::SqliteChannelRepository;
use revue_gate_lib::infrastructure::sqlite::init_pool;
use revue_gate_lib::infrastructure::sqlite::request_log::SqliteRequestLogRepository;
use revue_gate_lib::interface::http::handlers::AppState;
use revue_gate_lib::interface::http::router::build_router;
use revue_gate_lib::usecases::proxy::ProxyRequestUsecase;
use serde_json::{Value, json};
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;

struct Harness {
    pool: sqlx::SqlitePool,
    channel_repo: Arc<SqliteChannelRepository>,
    api_key_repo: Arc<SqliteApiKeyRepository>,
    log_repo: Arc<SqliteRequestLogRepository>,
}

async fn harness() -> Harness {
    let pool = init_pool("sqlite::memory:").await.expect("init pool");
    let channel_repo = Arc::new(SqliteChannelRepository::new(pool.clone()));
    let api_key_repo = Arc::new(SqliteApiKeyRepository::new(pool.clone()));
    let log_repo = Arc::new(SqliteRequestLogRepository::new(pool.clone()));
    Harness {
        pool,
        channel_repo,
        api_key_repo,
        log_repo,
    }
}

fn app(h: &Harness) -> Router {
    let usecase = ProxyRequestUsecase::new(
        Arc::clone(&h.api_key_repo) as Arc<dyn ApiKeyRepository>,
        Arc::clone(&h.channel_repo) as Arc<dyn ChannelRepository>,
        Arc::clone(&h.log_repo) as Arc<dyn RequestLogRepository>,
        Box::new(|c: &Channel| adaptor_for(c.channel_type)),
        Arc::new(RwLock::new(GatewaySettings::default())),
    );
    build_router(
        AppState {
            proxy: Arc::new(usecase),
            channel_repo: Arc::clone(&h.channel_repo) as Arc<dyn ChannelRepository>,
        },
        &GatewaySettings::default(),
    )
}

async fn seed(h: &Harness, base_url: &str) -> String {
    h.channel_repo
        .save(&Channel {
            id: Uuid::now_v7(),
            name: "openai".to_string(),
            channel_type: ChannelType::OpenAi,
            base_url: Some(base_url.to_string()),
            api_key: Some("sk-upstream".to_string()),
            models: vec!["gpt-4o".to_string()],
            priority: 0,
            weight: 1,
            model_mappings: Vec::new(),
            enabled: true,
            last_test_at: None,
            last_test_ok: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("save channel");
    let key = generate_local_key();
    h.api_key_repo
        .save(&ApiKey {
            id: Uuid::now_v7(),
            name: "client".to_string(),
            key: key.clone(),
            enabled: true,
            quota: Quota {
                limit: None,
                used: 0,
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("save key");
    key
}

async fn spawn_upstream(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        serve(listener, router).await.expect("serve");
    });
    (format!("http://{addr}"), handle)
}

fn chat_response() -> Value {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1700000000,
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "hello"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
    })
}

#[tokio::test]
async fn anthropic_messages_non_stream_converts_and_logs_canonical_shapes() {
    let h = harness().await;
    let received: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&received);
    let upstream = Router::new().route(
        "/chat/completions",
        post(move |body: Json<Value>| async move {
            *probe.lock().unwrap() = Some(body.0);
            Json(chat_response())
        }),
    );
    let (base, handle) = spawn_upstream(upstream).await;
    let key = seed(&h, &base).await;

    let response = app(&h)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-api-key", &key)
                .header("x-request-id", "trace-anthropic")
                .body(Body::from(
                    r#"{"model":"gpt-4o","max_tokens":64,"system":"be brief","messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    handle.abort();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .expect("json");
    assert_eq!(body["type"], "message");
    assert_eq!(body["content"][0]["text"], "hello");

    let sent = received.lock().unwrap().clone().expect("upstream body");
    assert_eq!(
        sent["messages"][0],
        json!({"role": "system", "content": "be brief"})
    );
    assert_eq!(
        sent["messages"][1],
        json!({"role": "user", "content": "hi"})
    );

    let row = sqlx::query("SELECT trace_id, request_body, response_choices FROM request_logs")
        .fetch_one(&h.pool)
        .await
        .expect("log row");
    assert_eq!(row.get::<String, _>("trace_id"), "trace-anthropic");
    let request_body: Value =
        serde_json::from_str(&row.get::<String, _>("request_body")).expect("request body json");
    assert_eq!(request_body["messages"][0]["role"], "system");
    let response_choices: Value =
        serde_json::from_str(&row.get::<String, _>("response_choices")).expect("choices json");
    assert_eq!(response_choices[0]["message"]["content"], "hello");
}

#[tokio::test]
async fn responses_non_stream_requires_bearer_and_returns_response_shape() {
    let h = harness().await;
    let upstream = Router::new().route(
        "/chat/completions",
        post(|| async { Json(chat_response()) }),
    );
    let (base, handle) = spawn_upstream(upstream).await;
    let key = seed(&h, &base).await;

    let unauthorized = app(&h)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-api-key", &key)
                .body(Body::from(r#"{"model":"gpt-4o","input":"hi"}"#))
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let response = app(&h)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .body(Body::from(
                    r#"{"model":"gpt-4o","instructions":"be brief","input":"hi"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    handle.abort();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .expect("json");
    assert_eq!(body["object"], "response");
    assert_eq!(body["output_text"], "hello");
}

#[tokio::test]
async fn chat_completions_non_success_response_stays_passthrough() {
    let h = harness().await;
    let upstream = Router::new().route(
        "/chat/completions",
        post(|| async {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": {"message": "masked upstream", "type": "upstream_error"}})),
            )
        }),
    );
    let (base, handle) = spawn_upstream(upstream).await;
    let key = seed(&h, &base).await;

    let response = app(&h)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .body(Body::from(
                    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("oneshot");

    handle.abort();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .expect("json");
    assert_eq!(body["error"]["type"], "upstream_error");
    assert_eq!(
        body["error"]["message"],
        "upstream request failed with status 400"
    );
}

#[tokio::test]
async fn protocol_streaming_endpoints_transform_sse() {
    let h = harness().await;
    let payload = concat!(
        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\n",
        "data: [DONE]\n\n",
    );
    let upstream = Router::new().route(
        "/chat/completions",
        post(move || async move {
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/event-stream")],
                Body::from(payload),
            )
        }),
    );
    let (base, handle) = spawn_upstream(upstream).await;
    let key = seed(&h, &base).await;

    let anthropic = app(&h)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-api-key", &key)
                .body(Body::from(
                    r#"{"model":"gpt-4o","max_tokens":64,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("anthropic oneshot");
    assert_eq!(anthropic.status(), StatusCode::OK);
    let anthropic_text = String::from_utf8(
        axum::body::to_bytes(anthropic.into_body(), usize::MAX)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("utf8");
    assert!(anthropic_text.contains("event: message_start"));
    assert!(anthropic_text.contains("event: message_stop"));
    assert!(!anthropic_text.contains("[DONE]"));

    let responses = app(&h)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/responses")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {key}"))
                .body(Body::from(
                    r#"{"model":"gpt-4o","stream":true,"input":"hi"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("responses oneshot");
    assert_eq!(responses.status(), StatusCode::OK);
    let responses_text = String::from_utf8(
        axum::body::to_bytes(responses.into_body(), usize::MAX)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("utf8");
    handle.abort();
    assert!(responses_text.contains("event: response.created"));
    assert!(responses_text.contains("event: response.completed"));
    assert!(!responses_text.contains("[DONE]"));

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM request_logs WHERE response_choices IS NOT NULL")
            .fetch_one(&h.pool)
            .await
            .expect("count logs");
    assert_eq!(count, 2);
}
