//! SQLite 请求日志仓储实现：`RequestLogRepository` trait 的 sqlx 落地。
//!
//! 与 `migrations/001_init.sql` 的 request_logs 表对应：INTEGER 列读为 i64 再转换为领域层
//! u16/u32/u64（负值视为行数据损坏）。`save` 为追加语义（每条日志 id 唯一），`list()`
//! 按创建时间倒序（最新在前，日志列表视图默认，见 domain/request_log.rs）。

use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::domain::error::RepositoryError;
use crate::domain::request_log::{RequestLog, RequestLogRepository};

/// 请求日志表行映射：与 `migrations/001_init.sql` 的 request_logs 列一一对应。
/// INTEGER 列读为 i64，再由 `TryFrom` 转换为领域层的窄类型。
#[derive(FromRow)]
struct RequestLogDb {
    id: String,
    api_key_id: Option<String>,
    channel_id: Option<String>,
    model: String,
    upstream_model: Option<String>,
    status_code: i64,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    total_tokens: Option<i64>,
    duration_ms: i64,
    error_message: Option<String>,
    is_stream: bool,
    is_retry: bool,
    trace_id: String,
    request_body: Option<String>,
    created_at: String,
}

/// 查询列清单（各查询共用，避免重复书写）。
const SELECT_COLUMNS: &str = "id, api_key_id, channel_id, model, upstream_model, status_code, \
     prompt_tokens, completion_tokens, total_tokens, duration_ms, error_message, is_stream, \
     is_retry, trace_id, request_body, created_at";

/// 基于 sqlx 连接池的 RequestLogRepository 实现。
pub struct SqliteRequestLogRepository {
    pool: SqlitePool,
}

