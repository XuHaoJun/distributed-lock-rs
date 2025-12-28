//! Redis distributed lock implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::DistributedLock;
use fred::prelude::*;
use fred::types::CustomCommand; // Correct import
use tracing::{instrument, Span};

use crate::redlock::{acquire::acquire_redlock, helper::RedLockHelper, timeouts::RedLockTimeouts};

/// Internal state for a Redis lock.
#[derive(Debug, Clone)]
pub struct RedisLockState {
    /// Redis key for the lock.
    pub key: String,
    /// Unique lock ID for this acquisition.
    pub lock_id: String,
    /// Timeout configuration.
    pub timeouts: RedLockTimeouts,
}

impl RedisLockState {
    /// Creates a new lock state.
    pub fn new(key: String, timeouts: RedLockTimeouts) -> Self {
        Self {
            key,
            lock_id: RedLockHelper::create_lock_id(),
            timeouts,
        }
    }

    /// Attempts to acquire the lock on a single Redis client.
    pub async fn try_acquire(&self, client: &RedisClient) -> LockResult<bool> {
        let expiry_millis = self.timeouts.expiry.as_millis() as i64;

        // Use SET NX PX to atomically set the key if it doesn't exist
        // Note: Using PX (milliseconds) instead of EX (seconds)
        let result: Option<String> = client
            .set(
                &self.key,
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

        // SET NX returns Some(value) if key was set, None if key already exists
        Ok(result.is_some())
    }

    // ... imports ...
    // No special imports needed for custom command if RedisClient is in scope,
    // but we need RedisValue which is in prelude.

    // ...

    /// Lua script to extend the lock duration.
    const EXTEND_SCRIPT_LUA: &'static str = r#"
        if redis.call('get', KEYS[1]) == ARGV[1] then
            return redis.call('pexpire', KEYS[1], ARGV[2])
        end
        return 0
    "#;

    /// Lua script to release the lock.
    const RELEASE_SCRIPT_LUA: &'static str = r#"
        if redis.call('get', KEYS[1]) == ARGV[1] then
            return redis.call('del', KEYS[1])
        end
        return 0
    "#;

    /// Attempts to extend the lock on a single Redis client.
    ///
    /// Uses a Lua script to atomically verify ownership and extend TTL.
    pub async fn try_extend(&self, client: &RedisClient) -> LockResult<bool> {
        let expiry_millis = self.timeouts.expiry.as_millis() as i64;

        let args: Vec<RedisValue> = vec![
            Self::EXTEND_SCRIPT_LUA.into(),
            1_i64.into(), // numkeys
            self.key.clone().into(),
            self.lock_id.clone().into(),
            expiry_millis.into(),
        ];

        // CustomCommand::new_static is common for static strings or just new
        let cmd = CustomCommand::new_static("EVAL", None, false);

        let result: i64 = client.custom(cmd, args).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "Redis custom EVAL (extend) failed: {}",
                e
            ))))
        })?;

        Ok(result == 1)
    }

    /// Attempts to release the lock on a single Redis client.
    ///
    /// Uses a Lua script to atomically verify ownership before deleting.
    pub async fn try_release(&self, client: &RedisClient) -> LockResult<()> {
        let args: Vec<RedisValue> = vec![
            Self::RELEASE_SCRIPT_LUA.into(),
            1_i64.into(), // numkeys
            self.key.clone().into(),
            self.lock_id.clone().into(),
        ];

        let cmd = CustomCommand::new_static("EVAL", None, false);

        let _: i64 = client.custom(cmd, args).await.map_err(|e| {
            LockError::Backend(Box::new(std::io::Error::other(format!(
                "Redis custom EVAL (release) failed: {}",
                e
            ))))
        })?;

        Ok(())
    }
}

/// A Redis-based distributed lock.
///
/// Supports single-server and multi-server (RedLock) configurations.
pub struct RedisDistributedLock {
    /// Lock state.
    state: RedisLockState,
    /// Redis clients (one for single-server, multiple for RedLock).
    clients: Vec<RedisClient>,
    /// Extension cadence for background renewal.
    extension_cadence: Duration,
}

