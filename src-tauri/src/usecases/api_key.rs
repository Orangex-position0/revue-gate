//! 密钥管理用例：CRUD / 启停 / 配额管理编排。
//!
//! 无状态用例：仓储以 `&dyn ApiKeyRepository` 注入，seam A 测试可用内存 mock。
//! 密钥格式 `sk-revue-<16 位随机 hex>`（8 字节熵，OS 级随机源 getrandom 生成）；
//! 密钥明文只在创建时可见，更新不改变密钥。配额管理：上限可设、已用额度随请求累加并
//! 持久化（`AccumulateUsageUsecase`，代理成功后调用）。超限判定由 QuotaPolicy 负责。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::api_key::{ApiKey, ApiKeyRepository, Quota};
use crate::domain::error::RepositoryError;

/// 密钥管理用例层错误。
#[derive(Debug, thiserror::Error)]
pub enum ApiKeyError {
    #[error("api key not found")]
    NotFound,
    #[error("invalid api key: {0}")]
    Validation(String),
    #[error("api key repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// 创建 / 编辑密钥的入参（create 与 update 共用；密钥本身不可由入参指定）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyInput {
    pub name: String,
    /// 配额上限；None 表示无上限。
    pub quota_limit: Option<u64>,
    pub enabled: bool,
}

/// 创建密钥：生成 `sk-revue-<16 hex>` 新密钥并保存，返回含完整明文的实体（一次性展示）。
pub struct CreateApiKeyUsecase;
impl CreateApiKeyUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ApiKeyRepository,
        input: ApiKeyInput,
    ) -> Result<ApiKey, ApiKeyError> {
        let input = normalize(input)?;
        let now = Utc::now();
        let api_key = ApiKey {
            id: Uuid::now_v7(),
            name: input.name,
            key: generate_key(),
            enabled: input.enabled,
            quota: Quota {
                limit: input.quota_limit,
                used: 0,
            },
            created_at: now,
            updated_at: now,
        };
        repo.save(&api_key).await?;
        Ok(api_key)
    }
}

/// 更新密钥：加载原实体 → 合并入参（name / 配额上限 / 启停；密钥保持不变）→ 保存。
pub struct UpdateApiKeyUsecase;
impl UpdateApiKeyUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ApiKeyRepository,
        id: Uuid,
        input: ApiKeyInput,
    ) -> Result<ApiKey, ApiKeyError> {
        let input = normalize(input)?;
        let mut api_key = repo.find_by_id(id).await?.ok_or(ApiKeyError::NotFound)?;
        api_key.name = input.name;
        api_key.quota.limit = input.quota_limit;
        api_key.enabled = input.enabled;
        api_key.updated_at = Utc::now();
        repo.save(&api_key).await?;
        Ok(api_key)
    }
}
/// 删除密钥；未命中返回 `NotFound`。
pub struct DeleteApiKeyUsecase;
impl DeleteApiKeyUsecase {
    pub async fn execute(&self, repo: &dyn ApiKeyRepository, id: Uuid) -> Result<(), ApiKeyError> {
        repo.delete(id).await.map_err(map_repo_error)?;
        Ok(())
    }
}

/// 列出全部密钥。
pub struct ListApiKeysUsecase;
impl ListApiKeysUsecase {
    pub async fn execute(&self, repo: &dyn ApiKeyRepository) -> Result<Vec<ApiKey>, ApiKeyError> {
        Ok(repo.list().await?)
    }
}

/// 启停密钥：设置 `enabled` 并持久化。
pub struct SetApiKeyEnabledUsecase;
impl SetApiKeyEnabledUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ApiKeyRepository,
        id: Uuid,
        enabled: bool,
    ) -> Result<ApiKey, ApiKeyError> {
        let mut api_key = repo.find_by_id(id).await?.ok_or(ApiKeyError::NotFound)?;
        api_key.enabled = enabled;
        api_key.updated_at = Utc::now();
        repo.save(&api_key).await?;
        Ok(api_key)
    }
}

/// 累加配额已用额度并持久化（代理成功后按解析出的 usage 记账；饱和加法防溢出）。
pub struct AccumulateUsageUsecase;
impl AccumulateUsageUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ApiKeyRepository,
        api_key_id: Uuid,
        tokens: u64,
    ) -> Result<ApiKey, ApiKeyError> {
        let mut api_key = repo
            .find_by_id(api_key_id)
            .await?
            .ok_or(ApiKeyError::NotFound)?;
        api_key.quota.used = api_key.quota.used.saturating_add(tokens);
        api_key.updated_at = Utc::now();
        repo.save(&api_key).await?;
        Ok(api_key)
    }
}

