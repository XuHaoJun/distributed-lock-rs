//! Redis reader-writer lock implementation.

use std::sync::Arc;
use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::timeout::TimeoutValue;
use distributed_lock_core::traits::{DistributedReaderWriterLock, LockHandle};
use fred::prelude::*;
use tokio::sync::watch;

use crate::redlock::{
    acquire::acquire_redlock, extend::extend_redlock, helper::RedLockHelper,
    release::release_redlock, timeouts::RedLockTimeouts,
};

/// Internal state for a Redis read lock.
#[derive(Debug, Clone)]
pub(crate) struct RedisReadLockState {
    /// Redis key for the reader set.
    reader_key: String,
    /// Redis key for the writer lock.
    writer_key: String,
    /// Unique lock ID for this acquisition.
    lock_id: String,
    /// Timeout configuration.
    timeouts: RedLockTimeouts,
}

impl RedisReadLockState {
    fn new(reader_key: String, writer_key: String, timeouts: RedLockTimeouts) -> Self {
        Self {
            reader_key,
            writer_key,
            lock_id: RedLockHelper::create_lock_id(),
            timeouts,
        }
    }

    /// Attempts to acquire a read lock on a single Redis client.
    async fn try_acquire(&self, client: &RedisClient) -> LockResult<bool> {
        let expiry_millis = self.timeouts.expiry.as_millis() as i64;

        // Check if writer lock exists
        let writer_exists: bool = client.exists(&self.writer_key).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "Redis EXISTS failed: {}",
                e
            ))))
        })?;

        if writer_exists {
            return Ok(false);
        }

        // Add our lock ID to the reader set
        let _: u32 = client
            .sadd(&self.reader_key, &self.lock_id)
            .await
            .map_err(|e| {
                LockError::Backend(Box::new(std::io::Error::other(format!(
                    "Redis SADD failed: {}",
                    e
                ))))
            })?;

        // Get current TTL and extend if needed
        let current_ttl: i64 = client.pttl(&self.reader_key).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "Redis PTTL failed: {}",
                e
            ))))
        })?;

        if current_ttl < expiry_millis {
            let _: bool = client
                .pexpire(&self.reader_key, expiry_millis, None)
                .await
                .map_err(|e| {
                    LockError::Backend(Box::new(std::io::Error::other(format!(
                        "Redis PEXPIRE failed: {}",
                        e
                    ))))
                })?;
        }

        Ok(true)
    }

    /// Attempts to extend a read lock on a single Redis client.
    async fn try_extend(&self, client: &RedisClient) -> LockResult<bool> {
        let expiry_millis = self.timeouts.expiry.as_millis() as i64;

        // Check if our lock ID is still in the reader set
        let is_member: bool = client
            .sismember(&self.reader_key, &self.lock_id)
            .await
            .map_err(|e| {
                LockError::Backend(Box::new(std::io::Error::other(format!(
                    "Redis SISMEMBER failed: {}",
                    e
                ))))
            })?;

        if !is_member {
            return Ok(false);
        }

        // Extend TTL if needed
        let current_ttl: i64 = client.pttl(&self.reader_key).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "Redis PTTL failed: {}",
                e
            ))))
        })?;

        if current_ttl < expiry_millis {
            let _: bool = client
                .pexpire(&self.reader_key, expiry_millis, None)
                .await
                .map_err(|e| {
                    LockError::Backend(Box::new(std::io::Error::other(format!(
                        "Redis PEXPIRE failed: {}",
                        e
                    ))))
                })?;
        }

        Ok(true)
    }

    /// Attempts to release a read lock on a single Redis client.
    async fn try_release(&self, client: &RedisClient) -> LockResult<()> {
        // Remove our lock ID from the reader set
        let _: u32 = client
            .srem(&self.reader_key, &self.lock_id)
            .await
            .map_err(|e| {
                LockError::Backend(Box::new(std::io::Error::other(format!(
                    "Redis SREM failed: {}",
                    e
                ))))
            })?;

        Ok(())
    }
}

/// Internal state for a Redis write lock.
#[derive(Debug, Clone)]
pub(crate) struct RedisWriteLockState {
    /// Redis key for the reader set.
    reader_key: String,
    /// Redis key for the writer lock.
    writer_key: String,
    /// Unique lock ID for this acquisition.
    lock_id: String,
    /// Writer waiting lock ID (with suffix).
    waiting_lock_id: String,
    /// Timeout configuration.
    timeouts: RedLockTimeouts,
}

const WRITER_WAITING_SUFFIX: &str = "_WRITERWAITING";

impl RedisWriteLockState {
    fn new(reader_key: String, writer_key: String, timeouts: RedLockTimeouts) -> Self {
        let lock_id = RedLockHelper::create_lock_id();
        Self {
            reader_key,
            writer_key,
            waiting_lock_id: format!("{}{}", lock_id, WRITER_WAITING_SUFFIX),
            lock_id,
            timeouts,
        }
    }

