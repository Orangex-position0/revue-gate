//! SQLite request log repository implementation: the sqlx implementation of the `RequestLogRepository` trait.
//!
//! Maps to the request_logs table in `migrations/001_init.sql`: INTEGER columns are read as i64, then converted
//! to the domain layer's u16/u32/u64 (negative values indicate corrupted row data). `save` is append-only (each
//! log id is unique); `query` paginates with multi-condition filtering, with WHERE semantics equivalent to
//! `LogQuery::matches` (see domain/request_log.rs); `delete_before` / `clear` maintain the log retention policy.
//! Timestamps are always stored as RFC3339 with a fixed 9 fractional digits.

use std::str::FromStr;

use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::FromRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::domain::error::RepositoryError;
use crate::domain::request_log::{LogPage, LogQuery, RequestLog, RequestLogRepository};
use crate::domain::security_audit::{AuditAction, AuditReport, RiskLevel};
use crate::domain::stats::LogStatRow;

/// Row mapping for the request log table: one-to-one with the current request_logs schema.
/// INTEGER columns are read as i64, then converted to the domain layer's narrow types via `TryFrom`.
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
    risk_level: Option<String>,
    audit_action: Option<String>,
    audit_report: Option<String>,
    created_at: String,
}

/// Row mapping for the stats projection: only the columns needed for aggregation (no request_body, see `LogStatRow`).
#[derive(FromRow)]
struct StatRowDb {
    channel_id: Option<String>,
    model: String,
    status_code: i64,
    total_tokens: Option<i64>,
    duration_ms: i64,
    created_at: String,
}

/// SELECT column list (shared by all queries to avoid repetition).
const SELECT_COLUMNS: &str = "id, api_key_id, channel_id, model, upstream_model, status_code, \
     prompt_tokens, completion_tokens, total_tokens, duration_ms, error_message, is_stream, \
     is_retry, trace_id, request_body, risk_level, audit_action, audit_report, created_at";

