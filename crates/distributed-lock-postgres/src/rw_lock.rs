//! PostgreSQL reader-writer lock implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::{DistributedReaderWriterLock, LockHandle};
use tokio::sync::watch;
use tracing::{instrument, Span};

use crate::key::PostgresAdvisoryLockKey;
use sqlx::pool::PoolConnection;
use sqlx::{Executor, PgPool, Postgres, Row};

/// A PostgreSQL-based distributed reader-writer lock.
pub struct PostgresDistributedReaderWriterLock {
    /// The lock key.
    key: PostgresAdvisoryLockKey,
    /// Original lock name.
    name: String,
    /// Connection pool.
    pool: PgPool,
    /// Whether to use transaction-scoped locks.
    use_transaction: bool,
    /// Keepalive cadence for long-held locks.
    keepalive_cadence: Option<Duration>,
}

impl PostgresDistributedReaderWriterLock {
    pub(crate) fn new(
        name: String,
        key: PostgresAdvisoryLockKey,
        pool: PgPool,
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

    async fn acquire_internal<H, F>(
        &self,
        timeout: Option<Duration>,
        lock_func_shared: bool,
        constructor: F,
    ) -> LockResult<Option<H>>
    where
        F: FnOnce(
            PoolConnection<Postgres>,
            bool,
            PostgresAdvisoryLockKey,
            watch::Sender<bool>,
            watch::Receiver<bool>,
        ) -> H,
    {
        let mut conn = self.pool.acquire().await.map_err(|e| {
            LockError::Connection(Box::new(std::io::Error::other(format!(
                "failed to get connection from pool: {e}"
            ))))
        })?;

        // Always start transaction to scope SET LOCAL
        conn.execute("BEGIN").await.map_err(|e| {
            LockError::Connection(Box::new(std::io::Error::other(format!(
                "failed to start transaction: {e}"
            ))))
        })?;

        let use_transaction_lock = self.use_transaction;
        let savepoint_name = "medallion_rwlock_acquire";

        let sql = format!("SAVEPOINT {}", savepoint_name);
        conn.execute(sql.as_str()).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "failed to create savepoint: {e}"
            ))))
        })?;

        let timeout_ms = timeout.map(|d| d.as_millis() as i64).unwrap_or(0);
        let set_timeout_sql = format!("SET LOCAL lock_timeout = {}", timeout_ms);
        if let Err(e) = conn.execute(set_timeout_sql.as_str()).await {
            let _ = conn
                .execute(format!("ROLLBACK TO SAVEPOINT {}", savepoint_name).as_str())
                .await;

            if !use_transaction_lock {
                let _ = conn.execute("ROLLBACK").await;
            }
            return Err(LockError::Backend(Box::new(std::io::Error::other(
                format!("failed to set lock_timeout: {e}"),
            ))));
        }

        let lock_func = match (use_transaction_lock, lock_func_shared) {
            (true, true) => "pg_advisory_xact_lock_shared",
            (true, false) => "pg_advisory_xact_lock",
            (false, true) => "pg_advisory_lock_shared",
            (false, false) => "pg_advisory_lock",
        };

        let sql = format!("SELECT {}({})", lock_func, self.key.to_sql_args());

        match conn.fetch_one(sql.as_str()).await {
            Ok(_) => {
                if !use_transaction_lock {
                    // Commit to persist session lock but close transaction
                    if let Err(e) = conn.execute("COMMIT").await {
                        return Err(LockError::Backend(Box::new(std::io::Error::other(
                            format!("failed to commit transaction after locking: {e}"),
                        ))));
                    }
                }

                let (sender, receiver) = watch::channel(false);
                Ok(Some(constructor(
                    conn,
                    use_transaction_lock,
                    self.key,
                    sender,
                    receiver,
                )))
            }
            Err(e) => {
                let db_err = e.as_database_error();
                let code = db_err.and_then(|db_err| db_err.code()).unwrap_or_default();

                let _ = conn
                    .execute(format!("ROLLBACK TO SAVEPOINT {}", savepoint_name).as_str())
                    .await;
                if !use_transaction_lock {
                    let _ = conn.execute("ROLLBACK").await;
                }

                if code == "55P03" {
                    return Ok(None);
                }
                if code == "40P01" {
                    return Err(LockError::Deadlock(
                        "deadlock detected by postgres".to_string(),
                    ));
                }

                Err(LockError::Backend(Box::new(std::io::Error::other(
                    format!("failed to acquire lock: {e}"),
                ))))
            }
        }
    }

    async fn try_acquire_internal_immediate<H, F>(
        &self,
        lock_func_shared: bool,
        constructor: F,
    ) -> LockResult<Option<H>>
    where
        F: FnOnce(
            PoolConnection<Postgres>,
            bool,
            PostgresAdvisoryLockKey,
            watch::Sender<bool>,
            watch::Receiver<bool>,
        ) -> H,
    {
        let mut conn = self.pool.acquire().await.map_err(|e| {
            LockError::Connection(Box::new(std::io::Error::other(format!(
                "failed to get connection from pool: {e}"
            ))))
        })?;

        let use_transaction = self.use_transaction;
        if use_transaction {
            conn.execute("BEGIN").await.map_err(|e| {
                LockError::Connection(Box::new(std::io::Error::other(format!(
                    "failed to start transaction: {e}"
                ))))
            })?;
        }

        let lock_func = match (use_transaction, lock_func_shared) {
            (true, true) => "pg_try_advisory_xact_lock_shared",
            (true, false) => "pg_try_advisory_xact_lock",
            (false, true) => "pg_try_advisory_lock_shared",
            (false, false) => "pg_try_advisory_lock",
        };

        let sql = format!("SELECT {}({})", lock_func, self.key.to_sql_args());
        let row = conn.fetch_one(sql.as_str()).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "failed to try_acquire lock: {e}"
            ))))
        })?;

        let acquired: bool = row.get(0);
        if !acquired {
            if use_transaction {
                let _ = conn.execute("ROLLBACK").await;
            }
            return Ok(None);
        }

        let (sender, receiver) = watch::channel(false);
        Ok(Some(constructor(
            conn,
            use_transaction,
            self.key,
            sender,
            receiver,
        )))
    }
}

