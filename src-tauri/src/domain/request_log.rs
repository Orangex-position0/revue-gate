//! RequestLog 聚合根：实体 + 查询筛选值对象 + RequestLogRepository trait。
//!
//! 每次请求全量记日志（`docs/Requirements.md` 请求日志）：密钥/渠道/模型/usage/耗时/
//! trace id/请求体/是否流式/是否重试。查询分页与多条件筛选（keyword/密钥/渠道/模型/
//! 日期范围）随阶段 10 加入；`LogQuery::matches` 是筛选语义的唯一权威定义，
//! sqlx 实现据此生成等价的 WHERE（见 infrastructure/sqlite/request_log.rs）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::error::RepositoryError;

/// RequestLog 实体：一次请求的完整审计记录。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
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

/// 日志查询筛选条件：全部可选，None = 不筛该维度。
/// 跨 Tauri Command 边界序列化（camelCase，与 `src/types/index.ts` 的 `LogQuery` 对齐）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogQuery {
    /// 关键词：对 model / upstream_model / trace_id / error_message 做大小写不敏感子串匹配。
    #[serde(default)]
    pub keyword: Option<String>,
    /// 按发起密钥筛选（api_key_id 精确匹配；无认证的日志天然排除）。
    #[serde(default)]
    pub api_key_id: Option<Uuid>,
    /// 按渠道筛选（channel_id 精确匹配）。
    #[serde(default)]
    pub channel_id: Option<Uuid>,
    /// 按请求模型名筛选（大小写不敏感子串匹配，便于输入模型名前缀）。
    #[serde(default)]
    pub model: Option<String>,
    /// 日期范围下界（左闭）：`created_at >= start_at`。
    #[serde(default)]
    pub start_at: Option<DateTime<Utc>>,
    /// 日期范围上界（右开）：`created_at < end_at`。
    #[serde(default)]
    pub end_at: Option<DateTime<Utc>>,
}

impl LogQuery {
    /// 判断一条日志是否命中全部筛选条件（None = 该维度不筛）。
    ///
    /// 这是筛选语义的**唯一权威定义**，sqlx 实现据此生成等价的 WHERE 子句
    /// （见 infrastructure/sqlite/request_log.rs）：keyword / model 为大小写不敏感
    /// 子串匹配（LIKE 侧对 `%`/`_` 做转义保持字面匹配），日期区间左闭右开
    /// `[start_at, end_at)`。InMemory 仓储直接调用本函数，与 SQL 行为一致。
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

/// 分页查询结果：当前页日志 + 满足筛选的总条数（供前端算总页数）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    /// 当前页的日志（按创建时间倒序，最新在前）。
    pub items: Vec<RequestLog>,
    /// 满足筛选条件的日志总数（与当前页无关）。
    pub total: u64,
}

/// 统计投影行：仪表盘聚合所需的轻量列（不含 request_body，避免统计扫描拖回大字段）。
/// `stat_rows` 返回此类型，聚合逻辑在 usecases/stats.rs（纯 Rust，单一代码路径，
/// sqlx 与 InMemory 实现数据一致，见 Spec §Testing seam A）。
#[derive(Debug, Clone, PartialEq)]
pub struct LogStatRow {
    /// 网关返回给客户端的 HTTP 状态码（<400 视为成功，用于渠道可用率）。
    pub status_code: u16,
    /// 本次请求总 token（流式解析后回填；未解析到则为 None，按 0 计）。
    pub total_tokens: Option<u32>,
    /// 请求总耗时（毫秒）。
    pub duration_ms: u64,
    pub created_at: DateTime<Utc>,
}

/// RequestLog 仓储 trait：领域层定义接口，infrastructure 提供 sqlx 实现。
#[async_trait::async_trait]
pub trait RequestLogRepository: Send + Sync {
    /// 写入一条请求日志。
    async fn save(&self, log: &RequestLog) -> Result<(), RepositoryError>;
    /// 按 id 查找日志详情。
    async fn find_by_id(&self, id: Uuid) -> Result<Option<RequestLog>, RepositoryError>;
    /// 分页查询日志：多条件筛选（keyword / 密钥 / 渠道 / 模型 / 日期范围），
    /// 按创建时间倒序（同时间按 id 倒序兜底），page 从 1 起。
    async fn query(
        &self,
        query: &LogQuery,
        page: u64,
        page_size: u64,
    ) -> Result<LogPage, RepositoryError>;
    /// 删除创建时间严格早于 `before` 的日志（左闭右开上界），返回删除条数。
    async fn delete_before(&self, before: DateTime<Utc>) -> Result<u64, RepositoryError>;
    /// 清空全部日志，返回删除条数。
    async fn clear(&self) -> Result<u64, RepositoryError>;
    /// 统计投影：返回落在左闭右开区间 `[start_at, end_at)` 内的轻量日志行
    /// （时间维度 None = 不限）。聚合逻辑在 usecases，本方法只做取数与过滤。
    async fn stat_rows(
        &self,
        start_at: Option<DateTime<Utc>>,
        end_at: Option<DateTime<Utc>>,
    ) -> Result<Vec<LogStatRow>, RepositoryError>;
}
