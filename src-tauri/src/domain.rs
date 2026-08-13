//! 领域层：纯业务规则与类型，零技术依赖（不出现 sqlx / reqwest / axum / tauri）。
//!
//! 依赖方向：`interface → usecases → domain`，`infrastructure → domain`，全部向内指向 domain。
//! 仓储依赖倒置：trait 定义在本层，sqlx 实现在 infrastructure，usecases 只依赖 trait。

pub mod api_key;
pub mod channel;
pub mod error;
pub mod provider;
pub mod quota;
pub mod request_log;
