//! PostgreSQL distributed lock implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::timeout::TimeoutValue;
use distributed_lock_core::traits::DistributedLock;
use tokio::sync::watch;
use tracing::{instrument, Span};

use crate::handle::PostgresLockHandle;
use crate::key::PostgresAdvisoryLockKey;
use sqlx::{PgPool, Postgres, Row, Transaction};

/// A PostgreSQL-based distributed lock.
pub struct PostgresDistributedLock {
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

impl PostgresDistributedLock {
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

    /// Attempts to acquire the lock without waiting.
    async fn try_acquire_internal(&self) -> LockResult<Option<PostgresLockHandle>> {
        if self.use_transaction {
            // Transaction-scoped lock
            let mut transaction = self.pool.begin().await.map_err(|e| {
                LockError::Connection(Box::new(std::io::Error::other(format!(
                    "failed to start transaction: {}",
                    e
                ))))
            })?;

            let sql = match self.key {
                PostgresAdvisoryLockKey::Single(_) => {
                    format!("SELECT pg_try_advisory_lock({})", self.key.to_sql_args())
                }
                PostgresAdvisoryLockKey::Pair(_, _) => {
                    format!("SELECT pg_try_advisory_lock({})", self.key.to_sql_args())
                }
            };

            let row = sqlx::query(&sql)
                .fetch_one(&mut *transaction)
                .await
                .map_err(|e| {
                    LockError::Backend(Box::new(std::io::Error::other(format!(
                        "failed to acquire lock: {}",
                        e
                    ))))
                })?;

            let acquired: bool = row.get(0);
            if !acquired {
                return Ok(None);
            }

            // Store transaction using raw pointer to avoid lifetime issues
            // SAFETY: We manually manage the transaction lifetime in the handle
            let transaction_ptr = unsafe {
                std::mem::transmute::<Transaction<'_, Postgres>, Transaction<'static, Postgres>>(
                    transaction,
                )
            };
            let transaction_ptr = Box::into_raw(Box::new(transaction_ptr));

            let (sender, receiver) = watch::channel(false);
            Ok(Some(PostgresLockHandle::new(
                crate::handle::PostgresConnectionInner::Transaction(transaction_ptr),
                self.key,
                sender,
                receiver,
                self.keepalive_cadence,
            )))
        } else {
            // Session-scoped lock
            let mut connection = self.pool.acquire().await.map_err(|e| {
                LockError::Connection(Box::new(std::io::Error::other(format!(
                    "failed to get connection from pool: {}",
                    e
                ))))
            })?;

            let sql = match self.key {
                PostgresAdvisoryLockKey::Single(_) => {
                    format!("SELECT pg_try_advisory_lock({})", self.key.to_sql_args())
                }
                PostgresAdvisoryLockKey::Pair(_, _) => {
                    format!("SELECT pg_try_advisory_lock({})", self.key.to_sql_args())
                }
            };

            let row = sqlx::query(&sql)
                .fetch_one(&mut *connection)
                .await
                .map_err(|e| {
                    LockError::Backend(Box::new(std::io::Error::other(format!(
                        "failed to acquire lock: {}",
                        e
                    ))))
                })?;

            let acquired: bool = row.get(0);
            if !acquired {
                return Ok(None);
            }

            // Store pool connection to keep it alive
            // PoolConnection will be returned to pool when dropped

            let (sender, receiver) = watch::channel(false);
            Ok(Some(PostgresLockHandle::new(
                crate::handle::PostgresConnectionInner::Connection(Box::new(connection)),
                self.key,
                sender,
                receiver,
                self.keepalive_cadence,
            )))
        }
    }
}

impl DistributedLock for PostgresDistributedLock {
    type Handle = PostgresLockHandle;

    fn name(&self) -> &str {
        &self.name
    }

    #[instrument(skip(self), fields(lock.name = %self.name, timeout = ?timeout, backend = "postgres", use_transaction = self.use_transaction))]
    async fn acquire(&self, timeout: Option<Duration>) -> LockResult<Self::Handle> {
        let timeout_value = TimeoutValue::from(timeout);
        let start = std::time::Instant::now();
        Span::current().record("operation", "acquire");

        // Busy-wait with exponential backoff
        let mut sleep_duration = Duration::from_millis(50);
        const MAX_SLEEP: Duration = Duration::from_secs(1);

        loop {
            match self.try_acquire_internal().await {
                Ok(Some(handle)) => {
                    let elapsed = start.elapsed();
                    Span::current().record("acquired", true);
                    Span::current().record("elapsed_ms", elapsed.as_millis() as u64);
                    return Ok(handle);
                }
                Ok(None) => {
                    // Check timeout
                    if !timeout_value.is_infinite()
                        && start.elapsed() >= timeout_value.as_duration().unwrap()
                    {
                        Span::current().record("acquired", false);
                        Span::current().record("error", "timeout");
                        return Err(LockError::Timeout(timeout_value.as_duration().unwrap()));
                    }

                    // Sleep before retry
                    tokio::time::sleep(sleep_duration).await;
                    sleep_duration = (sleep_duration * 2).min(MAX_SLEEP);
                }
                Err(e) => return Err(e),
            }
        }
    }

    #[instrument(skip(self), fields(lock.name = %self.name, backend = "postgres", use_transaction = self.use_transaction))]
    async fn try_acquire(&self) -> LockResult<Option<Self::Handle>> {
        Span::current().record("operation", "try_acquire");
        let result = self.try_acquire_internal().await;
        match &result {
            Ok(Some(_)) => {
                Span::current().record("acquired", true);
            }
            Ok(None) => {
                Span::current().record("acquired", false);
                Span::current().record("reason", "lock_held");
            }
            Err(e) => {
                Span::current().record("acquired", false);
                Span::current().record("error", e.to_string());
            }
        }
        result
    }
}
