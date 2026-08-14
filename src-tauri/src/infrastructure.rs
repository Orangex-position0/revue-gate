//! 基础设施层：技术实现（sqlite 仓储、provider 适配器）。
//!
//! 依赖方向：`infrastructure → domain`，实现 domain 定义的仓储/适配器接口。
//! 本骨架阶段仅含 SQLite 连接池与内嵌迁移。

pub mod providers;
pub mod sqlite;
pub mod store;
