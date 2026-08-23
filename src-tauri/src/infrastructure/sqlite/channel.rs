//! SQLite channel repository implementation: the sqlx implementation of the `ChannelRepository` trait.
//!
//! Non-JSON columns map directly; `models` / `model_mappings` are stored as JSON TEXT and manually encoded/decoded
//! on read/write (SQLite has no native array type). A missed `delete` returns `RepositoryError::NotFound`, aligned
//! with the NotFound semantics in usecases.

use std::str::FromStr;

use sqlx::FromRow;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::domain::channel::{Channel, ChannelRepository, ChannelType};
use crate::domain::error::RepositoryError;
use crate::infrastructure::sqlite::{bad_row, db_err, fmt_utc, parse_utc};

/// Row mapping for the channel table: one-to-one with the channels columns in `migrations/001_init.sql`.
/// JSON columns are read as String first, then parsed by `TryFrom` (avoiding array types unsupported by sqlx).
#[derive(FromRow)]
struct ChannelDb {
    id: String,
    name: String,
    channel_type: String,
    base_url: Option<String>,
    api_key: Option<String>,
    models: String,
    priority: i32,
    weight: i32,
    model_mappings: String,
    enabled: bool,
    last_test_at: Option<String>,
    last_test_ok: Option<bool>,
    created_at: String,
    updated_at: String,
}

/// SELECT column list (shared by all queries to avoid repetition).
const SELECT_COLUMNS: &str = "id, name, channel_type, base_url, api_key, models, priority, \
     weight, model_mappings, enabled, last_test_at, last_test_ok, created_at, updated_at";

/// ChannelRepository implementation backed by an sqlx pool.
pub struct SqliteChannelRepository {
    pool: SqlitePool,
}

