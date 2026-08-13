//! ApiKey 聚合根：实体 + 配额值对象 + ApiKeyRepository trait。
//!
//! 密钥格式 `sk-revue-<16 位随机 hex>`（25 字符、8 字节熵），HTTP 请求以
//! `Authorization: Bearer <key>` 认证。配额超限判定由领域服务 `QuotaPolicy`
//! （见 domain/quota.rs）负责，本文件只定义配额数据形态。
//!
//! serde 派生态用于跨 Tauri Command 边界序列化（前端类型见 `src/types/index.ts`）：
//! 字段输出 camelCase。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::error::RepositoryError;

/// 配额值对象：`limit` 为上限（None 表示无上限），`used` 为已用额度。
/// `quota_used >= quota_limit` 时请求应返回 429（判定见 domain/quota.rs 的 QuotaPolicy）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    /// 配额上限；None 表示不限制。
    pub limit: Option<u64>,
    /// 已用额度，随请求累加并持久化。
    pub used: u64,
}

/// ApiKey 实体：一条网关本地密钥，下游用其访问网关、不接触上游密钥。
/// `key` 明文仅本地存储：控制面创建命令返回一次，列表/编辑/启停返回遮蔽预览。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKey {
    pub id: Uuid,
    /// 管理员可读名称。
    pub name: String,
    /// 密钥明文 `sk-revue-*`；仅本地存储，绝不返回给下游。
    pub key: String,
    /// 启停状态；停用密钥认证应失败。
    pub enabled: bool,
    pub quota: Quota,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// ApiKey 仓储 trait：领域层定义接口，infrastructure 提供 sqlx 实现。
#[async_trait::async_trait]
pub trait ApiKeyRepository: Send + Sync {
    /// 按密钥明文查找（认证用）。
    async fn find_by_key(&self, key: &str) -> Result<Option<ApiKey>, RepositoryError>;
    /// 按 id 查找密钥。
    async fn find_by_id(&self, id: Uuid) -> Result<Option<ApiKey>, RepositoryError>;
    /// 列出全部密钥。
    async fn list(&self) -> Result<Vec<ApiKey>, RepositoryError>;
    /// 新建或覆盖保存密钥。
    async fn save(&self, api_key: &ApiKey) -> Result<(), RepositoryError>;
    /// 按 id 删除密钥。
    async fn delete(&self, id: Uuid) -> Result<(), RepositoryError>;
}