/// RequestLogRepository implementation backed by an sqlx pool.
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
    /// Appends a request log (id is unique; no upsert).
    async fn save(&self, log: &RequestLog) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO request_logs (id, api_key_id, channel_id, model, upstream_model, \
                status_code, prompt_tokens, completion_tokens, total_tokens, duration_ms, \
                error_message, is_stream, is_retry, trace_id, request_body, risk_level, \
                audit_action, audit_report, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        .bind(log.risk_level.map(audit_value).transpose()?)
        .bind(log.audit_action.map(audit_value).transpose()?)
        .bind(
            log.audit_report
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| bad_row(&format!("invalid audit_report: {e}")))?,
        )
        .bind(fmt_time(log.created_at))
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

    async fn query(
        &self,
        query: &LogQuery,
        page: u64,
        page_size: u64,
    ) -> Result<LogPage, RepositoryError> {
        let (where_sql, binds) = build_where(query);

        // Total count matching the filters (independent of the current page; lets the frontend compute total pages).
        let count_sql = format!("SELECT COUNT(*) FROM request_logs {where_sql}");
        let mut count = sqlx::query(&count_sql);
        for b in &binds {
            count = count.bind(b.as_str());
        }
        let total: i64 = count.fetch_one(&self.pool).await.map_err(db_err)?.get(0);

        // Current page: LIMIT/OFFSET + time descending (same-time rows fall back to id descending).
        // OFFSET starts at 0, page at 1.
        let offset = (page.saturating_sub(1)).saturating_mul(page_size);
        let page_sql = format!(
            "SELECT {SELECT_COLUMNS} FROM request_logs {where_sql} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let mut page_q = sqlx::query_as::<_, RequestLogDb>(&page_sql);
        for b in &binds {
            page_q = page_q.bind(b.as_str());
        }
        let rows: Vec<RequestLogDb> = page_q
            .bind(page_size as i64)
            .bind(offset as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;

        let items = rows
            .into_iter()
            .map(RequestLog::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(LogPage {
            items,
            total: total.max(0) as u64,
        })
    }

    async fn delete_before(&self, before: DateTime<Utc>) -> Result<u64, RepositoryError> {
        let result = sqlx::query("DELETE FROM request_logs WHERE created_at < ?")
            .bind(fmt_time(before))
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(result.rows_affected())
    }

    async fn clear(&self) -> Result<u64, RepositoryError> {
        let result = sqlx::query("DELETE FROM request_logs")
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(result.rows_affected())
    }

    /// Stats projection: only the columns needed for aggregation (no request_body), half-open interval
    /// `[start_at, end_at)`, matching the date semantics of `LogQuery::matches`. Aggregation happens in
    /// usecases/stats.rs.
    async fn stat_rows(
        &self,
        start_at: Option<DateTime<Utc>>,
        end_at: Option<DateTime<Utc>>,
    ) -> Result<Vec<LogStatRow>, RepositoryError> {
        let mut sql = String::from(
            "SELECT channel_id, model, status_code, total_tokens, duration_ms, created_at FROM request_logs",
        );
        let mut binds: Vec<String> = Vec::new();
        let mut clauses: Vec<&str> = Vec::new();
        if let Some(start) = start_at {
            clauses.push("created_at >= ?");
            binds.push(fmt_time(start));
        }
        if let Some(end) = end_at {
            clauses.push("created_at < ?");
            binds.push(fmt_time(end));
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }

        let mut q = sqlx::query_as::<_, StatRowDb>(&sql);
        for b in &binds {
            q = q.bind(b.as_str());
        }
        let rows: Vec<StatRowDb> = q.fetch_all(&self.pool).await.map_err(db_err)?;
        rows.into_iter().map(LogStatRow::try_from).collect()
    }
}

/// Stored time format: fixed 9 fractional digits + `Z` (`SecondsFormat::Nanos`).
/// A variable-precision format (`to_rfc3339`'s AutoSi) mismatches when comparing subsecond logs against whole-second
/// boundaries — `"12:00:00Z"` vs `"12:00:00.123456789Z"` differ at `'Z'`(0x5A) / `'.'`(0x2E), breaking the half-open
/// semantics of date filtering. Using this format everywhere (write and query binds) keeps values comparable,
/// and nanosecond precision round-trips fully (consistent with the domain layer's `DateTime<Utc>`).
fn fmt_time(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

/// Escapes `%` / `_` / `\` as literals and wraps them into a LIKE full-match pattern (with `ESCAPE '\'`).
/// Consistent with the substring semantics of `LogQuery::matches` (see domain/request_log.rs).
fn like_pattern(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len() + 2);
    escaped.push('%');
    for c in term.chars() {
        if c == '%' || c == '_' || c == '\\' {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped.push('%');
    escaped
}

/// Builds the WHERE clause and bind values from `LogQuery`: dimension order aligns with `LogQuery::matches`
/// (keyword / api_key_id / channel_id / model / date range), all combined with AND.
fn build_where(query: &LogQuery) -> (String, Vec<String>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(keyword) = &query.keyword {
        // keyword does a case-insensitive substring match on four fields (LIKE is case-insensitive for ASCII).
        let pattern = like_pattern(keyword);
        clauses.push(
            "(model LIKE ? ESCAPE '\\' OR upstream_model LIKE ? ESCAPE '\\' \
             OR trace_id LIKE ? ESCAPE '\\' OR error_message LIKE ? ESCAPE '\\')"
                .to_string(),
        );
        for _ in 0..4 {
            binds.push(pattern.clone());
        }
    }
    if let Some(id) = query.api_key_id {
        clauses.push("api_key_id = ?".to_string());
        binds.push(id.to_string());
    }
    if let Some(id) = query.channel_id {
        clauses.push("channel_id = ?".to_string());
        binds.push(id.to_string());
    }
    if let Some(model) = &query.model {
        clauses.push("model LIKE ? ESCAPE '\\'".to_string());
        binds.push(like_pattern(model));
    }
    if let Some(start) = query.start_at {
        clauses.push("created_at >= ?".to_string());
        binds.push(fmt_time(start));
    }
    if let Some(end) = query.end_at {
        clauses.push("created_at < ?".to_string());
        binds.push(fmt_time(end));
    }

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    (where_sql, binds)
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
            risk_level: row
                .risk_level
                .as_deref()
                .map(parse_audit_value::<RiskLevel>)
                .transpose()?,
            audit_action: row
                .audit_action
                .as_deref()
                .map(parse_audit_value::<AuditAction>)
                .transpose()?,
            audit_report: row
                .audit_report
                .as_deref()
                .map(parse_audit_report)
                .transpose()?,
            created_at: parse_utc(&row.created_at)?,
        })
    }
}

fn audit_value<T: serde::Serialize>(value: T) -> Result<String, RepositoryError> {
    serde_json::to_value(value)
        .map_err(|e| bad_row(&format!("invalid audit value: {e}")))?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| bad_row("audit value did not serialize as a string"))
}

