//! 基础设施层 SQLite 模块入口：连接池初始化 + 内嵌迁移（sqlite.rs 承载入口，
//! 各仓储实现落地于 sqlite/ 子目录，遵循 2024 Edition `foo.rs` 模块布局）。

pub mod channel;

use std::str::FromStr;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// 基础设施层错误：连接/查询与迁移失败统一在此收口。
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// 打开 SQLite 连接池并应用内嵌迁移（`sqlx::migrate!` 打包 `migrations/` 目录）。
///
/// `db_path` 支持文件路径或 `sqlite::memory:`（仅测试用）；文件不存在时自动创建。
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

    /// 内嵌迁移应用后，核心业务表（channels / api_keys / request_logs）应存在。
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
