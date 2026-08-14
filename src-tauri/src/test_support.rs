//! Test support: in-memory mock repository implementations for seam A (test-only compilation).
//!
//! Referenced by usecases orchestration tests (`crate::test_support::*`): in-memory implementations of the domain
//! Repository traits with controllable behavior and no real DB. Pure domain logic is covered through these usecases (see Spec §Testing).

use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::api_key::{ApiKey, ApiKeyRepository, Quota};
use crate::domain::channel::{Channel, ChannelRepository, ChannelType};
use crate::domain::error::RepositoryError;
use crate::domain::provider::{
    BoxStream, ChatRequest, ProviderAdaptor, ProviderError, ProviderResponse, StreamEvent,
    TestResult,
};
use crate::domain::request_log::{LogPage, LogQuery, LogStatRow, RequestLog, RequestLogRepository};
use crate::domain::settings::{GatewaySettings, SettingsRepository};

/// Build a minimal Channel test sample (reused across layers).
pub(crate) fn sample_channel() -> Channel {
    Channel {
        id: Uuid::now_v7(),
        name: "openai-prod".to_string(),
        channel_type: ChannelType::OpenAi,
        base_url: Some("https://api.openai.com/v1".to_string()),
        api_key: Some("sk-upstream".to_string()),
        models: vec!["gpt-4o".to_string()],
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

/// In-memory ChannelRepository: backed by a Vec<Channel>, save with upsert semantics.
#[derive(Default)]
pub struct InMemoryChannelRepository {
    channels: RwLock<Vec<Channel>>,
}

impl InMemoryChannelRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ChannelRepository for InMemoryChannelRepository {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Channel>, RepositoryError> {
        Ok(self
            .channels
            .read()
            .unwrap()
            .iter()
            .find(|c| c.id == id)
            .cloned())
    }

    async fn list(&self) -> Result<Vec<Channel>, RepositoryError> {
        Ok(self.channels.read().unwrap().clone())
    }

    async fn save(&self, channel: &Channel) -> Result<(), RepositoryError> {
        let mut guard = self.channels.write().unwrap();
        if let Some(existing) = guard.iter_mut().find(|c| c.id == channel.id) {
            *existing = channel.clone();
        } else {
            guard.push(channel.clone());
        }
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<(), RepositoryError> {
        let mut guard = self.channels.write().unwrap();
        let before = guard.len();
        guard.retain(|c| c.id != id);
        if guard.len() == before {
            return Err(RepositoryError::NotFound);
        }
        Ok(())
    }
}

/// Build a minimal ApiKey test sample (reused across layers).
/// The key derives from a random uuid v4 (all-random 32 hex, first 16 chars taken), unique per call
/// (api_keys.key column is UNIQUE). v7 is not used: its first 12 hex chars are a millisecond timestamp, so same-millisecond calls collide.
pub(crate) fn sample_api_key() -> ApiKey {
    let hex: String = Uuid::new_v4().simple().to_string()[..16].to_string();
    ApiKey {
        id: Uuid::now_v7(),
        name: "client".to_string(),
        key: format!("sk-revue-{hex}"),
        enabled: true,
        quota: Quota {
            limit: None,
            used: 0,
        },
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

/// Build a minimal RequestLog test sample (reused across layers; most fields are optional/empty).
pub(crate) fn sample_request_log() -> RequestLog {
    RequestLog {
        id: Uuid::now_v7(),
        api_key_id: None,
        channel_id: None,
        model: "gpt-4o".to_string(),
        upstream_model: None,
        status_code: 200,
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
        duration_ms: 42,
        error_message: None,
        is_stream: false,
        is_retry: false,
        trace_id: Uuid::now_v7().to_string(),
        request_body: None,
        created_at: Utc::now(),
    }
}

/// In-memory ApiKeyRepository: backed by a Vec<ApiKey>, save with upsert semantics.
#[derive(Default)]
pub struct InMemoryApiKeyRepository {
    api_keys: RwLock<Vec<ApiKey>>,
}

impl InMemoryApiKeyRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ApiKeyRepository for InMemoryApiKeyRepository {
    async fn find_by_key(&self, key: &str) -> Result<Option<ApiKey>, RepositoryError> {
        Ok(self
            .api_keys
            .read()
            .unwrap()
            .iter()
            .find(|k| k.key == key)
            .cloned())
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<ApiKey>, RepositoryError> {
        Ok(self
            .api_keys
            .read()
            .unwrap()
            .iter()
            .find(|k| k.id == id)
            .cloned())
    }

    async fn list(&self) -> Result<Vec<ApiKey>, RepositoryError> {
        Ok(self.api_keys.read().unwrap().clone())
    }

    async fn save(&self, api_key: &ApiKey) -> Result<(), RepositoryError> {
        let mut guard = self.api_keys.write().unwrap();
        if let Some(existing) = guard.iter_mut().find(|k| k.id == api_key.id) {
            *existing = api_key.clone();
        } else {
            guard.push(api_key.clone());
        }
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<(), RepositoryError> {
        let mut guard = self.api_keys.write().unwrap();
        let before = guard.len();
        guard.retain(|k| k.id != id);
        if guard.len() == before {
            return Err(RepositoryError::NotFound);
        }
        Ok(())
    }
}

/// In-memory RequestLogRepository: backed by a Vec<RequestLog>, save with append semantics.
#[derive(Default)]
pub struct InMemoryRequestLogRepository {
    logs: RwLock<Vec<RequestLog>>,
}

impl InMemoryRequestLogRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl RequestLogRepository for InMemoryRequestLogRepository {
    async fn save(&self, log: &RequestLog) -> Result<(), RepositoryError> {
        self.logs.write().unwrap().push(log.clone());
        Ok(())
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<RequestLog>, RepositoryError> {
        Ok(self
            .logs
            .read()
            .unwrap()
            .iter()
            .find(|l| l.id == id)
            .cloned())
    }

    /// Paginated query: reuses `LogQuery::matches` (authoritative domain-level filtering), consistent with the SQL implementation.
    /// Sorting aligns with the SQL side: created_at descending, id descending as a tie-breaker.
    async fn query(
        &self,
        query: &LogQuery,
        page: u64,
        page_size: u64,
    ) -> Result<LogPage, RepositoryError> {
        let logs = self.logs.read().unwrap();
        let mut matched: Vec<&RequestLog> = logs.iter().filter(|l| query.matches(l)).collect();
        matched.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        let total = matched.len() as u64;
        let start = ((page.saturating_sub(1)) as usize).min(matched.len());
        let end = (start.saturating_add(page_size as usize)).min(matched.len());
        let items = matched[start..end].iter().map(|l| (*l).clone()).collect();
        Ok(LogPage { items, total })
    }

    /// Delete logs whose created_at is strictly earlier than `before` (keep `>= before`).
    async fn delete_before(&self, before: DateTime<Utc>) -> Result<u64, RepositoryError> {
        let mut guard = self.logs.write().unwrap();
        let before_len = guard.len();
        guard.retain(|l| l.created_at >= before);
        Ok((before_len - guard.len()) as u64)
    }

    /// Clear all logs.
    async fn clear(&self) -> Result<u64, RepositoryError> {
        let mut guard = self.logs.write().unwrap();
        let n = guard.len() as u64;
        guard.clear();
        Ok(n)
    }

    /// Stats projection: reuses half-open interval semantics (consistent with the sqlx implementation); returns lightweight rows (no request body).
    async fn stat_rows(
        &self,
        start_at: Option<DateTime<Utc>>,
        end_at: Option<DateTime<Utc>>,
    ) -> Result<Vec<LogStatRow>, RepositoryError> {
        let logs = self.logs.read().unwrap();
        Ok(logs
            .iter()
            .filter(|l| {
                start_at.is_none_or(|s| l.created_at >= s)
                    && end_at.is_none_or(|e| l.created_at < e)
            })
            .map(|l| LogStatRow {
                status_code: l.status_code,
                total_tokens: l.total_tokens,
                duration_ms: l.duration_ms,
                created_at: l.created_at,
            })
            .collect())
    }
}

/// In-memory SettingsRepository: backed by `Option<GatewaySettings>`.
/// `load` returns defaults when empty (consistent with the sqlx implementation on a missing file); `save` overwrites.
#[derive(Default)]
pub struct InMemorySettingsRepository {
    settings: RwLock<Option<GatewaySettings>>,
}

impl InMemorySettingsRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SettingsRepository for InMemorySettingsRepository {
    async fn load(&self) -> Result<GatewaySettings, RepositoryError> {
        Ok(self.settings.read().unwrap().clone().unwrap_or_default())
    }

    async fn save(&self, settings: &GatewaySettings) -> Result<(), RepositoryError> {
        *self.settings.write().unwrap() = Some(settings.clone());
        Ok(())
    }
}

/// Fetch all logs currently in the repository: `RequestLogRepository::list()` was removed (unbounded read);
/// tests that need a full assertion should page through `query` with an unbounded LIMIT instead (equivalent semantics).
pub(crate) async fn all_request_logs(repo: &dyn RequestLogRepository) -> Vec<RequestLog> {
    repo.query(&LogQuery::default(), 1, u64::MAX)
        .await
        .expect("query logs")
        .items
}

/// In-memory ProviderAdaptor: `test()` returns a preset result (success / failure / config error) for the test_channel usecase.
/// `forward` / `forward_stream` do not participate in test-channel orchestration and return a config-error placeholder.
pub struct MockProviderAdaptor {
    result: TestResult,
    /// Some → `test()` returns a config error (simulates a missing api_key / base_url).
    error: Option<String>,
}

impl MockProviderAdaptor {
    /// Preset a successful connectivity test result.
    pub fn new(result: TestResult) -> Self {
        Self {
            result,
            error: None,
        }
    }

    /// Preset `test()` to return a config error (e.g. "api key is required").
    pub fn not_configured(reason: &str) -> Self {
        Self {
            result: TestResult {
                ok: false,
                latency_ms: 0,
                error: None,
            },
            error: Some(reason.to_string()),
        }
    }
}

#[async_trait]
impl ProviderAdaptor for MockProviderAdaptor {
    fn channel_type(&self) -> ChannelType {
        ChannelType::OpenAi
    }

    fn default_models(&self) -> Vec<String> {
        Vec::new()
    }

    fn default_base_url(&self) -> Option<&'static str> {
        None
    }

    async fn test(&self, _channel: &Channel) -> Result<TestResult, ProviderError> {
        match &self.error {
            Some(reason) => Err(ProviderError::NotConfigured(reason.clone())),
            None => Ok(self.result.clone()),
        }
    }

    async fn forward(
        &self,
        _channel: &Channel,
        _request: &ChatRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        Err(ProviderError::NotConfigured(
            "forward not used in test-channel".into(),
        ))
    }

    async fn forward_stream(
        &self,
        _channel: &Channel,
        _request: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError> {
        Err(ProviderError::NotConfigured(
            "forward_stream not used in test-channel".into(),
        ))
    }
}

/// Scripted forwarding mock: `forward` / `forward_stream` return preset results for seam A tests of the proxy usecase (proxy.rs).
/// Each call records the received `ChatRequest` into a shared recorder (the test side holds an Arc and can assert request bodies/mappings afterwards).
/// A `forward` returning `Ok(status >= 500 / 429)` or `Err` simulates one "failure" to drive retry.
pub struct MockForwardAdaptor {
    /// Scripted result for non-streaming forward.
    forward: Result<ProviderResponse, ProviderError>,
    /// Scripted result for streaming forward: `Err` means opening the stream failed; `Ok(events)` is the per-frame event sequence.
    stream: Result<Vec<Result<StreamEvent, ProviderError>>, ProviderError>,
    /// Received forwarding requests (shared by forward and forward_stream, appended in call order).
    pub received: Arc<Mutex<Vec<ChatRequest>>>,
}

impl MockForwardAdaptor {
    /// Construct with separate non-streaming / streaming scripts.
    pub fn new(
        forward: Result<ProviderResponse, ProviderError>,
        stream: Result<Vec<Result<StreamEvent, ProviderError>>, ProviderError>,
    ) -> Self {
        Self {
            forward,
            stream,
            received: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Construct with a shared recorder (the test side can assert request bodies afterwards).
    pub fn with_recorder(
        received: Arc<Mutex<Vec<ChatRequest>>>,
        forward: Result<ProviderResponse, ProviderError>,
        stream: Result<Vec<Result<StreamEvent, ProviderError>>, ProviderError>,
    ) -> Self {
        Self {
            forward,
            stream,
            received,
        }
    }
}

#[async_trait]
impl ProviderAdaptor for MockForwardAdaptor {
    fn channel_type(&self) -> ChannelType {
        ChannelType::OpenAi
    }

    fn default_models(&self) -> Vec<String> {
        Vec::new()
    }

    fn default_base_url(&self) -> Option<&'static str> {
        None
    }

    async fn test(&self, _channel: &Channel) -> Result<TestResult, ProviderError> {
        Ok(TestResult {
            ok: true,
            latency_ms: 0,
            error: None,
        })
    }

    async fn forward(
        &self,
        _channel: &Channel,
        request: &ChatRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        self.received.lock().unwrap().push(request.clone());
        self.forward.clone()
    }

    async fn forward_stream(
        &self,
        _channel: &Channel,
        request: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError> {
        self.received.lock().unwrap().push(request.clone());
        match &self.stream {
            Err(e) => Err(e.clone()),
            Ok(events) => Ok(Box::pin(futures_util::stream::iter(events.clone()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    /// seam A verification: the in-memory ChannelRepository can be referenced as `Box<dyn ChannelRepository>`
    /// (compiles) and has basic CRUD behavior.
    #[tokio::test]
    async fn channel_repository_mock_is_referenceable_as_trait_object() {
        let repo: Box<dyn ChannelRepository> = Box::new(InMemoryChannelRepository::new());
        assert!(repo.list().await.expect("list").is_empty());

        let channel = sample_channel();
        repo.save(&channel).await.expect("save");
        let found = repo.find_by_id(channel.id).await.expect("find_by_id");
        assert_eq!(found, Some(channel.clone()));

        repo.delete(channel.id).await.expect("delete");
        assert!(
            repo.find_by_id(channel.id)
                .await
                .expect("find after delete")
                .is_none()
        );
    }

    /// seam A verification: the in-memory ApiKeyRepository can be referenced as a trait object and performs save/find.
    #[tokio::test]
    async fn api_key_repository_mock_is_referenceable_as_trait_object() {
        let repo: Box<dyn ApiKeyRepository> = Box::new(InMemoryApiKeyRepository::new());
        let key = ApiKey {
            id: Uuid::now_v7(),
            name: "default".to_string(),
            key: "sk-revue-0123456789abcdef".to_string(),
            enabled: true,
            quota: crate::domain::api_key::Quota {
                limit: None,
                used: 0,
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        repo.save(&key).await.expect("save");
        assert_eq!(
            repo.find_by_key(&key.key).await.expect("find_by_key"),
            Some(key.clone())
        );
        assert_eq!(
            repo.find_by_id(key.id).await.expect("find_by_id"),
            Some(key)
        );
    }

    /// seam A verification: the in-memory RequestLogRepository can be referenced as a trait object and performs append/find.
    #[tokio::test]
    async fn request_log_repository_mock_is_referenceable_as_trait_object() {
        let repo: Box<dyn RequestLogRepository> = Box::new(InMemoryRequestLogRepository::new());
        let log = RequestLog {
            id: Uuid::now_v7(),
            api_key_id: Some(Uuid::now_v7()),
            channel_id: Some(Uuid::now_v7()),
            model: "gpt-4o".to_string(),
            upstream_model: None,
            status_code: 200,
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
            duration_ms: 123,
            error_message: None,
            is_stream: false,
            is_retry: false,
            trace_id: Uuid::now_v7().to_string(),
            request_body: None,
            created_at: Utc::now(),
        };
        repo.save(&log).await.expect("save");
        assert_eq!(
            repo.find_by_id(log.id).await.expect("find_by_id"),
            Some(log)
        );
        assert_eq!(all_request_logs(&*repo).await.len(), 1);
    }

    /// seam A verification: the in-memory SettingsRepository can be referenced as a trait object; returns defaults when empty, readable back after save.
    #[tokio::test]
    async fn settings_repository_mock_is_referenceable_as_trait_object() {
        let repo: Box<dyn SettingsRepository> = Box::new(InMemorySettingsRepository::new());
        assert_eq!(
            repo.load().await.expect("load"),
            GatewaySettings::default(),
            "无持久化值时返回默认设置"
        );

        let settings = GatewaySettings {
            port: 8080,
            autostart: true,
            ..GatewaySettings::default()
        };
        repo.save(&settings).await.expect("save");
        assert_eq!(
            repo.load().await.expect("load after save"),
            settings,
            "保存后可完整回读"
        );
    }
}
