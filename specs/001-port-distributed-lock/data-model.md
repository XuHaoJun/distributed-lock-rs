# Data Model: Port DistributedLock to Rust

**Feature Branch**: `001-port-distributed-lock`  
**Date**: 2025-12-27

## Core Types

### Lock Handle

The handle is returned when a lock is successfully acquired. Holding the handle means holding the lock. Dropping it releases the lock.

```rust
/// Handle representing a held distributed lock.
/// 
/// Dropping this handle releases the lock. For async contexts,
/// prefer calling `release()` explicitly to handle errors.
pub struct LockGuard<H: LockHandle> {
    inner: H,
}

impl<H: LockHandle> Drop for LockGuard<H> {
    fn drop(&mut self) {
        // Spawn release task if in async context
        // Otherwise block on release
    }
}
```

### Error Types

```rust
use thiserror::Error;

/// Errors that can occur during lock operations.
#[derive(Error, Debug)]
pub enum LockError {
    /// Lock acquisition timed out.
    #[error("lock acquisition timed out after {0:?}")]
    Timeout(Duration),
    
    /// Lock operation was cancelled.
    #[error("lock operation was cancelled")]
    Cancelled,
    
    /// Deadlock detected (e.g., same connection already holds lock).
    #[error("deadlock detected: {0}")]
    Deadlock(String),
    
    /// Backend connection failed.
    #[error("connection error: {0}")]
    Connection(#[source] Box<dyn std::error::Error + Send + Sync>),
    
    /// Lock was lost after acquisition (e.g., connection died).
    #[error("lock was lost: {0}")]
    LockLost(String),
    
    /// Invalid lock name.
    #[error("invalid lock name: {0}")]
    InvalidName(String),
    
    /// Backend-specific error.
    #[error("backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Result type for lock operations.
pub type LockResult<T> = Result<T, LockError>;
```

### Timeout Representation

```rust
/// Represents a timeout duration for lock operations.
/// 
/// - `Some(duration)` - Wait up to this duration
/// - `None` - Wait indefinitely
pub type Timeout = Option<Duration>;

/// Internal helper for timeout calculations.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TimeoutValue {
    millis: i64, // -1 for infinite
}

impl TimeoutValue {
    pub const INFINITE: Self = Self { millis: -1 };
    pub const ZERO: Self = Self { millis: 0 };
    
    pub fn is_infinite(&self) -> bool { self.millis < 0 }
    pub fn is_zero(&self) -> bool { self.millis == 0 }
    pub fn as_duration(&self) -> Option<Duration> {
        if self.is_infinite() { None } 
        else { Some(Duration::from_millis(self.millis as u64)) }
    }
}

impl From<Option<Duration>> for TimeoutValue {
    fn from(timeout: Option<Duration>) -> Self {
        match timeout {
            None => Self::INFINITE,
            Some(d) => Self { millis: d.as_millis() as i64 },
        }
    }
}
```

---

## Backend-Specific Types

### PostgreSQL

```rust
/// Key for PostgreSQL advisory locks.
/// 
/// Advisory locks use either a single 64-bit key or a pair of 32-bit keys.
/// These represent different key spaces and do not overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostgresAdvisoryLockKey {
    /// Single 64-bit key.
    Single(i64),
    /// Pair of 32-bit keys.
    Pair(i32, i32),
}

impl PostgresAdvisoryLockKey {
    /// Create a key from a string name.
    /// 
    /// - ASCII strings up to 9 chars are encoded directly (collision-free)
    /// - 16-char hex strings are parsed as i64
    /// - "XXXXXXXX,XXXXXXXX" format parsed as (i32, i32)
    /// - Other strings are hashed to i64 (if `allow_hashing` is true)
    pub fn from_name(name: &str, allow_hashing: bool) -> Result<Self, LockError> {
        // Implementation mirrors C# PostgresAdvisoryLockKey
        todo!()
    }
    
    /// Convert to SQL function arguments.
    pub fn to_sql_args(&self) -> String {
        match self {
            Self::Single(k) => format!("{}", k),
            Self::Pair(k1, k2) => format!("{}, {}", k1, k2),
        }
    }
}

/// Configuration for PostgreSQL distributed locks.
#[derive(Debug, Clone)]
pub struct PostgresLockConfig {
    /// Connection string or pool.
    pub connection: PostgresConnection,
    /// Whether to use transaction-scoped locks.
    pub use_transaction: bool,
    /// Keepalive cadence for long-held locks.
    pub keepalive_cadence: Option<Duration>,
}

/// PostgreSQL connection source.
#[derive(Debug, Clone)]
pub enum PostgresConnection {
    /// Connection string - library manages pooling.
    ConnectionString(String),
    /// External connection pool.
    Pool(deadpool_postgres::Pool),
}
```