    /// Attempts to acquire a write lock on a single Redis client.
    async fn try_acquire(&self, client: &RedisClient) -> LockResult<bool> {
        let expiry_millis = self.timeouts.expiry.as_millis() as i64;

        // Check current writer value
        let writer_value: Option<String> = client.get(&self.writer_key).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "Redis GET failed: {}",
                e
            ))))
        })?;

        // If writer exists and it's not our waiting ID, fail
        if let Some(ref value) = writer_value {
            if value != &self.waiting_lock_id {
                return Ok(false);
            }
        }

        // Check if there are any readers
        let reader_count: u32 = client.scard(&self.reader_key).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "Redis SCARD failed: {}",
                e
            ))))
        })?;

        if reader_count == 0 {
            // No readers - acquire write lock
            let _: Option<String> = client
                .set(
                    &self.writer_key,
                    &self.lock_id,
                    Some(Expiration::PX(expiry_millis)),
                    Some(SetOptions::NX),
                    false,
                )
                .await
                .map_err(|e| {
                    LockError::Backend(Box::new(std::io::Error::other(format!(
                        "Redis SET NX failed: {}",
                        e
                    ))))
                })?;
            Ok(true)
        } else {
            // Readers exist - set/update waiting lock if we don't have it yet
            if writer_value.is_none() {
                let _: Option<String> = client
                    .set(
                        &self.writer_key,
                        &self.waiting_lock_id,
                        Some(Expiration::PX(expiry_millis)),
                        Some(SetOptions::NX),
                        false,
                    )
                    .await
                    .map_err(|e| {
                        LockError::Backend(Box::new(std::io::Error::other(format!(
                            "Redis SET NX failed: {}",
                            e
                        ))))
                    })?;
            } else {
                // Extend waiting lock TTL
                let _: bool = client
                    .pexpire(&self.writer_key, expiry_millis, None)
                    .await
                    .map_err(|e| {
                        LockError::Backend(Box::new(std::io::Error::other(format!(
                            "Redis PEXPIRE failed: {}",
                            e
                        ))))
                    })?;
            }
            Ok(false)
        }
    }

    /// Attempts to extend a write lock on a single Redis client.
    async fn try_extend(&self, client: &RedisClient) -> LockResult<bool> {
        let expiry_millis = self.timeouts.expiry.as_millis() as i64;

        // Check if writer lock exists and value matches our lock ID
        let writer_value: Option<String> = client.get(&self.writer_key).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "Redis GET failed: {}",
                e
            ))))
        })?;

        match writer_value {
            Some(value) if value == self.lock_id => {
                // Value matches - extend TTL
                let _: bool = client
                    .pexpire(&self.writer_key, expiry_millis, None)
                    .await
                    .map_err(|e| {
                        LockError::Backend(Box::new(std::io::Error::other(format!(
                            "Redis PEXPIRE failed: {}",
                            e
                        ))))
                    })?;
                Ok(true)
            }
            _ => Ok(false), // Lock doesn't exist or value doesn't match
        }
    }

    /// Attempts to release a write lock on a single Redis client.
    async fn try_release(&self, client: &RedisClient) -> LockResult<()> {
        // Check if writer lock exists and value matches our lock ID
        let writer_value: Option<String> = client.get(&self.writer_key).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "Redis GET failed: {}",
                e
            ))))
        })?;

        match writer_value {
            Some(value) if value == self.lock_id => {
                // Value matches - delete the key
                let _: i64 = client.del(&self.writer_key).await.map_err(|e| {
                    LockError::Backend(Box::new(std::io::Error::other(format!(
                        "Redis DEL failed: {}",
                        e
                    ))))
                })?;
            }
            _ => {
                // Lock doesn't exist or value doesn't match - already released or not ours
            }
        }

        Ok(())
    }
}

/// A Redis-based distributed reader-writer lock.
///
/// Supports single-server and multi-server (RedLock) configurations.
pub struct RedisDistributedReaderWriterLock {
    /// Reader key.
    reader_key: String,
    /// Writer key.
    writer_key: String,
    /// Redis clients (one for single-server, multiple for RedLock).
    clients: Vec<RedisClient>,
    /// Extension cadence for background renewal.
    extension_cadence: Duration,
    /// Timeout configuration.
    timeouts: RedLockTimeouts,
}

