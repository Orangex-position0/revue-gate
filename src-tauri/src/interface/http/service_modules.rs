//! Service Module Registry: framework for extension HTTP capabilities mounted beside the core `/v1/*` gateway.

use std::collections::HashSet;

use axum::Router;
use serde_json::Value;

use crate::domain::settings::ServiceModuleSettings;
use crate::interface::http::handlers::AppState;
use crate::protocol::registry;
use crate::services::mcp::McpService;

pub struct KnowledgeServiceModule;

#[async_trait::async_trait]
impl ServiceModule for KnowledgeServiceModule {
    fn id(&self) -> &'static str {
        "knowledge"
    }
    fn name(&self) -> &'static str {
        "Knowledge Service"
    }
    fn description(&self) -> &'static str {
        "Local knowledge bases, document ingestion, chunking, and retrieval data."
    }
    fn path_prefixes(&self) -> &'static [&'static str] {
        &["/api/kb"]
    }
    async fn get_status(&self, state: &AppState, enabled: bool) -> ServiceModuleStatus {
        use crate::domain::knowledge::KnowledgeRepository;
        let configured = state.knowledge_repo.is_some();
        let result = match &state.knowledge_repo {
            Some(repo) => repo.service_stats().await.ok(),
            None => None,
        };
        let running =
            enabled && configured && result.as_ref().is_some_and(|stats| stats.failed_tasks == 0);
        ServiceModuleStatus {
            id: self.id().into(),
            name: self.name().into(),
            description: self.description().into(),
            path_prefixes: vec!["/api/kb".into()],
            enabled,
            running,
            stats: result
                .and_then(|v| serde_json::to_value(v).ok())
                .unwrap_or_else(|| serde_json::json!({})),
        }
    }
    fn routes(&self) -> Router<AppState> {
        crate::interface::http::knowledge::create_knowledge_router()
    }
}

pub fn production_service_registry() -> ServiceRegistry {
    let mut registry = ServiceRegistry::new();
    registry.register(Box::new(KnowledgeServiceModule));
    registry.register(Box::new(McpService));
    registry
}

#[async_trait::async_trait]
pub trait ServiceModule: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// HTTP path prefixes owned by this module.
    fn path_prefixes(&self) -> &'static [&'static str];
    /// Get Service Module status.
    async fn get_status(&self, state: &AppState, enabled: bool) -> ServiceModuleStatus;
    /// Axum routes exposed by this module.
    fn routes(&self) -> Router<AppState>;
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceModuleStatus {
    /// Stable module id.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Human-readable summary.
    pub description: String,
    /// Canonical HTTP path prefixes owned by the module.
    pub path_prefixes: Vec<String>,
    /// Whether routes are enabled by settings.
    pub enabled: bool,
    /// Whether the module is healthy and serviceable.
    pub running: bool,
    /// Module-specific status counters or diagnostics.
    pub stats: serde_json::Value,
}

pub struct ServiceRegistry {
    /// Registered service modules
    modules: Vec<Box<dyn ServiceModule>>,
}

impl ServiceRegistry {
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    pub fn register(&mut self, module: Box<dyn ServiceModule>) {
        self.modules.push(module);
    }

    pub fn validate(&self) -> Result<(), ServiceRegistryError> {
        let mut ids = HashSet::new();
        let mut prefixes: Vec<(&str, &str)> = Vec::new();

        for module in &self.modules {
            let id = module.id();
            if !valid_module_id(id) {
                return Err(ServiceRegistryError::InvalidId(id.to_string()));
            }
            if !ids.insert(id) {
                return Err(ServiceRegistryError::DuplicateId(id.to_string()));
            }

            let path_prefixes = module.path_prefixes();
            if path_prefixes.is_empty() {
                return Err(ServiceRegistryError::EmptyPathPrefixes {
                    module_id: id.to_string(),
                });
            }

            for path_prefix in path_prefixes {
                if !valid_path_prefix(path_prefix) {
                    return Err(ServiceRegistryError::InvalidPathPrefix {
                        module_id: id.to_string(),
                        path_prefix: (*path_prefix).to_string(),
                    });
                }
                if reserved_path_prefix(path_prefix) {
                    return Err(ServiceRegistryError::ReservedPathPrefix {
                        module_id: id.to_string(),
                        path_prefix: (*path_prefix).to_string(),
                    });
                }
                if let Some((other_id, other_prefix)) = prefixes
                    .iter()
                    .find(|(_, other_prefix)| path_prefixes_conflict(path_prefix, other_prefix))
                {
                    return Err(ServiceRegistryError::PathPrefixConflict {
                        left_module_id: (*other_id).to_string(),
                        left_path_prefix: (*other_prefix).to_string(),
                        right_module_id: id.to_string(),
                        right_path_prefix: (*path_prefix).to_string(),
                    });
                }
                prefixes.push((id, path_prefix));
            }
        }

        Ok(())
    }

