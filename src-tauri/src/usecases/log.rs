//! Request log query use cases: paginated filtering / detail parsing / delete-by-date / clear orchestration.
//!
//! Stateless use cases: repositories are injected as `&dyn RequestLogRepository`, seam A tests can use in-memory mocks.
//! Detail parsing extracts "conversation / request params / tool tags" from the request body JSON in the use case layer (serde_json parsing;
//! the domain layer keeps pure types and introduces no parsing logic, see domain/request_log.rs); route fields (channel /
//! upstream / usage / trace) come directly from `RequestLog`, and the frontend resolves names via the existing channel / key APIs.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::error::RepositoryError;
use crate::domain::request_log::{LogPage, LogQuery, RequestLog, RequestLogRepository};

/// Log use case layer error.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("request log not found")]
    NotFound,
    #[error("request log repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// Log detail: `RequestLog` (with route / usage / raw JSON) + the structured view parsed from the request body.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogDetail {
    #[serde(flatten)]
    pub log: RequestLog,
    /// Conversation: role / content / triggered tool calls of request_body.messages.
    pub conversation: Vec<ConversationMessage>,
    /// Request params: request_body top-level fields except messages / tools / stream.
    pub request_params: Value,
    /// Tool tags: top-level tools[].function.name and each message's tool_calls[].function.name merged with dedup.
    pub tool_names: Vec<String>,
}

/// A conversation message (for detail display; content is text or None — empty / non-text content is not shown).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub role: String,
    pub content: Option<String>,
    /// Tool call names triggered by this message (tool_calls[].function.name of assistant messages).
    pub tool_names: Vec<String>,
}

/// Paginated log query: multi-condition filtering + page number, delegated to the repository implementation (see `LogQuery` for the single source of filter semantics).
pub struct ListLogsUsecase;
impl ListLogsUsecase {
    pub async fn execute(
        &self,
        repo: &dyn RequestLogRepository,
        query: LogQuery,
        page: u64,
        page_size: u64,
    ) -> Result<LogPage, LogError> {
        Ok(repo.query(&query, page, page_size).await?)
    }
}

/// Log detail: load by id, parse the request body into conversation / params / tool tags. Returns `NotFound` if missing.
pub struct GetLogDetailUsecase;
impl GetLogDetailUsecase {
    pub async fn execute(
        &self,
        repo: &dyn RequestLogRepository,
        id: Uuid,
    ) -> Result<LogDetail, LogError> {
        let log = repo.find_by_id(id).await?.ok_or(LogError::NotFound)?;
        let (conversation, request_params, tool_names) = parse_detail(log.request_body.as_deref());
        Ok(LogDetail {
            log,
            conversation,
            request_params,
            tool_names,
        })
    }
}

/// Delete logs with created_at strictly before `before` (half-open upper bound), returning the number of deleted rows.
pub struct DeleteLogsBeforeUsecase;
impl DeleteLogsBeforeUsecase {
    pub async fn execute(
        &self,
        repo: &dyn RequestLogRepository,
        before: DateTime<Utc>,
    ) -> Result<u64, LogError> {
        Ok(repo.delete_before(before).await?)
    }
}

/// Clear all request logs, returning the number of deleted rows.
pub struct ClearLogsUsecase;
impl ClearLogsUsecase {
    pub async fn execute(&self, repo: &dyn RequestLogRepository) -> Result<u64, LogError> {
        Ok(repo.clear().await?)
    }
}

