//! 数据面：/v1/* 请求处理器 + trace id 贯穿中间件（见 docs/Architecture-backend.md）。
//!
//! 职责分层：
//! - `trace_id_middleware`（最外层）：读取/生成 `x-request-id` → 写入请求扩展，
//!   响应头原样回传，客户端可据此关联链路；
//! - `TraceIdSpan` / `TraceOnResponse`：TraceLayer 的 span 携带 trace_id，响应后打结构化日志；
//! - `chat_completions` / `models`：解析请求 → 调转发 / 模型列表用例 → 错误映射为 HTTP 状态码。
//!
//! 流式请求（`stream: true`）走 SSE 透传：转发用例返回的事件流逐帧写入响应体
//! （`text/event-stream`），`[DONE]` 由上游透传或转换适配器合成，收尾即响应体结束；
//! 记账与日志在用例侧流结束时内联完成。
//! 上游密钥永不回传：上游错误体由适配器收敛为通用错误体（红线，见 providers.rs）。

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tower_http::trace::{MakeSpan, OnResponse};
use tracing::Span;
use uuid::Uuid;

use crate::domain::channel::ChannelRepository;
use crate::usecases::models::ListModelsUsecase;
use crate::usecases::proxy::{ProxyError, ProxyRequest, ProxyRequestUsecase, ProxySuccess};

/// 数据面共享状态：注入转发用例与模型列表所需的仓储（axum State）。
#[derive(Clone)]
pub struct AppState {
    /// 转发闭环用例（认证 → 选渠道 → 映射 → 转发 → 记账 → 日志）。
    pub proxy: Arc<ProxyRequestUsecase>,
    /// 模型列表用例所需的渠道仓储（/v1/models 合并去重）。
    pub channel_repo: Arc<dyn ChannelRepository>,
}

/// 一次请求贯穿的 trace id：中间件写入请求扩展，handler / span / 日志落库共用。
#[derive(Debug, Clone)]
pub struct TraceId(pub String);

/// 客户端可注入的请求 ID 头（沿用则原样回传，未注入则中间件生成）。http crate 未预定义该头名。
static X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// 生成/沿用 trace id 并贯穿请求（最外层中间件，先于 TraceLayer span 创建执行）。
pub(crate) async fn trace_id_middleware(mut request: Request, next: Next) -> Response {
    let trace_id = request
        .headers()
        .get(&X_REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    request.extensions_mut().insert(TraceId(trace_id.clone()));
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&trace_id) {
        response.headers_mut().insert(&X_REQUEST_ID, value);
    }
    response
}

/// TraceLayer span 构造：读取中间件写入的 TraceId 扩展，带进 span 字段。
#[derive(Clone)]
pub struct TraceIdSpan;

impl<B> MakeSpan<B> for TraceIdSpan {
    fn make_span(&mut self, request: &axum::http::Request<B>) -> Span {
        let trace_id = request
            .extensions()
            .get::<TraceId>()
            .map(|t| t.0.as_str())
            .unwrap_or("unknown");
        tracing::info_span!(
            "request",
            method = %request.method(),
            uri = %request.uri(),
            trace_id = %trace_id
        )
    }
}

/// 响应完成后在请求 span 内打结构化日志（状态码 + 耗时）。
#[derive(Clone)]
pub struct TraceOnResponse;

impl<B> OnResponse<B> for TraceOnResponse {
    fn on_response(self, response: &axum::http::Response<B>, latency: Duration, span: &Span) {
        span.in_scope(|| {
            tracing::debug!(
                status = response.status().as_u16(),
                latency_ms = latency.as_millis() as u64,
                "finished processing request"
            );
        });
    }
}