fn parse_audit_value<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, RepositoryError> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .map_err(|e| bad_row(&format!("invalid audit value {value:?}: {e}")))
}

fn parse_audit_report(value: &str) -> Result<AuditReport, RepositoryError> {
    serde_json::from_str(value).map_err(|e| bad_row(&format!("invalid audit_report: {e}")))
}

/// Narrows a stored signed i64 to the domain layer's narrow integer type (negative or overflow values
/// indicate corrupted row data).
fn narrow<T: TryFrom<i64>>(column: &str, value: i64) -> Result<T, RepositoryError>
where
    T::Error: std::fmt::Debug,
{
    T::try_from(value).map_err(|_| bad_row(&format!("{column} out of range")))
}

impl TryFrom<StatRowDb> for LogStatRow {
    type Error = RepositoryError;

    fn try_from(row: StatRowDb) -> Result<Self, Self::Error> {
        Ok(LogStatRow {
            channel_id: row
                .channel_id
                .as_deref()
                .map(Uuid::from_str)
                .transpose()
                .map_err(|e| bad_row(&format!("invalid channel_id: {e}")))?,
            model: row.model,
            status_code: narrow("status_code", row.status_code)?,
            total_tokens: row
                .total_tokens
                .map(|t| narrow("total_tokens", t))
                .transpose()?,
            duration_ms: narrow("duration_ms", row.duration_ms)?,
            created_at: parse_utc(&row.created_at)?,
        })
    }
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
    RepositoryError::Database(format!("invalid request_log row: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::sqlite::init_pool;
    use crate::test_support::{all_request_logs, sample_request_log};

    async fn new_repo() -> SqliteRequestLogRepository {
        SqliteRequestLogRepository::new(init_pool("sqlite::memory:").await.expect("init pool"))
    }

    /// After save, find by id: all fields (nullable columns, booleans, token narrow types, timestamps) round-trip consistently.
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
        log.risk_level = Some(RiskLevel::Clean);
        log.audit_action = Some(AuditAction::Allow);
        log.audit_report = Some(AuditReport {
            mode: crate::domain::security_audit::AuditMode::Observe,
            risk_level: RiskLevel::Clean,
            risk_score: 0,
            action: AuditAction::Allow,
            findings: Vec::new(),
            total_findings: 0,
            findings_truncated: false,
            scanned_bytes: 0,
            candidate_bytes: 0,
            scan_byte_limit: 65536,
            truncated: false,
            evidence_level: crate::domain::security_audit::AuditEvidenceLevel::Summary,
            upstream_forwarded: true,
            planned_channel_id: log.channel_id,
            planned_upstream_model: log.upstream_model.clone(),
        });
        repo.save(&log).await.expect("save");

        let found = repo.find_by_id(log.id).await.expect("find").expect("found");
        assert_eq!(found, log);
    }

    /// A missing id returns None.
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

    /// Append-only: re-saving the same id fails with a conflict (no silent upsert overwrite).
    #[tokio::test]
    async fn save_is_append_only() {
        let repo = new_repo().await;
        let log = sample_request_log();
        repo.save(&log).await.expect("save");
        assert!(repo.save(&log).await.is_err(), "重复写入同 id 应报主键冲突");
        assert_eq!(all_request_logs(&repo).await.len(), 1);
    }

    /// Builds a log with a fixed created_at and a controllable trace_id (so filter/pagination tests control time order).
    fn log_at(trace: &str, created_at: DateTime<Utc>) -> RequestLog {
        let mut log = sample_request_log();
        log.trace_id = trace.to_string();
        log.created_at = created_at;
        log
    }

    /// Parses a fixed UTC time point (for tests, to avoid millisecond jitter from Utc::now).
    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .expect("valid rfc3339")
    }

