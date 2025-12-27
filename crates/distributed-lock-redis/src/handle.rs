//! Redis lock handle implementation.

use std::sync::Arc;
use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::LockHandle;
use fred::prelude::*;
use tokio::sync::watch;
use tracing::instrument;

use crate::lock::RedisLockState;
use crate::redlock::{extend::extend_redlock, release::release_redlock};

/// Handle for a held Redis lock.
///
/// The lock is automatically extended in the background while this handle exists.
/// Dropping or releasing the handle stops extension and releases the lock.
pub struct RedisLockHandle {
    /// Lock state.
    state: Arc<RedisLockState>,
    /// Acquire results indexed by client position.
    acquire_results: Arc<Vec<bool>>,
    /// Redis clients.
    clients: Arc<Vec<RedisClient>>,
    /// Extension cadence.
    extension_cadence: Duration,
    /// Lock expiry duration.
    expiry: Duration,
    /// Watch channel for lock lost detection.
    lost_receiver: watch::Receiver<bool>,
    /// Background extension task handle.
    extension_task: tokio::task::JoinHandle<()>,
}

impl RedisLockHandle {
    /// Creates a new lock handle.
    pub(crate) fn new(
        state: RedisLockState,
        acquire_results: Vec<bool>,
        clients: Vec<RedisClient>,
        extension_cadence: Duration,
        expiry: Duration,
    ) -> Self {
        let state = Arc::new(state);
        let acquire_results = Arc::new(acquire_results);
        let clients = Arc::new(clients);
        let (lost_sender, lost_receiver) = watch::channel(false);

        // Clone for background task
        let state_clone = state.clone();
        let acquire_results_clone = acquire_results.clone();
        let clients_clone = clients.clone();
        let extension_cadence_clone = extension_cadence;

        // Spawn background task for lock extension
        let extension_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(extension_cadence_clone);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;

                // Check if we should stop (receiver closed)
                if lost_sender.is_closed() {
                    break;
                }

                // Create cancellation token (not used for extend, but required by API)
                let (_cancel_sender, cancel_receiver) = watch::channel(false);

                // Extend the lock
                let state_for_extend = state_clone.clone();
                match extend_redlock(
                    move |client| {
                        let state = state_for_extend.clone();
                        let client = client.clone();
                        async move { state.try_extend(&client).await }
                    },
                    &clients_clone,
                    &acquire_results_clone,
                    &state_clone.timeouts,
                    &cancel_receiver,
                )
                .await
                {
                    Ok(Some(true)) => {
                        // Successfully extended
                        continue;
                    }
                    Ok(Some(false)) => {
                        // Failed to extend - lock lost
                        let _ = lost_sender.send(true);
                        break;
                    }
                    Ok(None) => {
                        // Inconclusive - continue trying
                        continue;
                    }
                    Err(_) => {
                        // Error extending - lock lost
                        let _ = lost_sender.send(true);
                        break;
                    }
                }
            }
        });

        Self {
            state,
            acquire_results,
            clients,
            extension_cadence,
            expiry,
            lost_receiver,
            extension_task,
        }
    }
}

impl LockHandle for RedisLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

    #[instrument(skip(self), fields(lock.key = %self.state.key, backend = "redis"))]
    async fn release(self) -> LockResult<()> {
        // Abort the extension task
        self.extension_task.abort();
        // Don't await - just abort and continue

        // Release the lock on all clients
        let state = self.state.clone();
        let clients = self.clients.clone();
        let acquire_results = self.acquire_results.clone();
        release_redlock(
            move |client| {
                let state = state.clone();
                let client = client.clone();
                async move { state.try_release(&client).await }
            },
            &clients,
            &acquire_results,
        )
        .await
    }
}

impl Drop for RedisLockHandle {
    fn drop(&mut self) {
        // Abort extension task
        self.extension_task.abort();
        // Note: We can't async release in Drop, so the lock will expire naturally
        // For proper cleanup, users should call release() explicitly
    }
}
