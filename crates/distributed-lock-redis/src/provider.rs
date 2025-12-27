//! Redis lock provider implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::{
    LockProvider, ReaderWriterLockProvider, SemaphoreProvider,
};

use crate::lock::RedisDistributedLock;
use crate::rw_lock::RedisDistributedReaderWriterLock;
use crate::semaphore::RedisDistributedSemaphore;
use fred::prelude::*;

/// Builder for Redis lock provider configuration.
pub struct RedisLockProviderBuilder {
    urls: Vec<String>,
    clients: Vec<RedisClient>,
    expiry: Duration,
    extension_cadence: Duration,
    min_validity: Duration,
}

impl RedisLockProviderBuilder {
    /// Creates a new builder with default settings.
    pub fn new() -> Self {
        Self {
            urls: vec![],
            clients: vec![],
            expiry: Duration::from_secs(30),
            extension_cadence: Duration::from_secs(10),
            min_validity: Duration::from_millis(27000), // 90% of expiry
        }
    }

    /// Adds a Redis server URL.
    ///
    /// For RedLock, add multiple URLs (ideally 3 or 5).
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.urls.push(url.into());
        self
    }

    /// Adds multiple Redis server URLs.
    pub fn urls(mut self, urls: &[impl AsRef<str>]) -> Self {
        for url in urls {
            self.urls.push(url.as_ref().to_string());
        }
        self
    }

    /// Uses an existing Redis client.
    pub fn client(mut self, client: RedisClient) -> Self {
        self.clients.push(client);
        self
    }

    /// Sets the lock expiry time.
    pub fn expiry(mut self, expiry: Duration) -> Self {
        self.expiry = expiry;
        self
    }

    /// Sets the lock extension cadence.
    pub fn extension_cadence(mut self, cadence: Duration) -> Self {
        self.extension_cadence = cadence;
        self
    }

    /// Sets the minimum validity time.
    ///
    /// After acquiring, at least this much time must remain on the lock
    /// for the acquisition to be considered successful.
    pub fn min_validity(mut self, validity: Duration) -> Self {
        self.min_validity = validity;
        self
    }

    /// Builds the provider.
    pub async fn build(self) -> LockResult<RedisLockProvider> {
        let mut clients = self.clients;

        // Create clients from URLs if provided
        for url in self.urls {
            let config = RedisConfig::from_url(&url).map_err(|e| {
                LockError::Connection(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid Redis URL: {}", e),
                )))
            })?;

            let client = RedisClient::new(config, None, None, None);
            client.connect();
            client.wait_for_connect().await.map_err(|e| {
                LockError::Connection(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("failed to connect to Redis: {}", e),
                )))
            })?;

            clients.push(client);
        }

        if clients.is_empty() {
            return Err(LockError::InvalidName("no Redis clients or URLs provided".to_string()));
        }

        Ok(RedisLockProvider {
            clients: clients.clone(),
            primary_client: clients.into_iter().next().unwrap(), // For semaphores, use single client
            expiry: self.expiry,
            extension_cadence: self.extension_cadence,
            min_validity: self.min_validity,
        })
    }
}

impl Default for RedisLockProviderBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Provider for Redis-based distributed locks.
///
/// Supports single-server and multi-server (RedLock) configurations.
pub struct RedisLockProvider {
    /// All Redis clients (for RedLock).
    clients: Vec<RedisClient>,
    /// Primary client (for semaphores which don't support RedLock).
    primary_client: RedisClient,
    /// Lock expiry time.
    expiry: Duration,
    /// Extension cadence.
    extension_cadence: Duration,
    /// Minimum validity time.
    min_validity: Duration,
}

impl RedisLockProvider {
    /// Returns a new builder for configuring the provider.
    pub fn builder() -> RedisLockProviderBuilder {
        RedisLockProviderBuilder::new()
    }

    /// Creates a provider using the specified Redis URL.
    pub async fn new(url: impl Into<String>) -> LockResult<Self> {
        Self::builder().url(url).build().await
    }
}

impl LockProvider for RedisLockProvider {
    type Lock = RedisDistributedLock;

    fn create_lock(&self, name: &str) -> Self::Lock {
        RedisDistributedLock::new(
            name.to_string(),
            self.clients.clone(),
            self.expiry,
            self.min_validity,
            self.extension_cadence,
        )
    }
}

impl ReaderWriterLockProvider for RedisLockProvider {
    type Lock = RedisDistributedReaderWriterLock;

    fn create_reader_writer_lock(&self, name: &str) -> Self::Lock {
        RedisDistributedReaderWriterLock::new(
            name.to_string(),
            self.clients.clone(),
            self.expiry,
            self.min_validity,
            self.extension_cadence,
        )
    }
}

impl SemaphoreProvider for RedisLockProvider {
    type Semaphore = RedisDistributedSemaphore;

    fn create_semaphore(&self, name: &str, max_count: u32) -> Self::Semaphore {
        RedisDistributedSemaphore::new(
            name.to_string(),
            max_count,
            self.primary_client.clone(),
            self.expiry,
            self.extension_cadence,
        )
    }
}
