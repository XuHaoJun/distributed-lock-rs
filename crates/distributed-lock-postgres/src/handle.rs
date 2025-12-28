//! PostgreSQL lock handle implementation.

use tokio::sync::watch;
use tracing::instrument;

use distributed_lock_core::error::LockResult;
use distributed_lock_core::traits::LockHandle;

use crate::key::PostgresAdvisoryLockKey;
use sqlx::pool::PoolConnection;
use sqlx::{Executor, Postgres};

/// Handle for a held PostgreSQL lock.
pub struct PostgresLockHandle {
    /// The database connection.
    /// When dropped, the lock is released (via connection drop or explicit release).
    conn: Option<PoolConnection<Postgres>>,
    /// Whether this lock is held within a transaction.
    is_transaction: bool,
    /// The lock key.
    key: PostgresAdvisoryLockKey,
    /// Watch channel for lock lost detection.
    lost_receiver: watch::Receiver<bool>,
    /// Background task handle for connection monitoring.
    _monitor_task: tokio::task::JoinHandle<()>,
}

impl PostgresLockHandle {
    pub(crate) fn new(
        conn: PoolConnection<Postgres>,
        is_transaction: bool,
        key: PostgresAdvisoryLockKey,
        _lost_sender: watch::Sender<bool>,
        lost_receiver: watch::Receiver<bool>,
        _keepalive_cadence: Option<std::time::Duration>,
    ) -> Self {
        // Monitoring is skipped as connection drop handles release
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

impl LockHandle for PostgresLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

    #[instrument(skip(self), fields(backend = "postgres"))]
    async fn release(mut self) -> LockResult<()> {
        if let Some(mut conn) = self.conn.take() {
            if self.is_transaction {
                // Rollback transaction to release lock
                match conn.execute("ROLLBACK").await {
                    Ok(_) => tracing::debug!("Transaction rolled back successfully"),
                    Err(e) => tracing::warn!("Failed to rollback transaction: {}", e),
                }
            } else {
                // Explicitly release session-scoped lock
                let sql = format!("SELECT pg_advisory_unlock({})", self.key.to_sql_args());
                if let Err(e) = conn.execute(sql.as_str()).await {
                    tracing::warn!("Failed to release lock explicitly: {}", e);
                }
            }
        }
        Ok(())
    }
}

impl Drop for PostgresLockHandle {
    fn drop(&mut self) {
        self._monitor_task.abort();
    }
}
