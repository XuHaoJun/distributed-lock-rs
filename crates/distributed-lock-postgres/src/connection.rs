//! Connection pool management for PostgreSQL locks.

use distributed_lock_core::error::{LockError, LockResult};
use sqlx::PgPool;

/// PostgreSQL connection source.
#[derive(Debug, Clone)]
pub enum PostgresConnection {
    /// Connection string - library manages pooling.
    ConnectionString(String),
    /// External connection pool.
    Pool(PgPool),
}

impl PostgresConnection {
    /// Creates a connection pool from a connection string.
    pub async fn create_pool(connection_string: &str) -> LockResult<PgPool> {
        sqlx::PgPool::connect(connection_string).await.map_err(|e| {
            LockError::Connection(Box::new(std::io::Error::other(format!(
                "failed to create connection pool: {e}"
            ))))
        })
    }

    /// Gets or creates a connection pool.
    pub async fn get_pool(&self) -> LockResult<PgPool> {
        match self {
            Self::ConnectionString(conn_str) => Self::create_pool(conn_str).await,
            Self::Pool(pool) => Ok(pool.clone()),
        }
    }
}

/// Configuration for PostgreSQL distributed locks.
#[derive(Debug, Clone)]
pub struct PostgresLockConfig {
    /// Connection source.
    pub connection: PostgresConnection,
    /// Whether to use transaction-scoped locks.
    pub use_transaction: bool,
    /// Keepalive cadence for long-held locks.
    pub keepalive_cadence: Option<std::time::Duration>,
}

impl PostgresLockConfig {
    /// Creates a new configuration.
    pub fn new(connection: PostgresConnection) -> Self {
        Self {
            connection,
            use_transaction: false,
            keepalive_cadence: None,
        }
    }
}
