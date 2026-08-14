//! 用例层：编排 domain + 调仓储 trait，组织数据流（见 docs/Architecture-backend.md）。
//!
//! 依赖方向 `interface → usecases → domain`；仓储经 trait 注入（seam A 测试可 mock）。
//! 命名约定：动词 + `Usecase` 后缀（如 `CreateChannelUsecase`）。

pub mod api_key;
pub mod auth;
pub mod channel;
pub mod log;
pub mod models;
pub mod proxy;
pub mod settings;
pub mod stats;