/// POST /v1/chat/completions：认证 → 转发 → 记账 → 日志。
/// 非流式原样透传响应体；流式把转发用例的事件流逐帧写入 SSE 响应体（`[DONE]` 收尾）。
pub(crate) async fn chat_completions(
    State(state): State<AppState>,
    Extension(trace_id): Extension<TraceId>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let body = match body {
        Ok(Json(body)) => body,
        Err(rejection) => return error_response(StatusCode::BAD_REQUEST, &rejection.body_text()),
    };
    let bearer_token = extract_bearer(&headers);
    // 认证优先于功能门：无 Bearer 一律 401（含流式请求，需求“无/无效 Bearer 返回 401”）。
    // 有效性校验仍由转发用例完成（present 但无效 → usecase → 401）。
    if bearer_token.is_none() {
        return error_response(StatusCode::UNAUTHORIZED, "missing bearer token");
    }
    let request = ProxyRequest {
        bearer_token,
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        body,
        trace_id: trace_id.0,
    };
    match state.proxy.execute(request).await {
        Ok(ProxySuccess::NonStream(resp)) => {
            let status =
                StatusCode::from_u16(resp.status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (
                status,
                [(header::CONTENT_TYPE, "application/json")],
                resp.body,
            )
                .into_response()
        }
        Ok(ProxySuccess::Stream(stream)) => {
            // 逐帧透传上游 SSE 字节，帧到达即写（实时）；`[DONE]` 由上游透传或转换适配器合成，
            // 流结束即响应体结束。流中错误：用例侧已写失败日志并终止流，此处停止下发，
            // 客户端见无 `[DONE]` 的截断流（响应头已发出，无法改状态码）。
            let resp_body = Body::from_stream(stream.filter_map(|event| async move {
                match event {
                    Ok(event) => Some(Ok::<_, std::convert::Infallible>(event.data)),
                    Err(_) => None,
                }
            }));
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/event-stream"),
                    (header::CACHE_CONTROL, "no-cache"),
                ],
                resp_body,
            )
                .into_response()
        }
        Err(err) => proxy_error_response(err),
    }
}

/// GET /v1/models：启用渠道的模型合并去重（与 /v1/chat/completions 的可路由模型一致）。
pub(crate) async fn models(State(state): State<AppState>) -> Response {
    match ListModelsUsecase.execute(state.channel_repo.as_ref()).await {
        Ok(models) => {
            let data: Vec<Value> = models
                .into_iter()
                .map(|id| {
                    json!({
                        "id": id,
                        "object": "model",
                        "created": 0,
                        "owned_by": "revue-gate",
                    })
                })
                .collect();
            Json(json!({"object": "list", "data": data})).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list models");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed to list models")
        }
    }
}

/// 从 Authorization 头提取 Bearer 令牌（剥离前缀；无/非法头 → None，由认证用例返回 401）。
fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

/// OpenAI 风格错误响应：`{"error": {"message": ..., "type": ...}}`，type 按状态码取 OpenAI 分类。
fn error_response(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({"error": {"message": message, "type": error_type(status)}})),
    )
        .into_response()
}

/// OpenAI 错误类型分类：下游 SDK 常按 type 分支处理（401→authentication_error、
/// 429→rate_limit_error 等），网关沿用同一分类保证兼容。
fn error_type(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "invalid_request_error",
        StatusCode::UNAUTHORIZED => "authentication_error",
        StatusCode::FORBIDDEN => "permission_error",
        StatusCode::NOT_FOUND => "not_found_error",
        StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
        _ => "api_error",
    }
}

