//! PostgreSQL distributed lock implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::DistributedLock;
use tokio::sync::watch;
use tracing::{Span, instrument};

use crate::handle::PostgresLockHandle;
use crate::key::PostgresAdvisoryLockKey;
use sqlx::{Executor, PgPool, Row};

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

    /// Attempts to acquire the lock.
    async fn acquire_internal(
        &self,
        timeout: Option<Duration>,
    ) -> LockResult<Option<PostgresLockHandle>> {
        let mut conn = self.pool.acquire().await.map_err(|e| {
            LockError::Connection(Box::new(std::io::Error::other(format!(
                "failed to get connection from pool: {e}"
            ))))
        })?;

        // Always start a transaction to ensure SET LOCAL lock_timeout applies to the lock command
        conn.execute("BEGIN").await.map_err(|e| {
            LockError::Connection(Box::new(std::io::Error::other(format!(
                "failed to start transaction: {e}"
            ))))
        })?;

        let use_transaction_lock = self.use_transaction;
        let savepoint_name = "medallion_lock_acquire";

        // We always use a savepoint to robustly handle errors without aborting the main transaction
        let sql = format!("SAVEPOINT {}", savepoint_name);
        conn.execute(sql.as_str()).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "failed to create savepoint: {e}"
            ))))
        })?;

        // Set timeout
        let timeout_ms = timeout.map(|d| d.as_millis() as i64).unwrap_or(0);
        let set_timeout_sql = format!("SET LOCAL lock_timeout = {}", timeout_ms);
        if let Err(e) = conn.execute(set_timeout_sql.as_str()).await {
            // Rollback savepoint if setting timeout fails
            let _ = conn
                .execute(format!("ROLLBACK TO SAVEPOINT {}", savepoint_name).as_str())
                .await;

            // If we aren't using transaction locks, we should rollback the whole thing
            if !use_transaction_lock {
                let _ = conn.execute("ROLLBACK").await;
            }

            return Err(LockError::Backend(Box::new(std::io::Error::other(
                format!("failed to set lock_timeout: {e}"),
            ))));
        }

        let lock_func = if use_transaction_lock {
            "pg_advisory_xact_lock"
        } else {
            "pg_advisory_lock"
        };

        let sql = format!("SELECT {}({})", lock_func, self.key.to_sql_args());

        match conn.fetch_one(sql.as_str()).await {
            Ok(_) => {
                if !use_transaction_lock {
                    // For session locks, we must COMMIT the transaction to release the "SET LOCAL" params
                    // and allow the connection to be used normally, BUT the lock persists (session scope).
                    if let Err(e) = conn.execute("COMMIT").await {
                        // If commit fails, we might have lost the lock or connection is bad
                        return Err(LockError::Backend(Box::new(std::io::Error::other(
                            format!("failed to commit transaction after locking: {e}"),
                        ))));
                    }
                }

                let (sender, receiver) = watch::channel(false);
                Ok(Some(PostgresLockHandle::new(
                    conn,
                    use_transaction_lock,
                    self.key,
                    sender,
                    receiver,
                    self.keepalive_cadence,
                )))
            }
            Err(e) => {
                let db_err = e.as_database_error();
                let code = db_err.and_then(|db_err| db_err.code()).unwrap_or_default();

                // Rollback to savepoint to clear error state
                let _ = conn
                    .execute(format!("ROLLBACK TO SAVEPOINT {}", savepoint_name).as_str())
                    .await;

                // For session locks, since we failed, rollback the whole transaction setup
                if !use_transaction_lock {
                    let _ = conn.execute("ROLLBACK").await;
                }

                if code == "55P03" {
                    return Ok(None); // Timeout -> None
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

    /// Attempts to acquire the lock immediately (try_lock).
    async fn try_acquire_internal_immediate(&self) -> LockResult<Option<PostgresLockHandle>> {
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

        let lock_func = if use_transaction {
            "pg_try_advisory_xact_lock"
        } else {
            "pg_try_advisory_lock"
        };

        let sql = format!("SELECT {}({})", lock_func, self.key.to_sql_args());
        let row = conn.fetch_one(sql.as_str()).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "failed to try_acquire lock: {e}"
            ))))
        })?;

        let acquired: bool = row.get(0);
        if !acquired {
            // cleanup logic if needed?
            if use_transaction {
                let _ = conn.execute("ROLLBACK").await;
            }
            return Ok(None);
        }

        let (sender, receiver) = watch::channel(false);
        Ok(Some(PostgresLockHandle::new(
            conn,
            use_transaction,
            self.key,
            sender,
            receiver,
            self.keepalive_cadence,
        )))
    }
}

impl DistributedLock for PostgresDistributedLock {
    type Handle = PostgresLockHandle;

    fn name(&self) -> &str {
        &self.name
    }

    #[instrument(skip(self), fields(lock.name = %self.name, timeout = ?timeout, backend = "postgres", use_transaction = self.use_transaction))]
    async fn acquire(&self, timeout: Option<Duration>) -> LockResult<Self::Handle> {
        Span::current().record("operation", "acquire");

        // Use the blocking implementation
        match self.acquire_internal(timeout).await {
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
    async fn try_acquire(&self) -> LockResult<Option<Self::Handle>> {
        Span::current().record("operation", "try_acquire");
        match self.try_acquire_internal_immediate().await {
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
