//! 数据面：Axum 路由树 + trace 贯穿层（见 docs/Architecture-backend.md）。
//!
//! `/v1/*` 路由挂载到 `AppState`（真实用例），`/health` 为静态探活端点。
//! 中间件顺序（最外层在前）：`trace_id_middleware`（生成/沿用 x-request-id）→
//! `TraceLayer`（span 携带 trace_id + 响应后结构化日志）。layer 调用顺序与包裹顺序相反：
//! 后添加的 layer 在外层，故 trace_id_middleware 最后添加（先生成 trace id，span 才能读到）。

use axum::Router;
use axum::middleware::from_fn;
use axum::routing::{get, post};
use tower_http::trace::TraceLayer;

use super::handlers::{
    AppState, TraceIdSpan, TraceOnResponse, chat_completions, models, trace_id_middleware,
};

/// 组装数据面路由树：/health + /v1/chat/completions + /v1/models，并挂 trace 贯穿层。
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/models", get(models))
        .with_state(state)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(TraceIdSpan)
                .on_response(TraceOnResponse),
        )
        .layer(from_fn(trace_id_middleware))
}

/// 健康检查：返回 200 与 `{"status":"ok"}`，供客户端确认网关在线。
async fn health() -> axum::Json<Health> {
    axum::Json(Health { status: "ok" })
}

#[derive(serde::Serialize)]
struct Health {
    status: &'static str,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;
    use crate::domain::api_key::ApiKeyRepository;
    use crate::domain::channel::{Channel, ChannelRepository};
    use crate::domain::provider::TestResult;
    use crate::domain::request_log::RequestLogRepository;
    use crate::test_support::{
        InMemoryApiKeyRepository, InMemoryChannelRepository, InMemoryRequestLogRepository,
        MockProviderAdaptor,
    };
    use crate::usecases::proxy::ProxyRequestUsecase;

    /// 最小 AppState：内存仓储 + 不会被调用的 stub 适配器（/health 与 404 不触发转发）。
    fn minimal_state() -> AppState {
        let keys: Arc<dyn ApiKeyRepository> = Arc::new(InMemoryApiKeyRepository::new());
        let channels: Arc<dyn ChannelRepository> = Arc::new(InMemoryChannelRepository::new());
        let logs: Arc<dyn RequestLogRepository> = Arc::new(InMemoryRequestLogRepository::new());
        let usecase = ProxyRequestUsecase::new(
            keys,
            Arc::clone(&channels),
            logs,
            Box::new(|_: &Channel| {
                Box::new(MockProviderAdaptor::new(TestResult {
                    ok: true,
                    latency_ms: 0,
                    error: None,
                }))
            }),
        );
        AppState {
            proxy: Arc::new(usecase),
            channel_repo: channels,
        }
    }

    /// seam B：`/health` 应返回 200 OK（oneshot 集成测试，不启真实端口）。
    #[tokio::test]
    async fn health_returns_200() {
        let app = build_router(minimal_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// seam B：未知路由应返回 404。
    #[tokio::test]
    async fn unknown_route_returns_404() {
        let app = build_router(minimal_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/nope")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
