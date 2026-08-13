//! RequestLog 聚合根：实体 + RequestLogRepository trait。
//!
//! 每次请求全量记日志（`docs/Requirements.md` 请求日志）：密钥/渠道/模型/usage/耗时/
//! trace id/请求体/是否流式/是否重试。查询分页与筛选随阶段 10 加入。

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::error::RepositoryError;

/// RequestLog 实体：一次请求的完整审计记录。
#[derive(Debug, Clone, PartialEq)]
pub struct RequestLog {
    pub id: Uuid,
    /// 发起请求的本地密钥 id（认证通过时必填）。
    pub api_key_id: Option<Uuid>,
    /// 实际命中的上游渠道 id（候选耗尽时为 None）。
    pub channel_id: Option<Uuid>,
    /// 客户端请求的模型名。
    pub model: String,
    /// 实际上游模型名（应用模型映射后）。
    pub upstream_model: Option<String>,
    /// 网关返回给客户端的 HTTP 状态码。
    pub status_code: u16,
    /// usage：prompt / completion / total tokens（流式解析后回填）。
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    /// 请求总耗时（毫秒）。
    pub duration_ms: u64,
    /// 失败时的错误消息（候选耗尽、转发失败等）。
    pub error_message: Option<String>,
    /// 是否流式请求。
    pub is_stream: bool,
    /// 是否发生过重试。
    pub is_retry: bool,
    /// 贯穿请求的 trace id，用于链路追踪。
    pub trace_id: String,
    /// 客户端请求体（原文 JSON）。
    pub request_body: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// RequestLog 仓储 trait：领域层定义接口，infrastructure 提供 sqlx 实现。
#[async_trait::async_trait]
pub trait RequestLogRepository: Send + Sync {
    /// 写入一条请求日志。
    async fn save(&self, log: &RequestLog) -> Result<(), RepositoryError>;
    /// 按 id 查找日志详情。
    async fn find_by_id(&self, id: Uuid) -> Result<Option<RequestLog>, RepositoryError>;
    /// 按创建时间倒序列出日志（分页/筛选参数在阶段 10 补充）。
    async fn list(&self) -> Result<Vec<RequestLog>, RepositoryError>;
}
