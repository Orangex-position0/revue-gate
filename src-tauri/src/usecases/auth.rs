//! Auth use cases: Bearer key authentication + quota check.
//!
//! `AuthenticateRequestUsecase` validates the local key carried by the HTTP request: missing / invalid / disabled keys return
//! `Unauthorized` (mapped to HTTP 401), quota exceeded returns `QuotaExceeded` (mapped to 429).
//! Bearer prefix stripping is done by the data-plane handler; this use case receives the already-extracted token (`None` means the request
//! has no valid Authorization header). Key lookup goes through the `ApiKeyRepository` trait (seam A can be mocked).

use crate::domain::api_key::{ApiKey, ApiKeyRepository};
use crate::domain::error::RepositoryError;
use crate::domain::quota::{QuotaCheck, QuotaPolicy};

/// Auth use case layer error: variants map one-to-one to data-plane HTTP status codes (401 / 429).
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid or missing api key")]
    /// Missing / invalid / disabled key, should return 401.
    Unauthorized,
    #[error("quota exceeded")]
    /// Quota exceeded, should return 429.
    QuotaExceeded,
    #[error("api key repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// Auth use case: validate the Bearer key and return the authenticated key (for billing / logging).
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

    /// Save a key with the given state directly to the in-memory repository and return its handle (auth tests do not need the create use case).
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

    /// Missing Bearer (no Authorization header / malformed) → 401 Unauthorized.
    #[tokio::test]
    async fn missing_bearer_is_unauthorized() {
        let repo = InMemoryApiKeyRepository::new();
        let err = AuthenticateRequestUsecase
            .execute(&repo, None)
            .await
            .expect_err("no bearer should be rejected");
        assert!(matches!(err, AuthError::Unauthorized));
    }

    /// Invalid key (not in the repository) → 401 Unauthorized.
    #[tokio::test]
    async fn unknown_key_is_unauthorized() {
        let repo = InMemoryApiKeyRepository::new();
        let err = AuthenticateRequestUsecase
            .execute(&repo, Some("sk-revue-ffffffffffffffff"))
            .await
            .expect_err("unknown key should be rejected");
        assert!(matches!(err, AuthError::Unauthorized));
    }

    /// Disabled key → 401 Unauthorized.
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

    /// Valid key (enabled + within quota) → returns the key (for billing / logging).
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

    /// Quota exceeded (used >= limit) → 429 QuotaExceeded (the boundary value itself counts as exceeded).
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

    /// Quota not exceeded (used < limit) → allowed.
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

    /// Any usage is allowed when there is no quota limit (limit = None).
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
