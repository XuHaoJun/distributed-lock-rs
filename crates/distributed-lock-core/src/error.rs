//! Error types for distributed lock operations.

use std::time::Duration;
use thiserror::Error;

/// Errors that can occur during lock operations.
#[derive(Error, Debug)]
pub enum LockError {
    /// Lock acquisition timed out.
    #[error("lock acquisition timed out after {0:?}")]
    Timeout(Duration),
    
    /// Lock operation was cancelled.
    #[error("lock operation was cancelled")]
    Cancelled,
    
    /// Deadlock detected (e.g., same connection already holds lock).
    #[error("deadlock detected: {0}")]
    Deadlock(String),
    
    /// Backend connection failed.
    #[error("connection error: {0}")]
    Connection(#[source] Box<dyn std::error::Error + Send + Sync>),
    
    /// Lock was lost after acquisition (e.g., connection died).
    #[error("lock was lost: {0}")]
    LockLost(String),
    
    /// Invalid lock name.
    #[error("invalid lock name: {0}")]
    InvalidName(String),
    
    /// Backend-specific error.
    #[error("backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Result type for lock operations.
pub type LockResult<T> = Result<T, LockError>;
