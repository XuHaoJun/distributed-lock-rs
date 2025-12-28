//! MySQL connection management for distributed locks.

use sqlx::{MySql, MySqlPool, Pool};
use std::fmt;

/// Represents different ways to connect to MySQL.
#[derive(Clone)]
pub enum MySqlConnection {
    /// Connect using a connection string.
    ConnectionString(String),
    /// Use an existing connection pool.
    Pool(Pool<MySql>),
}

impl MySqlConnection {
    /// Get or create a connection pool.
    ///
    /// For ConnectionString variant, creates a new pool.
    /// For Pool variant, clones the existing pool.
    pub async fn get_pool(&self) -> Result<Pool<MySql>, sqlx::Error> {
        match self {
            MySqlConnection::ConnectionString(url) => MySqlPool::connect(url).await,
            MySqlConnection::Pool(pool) => Ok(pool.clone()),
        }
    }
}

impl fmt::Debug for MySqlConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MySqlConnection::ConnectionString(_) => {
                write!(f, "MySqlConnection::ConnectionString([REDACTED])")
            }
            MySqlConnection::Pool(_) => write!(f, "MySqlConnection::Pool([POOL])"),
        }
    }
}
