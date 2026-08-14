//! SQLite API key repository implementation: the sqlx implementation of the `ApiKeyRepository` trait.
//!
//! Maps to the api_keys table in `migrations/001_init.sql`: `quota_limit` / `quota_used` are stored as
//! INTEGER (signed 64-bit in SQLite; read back as i64 and converted to u64, with negative values treated as
//! corrupted row data). The `key` column has a UNIQUE constraint; a missed `delete` returns `RepositoryError::NotFound`.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::domain::api_key::{ApiKey, ApiKeyRepository, Quota};
use crate::domain::error::RepositoryError;

/// Row mapping for the API key table: one-to-one with the api_keys columns in `migrations/001_init.sql`.
/// INTEGER columns are read as i64, then converted to the domain layer's u64 via `TryFrom`.
#[derive(FromRow)]
struct ApiKeyDb {
    id: String,
    name: String,
    key: String,
    enabled: bool,
    quota_limit: Option<i64>,
    quota_used: i64,
    created_at: String,
    updated_at: String,
}

/// SELECT column list (shared by all queries to avoid repetition).
const SELECT_COLUMNS: &str = "id, name, key, enabled, quota_limit, quota_used, \
     created_at, updated_at";

/// ApiKeyRepository implementation backed by an sqlx pool.
pub struct SqliteApiKeyRepository {
    pool: SqlitePool,
}