impl RedisDistributedReaderWriterLock {
    /// Creates a new Redis distributed reader-writer lock.
    pub(crate) fn new(
        name: String,
        clients: Vec<RedisClient>,
        expiry: Duration,
        min_validity: Duration,
        extension_cadence: Duration,
    ) -> Self {
        let reader_key = format!("distributed-lock:{}:readers", name);
        let writer_key = format!("distributed-lock:{}:writer", name);
        let timeouts = RedLockTimeouts::new(expiry, min_validity);

        Self {
            reader_key,
            writer_key,
            clients,
            extension_cadence,
            timeouts,
        }
    }

    /// Gets the lock name.
    pub fn name(&self) -> &str {
        // Extract name from reader key (remove "distributed-lock:" prefix and ":readers" suffix)
        self.reader_key
            .strip_prefix("distributed-lock:")
            .and_then(|s| s.strip_suffix(":readers"))
            .unwrap_or(&self.reader_key)
    }
}

impl DistributedReaderWriterLock for RedisDistributedReaderWriterLock {
    type ReadHandle = RedisReadLockHandle;
    type WriteHandle = RedisWriteLockHandle;

    fn name(&self) -> &str {
        self.name()
    }

    async fn acquire_read(&self, timeout: Option<Duration>) -> LockResult<Self::ReadHandle> {
        use tokio::sync::watch;

        // Create cancellation token
        let (cancel_sender, cancel_receiver) = watch::channel(false);

        // If timeout is provided, spawn a task to signal cancellation after timeout
        if let Some(timeout_duration) = timeout {
            let cancel_sender_clone = cancel_sender.clone();
            tokio::spawn(async move {
                tokio::time::sleep(timeout_duration).await;
                let _ = cancel_sender_clone.send(true);
            });
        }

        let state = RedisReadLockState::new(
            self.reader_key.clone(),
            self.writer_key.clone(),
            self.timeouts.clone(),
        );
        let clients = self.clients.clone();
        let timeouts = self.timeouts.clone();

        // Clone state for the closure
        let state_for_acquire = state.clone();

        // Acquire using RedLock algorithm
        let acquire_result = acquire_redlock(
            move |client| {
                let state = state_for_acquire.clone();
                let client = client.clone();
                async move { state.try_acquire(&client).await }
            },
            &clients,
            &timeouts,
            &cancel_receiver,
        )
        .await?;

        let acquire_result = match acquire_result {
            Some(result) if result.is_successful(clients.len()) => result,
            _ => {
                return Err(LockError::Timeout(
                    timeout.unwrap_or(Duration::from_secs(0)),
                ));
            }
        };

        // Create handle with background extension
        Ok(RedisReadLockHandle::new(
            state,
            acquire_result.acquire_results,
            clients,
            self.extension_cadence,
            self.timeouts.expiry,
        ))
    }

    async fn try_acquire_read(&self) -> LockResult<Option<Self::ReadHandle>> {
        use tokio::sync::watch;

        // Create cancellation token (not used for try_acquire, but required by API)
        let (_cancel_sender, cancel_receiver) = watch::channel(false);

        let state = RedisReadLockState::new(
            self.reader_key.clone(),
            self.writer_key.clone(),
            self.timeouts.clone(),
        );
        let clients = self.clients.clone();
        let timeouts = self.timeouts.clone();

        // Clone state for the closure
        let state_for_acquire = state.clone();

        // Acquire using RedLock algorithm
        let acquire_result = acquire_redlock(
            move |client| {
                let state = state_for_acquire.clone();
                let client = client.clone();
                async move { state.try_acquire(&client).await }
            },
            &clients,
            &timeouts,
            &cancel_receiver,
        )
        .await?;

        match acquire_result {
            Some(result) if result.is_successful(clients.len()) => {
                Ok(Some(RedisReadLockHandle::new(
                    state,
                    result.acquire_results,
                    clients,
                    self.extension_cadence,
                    self.timeouts.expiry,
                )))
            }
            _ => Ok(None),
        }
    }