/// 转发错误 → 状态码 + 错误体（401 / 429 / 404 / 502 / 500 / 400 与用例分支一一对应）。
fn proxy_error_response(err: ProxyError) -> Response {
    let status = match &err {
        ProxyError::Unauthorized => StatusCode::UNAUTHORIZED,
        ProxyError::QuotaExceeded => StatusCode::TOO_MANY_REQUESTS,
        ProxyError::NoCandidateChannel(_) => StatusCode::NOT_FOUND,
        ProxyError::NoChannelAvailable { .. } | ProxyError::Provider(_) => StatusCode::BAD_GATEWAY,
        ProxyError::Repository(_) => StatusCode::INTERNAL_SERVER_ERROR,
        ProxyError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
    };
    error_response(status, &err.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::body::{Body, Bytes};
    use axum::extract::Json;
    use axum::http::{Request, StatusCode, header};
    use axum::routing::post;
    use serde_json::Value;
    use sqlx::Row;
    use tower::ServiceExt;

    use super::*;
    use crate::domain::api_key::{ApiKeyRepository, Quota};
    use crate::domain::channel::{Channel, ChannelRepository, ChannelType};
    use crate::infrastructure::providers::adaptor_for;
    use crate::infrastructure::providers::test_util;
    use crate::infrastructure::sqlite::api_key::SqliteApiKeyRepository;
    use crate::infrastructure::sqlite::channel::SqliteChannelRepository;
    use crate::infrastructure::sqlite::init_pool;
    use crate::infrastructure::sqlite::request_log::SqliteRequestLogRepository;
    use crate::interface::http::router::build_router;
    use crate::test_support::sample_api_key;

    /// OpenAI 兼容非流式响应样本（含 usage），供 mock 上游返回。
    fn chat_completion_response() -> Value {
        serde_json::json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hello"},
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
        })
    }

    /// seam B 测试装置：真实 SQLite（内存）+ 真实 sqlx 仓储 + 真实适配器。
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

    /// 组装数据面路由（真实用例 + 真实适配器解析，渠道 base_url 指向 mock 上游）。
    fn app(h: &Harness) -> Router {
        let usecase = ProxyRequestUsecase::new(
            Arc::clone(&h.api_key_repo) as Arc<dyn ApiKeyRepository>,
            Arc::clone(&h.channel_repo) as Arc<dyn ChannelRepository>,
            Arc::clone(&h.log_repo) as Arc<dyn crate::domain::request_log::RequestLogRepository>,
            Box::new(|c: &Channel| adaptor_for(c.channel_type)),
        );
        let state = AppState {
            proxy: Arc::new(usecase),
            channel_repo: Arc::clone(&h.channel_repo) as Arc<dyn ChannelRepository>,
        };
        build_router(state)
    }

    /// 保存指向 mock 上游的启用渠道（模型 gpt-4o）+ 一条有效本地密钥，返回密钥明文。
    async fn seed_channel_and_key(h: &Harness, base_url: &str) -> String {
        let mut channel = test_util::test_channel(ChannelType::OpenAi, base_url);
        channel.models = vec!["gpt-4o".to_string()];
        h.channel_repo.save(&channel).await.expect("save channel");
        let key = sample_api_key();
        h.api_key_repo.save(&key).await.expect("save key");
        key.key
    }

    fn post_chat(body: &str, bearer: Option<&str>, trace_id: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-request-id", trace_id);
        if let Some(bearer) = bearer {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
        }
        builder
            .body(Body::from(body.to_string()))
            .expect("build request")
    }

    /// 捕获 fmt subscriber 输出到共享缓冲的 MakeWriter（结构化日志测试装置）。
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl tracing_subscriber::fmt::MakeWriter<'_> for Capture {
        type Writer = Capture;
        fn make_writer(&self) -> Self::Writer {
            self.clone()
        }
    }

    /// seam B 闭环：真实网络转发到 mock 上游 → 响应原样透传，请求日志落库（trace_id 一致），
    /// 配额按 usage total 累加，`x-request-id` 原样回传。
    #[tokio::test]
    async fn chat_completions_forwards_records_log_accumulates_quota_and_echoes_trace() {
        let h = harness().await;

        // mock 上游：记录收到的请求体并返回带 usage 的完成响应。
        let received: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let probe = received.clone();
        let upstream = Router::new().route(
            "/chat/completions",
            post(move |body: Json<Value>| async move {
                *probe.lock().unwrap() = Some(body.0);
                Json(chat_completion_response())
            }),
        );
        let (base, handle) = test_util::spawn(upstream).await;

        let key = seed_channel_and_key(&h, &base).await;
        let app = app(&h);

        let response = app
            .oneshot(post_chat(
                r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
                Some(&key),
                "trace-123",
            ))
            .await
            .expect("oneshot");

        handle.abort();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-request-id")
                .map(|v| v.to_str().unwrap()),
            Some("trace-123"),
            "trace id 应原样回传给客户端"
        );

        // 上游收到改写后的请求体（model 直传）。
        let sent = received
            .lock()
            .unwrap()
            .clone()
            .expect("upstream received body");
        assert_eq!(sent["model"], "gpt-4o");

        // 响应体原样透传（choices 内容来自上游）。
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read body"),
        )
        .expect("json");
        assert_eq!(body["choices"][0]["message"]["content"], "hello");

        // 请求日志落库：trace_id / 状态码 / 模型 / usage 与请求一致。
        let row =
            sqlx::query("SELECT trace_id, status_code, model, total_tokens FROM request_logs")
                .fetch_one(&h.pool)
                .await
                .expect("request_log row");
        assert_eq!(row.get::<String, _>(0), "trace-123");
        assert_eq!(row.get::<i64, _>(1), 200);
        assert_eq!(row.get::<String, _>(2), "gpt-4o");
        assert_eq!(row.get::<i64, _>(3), 15);

        // 配额按 usage total 累加。
        let saved = h
            .api_key_repo
            .find_by_key(&key)
            .await
            .expect("find")
            .expect("found");
        assert_eq!(saved.quota.used, 15);
    }

    /// 无 Bearer → 401，不产生请求日志。
    #[tokio::test]
    async fn chat_completions_without_bearer_returns_401() {
        let h = harness().await;
        let app = app(&h);

        let response = app
            .oneshot(post_chat(
                r#"{"model":"gpt-4o","messages":[]}"#,
                None,
                "trace-no-bearer",
            ))
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM request_logs")
            .fetch_one(&h.pool)
            .await
            .expect("count logs");
        assert_eq!(count, 0, "认证失败不写日志");
    }

    /// 无效 Bearer（库中不存在）→ 401。
    #[tokio::test]
    async fn chat_completions_with_invalid_bearer_returns_401() {
        let h = harness().await;
        let app = app(&h);

        let response = app
            .oneshot(post_chat(
                r#"{"model":"gpt-4o","messages":[]}"#,
                Some("sk-revue-ffffffffffffffff"),
                "trace-invalid-bearer",
            ))
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// seam B：流式请求（stream=true）→ 真实 reqwest 打 mock 上游，SSE 帧逐帧透传，
    /// `[DONE]` 收尾正确；流结束后按 usage 记账 + 写成功日志（is_stream=true）。
    #[tokio::test]
    async fn chat_completions_streaming_relays_sse_and_done_and_bills() {
        let h = harness().await;

        // mock 上游：返回完整 OpenAI 兼容 SSE 流（增量帧 + usage 末帧 + [DONE] 收尾）。
        let payload = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",",
            "\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",",
            "\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"}}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",",
            "\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],",
            "\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
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
        let (base, handle) = test_util::spawn(upstream).await;

        let key = seed_channel_and_key(&h, &base).await;
        let app = app(&h);

        let response = app
            .oneshot(post_chat(
                r#"{"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
                Some(&key),
                "trace-stream-1",
            ))
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .map(|v| v.to_str().unwrap()),
            Some("text/event-stream"),
            "SSE 响应应带 text/event-stream"
        );

        // 读完响应体后再关 mock 上游，避免截断仍在传输的流。
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        handle.abort();
        let text = String::from_utf8_lossy(&body);
        assert_eq!(text, payload, "SSE 帧原样透传，不改写增量内容");
        assert!(
            text.ends_with("data: [DONE]\n\n"),
            "[DONE] 收尾正确；实际：{text}"
        );

        // 流结束：usage 记账 + 写成功日志（is_stream=true，total_tokens 取聚合 usage）。
        let row =
            sqlx::query("SELECT trace_id, status_code, is_stream, total_tokens FROM request_logs")
                .fetch_one(&h.pool)
                .await
                .expect("request_log row");
        assert_eq!(row.get::<String, _>(0), "trace-stream-1");
        assert_eq!(row.get::<i64, _>(1), 200);
        assert_eq!(row.get::<i64, _>(2), 1, "is_stream 应标记为真");
        assert_eq!(row.get::<i64, _>(3), 5);

        let saved = h
            .api_key_repo
            .find_by_key(&key)
            .await
            .expect("find")
            .expect("found");
        assert_eq!(saved.quota.used, 5, "流式 usage 累加配额");
    }

    /// seam B：流中途上游连接中断（无 `[DONE]`）→ 客户端见已下发帧原样透传后的截断流
    /// （响应头已发出，状态码保持 200）；用例侧写 502 失败日志并已按增量 usage 记账。
    /// 流式中断与收尾只有该 seam 能可靠覆盖（Spec §Testing Decisions）。
    #[tokio::test]
    async fn chat_completions_streaming_interruption_truncates_and_bills_partial_usage() {
        let h = harness().await;

        // mock 上游：发出增量帧 + usage 帧后，流中途注入错误（连接中断，无 [DONE]）。
        // 每帧后短暂 sleep：确保网关已收到响应头成功打开发流、且帧已被可靠读取后再断连
        // （RST 会丢弃仍在途的字节，故 error 必须滞后于末帧送达）。
        let frame_delta = "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\n";
        let frame_usage = "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n";
        let upstream = Router::new().route(
            "/chat/completions",
            post(move || async move {
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    Body::from_stream(futures_util::stream::unfold(0u8, move |step| async move {
                        match step {
                            0 => Some((
                                Ok::<_, std::io::Error>(Bytes::from_static(frame_delta.as_bytes())),
                                1,
                            )),
                            1 => {
                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                                Some((
                                    Ok::<_, std::io::Error>(Bytes::from_static(
                                        frame_usage.as_bytes(),
                                    )),
                                    2,
                                ))
                            }
                            2 => {
                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                                Some((
                                    Err::<_, std::io::Error>(std::io::Error::other(
                                        "upstream connection reset",
                                    )),
                                    3,
                                ))
                            }
                            _ => None,
                        }
                    })),
                )
            }),
        );
        let (base, handle) = test_util::spawn(upstream).await;

        let key = seed_channel_and_key(&h, &base).await;
        let app = app(&h);

        let response = app
            .oneshot(post_chat(
                r#"{"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
                Some(&key),
                "trace-stream-err",
            ))
            .await
            .expect("oneshot");

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "响应头已发出，流中断无法改为 502"
        );
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .map(|v| v.to_str().unwrap()),
            Some("text/event-stream"),
            "SSE 响应应带 text/event-stream"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        handle.abort();
        let text = String::from_utf8_lossy(&body);
        assert_eq!(
            text,
            format!("{frame_delta}{frame_usage}"),
            "已下发的帧原样透传后截断；实际：{text}"
        );
        assert!(
            !text.contains("[DONE]"),
            "流中断：客户端不应看到 [DONE] 收尾；实际：{text}"
        );

        // 用例侧：502 失败日志（携带增量 usage）+ 按增量 usage 记账。
        let row = sqlx::query(
            "SELECT trace_id, status_code, is_stream, total_tokens, error_message FROM request_logs",
        )
        .fetch_one(&h.pool)
        .await
        .expect("request_log row");
        assert_eq!(row.get::<String, _>(0), "trace-stream-err");
        assert_eq!(
            row.get::<i64, _>(1),
            502,
            "流中断写失败日志（与线上 200 分离）"
        );
        assert_eq!(row.get::<i64, _>(2), 1, "is_stream 应标记为真");
        assert_eq!(row.get::<i64, _>(3), 5, "失败日志携带已聚合的增量 usage");
        assert!(
            row.get::<Option<String>, _>(4).is_some(),
            "流中断应记录失败原因"
        );

        let saved = h
            .api_key_repo
            .find_by_key(&key)
            .await
            .expect("find")
            .expect("found");
        assert_eq!(
            saved.quota.used, 5,
            "流中断时已下发的增量 usage 同样累加配额"
        );
    }

    /// stream=true 且无 Bearer → 401：认证优先于功能门（需求“无 Bearer 返回 401”无流式豁免）。
    #[tokio::test]
    async fn chat_completions_streaming_without_bearer_returns_401() {
        let h = harness().await;
        let app = app(&h);

        let response = app
            .oneshot(post_chat(
                r#"{"model":"gpt-4o","stream":true,"messages":[]}"#,
                None,
                "trace-stream-no-bearer",
            ))
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// seam B：配额超限 → 429（Spec-implementation §Testing Decisions 要求 seam B 覆盖配额）。
    #[tokio::test]
    async fn chat_completions_quota_exceeded_returns_429() {
        let h = harness().await;
        // 渠道指向占位地址即可：配额检查先于选渠道/转发，不会触达上游。
        let key_str = seed_channel_and_key(&h, "https://api.openai.com/v1").await;
        let mut key = h
            .api_key_repo
            .find_by_key(&key_str)
            .await
            .expect("find")
            .expect("found");
        key.quota = Quota {
            limit: Some(0),
            used: 0,
        };
        h.api_key_repo.save(&key).await.expect("save exhausted key");

        let response = app(&h)
            .oneshot(post_chat(
                r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
                Some(&key_str),
                "trace-quota",
            ))
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    /// 结构化日志带 trace id 贯穿（需求 checkbox 4）：TraceLayer span 携带 x-request-id。
    /// 401 请求同样经过 TraceLayer——span 在认证前创建，trace id 应出现在格式化日志里。
    #[tokio::test]
    async fn structured_log_span_carries_trace_id() {
        let h = harness().await;
        let app = app(&h);

        // 安装 fmt subscriber 捕获输出到共享缓冲；全测试进程仅此一处安装全局默认 subscriber。
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_max_level(tracing::Level::DEBUG)
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("only this test installs a global tracing subscriber");

        let _response = app
            .oneshot(post_chat(
                r#"{"model":"gpt-4o","messages":[]}"#,
                None,
                "trace-captured-1",
            ))
            .await
            .expect("oneshot");

        let logs = String::from_utf8(capture.0.lock().unwrap_or_else(|p| p.into_inner()).clone())
            .expect("utf8 logs");
        assert!(
            logs.contains("trace-captured-1"),
            "结构化日志应携带 trace id；实际输出：\n{logs}"
        );
    }

    /// 非法 JSON → 400。
    #[tokio::test]
    async fn chat_completions_malformed_json_returns_400() {
        let h = harness().await;
        let app = app(&h);

        let response = app
            .oneshot(post_chat("not-json", Some("whatever"), "trace-bad-json"))
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// GET /v1/models：仅启用渠道的模型合并去重。
    #[tokio::test]
    async fn models_lists_enabled_channel_models() {
        let h = harness().await;
        let mut a = test_util::test_channel(ChannelType::OpenAi, "https://api.openai.com/v1");
        a.name = "a".to_string();
        a.models = vec!["gpt-4o".to_string(), "claude-3".to_string()];
        h.channel_repo.save(&a).await.expect("save a");
        let mut b = test_util::test_channel(ChannelType::OpenAi, "https://api.openai.com/v1");
        b.name = "b".to_string();
        b.models = vec!["gpt-4o".to_string()];
        b.enabled = false;
        h.channel_repo.save(&b).await.expect("save b");

        let app = app(&h);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read body"),
        )
        .expect("json");
        let ids: Vec<&str> = body["data"]
            .as_array()
            .expect("data array")
            .iter()
            .map(|m| m["id"].as_str().expect("model id"))
            .collect();
        assert_eq!(ids, vec!["gpt-4o", "claude-3"], "禁用渠道的模型被剔除");
    }
}
