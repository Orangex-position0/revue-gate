//! ApiKey aggregate root: entity + quota value object + ApiKeyRepository trait.
//!
//! Key format `sk-revue-<16-char random hex>` (25 characters, 8 bytes of entropy); HTTP requests authenticate with
//! `Authorization: Bearer <key>`. Quota over-limit determination is handled by the domain service `QuotaPolicy`
//! (see domain/quota.rs); this file only defines the quota data shape.
//!
//! The serde derives are used for serialization across the Tauri Command boundary (frontend types in `src/types/index.ts`):
//! fields are output in camelCase.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::error::RepositoryError;

/// Quota value object: `limit` is the cap (None means no limit), `used` is the consumed quota.
/// When `quota_used >= quota_limit` the request should return 429 (determined by QuotaPolicy in domain/quota.rs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    /// Quota cap; None means unlimited.
    pub limit: Option<u64>,
    /// Consumed quota, accumulated per request and persisted.
    pub used: u64,
}

/// ApiKey entity: a gateway-local key that downstream uses to access the gateway without touching upstream keys.
/// `key` plaintext is stored only locally: the control-plane create command returns it once; list/edit/enable-disable return a masked preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKey {
    pub id: Uuid,
    /// Admin-readable name.
    pub name: String,
    /// Key plaintext `sk-revue-*`; stored only locally, never returned to downstream.
    pub key: String,
    /// Enable/disable state; authentication with a disabled key should fail.
    pub enabled: bool,
    pub quota: Quota,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Generate a Local API Key: `sk-revue-<16 random hex>`.
pub fn generate_local_key() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("OS random number generator is available");
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("sk-revue-{hex}")
}

/// ApiKey repository trait: interface defined in the domain layer, sqlx implementation provided by infrastructure.
#[async_trait::async_trait]
pub trait ApiKeyRepository: Send + Sync {
    /// Look up by key plaintext (for authentication).
    async fn find_by_key(&self, key: &str) -> Result<Option<ApiKey>, RepositoryError>;
    /// Look up a key by id.
    async fn find_by_id(&self, id: Uuid) -> Result<Option<ApiKey>, RepositoryError>;
    /// List all keys.
    async fn list(&self) -> Result<Vec<ApiKey>, RepositoryError>;
    /// Create or overwrite-save a key.
    async fn save(&self, api_key: &ApiKey) -> Result<(), RepositoryError>;
    /// Delete a key by id.
    async fn delete(&self, id: Uuid) -> Result<(), RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_local_key_uses_public_domain_shape() {
        let key = generate_local_key();
        let Some(hex_part) = key.strip_prefix("sk-revue-") else {
            panic!("key should start with sk-revue-: {key}");
        };
        assert_eq!(hex_part.len(), 16);
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(key.len(), 25);
    }
}
