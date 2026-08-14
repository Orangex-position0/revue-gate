//! Infrastructure layer: technical implementations (sqlite repositories, provider adapters).
//!
//! Dependency direction: `infrastructure → domain`, implementing the repository/adapter interfaces
//! defined by domain. At this skeleton stage it only contains the SQLite connection pool and embedded migrations.

pub mod providers;
pub mod sqlite;
pub mod store;
