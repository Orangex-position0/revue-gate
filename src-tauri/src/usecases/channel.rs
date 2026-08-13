//! 渠道管理用例：CRUD / 启停编排。
//!
//! 无状态用例：仓储以 `&dyn ChannelRepository` 注入，seam A 测试可用内存 mock。
//! 更新语义：`api_key` 为空表示「保持原值」——编辑表单不预填上游密钥，留空不覆盖
//! （红线段：上游密钥不落库明文暴露给下游，控制面返回前遮蔽，见 interface/commands/channel.rs）。
//! 调度规则（禁用渠道不参与调度等）在阶段 07 生效，本票只保证字段正确持久化。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::channel::{Channel, ChannelRepository, ChannelType, ModelMapping};
use crate::domain::error::RepositoryError;

/// 渠道管理用例层错误。
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("channel not found")]
    NotFound,
    #[error("invalid channel: {0}")]
    Validation(String),
    #[error("channel repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// 创建 / 编辑渠道的入参（create 与 update 共用；update 时 `api_key` 为空 = 保持原值）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInput {
    pub name: String,
    pub channel_type: ChannelType,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub models: Vec<String>,
    pub priority: i32,
    pub weight: i32,
    pub model_mappings: Vec<ModelMapping>,
    pub enabled: bool,
}

/// 创建渠道：校验入参、生成新实体并保存，返回持久化后的渠道。
pub struct CreateChannelUsecase;
impl CreateChannelUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
        input: ChannelInput,
    ) -> Result<Channel, ChannelError> {
        let input = normalize(input)?;
        let now = Utc::now();
        let channel = Channel {
            id: Uuid::now_v7(),
            name: input.name,
            channel_type: input.channel_type,
            base_url: input.base_url,
            api_key: input.api_key,
            models: input.models,
            priority: input.priority,
            weight: input.weight,
            model_mappings: input.model_mappings,
            enabled: input.enabled,
            last_test_at: None,
            last_test_ok: None,
            created_at: now,
            updated_at: now,
        };
        repo.save(&channel).await?;
        Ok(channel)
    }
}

/// 更新渠道：加载原实体 → 合并入参（api_key 为空保持原值）→ 保存。
pub struct UpdateChannelUsecase;
impl UpdateChannelUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
        id: Uuid,
        input: ChannelInput,
    ) -> Result<Channel, ChannelError> {
        let input = normalize(input)?;
        let mut channel = repo.find_by_id(id).await?.ok_or(ChannelError::NotFound)?;
        channel.name = input.name;
        channel.channel_type = input.channel_type;
        channel.base_url = input.base_url;
        if let Some(api_key) = input.api_key {
            channel.api_key = Some(api_key);
        }
        channel.models = input.models;
        channel.priority = input.priority;
        channel.weight = input.weight;
        channel.model_mappings = input.model_mappings;
        channel.enabled = input.enabled;
        channel.updated_at = Utc::now();
        repo.save(&channel).await?;
        Ok(channel)
    }
}

/// 删除渠道；未命中返回 `NotFound`。
pub struct DeleteChannelUsecase;
impl DeleteChannelUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
        id: Uuid,
    ) -> Result<(), ChannelError> {
        repo.delete(id).await.map_err(map_repo_error)?;
        Ok(())
    }
}

/// 列出全部渠道。
pub struct ListChannelsUsecase;
impl ListChannelsUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
    ) -> Result<Vec<Channel>, ChannelError> {
        Ok(repo.list().await?)
    }
}

/// 启停渠道：设置 `enabled` 并持久化。
pub struct SetChannelEnabledUsecase;
impl SetChannelEnabledUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
        id: Uuid,
        enabled: bool,
    ) -> Result<Channel, ChannelError> {
        let mut channel = repo.find_by_id(id).await?.ok_or(ChannelError::NotFound)?;
        channel.enabled = enabled;
        channel.updated_at = Utc::now();
        repo.save(&channel).await?;
        Ok(channel)
    }
}

/// 入参规范化：trim 名称（空名报错）；空 / 纯空白 Base URL / API Key 归一为 None，非空时也 trim。
fn normalize(input: ChannelInput) -> Result<ChannelInput, ChannelError> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ChannelError::Validation("name must not be empty".into()));
    }
    Ok(ChannelInput {
        name: name.to_string(),
        base_url: non_blank(input.base_url),
        api_key: non_blank(input.api_key),
        ..input
    })
}