    /// No filter: returns all rows paginated, ordered by created_at descending, total = all rows.
    #[tokio::test]
    async fn query_without_filter_paginates_all_desc() {
        let repo = new_repo().await;
        repo.save(&log_at("older", utc("2026-01-01T00:00:00Z")))
            .await
            .expect("save older");
        repo.save(&log_at("newer", utc("2026-01-02T00:00:00Z")))
            .await
            .expect("save newer");

        let page = repo
            .query(&LogQuery::default(), 1, 10)
            .await
            .expect("query");
        let ids: Vec<&str> = page.items.iter().map(|l| l.trace_id.as_str()).collect();
        assert_eq!(ids, vec!["newer", "older"]);
        assert_eq!(page.total, 2);
    }

    /// keyword: case-insensitive substring match on model (a fully-uppercase model still matches an all-lowercase keyword).
    #[tokio::test]
    async fn query_filters_by_keyword_case_insensitive() {
        let repo = new_repo().await;
        let mut hit = log_at("trace-abc", utc("2026-01-01T00:00:00Z"));
        hit.model = "GPT-4O".to_string();
        let mut miss = log_at("trace-xyz", utc("2026-01-02T00:00:00Z"));
        miss.model = "claude-3-5-sonnet".to_string();
        repo.save(&hit).await.expect("save hit");
        repo.save(&miss).await.expect("save miss");

        let page = repo
            .query(
                &LogQuery {
                    keyword: Some("gpt-4o".into()),
                    ..Default::default()
                },
                1,
                10,
            )
            .await
            .expect("query");
        let ids: Vec<&str> = page.items.iter().map(|l| l.trace_id.as_str()).collect();
        assert_eq!(ids, vec!["trace-abc"]);
        assert_eq!(page.total, 1);
    }

    /// keyword matches in error_message and trace_id also count as hits.
    #[tokio::test]
    async fn query_keyword_matches_error_message_and_trace_id() {
        let repo = new_repo().await;
        let mut by_error = log_at("err", utc("2026-01-01T00:00:00Z"));
        by_error.error_message = Some("upstream returned 500 on channel xyz".into());
        let mut by_trace = log_at("trace-xyz-99", utc("2026-01-02T00:00:00Z"));
        by_trace.model = "claude-3-5-sonnet".to_string();
        let mut miss = log_at("other", utc("2026-01-03T00:00:00Z"));
        miss.model = "claude-3-5-sonnet".to_string();
        repo.save(&by_error).await.expect("save by_error");
        repo.save(&by_trace).await.expect("save by_trace");
        repo.save(&miss).await.expect("save miss");

        let page = repo
            .query(
                &LogQuery {
                    keyword: Some("xyz".into()),
                    ..Default::default()
                },
                1,
                10,
            )
            .await
            .expect("query");
        // "xyz" appears both in by_error's error_message and by_trace's trace_id.
        let mut ids: Vec<&str> = page.items.iter().map(|l| l.trace_id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["err", "trace-xyz-99"]);
        assert_eq!(page.total, 2);
    }

    /// api_key_id: exact match; logs under other keys / unauthenticated logs are excluded.
    #[tokio::test]
    async fn query_filters_by_api_key_id_exact() {
        let repo = new_repo().await;
        let key_a = Uuid::now_v7();
        let mut a = log_at("a", utc("2026-01-01T00:00:00Z"));
        a.api_key_id = Some(key_a);
        let mut b = log_at("b", utc("2026-01-02T00:00:00Z"));
        b.api_key_id = Some(Uuid::now_v7());
        repo.save(&a).await.expect("save a");
        repo.save(&b).await.expect("save b");

        let page = repo
            .query(
                &LogQuery {
                    api_key_id: Some(key_a),
                    ..Default::default()
                },
                1,
                10,
            )
            .await
            .expect("query");
        let ids: Vec<&str> = page.items.iter().map(|l| l.trace_id.as_str()).collect();
        assert_eq!(ids, vec!["a"]);
        assert_eq!(page.total, 1);
    }

