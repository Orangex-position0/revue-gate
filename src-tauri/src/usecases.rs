//! Use case layer: orchestrates domain logic and repository traits, organizing the data flow (see docs/Architecture-backend.md).
//!
//! Dependency direction `interface → usecases → domain`; repositories are injected via traits (seam A tests can mock).
//! Naming convention: verb + `Usecase` suffix (e.g. `CreateChannelUsecase`).

pub mod api_key;
pub mod auth;
pub mod channel;
pub mod knowledge;
pub mod log;
pub mod models;
pub mod proxy;
pub mod settings;
pub mod stats;
