//! Core traits for distributed locks.

use std::future::Future;
use std::time::Duration;

use crate::error::LockResult;

// ============================================================================
// Lock Handle Trait
// ============================================================================

/// Handle to a held distributed lock.
///
/// Dropping this handle releases the lock. For proper error handling in async
/// contexts, call `release()` explicitly.
///
/// # Example
///
/// ```rust,ignore
/// let handle = lock.acquire(None).await?;
/// // Critical section - we hold the lock
/// do_work().await;
/// // Explicit release with error handling
/// handle.release().await?;
/// ```
pub trait LockHandle: Send + Sync + Sized {
    /// Returns a receiver that signals when the lock is lost.
    ///
    /// The receiver yields `true` when the lock is lost (e.g., connection died).
    /// Not all backends support this; unsupported backends return a receiver
    /// that never changes from `false`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// tokio::select! {
    ///     _ = handle.lost_token().changed() => {
    ///         eprintln!("Lock was lost!");
    ///     }
    ///     _ = do_work() => {
    ///         // Work completed while still holding lock
    ///     }
    /// }
    /// ```
    fn lost_token(&self) -> &tokio::sync::watch::Receiver<bool>;

    /// Explicitly releases the lock.
    ///
    /// This is also called automatically on drop, but the async version
    /// allows proper error handling.
    fn release(self) -> impl Future<Output = LockResult<()>> + Send;
}

// ============================================================================
// Distributed Lock Trait
// ============================================================================

/// A distributed mutual exclusion lock.
///
/// Provides exclusive access to a resource identified by `name` across
/// processes and machines. The specific backend (PostgreSQL, Redis, file
/// system, etc.) determines how the lock is implemented.
///
/// # Example
///
/// ```rust,ignore
/// use distributed_lock_core::DistributedLock;
///
/// async fn protected_operation(lock: &impl DistributedLock) -> Result<(), Error> {
///     // Acquire with 5 second timeout
///     let handle = lock.acquire(Some(Duration::from_secs(5))).await?;
///     
///     // We have exclusive access
///     perform_critical_section().await?;
///     
///     // Release (also happens on drop)
///     handle.release().await?;
///     Ok(())
/// }
/// ```
pub trait DistributedLock: Send + Sync {
    /// The handle type returned when the lock is acquired.
    type Handle: LockHandle + Send;

    /// Returns the unique name identifying this lock.
    fn name(&self) -> &str;

    /// Acquires the lock, waiting up to `timeout`.
    ///
    /// # Arguments
    ///
    /// * `timeout` - Maximum time to wait. `None` means wait indefinitely.
    ///
    /// # Returns
    ///
    /// * `Ok(handle)` - Lock acquired successfully
    /// * `Err(LockError::Timeout)` - Timeout expired before lock acquired
    /// * `Err(LockError::Cancelled)` - Operation was cancelled
    /// * `Err(LockError::Connection)` - Backend connection failed
    ///
    /// # Cancellation
    ///
    /// This operation can be cancelled by dropping the returned future or using
    /// `tokio::select!` with a cancellation branch. Backends should check for
    /// cancellation periodically during the wait.
    fn acquire(
        &self,
        timeout: Option<Duration>,
    ) -> impl Future<Output = LockResult<Self::Handle>> + Send;

    /// Attempts to acquire the lock without waiting.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(handle))` - Lock acquired successfully
    /// * `Ok(None)` - Lock is held by another process
    /// * `Err(...)` - Error occurred during attempt
    fn try_acquire(&self) -> impl Future<Output = LockResult<Option<Self::Handle>>> + Send;
}

// ============================================================================
// Reader-Writer Lock Trait
// ============================================================================

/// A distributed reader-writer lock.
///
/// Allows multiple concurrent readers OR a single exclusive writer.
/// Writers are given priority to prevent starvation.
///
/// # Example
///
/// ```rust,ignore
/// // Multiple readers can hold the lock simultaneously
/// let read_handle = lock.acquire_read(None).await?;
/// let data = read_shared_resource().await;
/// read_handle.release().await?;
///
/// // Writers get exclusive access
/// let write_handle = lock.acquire_write(None).await?;
/// modify_shared_resource().await;
/// write_handle.release().await?;
/// ```
pub trait DistributedReaderWriterLock: Send + Sync {
    /// Handle type for read (shared) locks.
    type ReadHandle: LockHandle + Send;
    /// Handle type for write (exclusive) locks.
    type WriteHandle: LockHandle + Send;

