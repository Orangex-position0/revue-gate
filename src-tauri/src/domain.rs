//! Domain layer: pure business rules and types, zero technical dependencies (no sqlx / reqwest / axum / tauri).
//!
//! Dependency direction: `interface → usecases → domain`, `infrastructure → domain`, all pointing inward to domain.
//! Repository dependency inversion: traits are defined in this layer, sqlx implementations live in infrastructure, usecases depend only on traits.

pub mod api_key;
pub mod channel;
pub mod dispatcher;
pub mod error;
pub mod provider;
pub mod quota;
pub mod request_log;
pub mod security_audit;
pub mod settings;
