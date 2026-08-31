//! Data plane: Axum route tree + trace propagation layer (see docs/Architecture-backend.md).
//!
//! `/v1/*` routes mount onto `AppState` (real usecases); `/health` is a static liveness endpoint.
//! Middleware order (outermost first): `CorsLayer` (browser downstream, e.g. NextChat web) →
//! `trace_id_middleware` (generate/reuse x-request-id) → `TraceLayer` (span carries trace_id +
//! structured logs after the response). Layer call order is the reverse of wrapping order: later
//! layers are outer, so CorsLayer is added last (it short-circuits preflight `OPTIONS` before auth).

use axum::Router;
use axum::middleware::from_fn;
use axum::routing::{get, post};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use super::handlers::{
    AppState, TraceIdSpan, TraceOnResponse, anthropic_messages, chat_completions, models,
    responses, trace_id_middleware,
};
use super::service_modules::ServiceRegistry;
use crate::domain::settings::GatewaySettings;

/// Build the data-plane route tree: /health + /v1/chat/completions + /v1/models, with the trace propagation layer attached.
pub fn build_router(state: AppState, settings: &GatewaySettings) -> Router {
    let service_registry = ServiceRegistry::new();
    service_registry
        .validate()
        .expect("service module registry must be valid");

    Router::new()
        .route("/health", get(health))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/messages", post(anthropic_messages))
        .route("/v1/responses", post(responses))
        .route("/v1/models", get(models))
        .merge(service_registry.merge_routes(&settings.service_modules))
        .with_state(state)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(TraceIdSpan)
                .on_response(TraceOnResponse),
        )
        .layer(from_fn(trace_id_middleware))
        // Outermost: answer CORS for browser-based downstream (NextChat web etc.). `permissive()`
        // = any origin / any method / any header, no credentials — the gateway is bound to 127.0.0.1
        // and guarded by the Bearer key, not by origin; `*` works with fetch + Authorization header
        // (non-credential), and short-circuits preflight `OPTIONS` before it can hit auth.
        .layer(CorsLayer::permissive())
}

/// Health check: returns 200 with `{"status":"ok"}` so clients can confirm the gateway is online.
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

    /// Minimal AppState: in-memory repositories + a stub adapter that is never called (/health and 404 do not trigger forwarding).
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
            Arc::new(std::sync::RwLock::new(
                crate::domain::settings::GatewaySettings::default(),
            )),
        );
        AppState {
            proxy: Arc::new(usecase),
            channel_repo: channels,
        }
    }

    /// seam B: `/health` should return 200 OK (oneshot integration test, no real port bound).
    #[tokio::test]
    async fn health_returns_200() {
        let app = build_router(
            minimal_state(),
            &crate::domain::settings::GatewaySettings::default(),
        );
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

    /// seam B: unknown routes should return 404.
    #[tokio::test]
    async fn unknown_route_returns_404() {
        let app = build_router(
            minimal_state(),
            &crate::domain::settings::GatewaySettings::default(),
        );
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