impl SqliteChannelRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ChannelRepository for SqliteChannelRepository {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Channel>, RepositoryError> {
        let row: Option<ChannelDb> = sqlx::query_as(&format!(
            "SELECT {SELECT_COLUMNS} FROM channels WHERE id = ?"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        row.map(Channel::try_from).transpose()
    }

    /// Lists all channels, ordered by "priority ASC → name ASC" (a stable order for the routing view).
    async fn list(&self) -> Result<Vec<Channel>, RepositoryError> {
        let rows: Vec<ChannelDb> = sqlx::query_as(&format!(
            "SELECT {SELECT_COLUMNS} FROM channels ORDER BY priority ASC, name ASC"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(Channel::try_from).collect()
    }

    /// Upsert semantics: overwrite when a row with the same id exists, otherwise insert.
    async fn save(&self, channel: &Channel) -> Result<(), RepositoryError> {
        let models = serde_json::to_string(&channel.models).map_err(json_err)?;
        let mappings = serde_json::to_string(&channel.model_mappings).map_err(json_err)?;
        sqlx::query(
            "INSERT INTO channels (id, name, channel_type, base_url, api_key, models, priority, \
                weight, model_mappings, enabled, last_test_at, last_test_ok, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(id) DO UPDATE SET \
                name = excluded.name, channel_type = excluded.channel_type, \
                base_url = excluded.base_url, api_key = excluded.api_key, \
                models = excluded.models, priority = excluded.priority, \
                weight = excluded.weight, model_mappings = excluded.model_mappings, \
                enabled = excluded.enabled, last_test_at = excluded.last_test_at, \
                last_test_ok = excluded.last_test_ok, updated_at = excluded.updated_at",
        )
        .bind(channel.id.to_string())
        .bind(&channel.name)
        .bind(channel.channel_type.to_string())
        .bind(&channel.base_url)
        .bind(&channel.api_key)
        .bind(models)
        .bind(channel.priority)
        .bind(channel.weight)
        .bind(mappings)
        .bind(channel.enabled)
        .bind(channel.last_test_at.map(fmt_utc))
        .bind(channel.last_test_ok)
        .bind(fmt_utc(channel.created_at))
        .bind(fmt_utc(channel.updated_at))
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// Deletes a channel; a miss (0 rows affected) returns `NotFound`.
    async fn delete(&self, id: Uuid) -> Result<(), RepositoryError> {
        let result = sqlx::query("DELETE FROM channels WHERE id = ?")
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

impl TryFrom<ChannelDb> for Channel {
    type Error = RepositoryError;

    fn try_from(row: ChannelDb) -> Result<Self, Self::Error> {
        let models = serde_json::from_str(&row.models)
            .map_err(|e| bad_row("channel", &format!("invalid models json: {e}")))?;
        let model_mappings = serde_json::from_str(&row.model_mappings)
            .map_err(|e| bad_row("channel", &format!("invalid model_mappings json: {e}")))?;
        let channel_type = ChannelType::from_str(&row.channel_type)
            .map_err(|e| bad_row("channel", &e.to_string()))?;
        Ok(Channel {
            id: Uuid::from_str(&row.id)
                .map_err(|e| bad_row("channel", &format!("invalid id: {e}")))?,
            name: row.name,
            channel_type,
            base_url: row.base_url,
            api_key: row.api_key,
            models,
            priority: row.priority,
            weight: row.weight,
            model_mappings,
            enabled: row.enabled,
            last_test_at: row
                .last_test_at
                .as_deref()
                .map(|s| parse_utc("channel", s))
                .transpose()?,
            last_test_ok: row.last_test_ok,
            created_at: parse_utc("channel", &row.created_at)?,
            updated_at: parse_utc("channel", &row.updated_at)?,
        })
    }
}

fn json_err(e: serde_json::Error) -> RepositoryError {
    RepositoryError::Database(format!("json encode error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channel::{Channel, ModelMapping};
    use crate::infrastructure::sqlite::init_pool;
    use sqlx::Row;

    async fn new_repo() -> SqliteChannelRepository {
        SqliteChannelRepository::new(init_pool("sqlite::memory:").await.expect("init pool"))
    }

    /// Builds a channel sample with JSON columns and nullable columns.
    fn sample() -> Channel {
        let mut c = crate::test_support::sample_channel();
        c.models = vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()];
        c.model_mappings = vec![ModelMapping {
            client_model: "chat".to_string(),
            upstream_model: "gpt-4o".to_string(),
        }];
        c
    }

    /// After save, find by id: all fields (JSON columns, nullable columns, booleans, timestamps) round-trip consistently.
    #[tokio::test]
    async fn save_then_find_roundtrips_all_fields() {
        let repo = new_repo().await;
        let channel = sample();
        repo.save(&channel).await.expect("save");

        let found = repo
            .find_by_id(channel.id)
            .await
            .expect("find")
            .expect("found");
        assert_eq!(found, channel);
    }

    /// Upsert: re-saving the same id overwrites without adding a row.
    #[tokio::test]
    async fn save_upserts_by_id_does_not_duplicate() {
        let repo = new_repo().await;
        let mut channel = sample();
        repo.save(&channel).await.expect("save");

        channel.priority = 10;
        channel.enabled = false;
        repo.save(&channel).await.expect("resave");

        let all = repo.list().await.expect("list");
        assert_eq!(all.len(), 1, "upsert 不产生重复行");
        let found = repo
            .find_by_id(channel.id)
            .await
            .expect("find")
            .expect("found");
        assert_eq!(found.priority, 10);
        assert!(!found.enabled);
    }

    /// Ordering: priority ascending first, then name ascending within the same priority.
    #[tokio::test]
    async fn list_orders_by_priority_then_name() {
        let repo = new_repo().await;
        let mut high = sample();
        high.priority = 5;
        high.name = "a-low-priority".to_string();
        let mut low = sample();
        low.priority = 0;
        low.name = "z-high-priority".to_string();
        let mut same = sample();
        same.priority = 0;
        same.name = "a-same-priority".to_string();
        repo.save(&high).await.expect("save high");
        repo.save(&low).await.expect("save low");
        repo.save(&same).await.expect("save same");

        let names: Vec<String> = repo
            .list()
            .await
            .expect("list")
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(
            names,
            vec!["a-same-priority", "z-high-priority", "a-low-priority"]
        );
    }

    /// Delete: the row is gone after deletion; deleting a missing id again returns NotFound.
    #[tokio::test]
    async fn delete_removes_row_and_missing_errors() {
        let repo = new_repo().await;
        let channel = sample();
        repo.save(&channel).await.expect("save");

        repo.delete(channel.id).await.expect("delete");
        assert!(
            repo.find_by_id(channel.id)
                .await
                .expect("find after delete")
                .is_none()
        );

        let err = repo.delete(channel.id).await.expect_err("delete missing");
        assert!(matches!(err, RepositoryError::NotFound));
    }

    /// All 5 channel types can be persisted and read back.
    #[tokio::test]
    async fn all_channel_types_roundtrip() {
        let repo = new_repo().await;
        for t in [
            ChannelType::OpenAi,
            ChannelType::DeepSeek,
            ChannelType::Custom,
            ChannelType::Claude,
            ChannelType::Gemini,
        ] {
            let mut channel = sample();
            channel.channel_type = t;
            repo.save(&channel).await.expect("save");
            let found = repo
                .find_by_id(channel.id)
                .await
                .expect("find")
                .expect("found");
            assert_eq!(found.channel_type, t);
        }
    }

    #[tokio::test]
    async fn save_writes_fixed_width_utc_timestamps() {
        let repo = new_repo().await;
        let channel = sample();
        repo.save(&channel).await.expect("save");

        let row =
            sqlx::query("SELECT created_at, updated_at, last_test_at FROM channels WHERE id = ?")
                .bind(channel.id.to_string())
                .fetch_one(&repo.pool)
                .await
                .expect("query row");
        let created_at: String = row.get("created_at");
        let updated_at: String = row.get("updated_at");
        let last_test_at: Option<String> = row.get("last_test_at");

        assert_eq!(created_at.len(), "2026-01-01T00:00:00.000000000Z".len());
        assert!(created_at.ends_with('Z'));
        assert_eq!(updated_at.len(), "2026-01-01T00:00:00.000000000Z".len());
        if let Some(last_test_at) = last_test_at {
            assert_eq!(last_test_at.len(), "2026-01-01T00:00:00.000000000Z".len());
        }
    }

    #[tokio::test]
    async fn old_variable_precision_timestamps_remain_readable() {
        let repo = new_repo().await;
        let channel = sample();
        repo.save(&channel).await.expect("save");
        sqlx::query("UPDATE channels SET created_at = ?, updated_at = ? WHERE id = ?")
            .bind("2026-01-01T00:00:00Z")
            .bind("2026-01-01T00:00:00.123Z")
            .bind(channel.id.to_string())
            .execute(&repo.pool)
            .await
            .expect("update timestamps");

        let found = repo
            .find_by_id(channel.id)
            .await
            .expect("find")
            .expect("found");
        assert_eq!(found.created_at.to_rfc3339(), "2026-01-01T00:00:00+00:00");
        assert_eq!(
            found.updated_at.to_rfc3339(),
            "2026-01-01T00:00:00.123+00:00"
        );
    }
}
