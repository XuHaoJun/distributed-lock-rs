//! MySQL lock handle implementation.

use std::sync::Arc;
use tokio::sync::{Mutex, watch};

use distributed_lock_core::error::LockResult;
use distributed_lock_core::traits::LockHandle;

/// Handle to a held MySQL distributed lock.
///
/// Dropping this handle releases the lock. For proper error handling in async
/// contexts, call `release()` explicitly.
/// Handle to a held MySQL distributed lock.
///
/// Dropping this handle releases the lock. For proper error handling in async
/// contexts, call `release()` explicitly.
pub struct MySqlLockHandle {
    /// The encoded lock name for MySQL GET_LOCK.
    lock_name: String,
    /// Dedicated connection holding the lock (must stay open).
    /// Wrapped in Option to take ownership in drop/release.
    connection: Option<Arc<Mutex<sqlx::pool::PoolConnection<sqlx::MySql>>>>,
    /// Channel to signal when the lock is lost.
    lost_sender: watch::Sender<bool>,
    /// Receiver for lost token.
    lost_receiver: watch::Receiver<bool>,
    /// Keepalive task handle, if keepalive is enabled.
    keepalive_handle: Option<tokio::task::JoinHandle<()>>,
}

impl MySqlLockHandle {
    /// Creates a new lock handle.
    pub(crate) fn new(
        lock_name: String,
        connection: sqlx::pool::PoolConnection<sqlx::MySql>,
        keepalive_cadence: Option<std::time::Duration>,
    ) -> Self {
        let (lost_token_tx, lost_token_rx) = watch::channel(false);

        let connection = Arc::new(Mutex::new(connection));
        let connection_clone = Arc::clone(&connection);

        let lost_token_tx_clone = lost_token_tx.clone();
        let keepalive_handle = keepalive_cadence.map(|cadence| {
            let mut lost_token_rx_clone = lost_token_tx_clone.subscribe();

            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(cadence) => {
                            // Run a keepalive query on the connection holding the lock
                            let mut conn = connection_clone.lock().await; // Wait for lock
                            let result = sqlx::query("SELECT 1")
                                .execute(&mut **conn)
                                .await;

                            if result.is_err() {
                                // Connection failed, signal lock loss
                                let _ = lost_token_tx_clone.send(true);
                                break;
                            }
                        }
                        _ = lost_token_rx_clone.changed() => {
                            // Lock was released, stop keepalive
                            break;
                        }
                    }
                }
            })
        });

        Self {
            lock_name,
            connection: Some(connection),
            lost_sender: lost_token_tx,
            lost_receiver: lost_token_rx,
            keepalive_handle,
        }
    }
}

impl LockHandle for MySqlLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

    async fn release(mut self) -> LockResult<()> {
        // Stop keepalive task
        if let Some(handle) = self.keepalive_handle.take() {
            handle.abort();
        }

        // Signal that the lock is being released
        let _ = self.lost_sender.send(true);

        // Take the connection to ensure it's not double-released in Drop
        if let Some(connection) = self.connection.take() {
            // Release the MySQL lock
            let mut conn = connection.lock().await;
            let result = sqlx::query("SELECT RELEASE_LOCK(?)")
                .bind(&self.lock_name)
                .execute(&mut **conn)
                .await;

            match result {
                Ok(_) => Ok(()),
                Err(e) => Err(distributed_lock_core::error::LockError::Connection(
                    Box::new(e),
                )),
            }
        } else {
            Ok(())
        }
    }
}

impl Drop for MySqlLockHandle {
    fn drop(&mut self) {
        // Stop keepalive task
        if let Some(handle) = self.keepalive_handle.take() {
            handle.abort();
        }

        // If connection is still present, it means release() wasn't called.
        // usage of take() guarantees we only try to release once.
        if let Some(connection) = self.connection.take() {
            // Signal that the lock is lost (being dropped)
            let _ = self.lost_sender.send(true);

            let lock_name = self.lock_name.clone();

            // We must release the lock or at least close the connection.
            // Since Drop is synchronous, we spawn a background task.
            tokio::spawn(async move {
                let mut conn = connection.lock().await;
                // Try to release gracefully
                let _ = sqlx::query("SELECT RELEASE_LOCK(?)")
                    .bind(lock_name)
                    .execute(&mut **conn)
                    .await;

                // Connection will be dropped here (returning to pool)
            });
        }
    }
}
