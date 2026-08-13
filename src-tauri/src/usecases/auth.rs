//! 认证用例：Bearer 密钥认证 + 配额校验。
//!
//! `AuthenticateRequestUsecase` 校验 HTTP 请求携带的本地密钥：无 / 无效 / 停用密钥返回
//! `Unauthorized`（应映射为 HTTP 401），配额超限返回 `QuotaExceeded`（应映射为 429）。
//! Bearer 前缀剥离由数据面 handler 完成，本用例接收已提取的 token（`None` 表示请求
//! 无有效 Authorization 头）。密钥查找走 `ApiKeyRepository` trait（seam A 可 mock）。

use crate::domain::api_key::{ApiKey, ApiKeyRepository};
use crate::domain::error::RepositoryError;
use crate::domain::quota::{QuotaCheck, QuotaPolicy};

/// 认证用例层错误：分支与数据面 HTTP 状态码一一对应（401 / 429）。
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid or missing api key")]
    /// 无 / 无效 / 停用密钥，应返回 401。
    Unauthorized,
    #[error("quota exceeded")]
    /// 配额超限，应返回 429。
    QuotaExceeded,
    #[error("api key repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// 认证用例：校验 Bearer 密钥并返回通过认证的密钥（供记账 / 日志使用）。
pub struct AuthenticateRequestUsecase;
impl AuthenticateRequestUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ApiKeyRepository,
        bearer_token: Option<&str>,
    ) -> Result<ApiKey, AuthError> {
        let Some(token) = bearer_token else {
            return Err(AuthError::Unauthorized);
        };
        let Some(api_key) = repo.find_by_key(token).await? else {
            return Err(AuthError::Unauthorized);
        };
        if !api_key.enabled {
            return Err(AuthError::Unauthorized);
        }
        match QuotaPolicy::check(&api_key.quota) {
            QuotaCheck::Exceeded => Err(AuthError::QuotaExceeded),
            QuotaCheck::Ok => Ok(api_key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    use crate::domain::api_key::Quota;
    use crate::test_support::InMemoryApiKeyRepository;

    /// 直接向内存仓储保存一条指定状态的密钥，返回其句柄（认证测试不需要走创建用例）。
    async fn save_key(
        repo: &InMemoryApiKeyRepository,
        enabled: bool,
        limit: Option<u64>,
        used: u64,
    ) -> ApiKey {
        let api_key = ApiKey {
            id: Uuid::now_v7(),
            name: "client".to_string(),
            key: "sk-revue-0123456789abcdef".to_string(),
            enabled,
            quota: Quota { limit, used },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        repo.save(&api_key).await.expect("save key");
        api_key
    }

    /// 无 Bearer（请求无 Authorization 头 / 格式非法）→ 401 Unauthorized。
    #[tokio::test]
    async fn missing_bearer_is_unauthorized() {
        let repo = InMemoryApiKeyRepository::new();
        let err = AuthenticateRequestUsecase
            .execute(&repo, None)
            .await
            .expect_err("no bearer should be rejected");
        assert!(matches!(err, AuthError::Unauthorized));
    }

    /// 无效密钥（库中不存在）→ 401 Unauthorized。
    #[tokio::test]
    async fn unknown_key_is_unauthorized() {
        let repo = InMemoryApiKeyRepository::new();
        let err = AuthenticateRequestUsecase
            .execute(&repo, Some("sk-revue-ffffffffffffffff"))
            .await
            .expect_err("unknown key should be rejected");
        assert!(matches!(err, AuthError::Unauthorized));
    }

    /// 停用密钥 → 401 Unauthorized。
    #[tokio::test]
    async fn disabled_key_is_unauthorized() {
        let repo = InMemoryApiKeyRepository::new();
        save_key(&repo, false, None, 0).await;
        let err = AuthenticateRequestUsecase
            .execute(&repo, Some("sk-revue-0123456789abcdef"))
            .await
            .expect_err("disabled key should be rejected");
        assert!(matches!(err, AuthError::Unauthorized));
    }

    /// 有效密钥（启用 + 未超限）→ 返回该密钥（供记账 / 日志使用）。
    #[tokio::test]
    async fn valid_key_returns_api_key() {
        let repo = InMemoryApiKeyRepository::new();
        let saved = save_key(&repo, true, Some(100), 10).await;
        let authenticated = AuthenticateRequestUsecase
            .execute(&repo, Some(&saved.key))
            .await
            .expect("valid key");
        assert_eq!(authenticated.id, saved.id);
        assert_eq!(authenticated.key, saved.key);
    }

    /// 配额超限（used >= limit）→ 429 QuotaExceeded（边界值本身即超限）。
    #[tokio::test]
    async fn quota_exceeded_is_quota_error() {
        let repo = InMemoryApiKeyRepository::new();
        save_key(&repo, true, Some(100), 100).await;
        let err = AuthenticateRequestUsecase
            .execute(&repo, Some("sk-revue-0123456789abcdef"))
            .await
            .expect_err("quota exceeded should be rejected");
        assert!(matches!(err, AuthError::QuotaExceeded));
    }

    /// 配额未超限（used < limit）→ 放行。
    #[tokio::test]
    async fn quota_not_exceeded_allows() {
        let repo = InMemoryApiKeyRepository::new();
        save_key(&repo, true, Some(100), 99).await;
        let authenticated = AuthenticateRequestUsecase
            .execute(&repo, Some("sk-revue-0123456789abcdef"))
            .await
            .expect("within quota");
        assert_eq!(authenticated.quota.used, 99);
    }

    /// 无配额上限（limit = None）时任意已用额度都放行。
    #[tokio::test]
    async fn no_limit_allows_any_usage() {
        let repo = InMemoryApiKeyRepository::new();
        save_key(&repo, true, None, 9_999).await;
        assert!(
            AuthenticateRequestUsecase
                .execute(&repo, Some("sk-revue-0123456789abcdef"))
                .await
                .is_ok()
        );
    }
}