impl DistributedReaderWriterLock for PostgresDistributedReaderWriterLock {
    type ReadHandle = PostgresReadLockHandle;
    type WriteHandle = PostgresWriteLockHandle;

    fn name(&self) -> &str {
        &self.name
    }

    #[instrument(skip(self), fields(lock.name = %self.name, timeout = ?timeout, backend = "postgres", use_transaction = self.use_transaction))]
    async fn acquire_read(&self, timeout: Option<Duration>) -> LockResult<Self::ReadHandle> {
        Span::current().record("operation", "acquire_read");
        match self
            .acquire_internal(timeout, true, |c, t, k, s, r| {
                PostgresReadLockHandle::new(c, t, k, s, r, self.keepalive_cadence)
            })
            .await
        {
            Ok(Some(handle)) => {
                Span::current().record("acquired", true);
                Ok(handle)
            }
            Ok(None) => {
                Span::current().record("acquired", false);
                Span::current().record("error", "timeout");
                Err(LockError::Timeout(timeout.unwrap_or(Duration::MAX)))
            }
            Err(e) => Err(e),
        }
    }

    #[instrument(skip(self), fields(lock.name = %self.name, backend = "postgres", use_transaction = self.use_transaction))]
    async fn try_acquire_read(&self) -> LockResult<Option<Self::ReadHandle>> {
        Span::current().record("operation", "try_acquire_read");
        match self
            .try_acquire_internal_immediate(true, |c, t, k, s, r| {
                PostgresReadLockHandle::new(c, t, k, s, r, self.keepalive_cadence)
            })
            .await
        {
            Ok(Some(handle)) => {
                Span::current().record("acquired", true);
                Ok(Some(handle))
            }
            Ok(None) => {
                Span::current().record("acquired", false);
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }

    #[instrument(skip(self), fields(lock.name = %self.name, timeout = ?timeout, backend = "postgres", use_transaction = self.use_transaction))]
    async fn acquire_write(&self, timeout: Option<Duration>) -> LockResult<Self::WriteHandle> {
        Span::current().record("operation", "acquire_write");
        match self
            .acquire_internal(timeout, false, |c, t, k, s, r| {
                PostgresWriteLockHandle::new(c, t, k, s, r, self.keepalive_cadence)
            })
            .await
        {
            Ok(Some(handle)) => {
                Span::current().record("acquired", true);
                Ok(handle)
            }
            Ok(None) => {
                Span::current().record("acquired", false);
                Span::current().record("error", "timeout");
                Err(LockError::Timeout(timeout.unwrap_or(Duration::MAX)))
            }
            Err(e) => Err(e),
        }
    }

    #[instrument(skip(self), fields(lock.name = %self.name, backend = "postgres", use_transaction = self.use_transaction))]
    async fn try_acquire_write(&self) -> LockResult<Option<Self::WriteHandle>> {
        Span::current().record("operation", "try_acquire_write");
        match self
            .try_acquire_internal_immediate(false, |c, t, k, s, r| {
                PostgresWriteLockHandle::new(c, t, k, s, r, self.keepalive_cadence)
            })
            .await
        {
            Ok(Some(handle)) => {
                Span::current().record("acquired", true);
                Ok(Some(handle))
            }
            Ok(None) => {
                Span::current().record("acquired", false);
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}

/// Handle for a held PostgreSQL read lock.
pub struct PostgresReadLockHandle {
    conn: Option<PoolConnection<Postgres>>,
    is_transaction: bool,
    key: PostgresAdvisoryLockKey,
    lost_receiver: watch::Receiver<bool>,
    _monitor_task: tokio::task::JoinHandle<()>,
}

impl PostgresReadLockHandle {
    pub(crate) fn new(
        conn: PoolConnection<Postgres>,
        is_transaction: bool,
        key: PostgresAdvisoryLockKey,
        _lost_sender: watch::Sender<bool>,
        lost_receiver: watch::Receiver<bool>,
        _keepalive_cadence: Option<Duration>,
    ) -> Self {
        let monitor_task = tokio::spawn(async move {});
        Self {
            conn: Some(conn),
            is_transaction,
            key,
            lost_receiver,
            _monitor_task: monitor_task,
        }
    }
}

impl LockHandle for PostgresReadLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

    async fn release(mut self) -> LockResult<()> {
        if let Some(mut conn) = self.conn.take() {
            if self.is_transaction {
                match conn.execute("ROLLBACK").await {
                    Ok(_) => tracing::debug!("Transaction rolled back successfully"),
                    Err(e) => tracing::warn!("Failed to rollback transaction: {}", e),
                }
            } else {
                let sql = format!(
                    "SELECT pg_advisory_unlock_shared({})",
                    self.key.to_sql_args()
                );
                if let Err(e) = conn.execute(sql.as_str()).await {
                    tracing::warn!("Failed to release read lock explicitly: {}", e);
                }
            }
        }
        Ok(())
    }
}

impl Drop for PostgresReadLockHandle {
    fn drop(&mut self) {
        self._monitor_task.abort();
    }
}

/// Handle for a held PostgreSQL write lock.
pub struct PostgresWriteLockHandle {
    conn: Option<PoolConnection<Postgres>>,
    is_transaction: bool,
    key: PostgresAdvisoryLockKey,
    lost_receiver: watch::Receiver<bool>,
    _monitor_task: tokio::task::JoinHandle<()>,
}

impl PostgresWriteLockHandle {
    pub(crate) fn new(
        conn: PoolConnection<Postgres>,
        is_transaction: bool,
        key: PostgresAdvisoryLockKey,
        _lost_sender: watch::Sender<bool>,
        lost_receiver: watch::Receiver<bool>,
        _keepalive_cadence: Option<Duration>,
    ) -> Self {
        let monitor_task = tokio::spawn(async move {});
        Self {
            conn: Some(conn),
            is_transaction,
            key,
            lost_receiver,
            _monitor_task: monitor_task,
        }
    }
}

impl LockHandle for PostgresWriteLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

    async fn release(mut self) -> LockResult<()> {
        if let Some(mut conn) = self.conn.take() {
            if self.is_transaction {
                match conn.execute("ROLLBACK").await {
                    Ok(_) => tracing::debug!("Transaction rolled back successfully"),
                    Err(e) => tracing::warn!("Failed to rollback transaction: {}", e),
                }
            } else {
                let sql = format!("SELECT pg_advisory_unlock({})", self.key.to_sql_args());
                if let Err(e) = conn.execute(sql.as_str()).await {
                    tracing::warn!("Failed to release write lock explicitly: {}", e);
                }
            }
        }
        Ok(())
    }
}

impl Drop for PostgresWriteLockHandle {
    fn drop(&mut self) {
        self._monitor_task.abort();
    }
}
