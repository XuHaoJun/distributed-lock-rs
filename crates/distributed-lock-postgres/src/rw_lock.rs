//! PostgreSQL reader-writer lock implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::timeout::TimeoutValue;
use distributed_lock_core::traits::{DistributedReaderWriterLock, LockHandle};
use tokio::sync::watch;

use crate::handle::PostgresConnectionInner;
use crate::key::PostgresAdvisoryLockKey;
use deadpool_postgres::Pool;

/// A PostgreSQL-based distributed reader-writer lock.
pub struct PostgresDistributedReaderWriterLock {
    /// The lock key.
    key: PostgresAdvisoryLockKey,
    /// Original lock name.
    name: String,
    /// Connection pool.
    pool: Pool,
    /// Whether to use transaction-scoped locks.
    use_transaction: bool,
    /// Keepalive cadence for long-held locks.
    keepalive_cadence: Option<Duration>,
}

impl PostgresDistributedReaderWriterLock {
    pub(crate) fn new(
        name: String,
        key: PostgresAdvisoryLockKey,
        pool: Pool,
        use_transaction: bool,
        keepalive_cadence: Option<Duration>,
    ) -> Self {
        Self {
            key,
            name,
            pool,
            use_transaction,
            keepalive_cadence,
        }
    }

    /// Attempts to acquire a read lock without waiting.
    async fn try_acquire_read_internal(&self) -> LockResult<Option<PostgresReadLockHandle>> {
        let mut client = self.pool.get().await.map_err(|e| {
            LockError::Connection(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("failed to get connection from pool: {}", e),
            )))
        })?;

        let sql = format!(
            "SELECT pg_try_advisory_lock_shared({})",
            self.key.to_sql_args()
        );

        let row = client.query_one(&sql, &[]).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("failed to acquire read lock: {}", e),
            )))
        })?;

        let acquired: bool = row.get(0);
        if !acquired {
            return Ok(None);
        }

        let (sender, receiver) = watch::channel(false);
        Ok(Some(PostgresReadLockHandle::new(
            PostgresConnectionInner::Pool(self.pool.clone()),
            sender,
            receiver,
            self.keepalive_cadence,
        )))
    }

    /// Attempts to acquire a write lock without waiting.
    async fn try_acquire_write_internal(&self) -> LockResult<Option<PostgresWriteLockHandle>> {
        let mut client = self.pool.get().await.map_err(|e| {
            LockError::Connection(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("failed to get connection from pool: {}", e),
            )))
        })?;

        if self.use_transaction {
            // Transaction-scoped lock
            let transaction = client.transaction().await.map_err(|e| {
                LockError::Connection(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("failed to start transaction: {}", e),
                )))
            })?;

            let sql = format!("SELECT pg_try_advisory_lock({})", self.key.to_sql_args());

            let row = transaction.query_one(&sql, &[]).await.map_err(|e| {
                LockError::Backend(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("failed to acquire write lock: {}", e),
                )))
            })?;

            let acquired: bool = row.get(0);
            if !acquired {
                return Ok(None);
            }

            // Extend transaction lifetime using unsafe
            let transaction_ptr =
                Box::into_raw(Box::new(transaction)) as *mut tokio_postgres::Transaction<'static>;
            let transaction = unsafe { *Box::from_raw(transaction_ptr) };

            let (sender, receiver) = watch::channel(false);
            Ok(Some(PostgresWriteLockHandle::new(
                PostgresConnectionInner::Transaction(transaction),
                sender,
                receiver,
                self.keepalive_cadence,
            )))
        } else {
            // Session-scoped lock
            let sql = format!("SELECT pg_try_advisory_lock({})", self.key.to_sql_args());

            let row = client.query_one(&sql, &[]).await.map_err(|e| {
                LockError::Backend(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("failed to acquire write lock: {}", e),
                )))
            })?;

            let acquired: bool = row.get(0);
            if !acquired {
                return Ok(None);
            }

            let (sender, receiver) = watch::channel(false);
            Ok(Some(PostgresWriteLockHandle::new(
                PostgresConnectionInner::Pool(self.pool.clone()),
                sender,
                receiver,
                self.keepalive_cadence,
            )))
        }
    }
}