    /// Returns the unique name identifying this lock.
    fn name(&self) -> &str;

    /// Acquires a read (shared) lock.
    ///
    /// Multiple readers can hold the lock concurrently.
    /// Blocks if a writer holds or is waiting for the lock.
    fn acquire_read(
        &self,
        timeout: Option<Duration>,
    ) -> impl Future<Output = LockResult<Self::ReadHandle>> + Send;

    /// Attempts to acquire a read lock without waiting.
    fn try_acquire_read(&self)
    -> impl Future<Output = LockResult<Option<Self::ReadHandle>>> + Send;

    /// Acquires a write (exclusive) lock.
    ///
    /// Only one writer can hold the lock. Blocks all readers.
    fn acquire_write(
        &self,
        timeout: Option<Duration>,
    ) -> impl Future<Output = LockResult<Self::WriteHandle>> + Send;

    /// Attempts to acquire a write lock without waiting.
    fn try_acquire_write(
        &self,
    ) -> impl Future<Output = LockResult<Option<Self::WriteHandle>>> + Send;
}

// ============================================================================
// Semaphore Trait
// ============================================================================

/// A distributed counting semaphore.
///
/// Allows up to `max_count` processes to hold the semaphore concurrently.
/// Useful for rate limiting or resource pooling.
///
/// # Example
///
/// ```rust,ignore
/// // Create a semaphore allowing 5 concurrent database connections
/// let semaphore = provider.create_semaphore("db-pool", 5);
///
/// // Acquire a "ticket"
/// let ticket = semaphore.acquire(None).await?;
///
/// // Use the limited resource
/// use_database_connection().await;
///
/// // Release the ticket
/// ticket.release().await?;
/// ```
pub trait DistributedSemaphore: Send + Sync {
    /// Handle type for semaphore tickets.
    type Handle: LockHandle + Send;

    /// Returns the unique name identifying this semaphore.
    fn name(&self) -> &str;

    /// Returns the maximum number of concurrent holders.
    fn max_count(&self) -> u32;

    /// Acquires a semaphore ticket.
    ///
    /// Blocks if `max_count` tickets are already held.
    fn acquire(
        &self,
        timeout: Option<Duration>,
    ) -> impl Future<Output = LockResult<Self::Handle>> + Send;

    /// Attempts to acquire a ticket without waiting.
    fn try_acquire(&self) -> impl Future<Output = LockResult<Option<Self::Handle>>> + Send;
}

// ============================================================================
// Provider Traits
// ============================================================================

/// Factory for creating distributed locks by name.
///
/// Providers encapsulate backend configuration, allowing application code
/// to be backend-agnostic.
///
/// # Example
///
/// ```rust,ignore
/// // Configure once at startup
/// let provider = PostgresLockProvider::new(connection_string);
///
/// // Create locks by name anywhere in the application
/// let lock = provider.create_lock("my-resource");
/// let handle = lock.acquire(None).await?;
/// ```
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

// ============================================================================
// Convenience Extensions
// ============================================================================

/// Extension trait providing convenience methods for lock providers.
pub trait LockProviderExt: LockProvider {
    /// Acquires a lock by name, returning the handle.
    ///
    /// Convenience method combining `create_lock` and `acquire`.
    fn acquire_lock(
        &self,
        name: &str,
        timeout: Option<Duration>,
    ) -> impl Future<Output = LockResult<<Self::Lock as DistributedLock>::Handle>> + Send
    where
        Self: Sync,
    {
        async move {
            let lock = self.create_lock(name);
            lock.acquire(timeout).await
        }
    }

    /// Tries to acquire a lock by name.
    ///
    /// Convenience method combining `create_lock` and `try_acquire`.
    fn try_acquire_lock(
        &self,
        name: &str,
    ) -> impl Future<Output = LockResult<Option<<Self::Lock as DistributedLock>::Handle>>> + Send
    where
        Self: Sync,
    {
        async move {
            let lock = self.create_lock(name);
            lock.try_acquire().await
        }
    }
}

// Blanket implementation for all LockProviders
impl<T: LockProvider> LockProviderExt for T {}