    pub fn merge_routes(&self, settings: &ServiceModuleSettings) -> Router<AppState> {
        self.modules
            .iter()
            .filter(|module| settings.is_enabled(module.id()))
            .fold(Router::new(), |router, module| {
                router.merge(module.routes())
            })
    }

    pub async fn list_statuses(
        &self,
        state: &AppState,
        settings: &ServiceModuleSettings,
    ) -> Vec<ServiceModuleStatus> {
        let mut statuses = Vec::with_capacity(self.modules.len());
        for module in &self.modules {
            statuses.push(
                module
                    .get_status(state, settings.is_enabled(module.id()))
                    .await,
            );
        }
        statuses
    }
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceRegistryError {
    #[error("service module id is invalid: {0}")]
    InvalidId(String),
    #[error("duplicate service module id: {0}")]
    DuplicateId(String),
    #[error("service module {module_id} must declare at least one path prefix")]
    EmptyPathPrefixes { module_id: String },
    #[error("service module {module_id} has invalid path prefix: {path_prefix}")]
    InvalidPathPrefix {
        module_id: String,
        path_prefix: String,
    },
    #[error("service module {module_id} uses reserved path prefix: {path_prefix}")]
    ReservedPathPrefix {
        module_id: String,
        path_prefix: String,
    },
    #[error(
        "service module path prefix conflict: {left_module_id}:{left_path_prefix} conflicts with {right_module_id}:{right_path_prefix}"
    )]
    PathPrefixConflict {
        left_module_id: String,
        left_path_prefix: String,
        right_module_id: String,
        right_path_prefix: String,
    },
}