    /// channel_id: exact match.
    #[tokio::test]
    async fn query_filters_by_channel_id_exact() {
        let repo = new_repo().await;
        let ch_a = Uuid::now_v7();
        let mut a = log_at("a", utc("2026-01-01T00:00:00Z"));
        a.channel_id = Some(ch_a);
        let mut b = log_at("b", utc("2026-01-02T00:00:00Z"));
        b.channel_id = Some(Uuid::now_v7());
        repo.save(&a).await.expect("save a");
        repo.save(&b).await.expect("save b");

        let page = repo
            .query(
                &LogQuery {
                    channel_id: Some(ch_a),
                    ..Default::default()
                },
                1,
                10,
            )
            .await
            .expect("query");
        let ids: Vec<&str> = page.items.iter().map(|l| l.trace_id.as_str()).collect();
        assert_eq!(ids, vec!["a"]);
    }

    /// model: case-insensitive substring match (a model name prefix can be entered).
    #[tokio::test]
    async fn query_filters_by_model_prefix_case_insensitive() {
        let repo = new_repo().await;
        let mut hit = log_at("hit", utc("2026-01-01T00:00:00Z"));
        hit.model = "gpt-4o-2024-08-06".to_string();
        let mut miss = log_at("miss", utc("2026-01-02T00:00:00Z"));
        miss.model = "claude-3-5-sonnet".to_string();
        repo.save(&hit).await.expect("save hit");
        repo.save(&miss).await.expect("save miss");

        let page = repo
            .query(
                &LogQuery {
                    model: Some("GPT-4O".into()),
                    ..Default::default()
                },
                1,
                10,
            )
            .await
            .expect("query");
        let ids: Vec<&str> = page.items.iter().map(|l| l.trace_id.as_str()).collect();
        assert_eq!(ids, vec!["hit"]);
        assert_eq!(page.total, 1);
    }

    /// Date range: half-open [start_at, end_at) — lower bound inclusive, upper bound exclusive.
    #[tokio::test]
    async fn query_filters_by_half_open_date_range() {
        let repo = new_repo().await;
        repo.save(&log_at("in-range", utc("2026-01-15T00:00:00Z")))
            .await
            .expect("save in");
        repo.save(&log_at("at-start", utc("2026-01-01T00:00:00Z")))
            .await
            .expect("save start");
        repo.save(&log_at("at-end", utc("2026-02-01T00:00:00Z")))
            .await
            .expect("save end");
        repo.save(&log_at("before", utc("2025-12-31T23:59:59Z")))
            .await
            .expect("save before");

        let page = repo
            .query(
                &LogQuery {
                    start_at: Some(utc("2026-01-01T00:00:00Z")),
                    end_at: Some(utc("2026-02-01T00:00:00Z")),
                    ..Default::default()
                },
                1,
                10,
            )
            .await
            .expect("query");
        let ids: Vec<&str> = page.items.iter().map(|l| l.trace_id.as_str()).collect();
        assert_eq!(ids, vec!["in-range", "at-start"]);
        assert_eq!(page.total, 2);
    }

