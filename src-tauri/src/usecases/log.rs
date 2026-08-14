//! 请求日志查询用例：分页筛选 / 详情解析 / 按日期删除 / 清空编排。
//!
//! 无状态用例：仓储以 `&dyn RequestLogRepository` 注入，seam A 测试可用内存 mock。
//! 详情在用例层从请求体 JSON 解析「对话构成 / 请求参数 / 工具标签」（serde_json 解析，
//! 领域层保持纯类型不引入解析逻辑，见 domain/request_log.rs）；路由字段（channel /
//! upstream / usage / trace）直接来自 `RequestLog`，前端经既有渠道/密钥 API 反查名称。

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::error::RepositoryError;
use crate::domain::request_log::{LogPage, LogQuery, RequestLog, RequestLogRepository};

/// 日志用例层错误。
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("request log not found")]
    NotFound,
    #[error("request log repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// 日志详情：`RequestLog`（含路由 / usage / 原始 JSON）+ 从请求体解析出的结构化视图。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogDetail {
    #[serde(flatten)]
    pub log: RequestLog,
    /// 对话构成：request_body.messages 的 role / content / 触发的工具调用。
    pub conversation: Vec<ConversationMessage>,
    /// 请求参数：request_body 顶层除 messages / tools / stream 之外的字段。
    pub request_params: Value,
    /// 工具标签：顶层 tools[].function.name 与各消息 tool_calls[].function.name 去重合并。
    pub tool_names: Vec<String>,
}

/// 一条对话消息（详情展示用；content 为文本或 None——空内容 / 非文本不展示）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub role: String,
    pub content: Option<String>,
    /// 该消息触发的工具调用名（assistant 消息的 tool_calls[].function.name）。
    pub tool_names: Vec<String>,
}

/// 分页查询日志：多条件筛选 + 页码，透传仓储实现（筛选语义唯一权威见 `LogQuery`）。
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

/// 日志详情：按 id 加载，解析请求体为对话 / 参数 / 工具标签。未命中返回 `NotFound`。
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

/// 删除创建时间严格早于 `before` 的日志（左闭右开上界），返回删除条数。
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

/// 清空全部请求日志，返回删除条数。
pub struct ClearLogsUsecase;
impl ClearLogsUsecase {
    pub async fn execute(&self, repo: &dyn RequestLogRepository) -> Result<u64, LogError> {
        Ok(repo.clear().await?)
    }
}

/// 从日志请求体 JSON 解析出（对话构成, 请求参数, 工具标签）。
/// 无 body / 非法 JSON / 非对象一律返回空值——详情仍可展示路由与原始 JSON，不因解析失败报错。
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
    // 顶层 tools[] 定义（声明但未必被调用）。
    if let Some(tools) = value.get("tools").and_then(Value::as_array) {
        for t in tools {
            if let Some(name) = t.pointer("/function/name").and_then(Value::as_str) {
                push_unique(&mut tool_names, name);
            }
        }
    }

    // 对话构成 + 各消息触发的工具调用（同时并入全局工具标签）。
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

    // 请求参数：顶层除 messages / tools / stream 之外的字段（stream 由日志 is_stream 承载）。
    let mut params = value.clone();
    if let Some(obj) = params.as_object_mut() {
        obj.remove("messages");
        obj.remove("tools");
        obj.remove("stream");
    }

    (conversation, params, tool_names)
}

/// 追加去重（保持首次出现顺序）。
fn push_unique(seen: &mut Vec<String>, name: &str) {
    if !seen.iter().any(|s| s == name) {
        seen.push(name.to_string());
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::test_support::{InMemoryRequestLogRepository, all_request_logs, sample_request_log};

    /// 构造一条带消息 / 参数 / 工具的请求体样本。
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

    /// 详情解析：对话构成按序提取 role/content，工具标签去重合并（顶层定义 + 消息内调用）。
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

        // 对话构成：4 条消息按序。
        assert_eq!(detail.conversation.len(), 4);
        assert_eq!(detail.conversation[0].role, "system");
        assert_eq!(detail.conversation[0].content.as_deref(), Some("be brief"));
        assert_eq!(detail.conversation[1].role, "user");
        // 工具调用消息：content 为空，tool_names 携带工具名。
        assert_eq!(detail.conversation[2].role, "assistant");
        assert_eq!(detail.conversation[2].content, None);
        assert_eq!(detail.conversation[2].tool_names, vec!["get_weather"]);
        // 工具结果消息：role=tool。
        assert_eq!(detail.conversation[3].role, "tool");
        assert_eq!(detail.conversation[3].content.as_deref(), Some("sunny"));

        // 工具标签：顶层定义 + 消息调用，去重且保持首次出现顺序。
        assert_eq!(detail.tool_names, vec!["get_weather", "get_time"]);

        // 路由字段回显。
        assert_eq!(detail.log.trace_id, "trace-detail");
        assert_eq!(
            detail.log.upstream_model.as_deref(),
            Some("gpt-4o-upstream")
        );
    }

    /// 请求参数：剔除 messages / tools / stream 后保留顶层参数。
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

    /// 无请求体：详情仍返回（对话 / 参数 / 工具为空），路由字段可读。
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

    /// 非法 JSON 请求体：视为无结构化视图，不报错（原始 JSON 仍在 log.request_body）。
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

    /// 未知 id：返回 NotFound。
    #[tokio::test]
    async fn get_log_detail_unknown_id_errors_not_found() {
        let repo = InMemoryRequestLogRepository::new();
        assert!(matches!(
            GetLogDetailUsecase.execute(&repo, Uuid::now_v7()).await,
            Err(LogError::NotFound)
        ));
    }

    /// 分页查询：透传筛选与分页到仓储，total 为命中总数，最新在前。
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

    /// 按日期删除：严格早于阈值删除，返回删除条数。
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

    /// 清空：全部删除并返回条数。
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
