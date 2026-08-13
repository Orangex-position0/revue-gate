//! SQLite 密钥仓储实现：`ApiKeyRepository` trait 的 sqlx 落地。
//!
//! 与 `migrations/001_init.sql` 的 api_keys 表对应：`quota_limit` / `quota_used` 以
//! INTEGER 存取（SQLite 为有符号 64 位，读回时从 i64 转换为 u64，负值视为行数据损坏）。
//! `key` 列带 UNIQUE 约束；`delete` 未命中返回 `RepositoryError::NotFound`。

use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::domain::api_key::{ApiKey, ApiKeyRepository, Quota};
use crate::domain::error::RepositoryError;

/// 密钥表行映射：与 `migrations/001_init.sql` 的 api_keys 列一一对应。
/// INTEGER 列读为 i64，再由 `TryFrom` 转换为领域层的 u64。
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

/// 查询列清单（各查询共用，避免重复书写）。
const SELECT_COLUMNS: &str = "id, name, key, enabled, quota_limit, quota_used, \
     created_at, updated_at";

/// 基于 sqlx 连接池的 ApiKeyRepository 实现。
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

    /// 列出全部密钥，按「名称升序 → 创建时间升序」排序（列表视图的稳定顺序）。
    async fn list(&self) -> Result<Vec<ApiKey>, RepositoryError> {
        let rows: Vec<ApiKeyDb> = sqlx::query_as(&format!(
            "SELECT {SELECT_COLUMNS} FROM api_keys ORDER BY name ASC, created_at ASC"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(ApiKey::try_from).collect()
    }

    /// upsert 语义：存在同 id 行则覆盖，否则插入。
    /// 密钥明文不随更新变更，故 ON CONFLICT 不覆盖 `key`（UNIQUE 列若参与覆盖，
    /// 更新 key 可能触发 UNIQUE 冲突导致整条写入失败）。
    async fn save(&self, api_key: &ApiKey) -> Result<(), RepositoryError> {
        // 配额列以有符号 i64 落库：超 i64 范围的值拒绝写入（否则读回时 i64_to_u64 判坏行）。
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

    /// 删除密钥；未命中（0 行受影响）返回 `NotFound`。
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

/// 把库内有符号 i64 转换为领域层 u64（负值说明行数据损坏）。
fn i64_to_u64(value: i64, column: &str) -> Result<u64, RepositoryError> {
    u64::try_from(value).map_err(|_| bad_row(&format!("{column} out of range for u64")))
}

/// 解析库内 RFC3339 时间字符串为 `DateTime<Utc>`。
fn parse_utc(s: &str) -> Result<DateTime<Utc>, RepositoryError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| bad_row(&format!("invalid timestamp {s:?}: {e}")))
}

fn db_err(e: sqlx::Error) -> RepositoryError {
    RepositoryError::Database(e.to_string())
}

/// 构造「行数据损坏」类错误：库内数据无法解析为领域模型。
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

    /// 保存后按 id 找回：全部字段（含配额、可空列、布尔、时间）往返一致。
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

    /// find_by_key：按密钥明文查找（认证用），未命中返回 None。
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

    /// upsert：同 id 重复保存只覆盖不新增行（配额与启停更新生效）。
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

    /// 排序：按名称升序（同名按创建时间升序）。
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

    /// 删除：删除后查无此行；对缺失 id 再删返回 NotFound。
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

    /// 无上限（quota_limit = NULL）读回为 None。
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