    /// Date range with subsecond created_at: a whole-second lower bound should match subsecond logs; a value equal
    /// to the upper bound should be excluded. Relies on the fixed 9-digit format (`SecondsFormat::Nanos`) — a
    /// variable-precision format makes string comparison of `12:00:00Z` vs `12:00:00.123456789Z` mismatch at `Z`/`.`.
    #[tokio::test]
    async fn query_date_range_handles_subsecond_created_at() {
        let repo = new_repo().await;
        repo.save(&log_at("sub", utc("2026-01-01T12:00:00.123456Z")))
            .await
            .expect("save sub");

        // A whole-second lower bound of 12:00:00Z should match 12:00:00.123456Z (>= semantics).
        let page = repo
            .query(
                &LogQuery {
                    start_at: Some(utc("2026-01-01T12:00:00Z")),
                    ..Default::default()
                },
                1,
                10,
            )
            .await
            .expect("query");
        let ids: Vec<&str> = page.items.iter().map(|l| l.trace_id.as_str()).collect();
        assert_eq!(ids, vec!["sub"]);

        // Subsecond upper bound 12:00:00.123456Z: sub equals the bound exactly, so `<` semantics should exclude it.
        let page = repo
            .query(
                &LogQuery {
                    end_at: Some(utc("2026-01-01T12:00:00.123456Z")),
                    ..Default::default()
                },
                1,
                10,
            )
            .await
            .expect("query");
        assert!(page.items.is_empty());
    }

    /// Combined filters: intersection of api_key_id + model + date range.
    #[tokio::test]
    async fn query_combines_multiple_filters() {
        let repo = new_repo().await;
        let key = Uuid::now_v7();
        let mut target = log_at("target", utc("2026-01-15T00:00:00Z"));
        target.api_key_id = Some(key);
        target.model = "gpt-4o".to_string();
        let mut wrong_key = log_at("wrong-key", utc("2026-01-15T00:00:00Z"));
        wrong_key.model = "gpt-4o".to_string();
        let mut wrong_model = log_at("wrong-model", utc("2026-01-15T00:00:00Z"));
        wrong_model.api_key_id = Some(key);
        wrong_model.model = "claude-3".to_string();
        let mut wrong_date = log_at("wrong-date", utc("2026-03-01T00:00:00Z"));
        wrong_date.api_key_id = Some(key);
        wrong_date.model = "gpt-4o".to_string();
        repo.save(&target).await.expect("save target");
        repo.save(&wrong_key).await.expect("save wrong_key");
        repo.save(&wrong_model).await.expect("save wrong_model");
        repo.save(&wrong_date).await.expect("save wrong_date");

        let page = repo
            .query(
                &LogQuery {
                    api_key_id: Some(key),
                    model: Some("gpt-4o".into()),
                    start_at: Some(utc("2026-01-01T00:00:00Z")),
                    end_at: Some(utc("2026-02-01T00:00:00Z")),
                    ..Default::default()
                },
                1,
                10,
            )
            .await
            .expect("query");
        let ids: Vec<&str> = page.items.iter().map(|l| l.trace_id.as_str()).collect();
        assert_eq!(ids, vec!["target"]);
        assert_eq!(page.total, 1);
    }

    /// LIKE wildcard literal matching: `%`/`_` in model are escaped as literals, not treated as wildcards.
    #[tokio::test]
    async fn query_escapes_like_wildcards_in_model() {
        let repo = new_repo().await;
        let mut literal = log_at("literal", utc("2026-01-01T00:00:00Z"));
        literal.model = "100%_custom".to_string();
        let mut normal = log_at("normal", utc("2026-01-02T00:00:00Z"));
        normal.model = "100percent".to_string();
        repo.save(&literal).await.expect("save literal");
        repo.save(&normal).await.expect("save normal");

        let page = repo
            .query(
                &LogQuery {
                    model: Some("100%_custom".into()),
                    ..Default::default()
                },
                1,
                10,
            )
            .await
            .expect("query");
        let ids: Vec<&str> = page.items.iter().map(|l| l.trace_id.as_str()).collect();
        assert_eq!(ids, vec!["literal"]);
        assert_eq!(page.total, 1);
    }

    /// Pagination: page_size truncates, the last page takes the remainder, total always = all hits (independent
    /// of the current page).
    #[tokio::test]
    async fn query_paginates_and_tracks_total() {
        let repo = new_repo().await;
        for i in 0..5 {
            let day = i + 1;
            let t = format!("2026-01-0{day}T00:00:00Z");
            repo.save(&log_at(&format!("log-{i}"), utc(&t)))
                .await
                .expect("save");
        }

        let page1 = repo
            .query(&LogQuery::default(), 1, 2)
            .await
            .expect("page 1");
        assert_eq!(page1.items.len(), 2);
        assert_eq!(page1.total, 5);
        // Newest first: log-4 (01-05) comes first.
        let ids: Vec<&str> = page1.items.iter().map(|l| l.trace_id.as_str()).collect();
        assert_eq!(ids, vec!["log-4", "log-3"]);

        let page3 = repo
            .query(&LogQuery::default(), 3, 2)
            .await
            .expect("page 3");
        let ids3: Vec<&str> = page3.items.iter().map(|l| l.trace_id.as_str()).collect();
        assert_eq!(ids3, vec!["log-0"]);
        assert_eq!(page3.total, 5);
    }

