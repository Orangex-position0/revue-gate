//! 测试支撑：seam A 的内存 mock 仓储实现（仅测试编译）。
//!
//! 供 usecases 编排测试引用（`crate::test_support::*`）：对 domain 的 Repository trait
//! 提供内存实现，行为可控、无真实 DB。domain 纯逻辑经这些用例被覆盖（见 Spec §Testing）。

use std::sync::RwLock;

use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::api_key::{ApiKey, ApiKeyRepository};
use crate::domain::channel::{Channel, ChannelRepository, ChannelType};
use crate::domain::error::RepositoryError;
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
        self.api_keys.write().unwrap().retain(|k| k.id != id);
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
