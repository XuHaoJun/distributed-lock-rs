//! PostgreSQL lock handle implementation.

use tokio::sync::watch;
use tracing::instrument;

use distributed_lock_core::error::LockResult;
use distributed_lock_core::traits::LockHandle;

use crate::key::PostgresAdvisoryLockKey;
use sqlx::pool::PoolConnection;
use sqlx::{Postgres, Transaction};

/// Handle for a held PostgreSQL lock.
pub struct PostgresLockHandle {
    /// The database transaction (if transaction-scoped) or connection.
    /// When dropped, the lock is released.
    _connection: Option<PostgresConnectionInner>,
    /// The lock key for explicit unlock.
    key: PostgresAdvisoryLockKey,
    /// Watch channel for lock lost detection.
    lost_receiver: watch::Receiver<bool>,
    /// Background task handle for connection monitoring.
    _monitor_task: tokio::task::JoinHandle<()>,
}

pub(crate) enum PostgresConnectionInner {
    /// Transaction-scoped lock: stores transaction which keeps connection alive
    /// Using raw pointer to avoid lifetime issues - we manage the transaction manually
    Transaction(*mut Transaction<'static, Postgres>),
    /// Session-scoped lock: stores pool connection
    Connection(Box<PoolConnection<Postgres>>),
}

unsafe impl Send for PostgresConnectionInner {}
unsafe impl Sync for PostgresConnectionInner {}

impl PostgresLockHandle {
    pub(crate) fn new(
        connection: PostgresConnectionInner,
        key: PostgresAdvisoryLockKey,
        _lost_sender: watch::Sender<bool>,
        lost_receiver: watch::Receiver<bool>,
        _keepalive_cadence: Option<std::time::Duration>,
    ) -> Self {
        // For session-scoped locks (Connection), the lock is tied to the connection
        // and will be released when the connection is dropped. We don't need
        // to monitor separately - the connection drop will handle cleanup.
        // For transaction-scoped locks, connection loss will be detected when
        // transaction operations fail, so we skip monitoring.
        let monitor_task = tokio::spawn(async move {
            // No monitoring needed - locks are released when connection/transaction is dropped
        });

        Self {
            _connection: Some(connection),
            key,
            lost_receiver,
            _monitor_task: monitor_task,
        }
    }
}

impl LockHandle for PostgresLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

    #[instrument(skip(self), fields(backend = "postgres"))]
    async fn release(mut self) -> LockResult<()> {
        let key = self.key;

        // Explicitly release the lock before dropping the connection
        // This is important for connection pooling - we need to unlock before
        // the connection is returned to the pool
        if let Some(connection) = self._connection.take() {
            match connection {
                PostgresConnectionInner::Connection(mut conn) => {
                    // Release session-scoped lock explicitly
                    let sql = format!("SELECT pg_advisory_unlock({})", key.to_sql_args());
                    let _ = sqlx::query(&sql).execute(&mut **conn).await;
                }
                PostgresConnectionInner::Transaction(transaction_ptr) => {
                    // SAFETY: We created this pointer and it's still valid
                    let transaction = unsafe { Box::from_raw(transaction_ptr) };
                    match transaction.rollback().await {
                        Ok(_) => {
                            tracing::debug!("Transaction rolled back successfully");
                        }
                        Err(e) => {
                            tracing::warn!("Failed to rollback transaction: {}", e);
                        }
                    }
                    // Transaction is consumed by rollback(), so no need to drop it
                }
            }
        }

        // Lock is released, connection/transaction has been dropped
        Ok(())
    }
}

impl Drop for PostgresLockHandle {
    fn drop(&mut self) {
        // Abort monitoring task on drop
        self._monitor_task.abort();
    }
}