impl RedisDistributedLock {
    /// Creates a new Redis distributed lock.
    pub(crate) fn new(
        name: String,
        clients: Vec<RedisClient>,
        expiry: Duration,
        min_validity: Duration,
        extension_cadence: Duration,
    ) -> Self {
        let key = format!("distributed-lock:{}", name);
        let timeouts = RedLockTimeouts::new(expiry, min_validity);

        Self {
            state: RedisLockState::new(key, timeouts),
            clients,
            extension_cadence,
        }
    }

    /// Gets the lock name.
    pub fn name(&self) -> &str {
        // Extract name from key (remove "distributed-lock:" prefix)
        self.state
            .key
            .strip_prefix("distributed-lock:")
            .unwrap_or(&self.state.key)
    }
}

impl DistributedLock for RedisDistributedLock {
    type Handle = crate::handle::RedisLockHandle;

    fn name(&self) -> &str {
        self.name()
    }

    #[instrument(skip(self), fields(lock.name = %self.name(), lock.key = %self.state.key, timeout = ?timeout, backend = "redis", servers = self.clients.len()))]
    async fn acquire(&self, timeout: Option<Duration>) -> LockResult<Self::Handle> {
        use tokio::sync::watch;

        let start = std::time::Instant::now();
        Span::current().record("operation", "acquire");

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

        // Acquire using RedLock algorithm
        let state = self.state.clone();
        let clients = self.clients.clone();
        let timeouts = self.state.timeouts.clone();
        let acquire_result = acquire_redlock(
            move |client| {
                let state = state.clone();
                let client = client.clone();
                async move { state.try_acquire(&client).await }
            },
            &clients,
            &timeouts,
            &cancel_receiver,
        )
        .await?;

        let acquire_result = match acquire_result {
            Some(result) if result.is_successful(clients.len()) => {
                let elapsed = start.elapsed();
                Span::current().record("acquired", true);
                Span::current().record("elapsed_ms", elapsed.as_millis() as u64);
                Span::current().record(
                    "servers_acquired",
                    result.acquire_results.iter().filter(|&&b| b).count(),
                );
                result
            }
            _ => {
                Span::current().record("acquired", false);
                Span::current().record("error", "timeout");
                return Err(LockError::Timeout(
                    timeout.unwrap_or(Duration::from_secs(0)),
                ));
            }
        };

        // Create handle with background extension
        Ok(crate::handle::RedisLockHandle::new(
            self.state.clone(),
            acquire_result.acquire_results,
            clients,
            self.extension_cadence,
            self.state.timeouts.expiry,
        ))
    }

    #[instrument(skip(self), fields(lock.name = %self.name(), lock.key = %self.state.key, backend = "redis", servers = self.clients.len()))]
    async fn try_acquire(&self) -> LockResult<Option<Self::Handle>> {
        use tokio::sync::watch;

        Span::current().record("operation", "try_acquire");

        // Create cancellation token (not used for try_acquire, but required by API)
        let (_cancel_sender, cancel_receiver) = watch::channel(false);

        // Acquire using RedLock algorithm
        let state = self.state.clone();
        let clients = self.clients.clone();
        let timeouts = self.state.timeouts.clone();
        let acquire_result = acquire_redlock(
            move |client| {
                let state = state.clone();
                let client = client.clone();
                async move { state.try_acquire(&client).await }
            },
            &clients,
            &timeouts,
            &cancel_receiver,
        )
        .await?;

        let result = match acquire_result {
            Some(result) if result.is_successful(clients.len()) => {
                Span::current().record("acquired", true);
                Span::current().record(
                    "servers_acquired",
                    result.acquire_results.iter().filter(|&&b| b).count(),
                );
                Ok(Some(crate::handle::RedisLockHandle::new(
                    self.state.clone(),
                    result.acquire_results,
                    clients,
                    self.extension_cadence,
                    self.state.timeouts.expiry,
                )))
            }
            _ => {
                Span::current().record("acquired", false);
                Span::current().record("reason", "lock_held");
                Ok(None)
            }
        };
        result
    }
}
