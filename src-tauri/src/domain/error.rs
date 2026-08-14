//! Shared error types for the domain layer: unified error enum for repository operation failures.
//!
//! Specific error messages evolve with the implementation (see the Spec test seam); this skeleton defines only the minimal variants.

/// Unified repository error: defined by domain, constructed by the sqlx implementation in infrastructure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepositoryError {
    /// The target record does not exist (lookup by id or unique key missed).
    #[error("record not found")]
    NotFound,
    /// Underlying storage error (sqlx error messages are converted to strings to keep sqlx types out of domain).
    #[error("database error: {0}")]
    Database(String),
}
