//! Crate-wide error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("llm error: {0}")]
    Llm(String),
    #[error("sandbox error: {0}")]
    Sandbox(String),
    #[error("policy violation: {0}")]
    Policy(String),
    /// The factory stopped the run by itself (blocking exception without a human available…).
    #[error("run aborted: {0}")]
    Aborted(String),
    /// A human supervisor rejected a proposal at a gate.
    #[error("rejected by supervisor: {0}")]
    Rejected(String),
    #[error("budget exceeded: spent ${spent:.4} of ${limit:.4}")]
    BudgetExceeded { spent: f64, limit: f64 },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