### Redis

```rust
/// Configuration for Redis distributed locks.
#[derive(Debug, Clone)]
pub struct RedisLockConfig {
    /// Redis connection(s). Multiple for RedLock algorithm.
    pub connections: Vec<RedisConnection>,
    /// Lock expiry time (default: 30s).
    pub expiry: Duration,
    /// Extension cadence (default: expiry / 3).
    pub extension_cadence: Duration,
    /// Minimum validity time after acquire (default: 90% of expiry).
    pub min_validity: Duration,
    /// Sleep range between busy-wait retries.
    pub busy_wait_sleep: (Duration, Duration),
}

impl Default for RedisLockConfig {
    fn default() -> Self {
        Self {
            connections: vec![],
            expiry: Duration::from_secs(30),
            extension_cadence: Duration::from_secs(10),
            min_validity: Duration::from_millis(27000), // 90% of 30s
            busy_wait_sleep: (Duration::from_millis(10), Duration::from_millis(200)),
        }
    }
}

/// Redis connection source.
#[derive(Debug, Clone)]
pub enum RedisConnection {
    /// Connection URL.
    Url(String),
    /// Existing client.
    Client(fred::prelude::RedisClient),
}

/// Internal state for a Redis lock.
pub(crate) struct RedisLockState {
    /// Unique lock identifier (random value).
    pub lock_id: String,
    /// Keys held on each database.
    pub held_databases: Vec<bool>,
    /// Lease expiry timestamp.
    pub lease_expires_at: Instant,
}
```

### File System

```rust
/// Configuration for file-based distributed locks.
#[derive(Debug, Clone)]
pub struct FileLockConfig {
    /// Directory for lock files.
    pub directory: PathBuf,
}

/// Internal state for a file lock.
pub(crate) struct FileLockState {
    /// The locked file handle.
    pub file: std::fs::File,
    /// Path to the lock file.
    pub path: PathBuf,
}
```

---

## Trait Definitions

### Core Lock Trait

```rust
use std::future::Future;
use std::time::Duration;

/// A distributed mutual exclusion lock.
/// 
/// Implementations provide backend-specific locking mechanisms
/// (PostgreSQL, Redis, file system, etc.) with a consistent API.
pub trait DistributedLock: Send + Sync {
    /// The handle type returned when lock is acquired.
    type Handle: LockHandle + Send;
    
    /// Returns the unique name identifying this lock.
    fn name(&self) -> &str;
    
    /// Acquires the lock, waiting up to `timeout`.
    /// 
    /// - `timeout = None` - Wait indefinitely
    /// - `timeout = Some(d)` - Wait up to duration `d`
    /// 
    /// Returns `Err(LockError::Timeout)` if timeout expires.
    fn acquire(
        &self, 
        timeout: Option<Duration>,
    ) -> impl Future<Output = LockResult<Self::Handle>> + Send;
    
    /// Attempts to acquire the lock without waiting.
    /// 
    /// Returns `Ok(Some(handle))` if acquired, `Ok(None)` if unavailable.
    fn try_acquire(
        &self,
    ) -> impl Future<Output = LockResult<Option<Self::Handle>>> + Send;
}

/// Handle to a held lock.
pub trait LockHandle: Send + Sync {
    /// Returns a receiver that signals if the lock was lost.
    /// 
    /// Check `lost_token().has_changed()` or await changes.
    /// Not all backends support this; unsupported returns receiver that never fires.
    fn lost_token(&self) -> &tokio::sync::watch::Receiver<bool>;
    
    /// Explicitly releases the lock.
    /// 
    /// This is also called automatically on drop, but async release
    /// allows proper error handling.
    fn release(self) -> impl Future<Output = LockResult<()>> + Send;
}
```

### Reader-Writer Lock Trait

```rust
/// A distributed reader-writer lock.
/// 
/// Allows multiple concurrent readers OR a single exclusive writer.
pub trait DistributedReaderWriterLock: Send + Sync {
    /// Handle type for read locks.
    type ReadHandle: LockHandle + Send;
    /// Handle type for write locks.
    type WriteHandle: LockHandle + Send;
    
    /// Returns the unique name identifying this lock.
    fn name(&self) -> &str;
    
    /// Acquires a read (shared) lock.
    fn acquire_read(
        &self,
        timeout: Option<Duration>,
    ) -> impl Future<Output = LockResult<Self::ReadHandle>> + Send;
    
    /// Attempts to acquire a read lock without waiting.
    fn try_acquire_read(
        &self,
    ) -> impl Future<Output = LockResult<Option<Self::ReadHandle>>> + Send;
    
    /// Acquires a write (exclusive) lock.
    fn acquire_write(
        &self,
        timeout: Option<Duration>,
    ) -> impl Future<Output = LockResult<Self::WriteHandle>> + Send;
    
    /// Attempts to acquire a write lock without waiting.
    fn try_acquire_write(
        &self,
    ) -> impl Future<Output = LockResult<Option<Self::WriteHandle>>> + Send;
}
```

