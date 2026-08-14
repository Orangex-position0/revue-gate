//! 测试支撑：seam A 的内存 mock 仓储实现（仅测试编译）。
//!
//! 供 usecases 编排测试引用（`crate::test_support::*`）：对 domain 的 Repository trait
//! 提供内存实现，行为可控、无真实 DB。domain 纯逻辑经这些用例被覆盖（见 Spec §Testing）。

use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::api_key::{ApiKey, ApiKeyRepository, Quota};
use crate::domain::channel::{Channel, ChannelRepository, ChannelType};
use crate::domain::error::RepositoryError;
use crate::domain::provider::{
    BoxStream, ChatRequest, ProviderAdaptor, ProviderError, ProviderResponse, StreamEvent,
    TestResult,
};
use crate::domain::request_log::{RequestLog, RequestLogRepository};

/// 构造一条最小 Channel 测试样本（供各层测试复用）。
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

/// 内存版 ChannelRepository：以 Vec<Channel> 为后端，upsert 语义的 save。
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

/// 构造一条最小 ApiKey 测试样本（供各层测试复用）。
/// key 由随机 uuid v4 派生（全随机 32 hex，取前 16 位），保证每次调用唯一
/// （api_keys.key 列 UNIQUE）。不用 v7：其前 12 位 hex 为毫秒时间戳，同毫秒调用会碰撞。
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

/// 内存版 ApiKeyRepository：以 Vec<ApiKey> 为后端，upsert 语义的 save。
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

/// 内存版 RequestLogRepository：以 Vec<RequestLog> 为后端，append 语义的 save。
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

    async fn list(&self) -> Result<Vec<RequestLog>, RepositoryError> {
        Ok(self.logs.read().unwrap().clone())
    }
}

/// 内存版 ProviderAdaptor：`test()` 返回预设结果（成功 / 失败 / 配置错误），供 test_channel 用例测试。
/// `forward` / `forward_stream` 不参与 test-channel 编排，返回配置错误占位。
pub struct MockProviderAdaptor {
    result: TestResult,
    /// Some → `test()` 返回配置错误（模拟缺 api_key / base_url）。
    error: Option<String>,
}

impl MockProviderAdaptor {
    /// 预设一次成功的连通性测试结果。
    pub fn new(result: TestResult) -> Self {
        Self {
            result,
            error: None,
        }
    }

    /// 预设 `test()` 返回配置错误（如「api key is required」）。
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

/// 脚本化转发 mock：`forward` / `forward_stream` 按预设结果返回，供代理用例（proxy.rs）seam A 测试。
/// 每次调用把收到的 `ChatRequest` 记入共享 recorder（测试侧持 Arc，可事后断言请求体/映射）。
/// `forward` 返回 `Ok(status>=500 / 429)` 或 `Err` 即模拟一次「失败」以驱动重试。
pub struct MockForwardAdaptor {
    /// 非流式转发脚本结果。
    forward: Result<ProviderResponse, ProviderError>,
    /// 流式转发脚本结果：`Err` 表示打开流失败；`Ok(events)` 为逐帧事件序列。
    stream: Result<Vec<Result<StreamEvent, ProviderError>>, ProviderError>,
    /// 收到的转发请求（forward 与 forward_stream 共用，按调用顺序追加）。
    pub received: Arc<Mutex<Vec<ChatRequest>>>,
}

impl MockForwardAdaptor {
    /// 构造：非流式 / 流式脚本各自指定。
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

    /// 用共享 recorder 构造（测试侧可事后断言请求体）。
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

    /// seam A 验证：内存 ChannelRepository 可被引用为 `Box<dyn ChannelRepository>`
    /// （编译通过）并具备基本 CRUD 行为。
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

    /// seam A 验证：内存 ApiKeyRepository 可被引用为 trait object 并完成保存/查找。
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

    /// seam A 验证：内存 RequestLogRepository 可被引用为 trait object 并完成追加/查找。
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
        assert_eq!(repo.list().await.expect("list").len(), 1);
    }
}
