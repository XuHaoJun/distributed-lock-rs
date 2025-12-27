//! PostgreSQL lock provider implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::{LockProvider, ReaderWriterLockProvider};

use crate::connection::PostgresConnection;
use crate::key::PostgresAdvisoryLockKey;
use crate::lock::PostgresDistributedLock;
use crate::rw_lock::PostgresDistributedReaderWriterLock;
use sqlx::PgPool;

/// Builder for PostgreSQL lock provider configuration.
pub struct PostgresLockProviderBuilder {
    connection: Option<PostgresConnection>,
    use_transaction: bool,
    keepalive_cadence: Option<Duration>,
}

impl PostgresLockProviderBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Self {
            connection: None,
            use_transaction: false,
            keepalive_cadence: None,
        }
    }

    /// Sets the PostgreSQL connection string.
    pub fn connection_string(mut self, conn_str: impl Into<String>) -> Self {
        self.connection = Some(PostgresConnection::ConnectionString(conn_str.into()));
        self
    }

    /// Sets an existing connection pool.
    pub fn pool(mut self, pool: PgPool) -> Self {
        self.connection = Some(PostgresConnection::Pool(pool));
        self
    }

    /// Sets whether to use transaction-scoped locks.
    pub fn use_transaction(mut self, use_transaction: bool) -> Self {
        self.use_transaction = use_transaction;
        self
    }

    /// Sets the keepalive cadence for long-held locks.
    pub fn keepalive_cadence(mut self, cadence: Duration) -> Self {
        self.keepalive_cadence = Some(cadence);
        self
    }

    /// Builds the provider.
    pub async fn build(self) -> LockResult<PostgresLockProvider> {
        let connection = self
            .connection
            .ok_or_else(|| LockError::InvalidName("connection not specified".to_string()))?;

        let pool = connection.get_pool().await?;

        Ok(PostgresLockProvider {
            pool,
            use_transaction: self.use_transaction,
            keepalive_cadence: self.keepalive_cadence,
        })
    }
}

impl Default for PostgresLockProviderBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Provider for PostgreSQL-based distributed locks.
pub struct PostgresLockProvider {
    pool: PgPool,
    use_transaction: bool,
    keepalive_cadence: Option<Duration>,
}

impl PostgresLockProvider {
    /// Returns a new builder for configuring the provider.
    pub fn builder() -> PostgresLockProviderBuilder {
        PostgresLockProviderBuilder::new()
    }

    /// Creates a provider using the specified connection string.
    pub async fn new(connection_string: impl Into<String>) -> LockResult<Self> {
        Self::builder()
            .connection_string(connection_string)
            .build()
            .await
    }
}

impl LockProvider for PostgresLockProvider {
    type Lock = PostgresDistributedLock;

    fn create_lock(&self, name: &str) -> Self::Lock {
        let key = PostgresAdvisoryLockKey::from_name(name, true)
            .expect("failed to encode lock name as key");
        PostgresDistributedLock::new(
            name.to_string(),
            key,
            self.pool.clone(),
            self.use_transaction,
            self.keepalive_cadence,
        )
    }
}

impl ReaderWriterLockProvider for PostgresLockProvider {
    type Lock = PostgresDistributedReaderWriterLock;

    fn create_reader_writer_lock(&self, name: &str) -> Self::Lock {
        let key = PostgresAdvisoryLockKey::from_name(name, true)
            .expect("failed to encode lock name as key");
        PostgresDistributedReaderWriterLock::new(
            name.to_string(),
            key,
            self.pool.clone(),
            self.use_transaction,
            self.keepalive_cadence,
        )
    }
}
