//! File system backend API contract
//!
//! Uses OS-level file locking for distributed synchronization.

use std::path::PathBuf;
use std::time::Duration;

// ============================================================================
// Configuration
// ============================================================================

/// Builder for file-based lock provider configuration.
///
/// # Example
///
/// ```rust,ignore
/// let provider = FileLockProvider::builder()
///     .directory("/var/lock/myapp")
///     .build()?;
///
/// let lock = provider.create_lock("my-resource");
/// let handle = lock.acquire(Some(Duration::from_secs(5))).await?;
/// ```
pub struct FileLockProviderBuilder {
    directory: Option<PathBuf>,
}

impl FileLockProviderBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Self { directory: None }
    }

    /// Sets the directory for lock files.
    ///
    /// The directory will be created if it doesn't exist.
    pub fn directory(mut self, path: impl Into<PathBuf>) -> Self {
        self.directory = Some(path.into());
        self
    }

    /// Builds the provider.
    ///
    /// # Errors
    ///
    /// Returns an error if no directory is specified or if the directory
    /// cannot be created.
    pub fn build(self) -> Result<FileLockProvider, LockError> {
        todo!()
    }
}

// ============================================================================
// Provider
// ============================================================================

/// Provider for file-based distributed locks.
///
/// Uses OS-level file locking to provide mutual exclusion across processes.
/// Works on any POSIX system (Linux, macOS) and Windows.
///
/// # Supported Primitives
///
/// - Mutex locks (`DistributedLock`) only
/// - Reader-writer locks and semaphores are not supported
///
/// # How It Works
///
/// 1. Create a lock file in the configured directory
/// 2. Acquire exclusive lock using `flock()` (POSIX) or `LockFileEx` (Windows)
/// 3. Lock is released when file handle is closed
/// 4. Lock file is deleted on release (cleanup)
///
/// # Limitations
///
/// - Only works across processes on the same machine or shared filesystem
/// - No timeout support at OS level; implemented via busy-wait polling
/// - Network filesystems (NFS) may have unreliable locking semantics
///
/// # Example
///
/// ```rust,ignore
/// let provider = FileLockProvider::builder()
///     .directory("/tmp/locks")
///     .build()?;
///
/// let lock = provider.create_lock("my-resource");
///
/// // Acquire with 5 second timeout
/// let handle = lock.acquire(Some(Duration::from_secs(5))).await?;
///
/// // Critical section
/// do_work().await;
///
/// // Release (file deleted)
/// handle.release().await?;
/// ```
pub struct FileLockProvider {
    directory: PathBuf,
}

impl FileLockProvider {
    /// Returns a new builder for configuring the provider.
    pub fn builder() -> FileLockProviderBuilder {
        FileLockProviderBuilder::new()
    }

    /// Creates a provider using the specified directory.
    ///
    /// Convenience method for simple use cases.
    pub fn new(directory: impl Into<PathBuf>) -> Result<Self, LockError> {
        Self::builder().directory(directory).build()
    }

    /// Returns the directory where lock files are stored.
    pub fn directory(&self) -> &PathBuf {
        &self.directory
    }
}

impl LockProvider for FileLockProvider {
    type Lock = FileDistributedLock;

    fn create_lock(&self, name: &str) -> Self::Lock {
        todo!()
    }
}

// Note: FileLockProvider does NOT implement ReaderWriterLockProvider or
// SemaphoreProvider because file locks don't support these semantics.

// ============================================================================
// Lock Types
// ============================================================================

/// A file-based distributed lock.
///
/// The lock is backed by a file in the provider's directory. The file name
/// is derived from the lock name with proper escaping for filesystem safety.
pub struct FileDistributedLock {
    /// Full path to the lock file.
    path: PathBuf,
    /// Original lock name.
    name: String,
}

impl FileDistributedLock {
    /// Creates a lock for a specific file path.
    ///
    /// Use this when you want to lock a specific file rather than using
    /// the provider's directory.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let lock = FileDistributedLock::from_path("/var/run/myapp.lock")?;
    /// let handle = lock.acquire(None).await?;
    /// ```
    pub fn from_path(path: impl Into<PathBuf>) -> Result<Self, LockError> {
        todo!()
    }

    /// Returns the path to the lock file.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

/// Handle for a held file lock.
///
/// Dropping this handle releases the lock and deletes the lock file.
pub struct FileLockHandle {
    // Internal fields: file handle, path for cleanup
}

// ============================================================================
// File Name Handling
// ============================================================================

/// Converts a lock name to a safe file name.
///
/// # Rules
///
/// - Names with only alphanumeric, `-`, and `_` are used as-is
/// - Other characters are escaped or the name is hashed
/// - Very long names are truncated with a hash suffix
///
/// # Example
///
/// ```rust,ignore
/// let safe = to_safe_filename("my-lock");     // "my-lock"
/// let safe = to_safe_filename("foo/bar");     // "foo_bar" or hash
/// let safe = to_safe_filename("very/long/name..."); // truncated + hash
/// ```
pub fn to_safe_filename(name: &str, max_length: usize) -> String {
    todo!()
}

// Placeholder imports
use super::core_traits::{DistributedLock, LockError, LockHandle, LockProvider, LockResult};
