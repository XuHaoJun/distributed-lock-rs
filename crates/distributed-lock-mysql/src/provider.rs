//! MySQL lock provider implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::{LockProvider, ReaderWriterLockProvider};

use crate::connection::MySqlConnection;
use crate::lock::MySqlDistributedLock;
use crate::rw_lock::MySqlDistributedReaderWriterLock;

/// Builder for MySQL lock provider configuration.
pub struct MySqlLockProviderBuilder {
    connection: Option<MySqlConnection>,
    keepalive_cadence: Option<Duration>,
}

impl MySqlLockProviderBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Self {
            connection: None,
            keepalive_cadence: None,
        }
    }

    /// Sets the MySQL connection string.
    pub fn connection_string(mut self, conn_str: impl Into<String>) -> Self {
        self.connection = Some(MySqlConnection::ConnectionString(conn_str.into()));
        self
    }

    /// Sets an existing connection pool.
    pub fn pool(mut self, pool: sqlx::MySqlPool) -> Self {
        self.connection = Some(MySqlConnection::Pool(pool));
        self
    }

    /// Sets the keepalive cadence for long-held locks.
    ///
    /// MySQL's `wait_timeout` system variable determines how long the server
    /// will allow a connection to be idle before killing it. This option sets
    /// the cadence at which we run a no-op "keepalive" query on a connection
    /// that is holding a lock.
    ///
    /// Setting a value of `None` disables keepalive.
    pub fn keepalive_cadence(mut self, cadence: Duration) -> Self {
        self.keepalive_cadence = Some(cadence);
        self
    }

    /// Builds the provider.
    pub async fn build(self) -> LockResult<MySqlLockProvider> {
        let connection = self
            .connection
            .ok_or_else(|| LockError::InvalidName("connection not specified".to_string()))?;

        let pool = connection
            .get_pool()
            .await
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        Ok(MySqlLockProvider {
            pool,
            keepalive_cadence: self.keepalive_cadence,
        })
    }
}

impl Default for MySqlLockProviderBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Provider for MySQL-based distributed locks.
pub struct MySqlLockProvider {
    pool: sqlx::MySqlPool,
    keepalive_cadence: Option<Duration>,
}

impl MySqlLockProvider {
    /// Returns a new builder for configuring the provider.
    pub fn builder() -> MySqlLockProviderBuilder {
        MySqlLockProviderBuilder::new()
    }

    /// Creates a provider using the specified connection string.
    pub async fn new(connection_string: impl Into<String>) -> LockResult<Self> {
        Self::builder()
            .connection_string(connection_string)
            .build()
            .await
    }
}

impl LockProvider for MySqlLockProvider {
    type Lock = MySqlDistributedLock;

    fn create_lock(&self, name: &str) -> Self::Lock {
        MySqlDistributedLock::new(name.to_string(), self.pool.clone(), self.keepalive_cadence)
    }
}

impl ReaderWriterLockProvider for MySqlLockProvider {
    type Lock = MySqlDistributedReaderWriterLock;

    fn create_reader_writer_lock(&self, name: &str) -> Self::Lock {
        MySqlDistributedReaderWriterLock::new(
            name.to_string(),
            self.pool.clone(),
            self.keepalive_cadence,
        )
    }
}
