//! 领域层共享错误类型：仓储操作失败的统一错误枚举。
//!
//! 具体错误信息随实现演进（见 Spec 测试 seam），本骨架只定义最小分支。

/// 仓储层统一错误：由 domain 定义、infrastructure 的 sqlx 实现构造。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepositoryError {
    /// 目标记录不存在（按 id 或唯一键查找未命中）。
    #[error("record not found")]
    NotFound,
    /// 底层存储错误（sqlx 错误消息已转为字符串，避免 sqlx 类型进入 domain）。
    #[error("database error: {0}")]
    Database(String),
}
