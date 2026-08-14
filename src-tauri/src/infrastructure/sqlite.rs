//! Infrastructure SQLite module entry: connection pool init + embedded migrations (sqlite.rs is the entry;
//! repository implementations live under sqlite/, following the 2024 Edition `foo.rs` module layout).

pub mod api_key;
pub mod channel;
pub mod request_log;

use std::str::FromStr;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

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

        let _ = std::fs::remove_dir_all(&dir);
    }
}