    async fn acquire_write(&self, timeout: Option<Duration>) -> LockResult<Self::WriteHandle> {
        use tokio::sync::watch;

        // Create cancellation token
        let (cancel_sender, cancel_receiver) = watch::channel(false);

        // If timeout is provided, spawn a task to signal cancellation after timeout
        if let Some(timeout_duration) = timeout {
            let cancel_sender_clone = cancel_sender.clone();
            tokio::spawn(async move {
                tokio::time::sleep(timeout_duration).await;
                let _ = cancel_sender_clone.send(true);
            });
        }

        let state = RedisWriteLockState::new(
            self.reader_key.clone(),
            self.writer_key.clone(),
            self.timeouts.clone(),
        );
        let clients = self.clients.clone();
        let timeouts = self.timeouts.clone();

        // Busy-wait with exponential backoff
        let timeout_value = TimeoutValue::from(timeout);
        let start = std::time::Instant::now();
        let mut sleep_duration = Duration::from_millis(50);
        const MAX_SLEEP: Duration = Duration::from_secs(1);

        loop {
            // Check timeout
            if !timeout_value.is_infinite()
                && start.elapsed() >= timeout_value.as_duration().unwrap()
            {
                return Err(LockError::Timeout(timeout_value.as_duration().unwrap()));
            }

            // Check for cancellation
            if cancel_receiver.has_changed().unwrap_or(false) && *cancel_receiver.borrow() {
                return Err(LockError::Cancelled);
            }

            // Clone state for this iteration
            let state_for_acquire = state.clone();

            // Try to acquire write lock
            let acquire_result = acquire_redlock(
                move |client| {
                    let state = state_for_acquire.clone();
                    let client = client.clone();
                    async move { state.try_acquire(&client).await }
                },
                &clients,
                &timeouts,
                &cancel_receiver,
            )
            .await?;

            match acquire_result {
                Some(result) if result.is_successful(clients.len()) => {
                    // Successfully acquired
                    return Ok(RedisWriteLockHandle::new(
                        state,
                        result.acquire_results,
                        clients,
                        self.extension_cadence,
                        self.timeouts.expiry,
                    ));
                }
                _ => {
                    // Failed - sleep and retry
                    tokio::time::sleep(sleep_duration).await;
                    sleep_duration = (sleep_duration * 2).min(MAX_SLEEP);
                }
            }
        }
    }

    async fn try_acquire_write(&self) -> LockResult<Option<Self::WriteHandle>> {
        use tokio::sync::watch;

        // Create cancellation token (not used for try_acquire, but required by API)
        let (_cancel_sender, cancel_receiver) = watch::channel(false);

        let state = RedisWriteLockState::new(
            self.reader_key.clone(),
            self.writer_key.clone(),
            self.timeouts.clone(),
        );
        let clients = self.clients.clone();
        let timeouts = self.timeouts.clone();

        // Clone state for the closure
        let state_for_acquire = state.clone();

        // Acquire using RedLock algorithm
        let acquire_result = acquire_redlock(
            move |client| {
                let state = state_for_acquire.clone();
                let client = client.clone();
                async move { state.try_acquire(&client).await }
            },
            &clients,
            &timeouts,
            &cancel_receiver,
        )
        .await?;

        match acquire_result {
            Some(result) if result.is_successful(clients.len()) => {
                Ok(Some(RedisWriteLockHandle::new(
                    state,
                    result.acquire_results,
                    clients,
                    self.extension_cadence,
                    self.timeouts.expiry,
                )))
            }
            _ => Ok(None),
        }
    }
}

/// Handle for a held Redis read lock.
pub struct RedisReadLockHandle {
    /// Lock state.
    state: Arc<RedisReadLockState>,
    /// Acquire results indexed by client position.
    acquire_results: Arc<Vec<bool>>,
    /// Redis clients.
    clients: Arc<Vec<RedisClient>>,
    /// Extension cadence.
    #[allow(dead_code)]
    extension_cadence: Duration,
    /// Lock expiry duration.
    #[allow(dead_code)]
    expiry: Duration,
    /// Watch channel for lock lost detection.
    lost_receiver: watch::Receiver<bool>,
    /// Background extension task handle.
    extension_task: tokio::task::JoinHandle<()>,
}

impl RedisReadLockHandle {
    pub(crate) fn new(
        state: RedisReadLockState,
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

impl LockHandle for RedisReadLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

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

impl Drop for RedisReadLockHandle {
    fn drop(&mut self) {
        // Abort extension task
        self.extension_task.abort();
        // Note: We can't async release in Drop, so the lock will expire naturally
        // For proper cleanup, users should call release() explicitly
    }
}

/// Handle for a held Redis write lock.
pub struct RedisWriteLockHandle {
    /// Lock state.
    state: Arc<RedisWriteLockState>,
    /// Acquire results indexed by client position.
    acquire_results: Arc<Vec<bool>>,
    /// Redis clients.
    clients: Arc<Vec<RedisClient>>,
    /// Extension cadence.
    #[allow(dead_code)]
    extension_cadence: Duration,
    /// Lock expiry duration.
    #[allow(dead_code)]
    expiry: Duration,
    /// Watch channel for lock lost detection.
    lost_receiver: watch::Receiver<bool>,
    /// Background extension task handle.
    extension_task: tokio::task::JoinHandle<()>,
}

impl RedisWriteLockHandle {
    pub(crate) fn new(
        state: RedisWriteLockState,
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

impl LockHandle for RedisWriteLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

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

impl Drop for RedisWriteLockHandle {
    fn drop(&mut self) {
        // Abort extension task
        self.extension_task.abort();
        // Note: We cannot async release in Drop, so the lock will expire naturally
        // For proper cleanup, users should call release() explicitly
    }
}