fn valid_module_id(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

fn valid_path_prefix(path_prefix: &str) -> bool {
    path_prefix.starts_with('/') && path_prefix != "/" && !path_prefix.ends_with('/')
}

fn reserved_path_prefix(path_prefix: &str) -> bool {
    path_prefix == "/health" || path_prefix == "/v1" || path_prefix.starts_with("/v1/")
}

fn path_prefixes_conflict(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use serde_json::json;
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

    struct FakeServiceModule {
        id: &'static str,
        path_prefixes: &'static [&'static str],
    }

    #[async_trait::async_trait]
    impl ServiceModule for FakeServiceModule {
        fn id(&self) -> &'static str {
            self.id
        }

        fn name(&self) -> &'static str {
            "Fake"
        }

        fn description(&self) -> &'static str {
            "Fake service module"
        }

        fn path_prefixes(&self) -> &'static [&'static str] {
            self.path_prefixes
        }

        async fn get_status(&self, _state: &AppState, enabled: bool) -> ServiceModuleStatus {
            ServiceModuleStatus {
                id: self.id.to_string(),
                name: self.name().to_string(),
                description: self.description().to_string(),
                path_prefixes: self
                    .path_prefixes
                    .iter()
                    .map(|path| (*path).to_string())
                    .collect(),
                enabled,
                running: enabled,
                stats: json!({ "requests": 0 }),
            }
        }

        fn routes(&self) -> Router<AppState> {
            Router::new().route("/fake", get(|| async { axum::Json(json!({ "ok": true })) }))
        }
    }

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
            knowledge_repo: None,
            channel_repo: channels,
        }
    }

    fn fake(id: &'static str, path_prefixes: &'static [&'static str]) -> Box<dyn ServiceModule> {
        Box::new(FakeServiceModule { id, path_prefixes })
    }

    #[test]
    fn production_registry_registers_knowledge() {
        let registry = production_service_registry();
        assert_eq!(registry.modules.len(), 2);
        assert_eq!(registry.modules[0].id(), "knowledge");
        assert_eq!(registry.modules[1].id(), "mcp");
    }

    #[tokio::test]
    async fn enabled_fake_module_routes_are_merged() {
        let mut registry = ServiceRegistry::new();
        registry.register(fake("knowledge", &["/fake"]));
        let app = registry
            .merge_routes(&ServiceModuleSettings {
                knowledge: true,
                mcp: true,
            })
            .with_state(minimal_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/fake")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn disabled_fake_module_routes_are_not_merged() {
        let mut registry = ServiceRegistry::new();
        registry.register(fake("knowledge", &["/fake"]));
        let app = registry
            .merge_routes(&ServiceModuleSettings {
                knowledge: false,
                mcp: true,
            })
            .with_state(minimal_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/fake")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_statuses_includes_disabled_modules_in_registration_order() {
        let mut registry = ServiceRegistry::new();
        registry.register(fake("mcp", &["/mcp"]));
        registry.register(fake("knowledge", &["/fake"]));

        let statuses = registry
            .list_statuses(
                &minimal_state(),
                &ServiceModuleSettings {
                    knowledge: false,
                    mcp: true,
                },
            )
            .await;

        assert_eq!(
            statuses
                .iter()
                .map(|status| status.id.as_str())
                .collect::<Vec<_>>(),
            vec!["mcp", "knowledge"]
        );
        assert!(statuses[0].enabled);
        assert!(!statuses[1].enabled);
    }

    #[test]
    fn validate_rejects_duplicate_module_ids() {
        let mut registry = ServiceRegistry::new();
        registry.register(fake("knowledge", &["/fake"]));
        registry.register(fake("knowledge", &["/fake2"]));

        assert!(matches!(
            registry.validate(),
            Err(ServiceRegistryError::DuplicateId(id)) if id == "knowledge"
        ));
    }

    #[test]
    fn validate_rejects_invalid_module_ids() {
        for id in ["", "Knowledge", "/mcp", "mcp server"] {
            let mut registry = ServiceRegistry::new();
            registry.register(fake(id, &["/fake"]));

            assert!(matches!(
                registry.validate(),
                Err(ServiceRegistryError::InvalidId(_))
            ));
        }
    }

    #[test]
    fn validate_allows_expected_module_id_formats() {
        for id in ["knowledge", "mcp", "local-tools", "rag_v2"] {
            let mut registry = ServiceRegistry::new();
            registry.register(fake(id, &["/fake"]));

            assert!(registry.validate().is_ok());
        }
    }

    #[test]
    fn validate_rejects_empty_path_prefixes() {
        let mut registry = ServiceRegistry::new();
        registry.register(fake("knowledge", &[]));

        assert!(matches!(
            registry.validate(),
            Err(ServiceRegistryError::EmptyPathPrefixes { module_id }) if module_id == "knowledge"
        ));
    }

    #[test]
    fn validate_rejects_invalid_path_prefix_formats() {
        for path_prefixes in [&["fake"][..], &["/"][..], &["/fake/"][..]] {
            let mut registry = ServiceRegistry::new();
            registry.register(fake("knowledge", path_prefixes));

            assert!(matches!(
                registry.validate(),
                Err(ServiceRegistryError::InvalidPathPrefix { .. })
            ));
        }
    }

    #[test]
    fn validate_rejects_reserved_path_prefixes() {
        for path_prefixes in [&["/v1"][..], &["/v1/models"][..], &["/health"][..]] {
            let mut registry = ServiceRegistry::new();
            registry.register(fake("knowledge", path_prefixes));

            assert!(matches!(
                registry.validate(),
                Err(ServiceRegistryError::ReservedPathPrefix { .. })
            ));
        }
    }

    #[test]
    fn validate_rejects_path_segment_conflicts() {
        const RIGHT_PREFIXES: [&[&str]; 2] = [&["/api/kb"], &["/api/kb/admin"]];

        for right in RIGHT_PREFIXES {
            let mut registry = ServiceRegistry::new();
            registry.register(fake("knowledge", &["/api/kb"]));
            registry.register(fake("mcp", right));

            assert!(matches!(
                registry.validate(),
                Err(ServiceRegistryError::PathPrefixConflict { .. })
            ));
        }
    }

    #[test]
    fn validate_allows_similar_non_conflicting_path_prefixes() {
        let mut registry = ServiceRegistry::new();
        registry.register(fake("knowledge", &["/api/kb"]));
        registry.register(fake("mcp", &["/api/kbase"]));

        assert!(registry.validate().is_ok());
    }
}
