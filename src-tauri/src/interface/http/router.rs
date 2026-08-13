//! 数据面：Axum 路由定义。骨架阶段仅 `/health`；`/v1/*` 随阶段 05/08/09 加入。

use axum::Router;
use axum::routing::get;

/// 组装数据面路由树。
pub fn build_router() -> Router {
    Router::new().route("/health", get(health))
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
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::build_router;

    /// seam B：`/health` 应返回 200 OK（oneshot 集成测试，不启真实端口）。
    #[tokio::test]
    async fn health_returns_200() {
        let app = build_router();
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
        let app = build_router();
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