    /// Deletes logs strictly older than the threshold (< before); boundary logs are kept.
    #[tokio::test]
    async fn delete_before_removes_only_older_logs() {
        let repo = new_repo().await;
        repo.save(&log_at("old", utc("2025-12-31T23:59:59Z")))
            .await
            .expect("save old");
        repo.save(&log_at("boundary", utc("2026-01-01T00:00:00Z")))
            .await
            .expect("save boundary");
        repo.save(&log_at("new", utc("2026-01-02T00:00:00Z")))
            .await
            .expect("save new");

        let n = repo
            .delete_before(utc("2026-01-01T00:00:00Z"))
            .await
            .expect("delete");
        assert_eq!(n, 1);

        let remaining: Vec<String> = all_request_logs(&repo)
            .await
            .into_iter()
            .map(|l| l.trace_id)
            .collect();
        assert_eq!(remaining, vec!["new".to_string(), "boundary".to_string()]);
    }

    /// Clear: deletes all logs, returning the number deleted.
    #[tokio::test]
    async fn clear_removes_all_logs() {
        let repo = new_repo().await;
        repo.save(&log_at("a", utc("2026-01-01T00:00:00Z")))
            .await
            .expect("save a");
        repo.save(&log_at("b", utc("2026-01-02T00:00:00Z")))
            .await
            .expect("save b");

        let n = repo.clear().await.expect("clear");
        assert_eq!(n, 2);
        assert!(all_request_logs(&repo).await.is_empty());
    }

    /// Stats projection: only aggregation columns (no request_body field), filtered by a half-open range
    /// (lower inclusive, upper exclusive).
    #[tokio::test]
    async fn stat_rows_projects_within_half_open_range() {
        let repo = new_repo().await;
        let mut in_range = log_at("in", utc("2026-01-15T12:00:00Z"));
        in_range.status_code = 502;
        in_range.total_tokens = Some(99);
        in_range.duration_ms = 1234;
        in_range.request_body = Some("should-not-be-loaded".to_string());
        repo.save(&in_range).await.expect("save in");
        repo.save(&log_at("at-start", utc("2026-01-01T00:00:00Z")))
            .await
            .expect("save start");
        repo.save(&log_at("at-end", utc("2026-02-01T00:00:00Z")))
            .await
            .expect("save end");

        let rows = repo
            .stat_rows(
                Some(utc("2026-01-01T00:00:00Z")),
                Some(utc("2026-02-01T00:00:00Z")),
            )
            .await
            .expect("stat rows");
        assert_eq!(rows.len(), 2, "下界含、上界不含");
        let row = rows.iter().find(|r| r.status_code == 502).expect("in row");
        assert_eq!(row.channel_id, in_range.channel_id);
        assert_eq!(row.model, in_range.model);
        assert_eq!(row.total_tokens, Some(99));
        assert_eq!(row.duration_ms, 1234);
        assert_eq!(row.created_at, utc("2026-01-15T12:00:00Z"));
    }

    /// Stats projection: with no time range, all log rows are returned.
    #[tokio::test]
    async fn stat_rows_without_range_returns_all_rows() {
        let repo = new_repo().await;
        repo.save(&log_at("a", utc("2026-01-01T00:00:00Z")))
            .await
            .expect("save a");
        repo.save(&log_at("b", utc("2026-01-02T00:00:00Z")))
            .await
            .expect("save b");

        let rows = repo.stat_rows(None, None).await.expect("stat rows");
        assert_eq!(rows.len(), 2);
    }
}