/// Parse the log request body JSON into (conversation, request params, tool tags).
/// Missing body / invalid JSON / non-object all return empty values — the detail can still show the route and raw JSON, and parsing failure does not error.
fn parse_detail(body: Option<&str>) -> (Vec<ConversationMessage>, Value, Vec<String>) {
    let Some(body) = body else {
        return (Vec::new(), Value::Null, Vec::new());
    };
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return (Vec::new(), Value::Null, Vec::new());
    };
    if !value.is_object() {
        return (Vec::new(), Value::Null, Vec::new());
    }

    let mut tool_names: Vec<String> = Vec::new();
    // Top-level tools[] definitions (declared but not necessarily called).
    if let Some(tools) = value.get("tools").and_then(Value::as_array) {
        for t in tools {
            if let Some(name) = t.pointer("/function/name").and_then(Value::as_str) {
                push_unique(&mut tool_names, name);
            }
        }
    }

    // Conversation + tool calls triggered by each message (also merged into the global tool tags).
    let conversation = value
        .get("messages")
        .and_then(Value::as_array)
        .map(|msgs| {
            msgs.iter()
                .map(|m| {
                    let mut msg_tools: Vec<String> = Vec::new();
                    if let Some(calls) = m.get("tool_calls").and_then(Value::as_array) {
                        for c in calls {
                            if let Some(name) = c.pointer("/function/name").and_then(Value::as_str)
                            {
                                push_unique(&mut msg_tools, name);
                                push_unique(&mut tool_names, name);
                            }
                        }
                    }
                    ConversationMessage {
                        role: m
                            .get("role")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        content: m.get("content").and_then(Value::as_str).map(str::to_string),
                        tool_names: msg_tools,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Request params: top-level fields except messages / tools / stream (stream is carried by the log's is_stream).
    let mut params = value.clone();
    if let Some(obj) = params.as_object_mut() {
        obj.remove("messages");
        obj.remove("tools");
        obj.remove("stream");
    }

    (conversation, params, tool_names)
}

/// Append with dedup (keeps first-occurrence order).
fn push_unique(seen: &mut Vec<String>, name: &str) {
    if !seen.iter().any(|s| s == name) {
        seen.push(name.to_string());
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::domain::security_audit::{
        AuditAction, AuditEvidenceLevel, AuditMode, AuditPolicy, AuditReport, RiskLevel,
    };
    use crate::test_support::{InMemoryRequestLogRepository, all_request_logs, sample_request_log};

    /// Build a sample request body with messages / params / tools.
    fn sample_body() -> String {
        serde_json::json!({
            "model": "gpt-4o",
            "temperature": 0.7,
            "stream": false,
            "messages": [
                {"role": "system", "content": "be brief"},
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_1", "content": "sunny"}
            ],
            "tools": [
                {"type": "function", "function": {"name": "get_weather", "description": "w"}},
                {"type": "function", "function": {"name": "get_time", "description": "t"}}
            ]
        })
        .to_string()
    }

    /// Detail parsing: the conversation extracts role/content in order, tool tags merge with dedup (top-level definitions + message calls).
    #[tokio::test]
    async fn get_log_detail_parses_conversation_and_tools() {
        let repo = InMemoryRequestLogRepository::new();
        let mut log = sample_request_log();
        log.request_body = Some(sample_body());
        log.upstream_model = Some("gpt-4o-upstream".to_string());
        log.channel_id = Some(Uuid::now_v7());
        log.trace_id = "trace-detail".to_string();
        repo.save(&log).await.expect("save");

        let detail = GetLogDetailUsecase
            .execute(&repo, log.id)
            .await
            .expect("detail");

        // Conversation: 4 messages in order.
        assert_eq!(detail.conversation.len(), 4);
        assert_eq!(detail.conversation[0].role, "system");
        assert_eq!(detail.conversation[0].content.as_deref(), Some("be brief"));
        assert_eq!(detail.conversation[1].role, "user");
        // Tool call message: content is empty, tool_names carries the tool names.
        assert_eq!(detail.conversation[2].role, "assistant");
        assert_eq!(detail.conversation[2].content, None);
        assert_eq!(detail.conversation[2].tool_names, vec!["get_weather"]);
        // Tool result message: role=tool.
        assert_eq!(detail.conversation[3].role, "tool");
        assert_eq!(detail.conversation[3].content.as_deref(), Some("sunny"));

        // Tool tags: top-level definitions + message calls, deduped and keeping first-occurrence order.
        assert_eq!(detail.tool_names, vec!["get_weather", "get_time"]);

        // Route fields echoed.
        assert_eq!(detail.log.trace_id, "trace-detail");
        assert_eq!(
            detail.log.upstream_model.as_deref(),
            Some("gpt-4o-upstream")
        );
    }

    /// Request params: keep top-level params after removing messages / tools / stream.
    #[tokio::test]
    async fn get_log_detail_keeps_top_level_params() {
        let repo = InMemoryRequestLogRepository::new();
        let mut log = sample_request_log();
        log.request_body = Some(sample_body());
        repo.save(&log).await.expect("save");

        let detail = GetLogDetailUsecase
            .execute(&repo, log.id)
            .await
            .expect("detail");
        let params = detail.request_params.as_object().expect("params object");
        assert_eq!(params.get("model").and_then(Value::as_str), Some("gpt-4o"));
        assert_eq!(params.get("temperature").and_then(Value::as_f64), Some(0.7));
        assert!(params.get("messages").is_none(), "messages 不属请求参数");
        assert!(params.get("tools").is_none(), "tools 不属请求参数");
        assert!(
            params.get("stream").is_none(),
            "stream 由日志 is_stream 承载"
        );
    }

    /// No request body: the detail still returns (conversation / params / tools empty), route fields readable.
    #[tokio::test]
    async fn get_log_detail_without_body_returns_empty_views() {
        let repo = InMemoryRequestLogRepository::new();
        let log = sample_request_log(); // request_body = None
        repo.save(&log).await.expect("save");

        let detail = GetLogDetailUsecase
            .execute(&repo, log.id)
            .await
            .expect("detail");
        assert!(detail.conversation.is_empty());
        assert!(detail.tool_names.is_empty());
        assert_eq!(detail.request_params, Value::Null);
        assert_eq!(detail.log.status_code, 200);
    }

    /// Audit summary display data is carried by the log itself; request_body only controls the optional
    /// conversation/params views used by the detail modal.
    #[tokio::test]
    async fn get_log_detail_preserves_audit_report_with_and_without_request_body() {
        let repo = InMemoryRequestLogRepository::new();
        let audit_report = AuditReport::clean(&AuditPolicy {
            enabled: true,
            mode: AuditMode::Observe,
            block_critical: true,
            scan_system_messages: false,
            scan_byte_limit: 64 * 1024,
            store_payload: false,
            evidence_level: AuditEvidenceLevel::Summary,
        });

        let mut with_body = sample_request_log();
        with_body.request_body = Some(sample_body());
        with_body.risk_level = Some(RiskLevel::Clean);
        with_body.audit_action = Some(AuditAction::Allow);
        with_body.audit_report = Some(audit_report.clone());
        repo.save(&with_body).await.expect("save with body");

        let mut without_body = sample_request_log();
        without_body.risk_level = Some(RiskLevel::Clean);
        without_body.audit_action = Some(AuditAction::Allow);
        without_body.audit_report = Some(audit_report.clone());
        repo.save(&without_body).await.expect("save without body");

        let detail_with_body = GetLogDetailUsecase
            .execute(&repo, with_body.id)
            .await
            .expect("detail with body");
        assert!(!detail_with_body.conversation.is_empty());
        assert_eq!(
            detail_with_body.log.audit_report,
            Some(audit_report.clone())
        );

        let detail_without_body = GetLogDetailUsecase
            .execute(&repo, without_body.id)
            .await
            .expect("detail without body");
        assert!(detail_without_body.conversation.is_empty());
        assert_eq!(detail_without_body.request_params, Value::Null);
        assert_eq!(detail_without_body.log.audit_report, Some(audit_report));
    }

    /// Invalid JSON body: treated as having no structured view, no error (the raw JSON is still in log.request_body).
    #[tokio::test]
    async fn get_log_detail_tolerates_invalid_json_body() {
        let repo = InMemoryRequestLogRepository::new();
        let mut log = sample_request_log();
        log.request_body = Some("not-json{{{".to_string());
        repo.save(&log).await.expect("save");

        let detail = GetLogDetailUsecase
            .execute(&repo, log.id)
            .await
            .expect("detail");
        assert!(detail.conversation.is_empty());
        assert_eq!(detail.request_params, Value::Null);
    }

    /// Unknown id: returns NotFound.
    #[tokio::test]
    async fn get_log_detail_unknown_id_errors_not_found() {
        let repo = InMemoryRequestLogRepository::new();
        assert!(matches!(
            GetLogDetailUsecase.execute(&repo, Uuid::now_v7()).await,
            Err(LogError::NotFound)
        ));
    }

    /// Paginated query: delegates filtering and pagination to the repository, total is the match count, newest first.
    #[tokio::test]
    async fn list_logs_delegates_query_and_pagination() {
        let repo = InMemoryRequestLogRepository::new();
        let mut a = sample_request_log();
        a.trace_id = "a".to_string();
        a.created_at = Utc::now() - Duration::seconds(10);
        let mut b = sample_request_log();
        b.trace_id = "b".to_string();
        repo.save(&a).await.expect("save a");
        repo.save(&b).await.expect("save b");

        let page = ListLogsUsecase
            .execute(&repo, LogQuery::default(), 1, 1)
            .await
            .expect("list");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.total, 2);
        assert_eq!(page.items[0].trace_id, "b", "最新在前");
    }

    /// Delete by date: deletes rows strictly before the threshold, returning the count.
    #[tokio::test]
    async fn delete_before_delegates_and_returns_count() {
        let repo = InMemoryRequestLogRepository::new();
        let mut old = sample_request_log();
        old.trace_id = "old".to_string();
        old.created_at = Utc::now() - Duration::days(2);
        let mut recent = sample_request_log();
        recent.trace_id = "recent".to_string();
        repo.save(&old).await.expect("save old");
        repo.save(&recent).await.expect("save recent");

        let n = DeleteLogsBeforeUsecase
            .execute(&repo, Utc::now() - Duration::days(1))
            .await
            .expect("delete");
        assert_eq!(n, 1);
    }

    /// Clear: deletes all and returns the count.
    #[tokio::test]
    async fn clear_logs_empties_repo() {
        let repo = InMemoryRequestLogRepository::new();
        repo.save(&sample_request_log()).await.expect("save a");
        repo.save(&sample_request_log()).await.expect("save b");

        let n = ClearLogsUsecase.execute(&repo).await.expect("clear");
        assert_eq!(n, 2);
        assert!(all_request_logs(&repo).await.is_empty());
    }
}