impl SqliteRequestLogRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl RequestLogRepository for SqliteRequestLogRepository {
    /// 追加写入一条请求日志（id 唯一，不做 upsert）。
    async fn save(&self, log: &RequestLog) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO request_logs (id, api_key_id, channel_id, model, upstream_model, \
                status_code, prompt_tokens, completion_tokens, total_tokens, duration_ms, \
                error_message, is_stream, is_retry, trace_id, request_body, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(log.id.to_string())
        .bind(log.api_key_id.map(|id| id.to_string()))
        .bind(log.channel_id.map(|id| id.to_string()))
        .bind(&log.model)
        .bind(&log.upstream_model)
        .bind(i64::from(log.status_code))
        .bind(log.prompt_tokens.map(i64::from))
        .bind(log.completion_tokens.map(i64::from))
        .bind(log.total_tokens.map(i64::from))
        .bind(i64::try_from(log.duration_ms).map_err(|_| bad_row("duration_ms out of range"))?)
        .bind(&log.error_message)
        .bind(log.is_stream)
        .bind(log.is_retry)
        .bind(&log.trace_id)
        .bind(&log.request_body)
        .bind(log.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<RequestLog>, RepositoryError> {
        let row: Option<RequestLogDb> = sqlx::query_as(&format!(
            "SELECT {SELECT_COLUMNS} FROM request_logs WHERE id = ?"
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        row.map(RequestLog::try_from).transpose()
    }

    /// 按创建时间倒序（最新在前）；同时间按 id 倒序兜底（uuid v7 时间有序）。
    async fn list(&self) -> Result<Vec<RequestLog>, RepositoryError> {
        let rows: Vec<RequestLogDb> = sqlx::query_as(&format!(
            "SELECT {SELECT_COLUMNS} FROM request_logs ORDER BY created_at DESC, id DESC"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(RequestLog::try_from).collect()
    }
}

impl TryFrom<RequestLogDb> for RequestLog {
    type Error = RepositoryError;

    fn try_from(row: RequestLogDb) -> Result<Self, Self::Error> {
        Ok(RequestLog {
            id: Uuid::from_str(&row.id).map_err(|e| bad_row(&format!("invalid id: {e}")))?,
            api_key_id: row
                .api_key_id
                .as_deref()
                .map(Uuid::from_str)
                .transpose()
                .map_err(|e| bad_row(&format!("invalid api_key_id: {e}")))?,
            channel_id: row
                .channel_id
                .as_deref()
                .map(Uuid::from_str)
                .transpose()
                .map_err(|e| bad_row(&format!("invalid channel_id: {e}")))?,
            model: row.model,
            upstream_model: row.upstream_model,
            status_code: narrow("status_code", row.status_code)?,
            prompt_tokens: row
                .prompt_tokens
                .map(|t| narrow("prompt_tokens", t))
                .transpose()?,
            completion_tokens: row
                .completion_tokens
                .map(|t| narrow("completion_tokens", t))
                .transpose()?,
            total_tokens: row
                .total_tokens
                .map(|t| narrow("total_tokens", t))
                .transpose()?,
            duration_ms: narrow("duration_ms", row.duration_ms)?,
            error_message: row.error_message,
            is_stream: row.is_stream,
            is_retry: row.is_retry,
            trace_id: row.trace_id,
            request_body: row.request_body,
            created_at: parse_utc(&row.created_at)?,
        })
    }
}

/// 把库内有符号 i64 收窄为领域层的窄整数类型（负值或溢出视为行数据损坏）。
fn narrow<T: TryFrom<i64>>(column: &str, value: i64) -> Result<T, RepositoryError>
where
    T::Error: std::fmt::Debug,
{
    T::try_from(value).map_err(|_| bad_row(&format!("{column} out of range")))
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
    RepositoryError::Database(format!("invalid request_log row: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::sqlite::init_pool;
    use crate::test_support::sample_request_log;

    async fn new_repo() -> SqliteRequestLogRepository {
        SqliteRequestLogRepository::new(init_pool("sqlite::memory:").await.expect("init pool"))
    }

    /// 保存后按 id 找回：全部字段（含可空列、布尔、token 窄类型、时间）往返一致。
    #[tokio::test]
    async fn save_then_find_roundtrips_all_fields() {
        let repo = new_repo().await;
        let mut log = sample_request_log();
        log.api_key_id = Some(Uuid::now_v7());
        log.channel_id = Some(Uuid::now_v7());
        log.upstream_model = Some("gpt-4o-upstream".to_string());
        log.status_code = 502;
        log.prompt_tokens = Some(10);
        log.completion_tokens = Some(5);
        log.total_tokens = Some(15);
        log.duration_ms = 1234;
        log.error_message = Some("upstream returned status 500".to_string());
        log.is_stream = false;
        log.is_retry = true;
        log.trace_id = "trace-abc".to_string();
        log.request_body = Some(r#"{"model":"gpt-4o"}"#.to_string());
        repo.save(&log).await.expect("save");

        let found = repo.find_by_id(log.id).await.expect("find").expect("found");
        assert_eq!(found, log);
    }

    /// 未命中 id 返回 None。
    #[tokio::test]
    async fn find_missing_returns_none() {
        let repo = new_repo().await;
        assert!(
            repo.find_by_id(Uuid::now_v7())
                .await
                .expect("find")
                .is_none()
        );
    }

    /// 排序：按创建时间倒序，最新在前（日志列表默认视图）。
    #[tokio::test]
    async fn list_orders_by_created_desc() {
        let repo = new_repo().await;
        let mut older = sample_request_log();
        older.trace_id = "older".to_string();
        older.created_at = Utc::now() - chrono::Duration::seconds(60);
        let mut newer = sample_request_log();
        newer.trace_id = "newer".to_string();
        repo.save(&older).await.expect("save older");
        repo.save(&newer).await.expect("save newer");

        let ids: Vec<String> = repo
            .list()
            .await
            .expect("list")
            .into_iter()
            .map(|l| l.trace_id)
            .collect();
        assert_eq!(ids, vec!["newer", "older"]);
    }

    /// 追加语义：同 id 重复 save 直接冲突报错（不做 upsert 静默覆盖）。
    #[tokio::test]
    async fn save_is_append_only() {
        let repo = new_repo().await;
        let log = sample_request_log();
        repo.save(&log).await.expect("save");
        assert!(repo.save(&log).await.is_err(), "重复写入同 id 应报主键冲突");
        assert_eq!(repo.list().await.expect("list").len(), 1);
    }
}