### Semaphore Trait

```rust
/// A distributed counting semaphore.
/// 
/// Allows up to `max_count` concurrent holders.
pub trait DistributedSemaphore: Send + Sync {
    /// Handle type for semaphore tickets.
    type Handle: LockHandle + Send;
    
    /// Returns the unique name identifying this semaphore.
    fn name(&self) -> &str;
    
    /// Returns the maximum concurrent holders allowed.
    fn max_count(&self) -> u32;
    
    /// Acquires a semaphore ticket.
    fn acquire(
        &self,
        timeout: Option<Duration>,
    ) -> impl Future<Output = LockResult<Self::Handle>> + Send;
    
    /// Attempts to acquire a ticket without waiting.
    fn try_acquire(
        &self,
    ) -> impl Future<Output = LockResult<Option<Self::Handle>>> + Send;
}
```

### Provider Trait

```rust
/// Factory for creating distributed locks by name.
/// 
/// Providers encapsulate backend configuration, allowing application
/// code to be backend-agnostic.
pub trait LockProvider: Send + Sync {
    /// The lock type created by this provider.
    type Lock: DistributedLock;
    
    /// Creates a lock with the given name.
    fn create_lock(&self, name: &str) -> Self::Lock;
}

/// Factory for creating reader-writer locks by name.
pub trait ReaderWriterLockProvider: Send + Sync {
    /// The lock type created by this provider.
    type Lock: DistributedReaderWriterLock;
    
    /// Creates a reader-writer lock with the given name.
    fn create_reader_writer_lock(&self, name: &str) -> Self::Lock;
}

/// Factory for creating semaphores by name.
pub trait SemaphoreProvider: Send + Sync {
    /// The semaphore type created by this provider.
    type Semaphore: DistributedSemaphore;
    
    /// Creates a semaphore with the given name and max count.
    fn create_semaphore(&self, name: &str, max_count: u32) -> Self::Semaphore;
}
```

---

## State Transitions

### Lock Lifecycle

```
┌─────────────┐
│   Created   │
└──────┬──────┘
       │ acquire() / try_acquire()
       ▼
┌─────────────┐     timeout/cancel     ┌─────────────┐
│  Acquiring  │ ────────────────────▶  │   Failed    │
└──────┬──────┘                        └─────────────┘
       │ success
       ▼
┌─────────────┐     connection lost    ┌─────────────┐
│    Held     │ ────────────────────▶  │    Lost     │
└──────┬──────┘                        └─────────────┘
       │ drop() / release()
       ▼
┌─────────────┐
│  Released   │
└─────────────┘
```

### Handle Lost Detection

```
┌───────────────────────────────────────────────────────────────┐
│                      Lock Held State                          │
├───────────────────────────────────────────────────────────────┤
│                                                               │
│  ┌─────────────┐                      ┌─────────────────────┐ │
│  │  Monitor    │                      │  User Code          │ │
│  │  Task       │                      │                     │ │
│  │             │   watch channel      │  handle.lost_token()│ │
│  │  - Keepalive│ ──────────────────▶  │    .changed()       │ │
│  │  - Heartbeat│   (false → true)     │    .await           │ │
│  │  - Lease    │                      │                     │ │
│  └─────────────┘                      └─────────────────────┘ │
│                                                               │
└───────────────────────────────────────────────────────────────┘
```

---

## Validation Rules

### Lock Names

| Backend | Max Length | Allowed Characters | Escaping |
|---------|------------|-------------------|----------|
| Postgres | N/A (hashed to i64) | Any | SHA-1 hash if needed |
| Redis | Configurable | Any | Prefix added for namespace |
| File | OS-dependent (~255) | Alphanumeric, `-`, `_` | Hash invalid chars |

### Timeout Values

- Must be non-negative or None (infinite)
- Maximum value: `i32::MAX` milliseconds (~24.8 days)
- Zero timeout means "try once, don't wait"

### Semaphore Max Count

- Must be >= 1
- Maximum value: Backend-dependent (typically u32::MAX for Redis, practical limits for DB)