/// 入参规范化：trim 名称（空名报错）。
fn normalize(input: ApiKeyInput) -> Result<ApiKeyInput, ApiKeyError> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ApiKeyError::Validation("name must not be empty".into()));
    }
    Ok(ApiKeyInput {
        name: name.to_string(),
        ..input
    })
}

/// 生成 `sk-revue-<16 位随机 hex>`：8 字节来自 OS 级随机源（getrandom）。
fn generate_key() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("OS random number generator is available");
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("sk-revue-{hex}")
}

/// 把仓储层错误映射为用例层错误：`NotFound` 语义上等同用例的 `NotFound`。
fn map_repo_error(e: RepositoryError) -> ApiKeyError {
    match e {
        RepositoryError::NotFound => ApiKeyError::NotFound,
        other => ApiKeyError::Repository(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::InMemoryApiKeyRepository;

    /// 构造一条创建入参样本。
    fn input(name: &str) -> ApiKeyInput {
        ApiKeyInput {
            name: name.to_string(),
            quota_limit: Some(100),
            enabled: true,
        }
    }

    /// 密钥格式：`sk-revue-` 前缀 + 16 位小写 hex（25 字符，8 字节熵）。
    #[tokio::test]
    async fn create_generates_sk_revue_key_with_16_hex_chars() {
        let repo = InMemoryApiKeyRepository::new();
        let created = CreateApiKeyUsecase
            .execute(&repo, input("my-client"))
            .await
            .expect("create");

        let Some(hex_part) = created.key.strip_prefix("sk-revue-") else {
            panic!("key should start with sk-revue-: {}", created.key);
        };
        assert_eq!(hex_part.len(), 16, "hex 部分应为 16 位");
        assert!(
            hex_part.chars().all(|c| c.is_ascii_hexdigit()),
            "hex 部分应全为 hex 字符: {hex_part}"
        );
        assert_eq!(created.key.len(), 25);
        assert_eq!(
            created.quota,
            Quota {
                limit: Some(100),
                used: 0
            }
        );

        let all = repo.list().await.expect("list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], created);
    }

    /// 每次创建生成不同的密钥（8 字节熵碰撞概率可忽略）。
    #[tokio::test]
    async fn create_generates_distinct_keys() {
        let repo = InMemoryApiKeyRepository::new();
        let a = CreateApiKeyUsecase
            .execute(&repo, input("a"))
            .await
            .expect("a");
        let b = CreateApiKeyUsecase
            .execute(&repo, input("b"))
            .await
            .expect("b");
        assert_ne!(a.key, b.key);
    }

    /// 创建：空名称（含纯空白）应拒绝且不落库。
    #[tokio::test]
    async fn create_rejects_blank_name() {
        let repo = InMemoryApiKeyRepository::new();
        let err = CreateApiKeyUsecase
            .execute(&repo, input("   "))
            .await
            .expect_err("blank name should be rejected");
        assert!(matches!(err, ApiKeyError::Validation(_)));
        assert!(repo.list().await.expect("list").is_empty());
    }

    /// 创建：名称去首尾空白后落库。
    #[tokio::test]
    async fn create_normalizes_whitespace() {
        let repo = InMemoryApiKeyRepository::new();
        let created = CreateApiKeyUsecase
            .execute(&repo, input("  my-client  "))
            .await
            .expect("create");
        assert_eq!(created.name, "my-client");
    }

    /// 更新：合并 name / 配额上限 / 启停，密钥保持不变，upsert 不新增行。
    #[tokio::test]
    async fn update_merges_fields_and_keeps_key() {
        let repo = InMemoryApiKeyRepository::new();
        let created = CreateApiKeyUsecase
            .execute(&repo, input("my-client"))
            .await
            .expect("create");

        let updated = UpdateApiKeyUsecase
            .execute(
                &repo,
                created.id,
                ApiKeyInput {
                    name: "my-client-v2".to_string(),
                    quota_limit: None, // 清空上限
                    enabled: false,
                },
            )
            .await
            .expect("update");
        assert_eq!(updated.name, "my-client-v2");
        assert_eq!(updated.quota.limit, None);
        assert!(!updated.enabled);
        assert_eq!(updated.key, created.key, "更新不改变密钥");
        assert_eq!(
            updated.quota.used, created.quota.used,
            "已用额度不受编辑影响"
        );

        let all = repo.list().await.expect("list");
        assert_eq!(all.len(), 1, "upsert 不新增行");
        assert_eq!(all[0], updated);
    }

    /// 更新 / 删除 / 启停 / 记账：未知 id 应返回 NotFound。
    #[tokio::test]
    async fn mutate_unknown_id_errors_not_found() {
        let repo = InMemoryApiKeyRepository::new();
        let id = Uuid::now_v7();
        assert!(matches!(
            UpdateApiKeyUsecase.execute(&repo, id, input("x")).await,
            Err(ApiKeyError::NotFound)
        ));
        assert!(matches!(
            DeleteApiKeyUsecase.execute(&repo, id).await,
            Err(ApiKeyError::NotFound)
        ));
        assert!(matches!(
            SetApiKeyEnabledUsecase.execute(&repo, id, false).await,
            Err(ApiKeyError::NotFound)
        ));
        assert!(matches!(
            AccumulateUsageUsecase.execute(&repo, id, 10).await,
            Err(ApiKeyError::NotFound)
        ));
    }

    /// 删除：删除后列表为空，二次删除报 NotFound。
    #[tokio::test]
    async fn delete_removes_key_and_missing_errors() {
        let repo = InMemoryApiKeyRepository::new();
        let created = CreateApiKeyUsecase
            .execute(&repo, input("a"))
            .await
            .expect("create");

        DeleteApiKeyUsecase
            .execute(&repo, created.id)
            .await
            .expect("delete");
        assert!(repo.list().await.expect("list").is_empty());
        assert!(matches!(
            DeleteApiKeyUsecase.execute(&repo, created.id).await,
            Err(ApiKeyError::NotFound)
        ));
    }

    /// 启停：false → true 往返，状态正确持久化。
    #[tokio::test]
    async fn set_enabled_toggles_state() {
        let repo = InMemoryApiKeyRepository::new();
        let created = CreateApiKeyUsecase
            .execute(&repo, input("a"))
            .await
            .expect("create");

        let disabled = SetApiKeyEnabledUsecase
            .execute(&repo, created.id, false)
            .await
            .expect("disable");
        assert!(!disabled.enabled);

        let enabled = SetApiKeyEnabledUsecase
            .execute(&repo, created.id, true)
            .await
            .expect("enable");
        assert!(enabled.enabled);
    }

    /// 列表：列出全部密钥。
    #[tokio::test]
    async fn list_returns_all_keys() {
        let repo = InMemoryApiKeyRepository::new();
        CreateApiKeyUsecase
            .execute(&repo, input("a"))
            .await
            .expect("a");
        CreateApiKeyUsecase
            .execute(&repo, input("b"))
            .await
            .expect("b");

        let all = ListApiKeysUsecase.execute(&repo).await.expect("list");
        assert_eq!(all.len(), 2);
    }

    /// 记账：累加后 used 正确且持久化；跨多次累加累计。
    #[tokio::test]
    async fn accumulate_usage_adds_and_persists() {
        let repo = InMemoryApiKeyRepository::new();
        let created = CreateApiKeyUsecase
            .execute(&repo, input("a"))
            .await
            .expect("create");

        let after_first = AccumulateUsageUsecase
            .execute(&repo, created.id, 40)
            .await
            .expect("accumulate 40");
        assert_eq!(after_first.quota.used, 40);

        let after_second = AccumulateUsageUsecase
            .execute(&repo, created.id, 70)
            .await
            .expect("accumulate 70");
        assert_eq!(after_second.quota.used, 110);

        let found = repo
            .find_by_id(created.id)
            .await
            .expect("find")
            .expect("found");
        assert_eq!(found.quota.used, 110, "已用额度应持久化");
    }

    /// 记账：超过上限后 used 继续累加（超限判定由 QuotaPolicy / 认证用例负责，不做静默截断）。
    #[tokio::test]
    async fn accumulate_usage_can_exceed_quota_limit() {
        let repo = InMemoryApiKeyRepository::new();
        let created = CreateApiKeyUsecase
            .execute(&repo, input("a"))
            .await
            .expect("create");

        let updated = AccumulateUsageUsecase
            .execute(&repo, created.id, 200)
            .await
            .expect("accumulate");
        assert_eq!(updated.quota.used, 200);
    }
}
