//! Redis distributed semaphore implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::timeout::TimeoutValue;
use distributed_lock_core::traits::{DistributedSemaphore, LockHandle};
use fred::prelude::*;
use fred::types::CustomCommand;
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
    /// Lua script for semaphore acquisition.
    /// 1. Gets Redis time
    /// 2. Removes expired tickets (zremrangebyscore)
    /// 3. Checks if count < max_count
    /// 4. Adds new ticket with expiry score if allowed
    /// 5. Extends set TTL
    const ACQUIRE_SCRIPT: &'static str = r#"
        redis.replicate_commands()
        local nowResult = redis.call('time')
        local nowMillis = (tonumber(nowResult[1]) * 1000.0) + (tonumber(nowResult[2]) / 1000.0)
        
        redis.call('zremrangebyscore', KEYS[1], '-inf', nowMillis)
        
        if redis.call('zcard', KEYS[1]) < tonumber(ARGV[1]) then
            redis.call('zadd', KEYS[1], nowMillis + tonumber(ARGV[2]), ARGV[3])
            
            -- Extend key TTL (set to 3x ticket expiry to be safe)
            local keyTtl = redis.call('pttl', KEYS[1])
            if keyTtl < tonumber(ARGV[4]) then
                redis.call('pexpire', KEYS[1], ARGV[4])
            end
            return 1
        end
        return 0
    "#;

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

    /// Attempts to acquire a semaphore ticket without waiting.
    async fn try_acquire_internal(&self) -> LockResult<Option<RedisSemaphoreHandle>> {
        let lock_id = Self::generate_lock_id();
        let expiry_millis = self.expiry.as_millis() as u64;

        // TTL for the whole set (3x expiry)
        let set_expiry_millis = expiry_millis * 3;

        let args: Vec<RedisValue> = vec![
            Self::ACQUIRE_SCRIPT.into(),
            1_i64.into(),                      // numkeys
            self.key.clone().into(),           // KEYS[1]
            (self.max_count as i64).into(),    // ARGV[1]
            (expiry_millis as i64).into(),     // ARGV[2]
            lock_id.clone().into(),            // ARGV[3]
            (set_expiry_millis as i64).into(), // ARGV[4]
        ];

        let cmd = CustomCommand::new_static("EVAL", None, false);
        let result: i64 = self.client.custom(cmd, args).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "Redis custom EVAL (acquire semaphore) failed: {}",
                e
            ))))
        })?;

        if result == 1 {
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
        } else {
            Ok(None)
        }
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
        // TODO: Could optimize this further but atomicity is primary goal now
        let mut sleep_duration = Duration::from_millis(10);
        const MAX_SLEEP: Duration = Duration::from_millis(200);

        loop {
            match self.try_acquire_internal().await {
                Ok(Some(handle)) => return Ok(handle),
                Ok(None) => {
                    // Check timeout
                    if !timeout_value.is_infinite()
                        && start.elapsed() >= timeout_value.as_duration().unwrap()
                    {
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
    #[allow(dead_code)]
    expiry: Duration,
    /// Extension cadence.
    #[allow(dead_code)]
    extension_cadence: Duration,
    /// Watch channel for lock lost detection.
    lost_receiver: watch::Receiver<bool>,
    /// Background task handle for lock extension.
    _extension_task: tokio::task::JoinHandle<()>,
}

impl RedisSemaphoreHandle {
    /// Lua script for semaphore extension.
    /// 1. Gets Redis time
    /// 2. Updates score (expiry) for our lock ID only if it exists (XX)
    /// 3. Extends set TTL
    const EXTEND_SCRIPT: &'static str = r#"
        redis.replicate_commands()
        local nowResult = redis.call('time')
        local nowMillis = (tonumber(nowResult[1]) * 1000.0) + (tonumber(nowResult[2]) / 1000.0)
        
        local result = redis.call('zadd', KEYS[1], 'XX', 'CH', nowMillis + tonumber(ARGV[1]), ARGV[2])
        
        -- Extend key TTL
        local keyTtl = redis.call('pttl', KEYS[1])
        if keyTtl < tonumber(ARGV[3]) then
            redis.call('pexpire', KEYS[1], ARGV[3])
        end
        return result
    "#;

    /// Lua script for semaphore release.
    const RELEASE_SCRIPT: &'static str = r#"
        return redis.call('zrem', KEYS[1], ARGV[1])
    "#;

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
        let extension_lost_sender = lost_sender.clone();

        // Spawn background task to extend the lock
        let extension_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(extension_cadence);
            // TTL for the whole set (3x expiry)
            let set_expiry_millis = extension_expiry.as_millis() * 3;

            loop {
                interval.tick().await;

                // Check if we should stop (sender closed)
                if extension_lost_sender.is_closed() {
                    break;
                }

                let expiry_millis = extension_expiry.as_millis() as u64;

                let args: Vec<RedisValue> = vec![
                    Self::EXTEND_SCRIPT.into(),
                    1_i64.into(), // numkeys
                    extension_key.clone().into(),
                    (expiry_millis as i64).into(),
                    extension_lock_id.clone().into(),
                    (set_expiry_millis as i64).into(),
                ];

                let cmd = CustomCommand::new_static("EVAL", None, false);
                let result_op: Result<i64, _> = extension_client.custom(cmd, args).await;

                match result_op {
                    Ok(changed_count) => {
                        // zadd with CH returns number of changed elements.
                        // If 0, it means the element wasn't found (expired/lost).
                        if changed_count == 0 {
                            let _ = extension_lost_sender.send(true);
                            break;
                        }
                    }
                    Err(_) => {
                        // Connection error - signal lock lost
                        let _ = extension_lost_sender.send(true);
                        break;
                    }
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

        let args: Vec<RedisValue> = vec![
            Self::RELEASE_SCRIPT.into(),
            1_i64.into(), // numkeys
            self.key.clone().into(),
            self.lock_id.clone().into(),
        ];

        let cmd = CustomCommand::new_static("EVAL", None, false);

        // Remove our ticket from the sorted set
        let _: i64 = self.client.custom(cmd, args).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "failed to release semaphore ticket: {}",
                e
            ))))
        })?;

        Ok(())
    }
}

impl Drop for RedisSemaphoreHandle {
    fn drop(&mut self) {
        // Abort extension task
        self._extension_task.abort();
        // Note: We cannot async release in Drop, so the ticket will expire naturally
        // For proper cleanup, users should call release() explicitly
    }
}
