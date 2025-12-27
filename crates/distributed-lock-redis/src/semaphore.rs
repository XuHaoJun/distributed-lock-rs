//! Redis distributed semaphore implementation.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::timeout::TimeoutValue;
use distributed_lock_core::traits::{DistributedSemaphore, LockHandle};
use fred::prelude::*;
use rand::Rng;
use tokio::sync::watch;

/// A Redis-based distributed semaphore.
///
/// Uses Redis sorted sets to track semaphore tickets. Each ticket has an expiry time,
/// and expired tickets are automatically purged before acquisition attempts.
pub struct RedisDistributedSemaphore {
    /// Redis key for the semaphore.
    key: String,
    /// Original semaphore name.
    name: String,
    /// Maximum concurrent holders.
    max_count: u32,
    /// Redis client.
    client: RedisClient,
    /// Lock expiry time.
    expiry: Duration,
    /// Extension cadence for held locks.
    extension_cadence: Duration,
}

impl RedisDistributedSemaphore {
    pub(crate) fn new(
        name: String,
        max_count: u32,
        client: RedisClient,
        expiry: Duration,
        extension_cadence: Duration,
    ) -> Self {
        // Prefix the key to avoid collisions
        let key = format!("distributed-lock:semaphore:{}", name);
        Self {
            key,
            name,
            max_count,
            client,
            expiry,
            extension_cadence,
        }
    }

    /// Generates a unique lock ID for this acquisition.
    fn generate_lock_id() -> String {
        let mut rng = rand::thread_rng();
        format!("{:016x}", rng.gen::<u64>())
    }

    /// Gets current time in milliseconds since Unix epoch.
    fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    /// Attempts to acquire a semaphore ticket without waiting.
    async fn try_acquire_internal(&self) -> LockResult<Option<RedisSemaphoreHandle>> {
        let lock_id = Self::generate_lock_id();
        let now_millis = Self::now_millis();
        let expiry_millis = self.expiry.as_millis() as u64;
        let expiry_time = now_millis + expiry_millis;

        // Use Redis commands (non-atomic but simpler for now)
        // TODO: Use Lua script for atomicity when fred API is clarified

        // Remove expired entries
        let _: u32 = self
            .client
            .zremrangebyscore(&self.key, 0.0, now_millis as f64)
            .await
            .map_err(|e| {
                LockError::Backend(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Redis error: {}", e),
                )))
            })?;

        // Check current count
        let count: u32 = self.client.zcard(&self.key).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Redis error: {}", e),
            )))
        })?;

        if count >= self.max_count {
            return Ok(None);
        }

        // Add our ticket
        let _: () = self
            .client
            .zadd(
                &self.key,
                None,
                None,
                false,
                false,
                (expiry_time as f64, lock_id.clone()),
            )
            .await
            .map_err(|e| {
                LockError::Backend(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Redis error: {}", e),
                )))
            })?;

        // Set TTL on the key
        let set_expiry = expiry_millis * 2;
        let _: bool = self
            .client
            .pexpire(&self.key, set_expiry as i64, None)
            .await
            .map_err(|e| {
                LockError::Backend(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Redis error: {}", e),
                )))
            })?;

        // Successfully acquired
        let (sender, receiver) = watch::channel(false);
        Ok(Some(RedisSemaphoreHandle::new(
            self.key.clone(),
            lock_id,
            self.client.clone(),
            self.expiry,
            self.extension_cadence,
            sender,
            receiver,
        )))
    }
}

impl DistributedSemaphore for RedisDistributedSemaphore {
    type Handle = RedisSemaphoreHandle;

    fn name(&self) -> &str {
        &self.name
    }

    fn max_count(&self) -> u32 {
        self.max_count
    }

