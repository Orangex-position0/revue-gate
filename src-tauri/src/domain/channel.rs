//! Channel 聚合根：实体 + 模型映射值对象 + ChannelRepository trait。
//!
//! 字段与需求对齐（`docs/Requirements.md` 渠道管理）：名称/类型/Base URL/API Key/
//! 模型列表/优先级/权重/模型映射/启停状态。仓储 trait 随聚合根放置，不拆子文件夹。

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::domain::error::RepositoryError;

/// 内置渠道类型。OpenAI / DeepSeek / Custom 走 OpenAI-compatible 直通，
/// Claude / Gemini 走协议转换适配器（实现位于 infrastructure/providers，本骨架不涉及）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    OpenAi,
    DeepSeek,
    Custom,
    Claude,
    Gemini,
}

/// 模型映射值对象：客户端统一模型名 ↔ 上游实际模型名；未映射时直传。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelMapping {
    /// 客户端（下游）使用的统一模型名。
    pub client_model: String,
    /// 上游渠道实际模型名。
    pub upstream_model: String,
}

/// Channel 实体：一条可调度到上游的渠道配置。
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub id: Uuid,
    /// 管理员可读名称，如 `openai-prod`。
    pub name: String,
    pub channel_type: ChannelType,
    /// 上游 Base URL；为 None 时由 ProviderAdaptor 提供默认值。
    pub base_url: Option<String>,
    /// 上游 API Key。红线：任何路径不得下发给下游或写入请求日志明文。
    pub api_key: Option<String>,
    /// 该渠道支持的上游模型列表。
    pub models: Vec<String>,
    /// 调度优先级（数值越小越优先；候选排序按优先级升序）。
    pub priority: i32,
    /// 权重字段：v0.1 仅持久化与回显，加权调度属高可用增强（Out of Scope）。
    pub weight: i32,
    /// 模型映射表。
    pub model_mappings: Vec<ModelMapping>,
    /// 启停状态；禁用渠道不参与调度（行为在阶段 07 生效）。
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Channel 仓储 trait：领域层定义接口，infrastructure 提供 sqlx 实现，usecases 只依赖 trait。
#[async_trait::async_trait]
pub trait ChannelRepository: Send + Sync {
    /// 按 id 查找渠道。
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Channel>, RepositoryError>;
    /// 列出全部渠道。
    async fn list(&self) -> Result<Vec<Channel>, RepositoryError>;
    /// 新建或覆盖保存渠道。
    async fn save(&self, channel: &Channel) -> Result<(), RepositoryError>;
    /// 按 id 删除渠道。
    async fn delete(&self, id: Uuid) -> Result<(), RepositoryError>;
}