impl SqliteApiKeyRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ApiKeyRepository for SqliteApiKeyRepository {
    async fn find_by_key(&self, key: &str) -> Result<Option<ApiKey>, RepositoryError> {
        let row: Option<ApiKeyDb> = sqlx::query_as(&format!(
            "SELECT {SELECT_COLUMNS} FROM api_keys WHERE key = ?"
        ))
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        row.map(ApiKey::try_from).transpose()
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<ApiKey>, RepositoryError> {
        let row: Option<ApiKeyDb> = sqlx::query_as(&format!(
            "SELECT {SELECT_COLUMNS} FROM api_keys WHERE id = ?"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        row.map(ApiKey::try_from).transpose()
    }

    /// Lists all keys, ordered by "name ASC → created_at ASC" (a stable order for the list view).
    async fn list(&self) -> Result<Vec<ApiKey>, RepositoryError> {
        let rows: Vec<ApiKeyDb> = sqlx::query_as(&format!(
            "SELECT {SELECT_COLUMNS} FROM api_keys ORDER BY name ASC, created_at ASC"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(ApiKey::try_from).collect()
    }

    /// Upsert semantics: overwrite when a row with the same id exists, otherwise insert.
    /// The key plaintext never changes on update, so ON CONFLICT does not overwrite `key` (if the UNIQUE
    /// column were overwritten, updating key could trigger a UNIQUE conflict and fail the whole write).
    async fn save(&self, api_key: &ApiKey) -> Result<(), RepositoryError> {
        // Quota columns are stored as signed i64: values beyond the i64 range are rejected (otherwise
        // i64_to_u64 would flag a bad row on read-back).
        let quota_limit = api_key
            .quota
            .limit
            .map(i64::try_from)
            .transpose()
            .map_err(|_| bad_row("quota_limit exceeds i64 range"))?;
        let quota_used = i64::try_from(api_key.quota.used)
            .map_err(|_| bad_row("quota_used exceeds i64 range"))?;
        sqlx::query(
            "INSERT INTO api_keys (id, name, key, enabled, quota_limit, quota_used, \
                created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                name = excluded.name, enabled = excluded.enabled, \
                quota_limit = excluded.quota_limit, quota_used = excluded.quota_used, \
                updated_at = excluded.updated_at",
        )
        .bind(api_key.id.to_string())
        .bind(&api_key.name)
        .bind(&api_key.key)
        .bind(api_key.enabled)
        .bind(quota_limit)
        .bind(quota_used)
        .bind(api_key.created_at.to_rfc3339())
        .bind(api_key.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// Deletes a key; a miss (0 rows affected) returns `NotFound`.
    async fn delete(&self, id: Uuid) -> Result<(), RepositoryError> {
        let result = sqlx::query("DELETE FROM api_keys WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        if result.rows_affected() == 0 {
            return Err(RepositoryError::NotFound);
        }
        Ok(())
    }
}

impl TryFrom<ApiKeyDb> for ApiKey {
    type Error = RepositoryError;

    fn try_from(row: ApiKeyDb) -> Result<Self, Self::Error> {
        Ok(ApiKey {
            id: Uuid::from_str(&row.id).map_err(|e| bad_row(&format!("invalid id: {e}")))?,
            name: row.name,
            key: row.key,
            enabled: row.enabled,
            quota: Quota {
                limit: row
                    .quota_limit
                    .map(|l| i64_to_u64(l, "quota_limit"))
                    .transpose()?,
                used: i64_to_u64(row.quota_used, "quota_used")?,
            },
            created_at: parse_utc(&row.created_at)?,
            updated_at: parse_utc(&row.updated_at)?,
        })
    }
}

/// Converts a stored signed i64 to the domain layer's u64 (negative values indicate corrupted row data).
fn i64_to_u64(value: i64, column: &str) -> Result<u64, RepositoryError> {
    u64::try_from(value).map_err(|_| bad_row(&format!("{column} out of range for u64")))
}

/// Parses a stored RFC3339 time string into `DateTime<Utc>`.
fn parse_utc(s: &str) -> Result<DateTime<Utc>, RepositoryError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| bad_row(&format!("invalid timestamp {s:?}: {e}")))
}

fn db_err(e: sqlx::Error) -> RepositoryError {
    RepositoryError::Database(e.to_string())
}

/// Builds a "corrupted row data" error: stored data cannot be parsed into a domain model.
fn bad_row(reason: &str) -> RepositoryError {
    RepositoryError::Database(format!("invalid api_key row: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::api_key::Quota;
    use crate::infrastructure::sqlite::init_pool;

    async fn new_repo() -> SqliteApiKeyRepository {
        SqliteApiKeyRepository::new(init_pool("sqlite::memory:").await.expect("init pool"))
    }

    /// After save, find by id: all fields (quota, nullable columns, booleans, timestamps) round-trip consistently.
    #[tokio::test]
    async fn save_then_find_roundtrips_all_fields() {
        let repo = new_repo().await;
        let mut api_key = crate::test_support::sample_api_key();
        api_key.name = "my-client".to_string();
        api_key.quota = Quota {
            limit: Some(500),
            used: 120,
        };
        api_key.enabled = false;
        repo.save(&api_key).await.expect("save");

        let found = repo
            .find_by_id(api_key.id)
            .await
            .expect("find")
            .expect("found");
        assert_eq!(found, api_key);
    }

    /// find_by_key: looks up by key plaintext (used for auth); returns None when missing.
    #[tokio::test]
    async fn find_by_key_locates_exact_key() {
        let repo = new_repo().await;
        let api_key = crate::test_support::sample_api_key();
        repo.save(&api_key).await.expect("save");

        let found = repo
            .find_by_key(&api_key.key)
            .await
            .expect("find")
            .expect("found");
        assert_eq!(found.id, api_key.id);
        assert!(
            repo.find_by_key("sk-revue-ffffffffffffffff")
                .await
                .expect("find missing")
                .is_none()
        );
    }

    /// Upsert: re-saving the same id overwrites without adding a row (quota and enable/disable updates take effect).
    #[tokio::test]
    async fn save_upserts_by_id_does_not_duplicate() {
        let repo = new_repo().await;
        let mut api_key = crate::test_support::sample_api_key();
        repo.save(&api_key).await.expect("save");

        api_key.quota.used = 42;
        api_key.enabled = false;
        repo.save(&api_key).await.expect("resave");

        let all = repo.list().await.expect("list");
        assert_eq!(all.len(), 1, "upsert 不产生重复行");
        let found = repo
            .find_by_id(api_key.id)
            .await
            .expect("find")
            .expect("found");
        assert_eq!(found.quota.used, 42);
        assert!(!found.enabled);
    }

    /// Ordering: name ascending (same name sorted by created_at ascending).
    #[tokio::test]
    async fn list_orders_by_name_then_created() {
        let repo = new_repo().await;
        let mut beta = crate::test_support::sample_api_key();
        beta.name = "b-key".to_string();
        let mut alpha = crate::test_support::sample_api_key();
        alpha.name = "a-key".to_string();
        repo.save(&beta).await.expect("save beta");
        repo.save(&alpha).await.expect("save alpha");

        let names: Vec<String> = repo
            .list()
            .await
            .expect("list")
            .into_iter()
            .map(|k| k.name)
            .collect();
        assert_eq!(names, vec!["a-key", "b-key"]);
    }

    /// Delete: the row is gone after deletion; deleting a missing id again returns NotFound.
    #[tokio::test]
    async fn delete_removes_row_and_missing_errors() {
        let repo = new_repo().await;
        let api_key = crate::test_support::sample_api_key();
        repo.save(&api_key).await.expect("save");

        repo.delete(api_key.id).await.expect("delete");
        assert!(
            repo.find_by_id(api_key.id)
                .await
                .expect("find after delete")
                .is_none()
        );

        let err = repo.delete(api_key.id).await.expect_err("delete missing");
        assert!(matches!(err, RepositoryError::NotFound));
    }

    /// No limit (quota_limit = NULL) reads back as None.
    #[tokio::test]
    async fn null_quota_limit_roundtrips_as_none() {
        let repo = new_repo().await;
        let api_key = crate::test_support::sample_api_key(); // limit: None
        repo.save(&api_key).await.expect("save");

        let found = repo
            .find_by_id(api_key.id)
            .await
            .expect("find")
            .expect("found");
        assert_eq!(found.quota.limit, None);
    }
}