    async fn acquire(&self, timeout: Option<Duration>) -> LockResult<Self::Handle> {
        let timeout_value = TimeoutValue::from(timeout);
        let start = std::time::Instant::now();

        // Busy-wait with exponential backoff
        let mut sleep_duration = Duration::from_millis(10);
        const MAX_SLEEP: Duration = Duration::from_millis(200);

        loop {
            match self.try_acquire_internal().await {
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

    async fn try_acquire(&self) -> LockResult<Option<Self::Handle>> {
        self.try_acquire_internal().await
    }
}

/// Handle for a held semaphore ticket.
pub struct RedisSemaphoreHandle {
    /// Redis key for the semaphore.
    key: String,
    /// Unique lock ID for this ticket.
    lock_id: String,
    /// Redis client.
    client: RedisClient,
    /// Lock expiry time.
    expiry: Duration,
    /// Extension cadence.
    extension_cadence: Duration,
    /// Watch channel for lock lost detection.
    lost_receiver: watch::Receiver<bool>,
    /// Background task handle for lock extension.
    _extension_task: tokio::task::JoinHandle<()>,
}

impl RedisSemaphoreHandle {
    pub(crate) fn new(
        key: String,
        lock_id: String,
        client: RedisClient,
        expiry: Duration,
        extension_cadence: Duration,
        lost_sender: watch::Sender<bool>,
        lost_receiver: watch::Receiver<bool>,
    ) -> Self {
        let extension_key = key.clone();
        let extension_lock_id = lock_id.clone();
        let extension_client = client.clone();
        let extension_expiry = expiry;
        let extension_cadence = extension_cadence;
        let extension_lost_sender = lost_sender.clone();

        // Spawn background task to extend the lock
        let extension_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(extension_cadence);
            loop {
                interval.tick().await;

                // Check if we should stop (sender closed)
                if extension_lost_sender.is_closed() {
                    break;
                }

                // Extend the lock
                let now_millis = RedisDistributedSemaphore::now_millis();
                let expiry_millis = extension_expiry.as_millis() as u64;
                let expiry_time = now_millis + expiry_millis;

                // Remove expired entries
                let _: u32 = match extension_client
                    .zremrangebyscore(&extension_key, 0.0, now_millis as f64)
                    .await
                {
                    Ok(count) => count,
                    Err(_) => {
                        // Connection error - signal lock lost
                        let _ = extension_lost_sender.send(true);
                        break;
                    }
                };

                // Update our ticket expiry
                // Note: We use zadd with the same score to update expiry time
                let result: u32 = match extension_client
                    .zadd(
                        &extension_key,
                        None,
                        None,
                        false,
                        false,
                        (expiry_time as f64, extension_lock_id.clone()),
                    )
                    .await
                {
                    Ok(count) => count,
                    Err(_) => {
                        // Connection error - signal lock lost
                        let _ = extension_lost_sender.send(true);
                        break;
                    }
                };

                // Renew set TTL if needed
                let set_expiry = expiry_millis * 2;
                let _: bool = match extension_client
                    .pexpire(&extension_key, set_expiry as i64, None)
                    .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        // Connection error - signal lock lost
                        let _ = extension_lost_sender.send(true);
                        break;
                    }
                };

                // If update failed (result == 0), lock was removed - signal lost
                // Note: zadd returns the number of elements added, so 0 means it wasn't found
                if result == 0 {
                    let _ = extension_lost_sender.send(true);
                    break;
                }
            }
        });

        Self {
            key,
            lock_id,
            client,
            expiry,
            extension_cadence,
            lost_receiver,
            _extension_task: extension_task,
        }
    }
}

impl LockHandle for RedisSemaphoreHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

    async fn release(self) -> LockResult<()> {
        // Abort the extension task
        self._extension_task.abort();

        // Remove our ticket from the sorted set
        let _: () = self
            .client
            .zrem(&self.key, &self.lock_id)
            .await
            .map_err(|e| {
                LockError::Backend(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("failed to release semaphore ticket: {}", e),
                )))
            })?;

        Ok(())
    }
}

impl Drop for RedisSemaphoreHandle {
    fn drop(&mut self) {
        // Abort extension task
        self._extension_task.abort();
        // Note: We can't async release in Drop, so the ticket will expire naturally
        // For proper cleanup, users should call release() explicitly
    }
}
