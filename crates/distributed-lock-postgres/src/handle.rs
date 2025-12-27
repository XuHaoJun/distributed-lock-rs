//! PostgreSQL lock handle implementation.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::interval;
use tracing::{instrument, warn};

use distributed_lock_core::error::LockResult;
use distributed_lock_core::traits::LockHandle;

use deadpool_postgres::Pool;
use tokio_postgres::Transaction;

/// Handle for a held PostgreSQL lock.
pub struct PostgresLockHandle {
    /// The database transaction (if transaction-scoped) or connection pool.
    /// When dropped, the lock is released.
    _connection: PostgresConnectionInner,
    /// Watch channel for lock lost detection.
    lost_receiver: watch::Receiver<bool>,
    /// Background task handle for connection monitoring.
    _monitor_task: tokio::task::JoinHandle<()>,
}

pub(crate) enum PostgresConnectionInner {
    Transaction(Transaction<'static>),
    Pool(Pool),
}

unsafe impl Send for PostgresConnectionInner {}
unsafe impl Sync for PostgresConnectionInner {}

impl PostgresLockHandle {
    pub(crate) fn new(
        connection: PostgresConnectionInner,
        lost_sender: watch::Sender<bool>,
        lost_receiver: watch::Receiver<bool>,
        keepalive_cadence: Option<Duration>,
    ) -> Self {
        let lost_sender_arc = Arc::new(lost_sender);

        // Spawn background task to monitor connection health and perform keepalive
        let monitor_pool = match &connection {
            PostgresConnectionInner::Transaction(_) => {
                // For transactions, connection loss will be detected when transaction operations fail
                // We can't easily monitor without affecting the transaction, so we skip monitoring
                None
            }
            PostgresConnectionInner::Pool(pool) => Some(pool.clone()),
        };

        let monitor_sender = lost_sender_arc.clone();
        let keepalive_interval = keepalive_cadence.unwrap_or(Duration::from_secs(30));

        let monitor_task = tokio::spawn(async move {
            if let Some(pool) = monitor_pool {
                // Use keepalive cadence if provided, otherwise default to 30s
                let mut interval_timer = interval(keepalive_interval);
                loop {
                    interval_timer.tick().await;

                    // Check if sender is closed (handle dropped)
                    if monitor_sender.is_closed() {
                        break;
                    }

                    // Perform keepalive: get a connection and execute a simple query
                    match pool.get().await {
                        Ok(client) => {
                            // Execute a simple query to keep the connection alive
                            if let Err(e) = client.query_one("SELECT 1", &[]).await {
                                // Connection error - signal lock lost
                                warn!("PostgreSQL keepalive query failed: {}", e);
                                let _ = monitor_sender.send(true);
                                break;
                            }
                        }
                        Err(e) => {
                            // Connection error - signal lock lost
                            warn!("PostgreSQL connection pool error detected: {}", e);
                            let _ = monitor_sender.send(true);
                            break;
                        }
                    }
                }
            }
        });

        Self {
            _connection: connection,
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
    async fn release(self) -> LockResult<()> {
        // Lock is released when connection/transaction is dropped
        // The Drop implementation will abort the monitoring task
        drop(self);
        Ok(())
    }
}

impl Drop for PostgresLockHandle {
    fn drop(&mut self) {
        // Abort monitoring task on drop
        self._monitor_task.abort();
    }
}