/// trim 后为空则归一为 None。
fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 把仓储层错误映射为用例层错误：`NotFound` 语义上等同用例的 `NotFound`。
fn map_repo_error(e: RepositoryError) -> ChannelError {
    match e {
        RepositoryError::NotFound => ChannelError::NotFound,
        other => ChannelError::Repository(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::InMemoryChannelRepository;

    /// 构造一条合法的创建入参样本。
    fn input(name: &str) -> ChannelInput {
        ChannelInput {
            name: name.to_string(),
            channel_type: ChannelType::OpenAi,
            base_url: Some("https://api.openai.com/v1".to_string()),
            api_key: Some("sk-upstream".to_string()),
            models: vec!["gpt-4o".to_string()],
            priority: 0,
            weight: 1,
            model_mappings: vec![ModelMapping {
                client_model: "chat".to_string(),
                upstream_model: "gpt-4o".to_string(),
            }],
            enabled: true,
        }
    }

    /// 创建：字段完整往返，仓储落库一条。
    #[tokio::test]
    async fn create_persists_channel_and_roundtrips_fields() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("openai-prod"))
            .await
            .expect("create");

        assert_eq!(created.name, "openai-prod");
        assert_eq!(created.channel_type, ChannelType::OpenAi);
        assert_eq!(created.api_key.as_deref(), Some("sk-upstream"));
        assert_eq!(created.models, vec!["gpt-4o"]);
        assert_eq!(created.priority, 0);
        assert_eq!(created.model_mappings.len(), 1);
        assert!(created.enabled);
        assert_eq!(created.last_test_at, None);

        let all = repo.list().await.expect("list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], created);
    }

    /// 规范化：名称去首尾空白；纯空白 Base URL 归一为 None；非空 Base URL 去首尾空白后落库。
    #[tokio::test]
    async fn create_normalizes_whitespace() {
        let repo = InMemoryChannelRepository::new();

        let mut trimmed = input("  openai-prod  ");
        trimmed.base_url = Some("  https://api.openai.com/v1  ".to_string());
        let created = CreateChannelUsecase
            .execute(&repo, trimmed)
            .await
            .expect("create with padded input");
        assert_eq!(created.name, "openai-prod");
        assert_eq!(
            created.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );

        let mut blank_input = input("blank-url");
        blank_input.base_url = Some("   ".to_string());
        let created_blank = CreateChannelUsecase
            .execute(&repo, blank_input)
            .await
            .expect("create with blank base url");
        assert_eq!(created_blank.name, "blank-url");
        assert_eq!(created_blank.base_url, None);
    }

    /// 创建：空名称（含纯空白）应拒绝且不落库。
    #[tokio::test]
    async fn create_rejects_blank_name() {
        let repo = InMemoryChannelRepository::new();
        let err = CreateChannelUsecase
            .execute(&repo, input("   "))
            .await
            .expect_err("blank name should be rejected");
        assert!(matches!(err, ChannelError::Validation(_)));
        assert!(repo.list().await.expect("list").is_empty());
    }

    /// 更新：字段合并，api_key 留空时保持原密钥，upsert 不产生新行。
    #[tokio::test]
    async fn update_merges_fields_and_keeps_existing_key_when_blank() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("openai-prod"))
            .await
            .expect("create");

        let mut update = input("openai-prod-v2");
        update.api_key = None; // 编辑表单留空 → 保持原密钥
        update.priority = 5;
        update.enabled = false;

        let updated = UpdateChannelUsecase
            .execute(&repo, created.id, update)
            .await
            .expect("update");
        assert_eq!(updated.name, "openai-prod-v2");
        assert_eq!(updated.priority, 5);
        assert!(!updated.enabled);
        assert_eq!(updated.api_key.as_deref(), Some("sk-upstream"));
        assert_eq!(updated.created_at, created.created_at, "创建时间不变");

        let all = repo.list().await.expect("list");
        assert_eq!(all.len(), 1, "upsert 不新增行");
        assert_eq!(all[0], updated);
    }

    /// 更新：显式传入新的 api_key 应覆盖原密钥。
    #[tokio::test]
    async fn update_overwrites_key_when_provided() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("openai-prod"))
            .await
            .expect("create");

        let mut update = input("openai-prod");
        update.api_key = Some("sk-upstream-new".to_string());
        let updated = UpdateChannelUsecase
            .execute(&repo, created.id, update)
            .await
            .expect("update");
        assert_eq!(updated.api_key.as_deref(), Some("sk-upstream-new"));
    }

    /// 更新 / 删除 / 启停：未知 id 应返回 NotFound。
    #[tokio::test]
    async fn mutate_unknown_id_errors_not_found() {
        let repo = InMemoryChannelRepository::new();
        let id = Uuid::now_v7();
        assert!(matches!(
            UpdateChannelUsecase.execute(&repo, id, input("x")).await,
            Err(ChannelError::NotFound)
        ));
        assert!(matches!(
            DeleteChannelUsecase.execute(&repo, id).await,
            Err(ChannelError::NotFound)
        ));
        assert!(matches!(
            SetChannelEnabledUsecase.execute(&repo, id, false).await,
            Err(ChannelError::NotFound)
        ));
    }

    /// 删除：删除后列表为空，二次删除报 NotFound。
    #[tokio::test]
    async fn delete_removes_channel_and_missing_errors() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("a"))
            .await
            .expect("create");

        DeleteChannelUsecase
            .execute(&repo, created.id)
            .await
            .expect("delete");
        assert!(repo.list().await.expect("list").is_empty());
        assert!(matches!(
            DeleteChannelUsecase.execute(&repo, created.id).await,
            Err(ChannelError::NotFound)
        ));
    }

    /// 启停：false → true 往返，状态正确持久化。
    #[tokio::test]
    async fn set_enabled_toggles_state() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("a"))
            .await
            .expect("create");

        let disabled = SetChannelEnabledUsecase
            .execute(&repo, created.id, false)
            .await
            .expect("disable");
        assert!(!disabled.enabled);

        let enabled = SetChannelEnabledUsecase
            .execute(&repo, created.id, true)
            .await
            .expect("enable");
        assert!(enabled.enabled);
    }

    /// 列表：列出全部渠道（顺序交由仓储实现，此处只验证内容）。
    #[tokio::test]
    async fn list_returns_all_channels() {
        let repo = InMemoryChannelRepository::new();
        CreateChannelUsecase
            .execute(&repo, input("a"))
            .await
            .expect("create a");
        CreateChannelUsecase
            .execute(&repo, input("b"))
            .await
            .expect("create b");

        let all = ListChannelsUsecase.execute(&repo).await.expect("list");
        assert_eq!(all.len(), 2);
    }
}