impl DistributedReaderWriterLock for PostgresDistributedReaderWriterLock {
    type ReadHandle = PostgresReadLockHandle;
    type WriteHandle = PostgresWriteLockHandle;

    fn name(&self) -> &str {
        &self.name
    }

    async fn acquire_read(&self, timeout: Option<Duration>) -> LockResult<Self::ReadHandle> {
        let timeout_value = TimeoutValue::from(timeout);
        let start = std::time::Instant::now();

        // Busy-wait with exponential backoff
        let mut sleep_duration = Duration::from_millis(50);
        const MAX_SLEEP: Duration = Duration::from_secs(1);

        loop {
            match self.try_acquire_read_internal().await {
                Ok(Some(handle)) => return Ok(handle),
                Ok(None) => {
                    // Check timeout
                    if !timeout_value.is_infinite() {
                        if start.elapsed() >= timeout_value.as_duration().unwrap() {
                            return Err(LockError::Timeout(timeout_value.as_duration().unwrap()));
                        }
                    }

                    // Sleep before retry
                    tokio::time::sleep(sleep_duration).await;
                    sleep_duration = (sleep_duration * 2).min(MAX_SLEEP);
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn try_acquire_read(&self) -> LockResult<Option<Self::ReadHandle>> {
        self.try_acquire_read_internal().await
    }

    async fn acquire_write(&self, timeout: Option<Duration>) -> LockResult<Self::WriteHandle> {
        let timeout_value = TimeoutValue::from(timeout);
        let start = std::time::Instant::now();

        // Busy-wait with exponential backoff
        let mut sleep_duration = Duration::from_millis(50);
        const MAX_SLEEP: Duration = Duration::from_secs(1);

        loop {
            match self.try_acquire_write_internal().await {
                Ok(Some(handle)) => return Ok(handle),
                Ok(None) => {
                    // Check timeout
                    if !timeout_value.is_infinite() {
                        if start.elapsed() >= timeout_value.as_duration().unwrap() {
                            return Err(LockError::Timeout(timeout_value.as_duration().unwrap()));
                        }
                    }

                    // Sleep before retry
                    tokio::time::sleep(sleep_duration).await;
                    sleep_duration = (sleep_duration * 2).min(MAX_SLEEP);
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn try_acquire_write(&self) -> LockResult<Option<Self::WriteHandle>> {
        self.try_acquire_write_internal().await
    }
}

/// Handle for a held PostgreSQL read lock.
pub struct PostgresReadLockHandle {
    /// The database connection (when dropped, the lock is released).
    _connection: PostgresConnectionInner,
    /// Watch channel for lock lost detection.
    lost_receiver: watch::Receiver<bool>,
}

impl PostgresReadLockHandle {
    pub(crate) fn new(
        connection: PostgresConnectionInner,
        _lost_sender: watch::Sender<bool>,
        lost_receiver: watch::Receiver<bool>,
        _keepalive_cadence: Option<Duration>,
    ) -> Self {
        Self {
            _connection: connection,
            lost_receiver,
        }
    }
}

impl LockHandle for PostgresReadLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

    async fn release(self) -> LockResult<()> {
        // Release the shared lock
        drop(self._connection);
        Ok(())
    }
}

/// Handle for a held PostgreSQL write lock.
pub struct PostgresWriteLockHandle {
    /// The database connection/transaction (when dropped, the lock is released).
    _connection: PostgresConnectionInner,
    /// Watch channel for lock lost detection.
    lost_receiver: watch::Receiver<bool>,
}

impl PostgresWriteLockHandle {
    pub(crate) fn new(
        connection: PostgresConnectionInner,
        _lost_sender: watch::Sender<bool>,
        lost_receiver: watch::Receiver<bool>,
        _keepalive_cadence: Option<Duration>,
    ) -> Self {
        Self {
            _connection: connection,
            lost_receiver,
        }
    }
}

impl LockHandle for PostgresWriteLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

    async fn release(self) -> LockResult<()> {
        // Release the exclusive lock
        drop(self._connection);
        Ok(())
    }
}
