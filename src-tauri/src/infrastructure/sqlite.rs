//! Infrastructure SQLite module entry: connection pool init + embedded migrations (sqlite.rs is the entry;
//! repository implementations live under sqlite/, following the 2024 Edition `foo.rs` module layout).

pub mod api_key;
pub mod channel;
pub mod knowledge;
pub mod request_log;

use chrono::{DateTime, SecondsFormat, Utc};
use std::str::FromStr;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::domain::error::RepositoryError;

/// Infrastructure layer error: connection/query and migration failures are unified here.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Opens a SQLite connection pool and applies embedded migrations (`sqlx::migrate!` bundles the `migrations/` directory).
///
/// `db_path` supports a file path or `sqlite::memory:` (tests only); the file is auto-created when missing.
pub async fn init_pool(db_path: &str) -> Result<SqlitePool, DbError> {
    let options = SqliteConnectOptions::from_str(db_path)?.create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

pub(crate) fn fmt_utc(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

pub(crate) fn parse_utc(row_kind: &str, s: &str) -> Result<DateTime<Utc>, RepositoryError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| bad_row(row_kind, &format!("invalid timestamp {s:?}: {e}")))
}

pub(crate) fn db_err(e: sqlx::Error) -> RepositoryError {
    RepositoryError::Database(e.to_string())
}

pub(crate) fn bad_row(row_kind: &str, reason: &str) -> RepositoryError {
    RepositoryError::Database(format!("invalid {row_kind} row: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Row;

    /// After embedded migrations are applied, the core business tables (channels / api_keys / request_logs) should exist.
    #[tokio::test]
    async fn init_pool_applies_embedded_migrations() {
        let dir = std::env::temp_dir().join(format!("revue-gate-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("test.db");
        let path_str = path.to_str().expect("utf8 temp path");

        let pool = init_pool(path_str).await.expect("init pool");

        let rows = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
            .fetch_all(&pool)
            .await
            .expect("query sqlite_master");
        let names: Vec<String> = rows.iter().map(|row| row.get::<String, _>(0)).collect();
        assert!(names.contains(&"channels".to_string()), "tables: {names:?}");
        assert!(names.contains(&"api_keys".to_string()), "tables: {names:?}");
        assert!(
            names.contains(&"request_logs".to_string()),
            "tables: {names:?}"
        );
        for table in [
            "kb_knowledge_bases",
            "kb_sources",
            "kb_documents",
            "kb_chunks",
            "kb_tasks",
            "kb_index_meta",
            "kb_conversations",
        ] {
            assert!(
                names.contains(&table.to_string()),
                "missing {table}: {names:?}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
