//! Connection pool management for PostgreSQL locks.

use deadpool_postgres::{Config, Pool, Runtime};
use distributed_lock_core::error::{LockError, LockResult};
use std::str::FromStr;
use std::time::Duration;
use tokio_postgres::NoTls;

/// PostgreSQL connection source.
#[derive(Debug, Clone)]
pub enum PostgresConnection {
    /// Connection string - library manages pooling.
    ConnectionString(String),
    /// External connection pool.
    Pool(Pool),
}

impl PostgresConnection {
    /// Creates a connection pool from a connection string.
    pub async fn create_pool(connection_string: &str) -> LockResult<Pool> {
        let tokio_config = tokio_postgres::Config::from_str(connection_string).map_err(|e| {
            LockError::Connection(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid connection string: {}", e),
            )))
        })?;

        let mut pg_config = Config::new();
        if let Some(user) = tokio_config.get_user() {
            pg_config.user = Some(user.to_string());
        }
        if let Some(password) = tokio_config.get_password() {
            pg_config.password = Some(String::from_utf8_lossy(password).to_string());
        }
        if let Some(dbname) = tokio_config.get_dbname() {
            pg_config.dbname = Some(dbname.to_string());
        }
        if let Some(host) = tokio_config.get_hosts().first() {
            if let tokio_postgres::config::Host::Tcp(host_str) = host {
                pg_config.host = Some(host_str.clone());
            }
        }
        if let Some(port) = tokio_config.get_ports().first() {
            pg_config.port = Some(*port);
        } else {
            pg_config.port = Some(5432);
        }

        pg_config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| {
                LockError::Connection(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("failed to create connection pool: {}", e),
                )))
            })
    }

    /// Gets or creates a connection pool.
    pub async fn get_pool(&self) -> LockResult<Pool> {
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
    pub keepalive_cadence: Option<Duration>,
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
