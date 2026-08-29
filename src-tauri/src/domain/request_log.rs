//! RequestLog aggregate root: entity + query filter value object + RequestLogRepository trait.
//!
//! Every request is fully logged (`docs/Requirements.md` request logging): key/channel/model/usage/duration/
//! trace id/request body/is-stream/is-retry. Query pagination and multi-criteria filtering (keyword/key/channel/model/
//! date range) arrive in stage 10; `LogQuery::matches` is the single authoritative definition of the filter semantics,
//! and the sqlx implementation generates an equivalent WHERE from it (see infrastructure/sqlite/request_log.rs).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::error::RepositoryError;
use crate::domain::security_audit::{AuditAction, AuditReport, RiskLevel};
use crate::domain::stats::LogStatRow;

/// RequestLog entity: the complete audit record of one request.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLog {
    pub id: Uuid,
    /// Id of the local key that made the request (always present when authentication succeeded).
    pub api_key_id: Option<Uuid>,
    /// Id of the upstream channel actually used (None when no candidate remained).
    pub channel_id: Option<Uuid>,
    /// Model name requested by the client.
    pub model: String,
    /// Actual upstream model name (after model mapping).
    pub upstream_model: Option<String>,
    /// HTTP status code the gateway returned to the client.
    pub status_code: u16,
    /// usage: prompt / completion / total tokens (backfilled after streaming parse).
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    /// Total request duration (milliseconds).
    pub duration_ms: u64,
    /// Error message on failure (no candidates left, forward failure, etc.).
    pub error_message: Option<String>,
    /// Whether the request was streaming.
    pub is_stream: bool,
    /// Whether a retry occurred.
    pub is_retry: bool,
    /// Trace id spanning the request, for request tracing.
    pub trace_id: String,
    /// Client request body (raw JSON).
    pub request_body: Option<String>,
    /// Canonical OpenAI Chat choices array returned by the model.
    pub response_choices: Option<String>,
    /// Request-level audit risk. None means audit was disabled for this request.
    pub risk_level: Option<RiskLevel>,
    /// Final audit action. None means audit was disabled for this request.
    pub audit_action: Option<AuditAction>,
    /// Structured audit report projection. None means audit was disabled for this request.
    pub audit_report: Option<AuditReport>,
    pub created_at: DateTime<Utc>,
}

/// Log query filter criteria: all optional, None = do not filter that dimension.
/// Serialized across the Tauri Command boundary (camelCase, aligned with `LogQuery` in `src/types/index.ts`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogQuery {
    /// Keyword: case-insensitive substring match over model / upstream_model / trace_id / error_message.
    #[serde(default)]
    pub keyword: Option<String>,
    /// Filter by originating key (exact api_key_id match; unauthenticated logs are naturally excluded).
    #[serde(default)]
    pub api_key_id: Option<Uuid>,
    /// Filter by channel (exact channel_id match).
    #[serde(default)]
    pub channel_id: Option<Uuid>,
    /// Filter by requested model name (case-insensitive substring match, convenient for typing a model prefix).
    #[serde(default)]
    pub model: Option<String>,
    /// Date range lower bound (inclusive): `created_at >= start_at`.
    #[serde(default)]
    pub start_at: Option<DateTime<Utc>>,
    /// Date range upper bound (exclusive): `created_at < end_at`.
    #[serde(default)]
    pub end_at: Option<DateTime<Utc>>,
}

impl LogQuery {
    /// Whether a log matches all filter criteria (None = dimension not filtered).
    ///
    /// This is the **single authoritative definition** of the filter semantics; the sqlx implementation generates an equivalent WHERE clause from it
    /// (see infrastructure/sqlite/request_log.rs): keyword / model are case-insensitive
    /// substring matches (the LIKE side escapes `%`/`_` to keep literal matching), and the date range is half-open
    /// `[start_at, end_at)`. The in-memory repository calls this function directly, consistent with the SQL behavior.
    pub fn matches(&self, log: &RequestLog) -> bool {
        if let Some(keyword) = self.keyword.as_deref() {
            let needle = keyword.to_lowercase();
            let hit = [
                Some(log.model.as_str()),
                log.upstream_model.as_deref(),
                Some(log.trace_id.as_str()),
                log.error_message.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|field| field.to_lowercase().contains(&needle));
            if !hit {
                return false;
            }
        }
        if let Some(id) = self.api_key_id
            && log.api_key_id != Some(id)
        {
            return false;
        }
        if let Some(id) = self.channel_id
            && log.channel_id != Some(id)
        {
            return false;
        }
        if let Some(model) = self.model.as_deref()
            && !log.model.to_lowercase().contains(&model.to_lowercase())
        {
            return false;
        }
        if let Some(start_at) = self.start_at
            && log.created_at < start_at
        {
            return false;
        }
        if let Some(end_at) = self.end_at
            && log.created_at >= end_at
        {
            return false;
        }
        true
    }
}

/// Paginated query result: current page of logs + total count matching the filter (for the frontend to compute total pages).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    /// Logs on the current page (descending by creation time, newest first).
    pub items: Vec<RequestLog>,
    /// Total number of logs matching the filter (independent of the current page).
    pub total: u64,
}

/// RequestLog repository trait: interface defined in the domain layer, sqlx implementation provided by infrastructure.
#[async_trait::async_trait]
pub trait RequestLogRepository: Send + Sync {
    /// Write one request log.
    async fn save(&self, log: &RequestLog) -> Result<(), RepositoryError>;
    /// Look up log details by id.
    async fn find_by_id(&self, id: Uuid) -> Result<Option<RequestLog>, RepositoryError>;
    /// Paginated log query: multi-criteria filter (keyword / key / channel / model / date range),
    /// descending by creation time (ties broken by descending id), page starts at 1.
    async fn query(
        &self,
        query: &LogQuery,
        page: u64,
        page_size: u64,
    ) -> Result<LogPage, RepositoryError>;
    /// Delete logs created strictly before `before` (half-open upper bound), returning the deleted count.
    async fn delete_before(&self, before: DateTime<Utc>) -> Result<u64, RepositoryError>;
    /// Clear all logs, returning the deleted count.
    async fn clear(&self) -> Result<u64, RepositoryError>;
    /// Stat projection: returns lightweight log rows in the half-open interval `[start_at, end_at)`
    /// (None time dimension = no bound). Aggregation lives in usecases; this method only fetches and filters.
    async fn stat_rows(
        &self,
        start_at: Option<DateTime<Utc>>,
        end_at: Option<DateTime<Utc>>,
    ) -> Result<Vec<LogStatRow>, RepositoryError>;
}
