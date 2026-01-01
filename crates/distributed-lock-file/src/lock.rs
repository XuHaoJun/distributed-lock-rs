//! File-based distributed lock implementation.

use std::path::PathBuf;
use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::timeout::TimeoutValue;
use distributed_lock_core::traits::DistributedLock;
use tracing::{Span, instrument};

use crate::handle::FileLockHandle;
use crate::name::get_lock_file_name;

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
    pub fn from_path(path: impl Into<PathBuf>) -> Result<Self, LockError> {
        let path = path.into();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| LockError::InvalidName("invalid file path".to_string()))?
            .to_string();

        Ok(Self { path, name })
    }

    /// Creates a lock from a directory and name.
    pub(crate) fn new(directory: &std::path::Path, name: &str) -> LockResult<Self> {
        let path = get_lock_file_name(directory, name)?;
        Ok(Self {
            path,
            name: name.to_string(),
        })
    }

    /// Returns the path to the lock file.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Attempts to acquire the lock without waiting.
    ///
    /// Returns `Ok(Some(handle))` if acquired, `Ok(None)` if unavailable.
    async fn try_acquire_internal(&self) -> LockResult<Option<FileLockHandle>> {
        use fd_lock::RwLock;
        use std::fs::OpenOptions;
        use std::io::ErrorKind;

        const MAX_RETRIES: i32 = 1600;
        let mut retry_count = 0;

        loop {
            // Ensure parent directory exists
            if let Some(parent) = self.path.parent() {
                let mut dir_retry_count = 0;
                loop {
                    match std::fs::create_dir_all(parent) {
                        Ok(_) => break,
                        Err(e)
                            if dir_retry_count < MAX_RETRIES
                                && (e.kind() == ErrorKind::PermissionDenied
                                    || e.kind() == ErrorKind::AlreadyExists) =>
                        {
                            dir_retry_count += 1;
                            continue;
                        }
                        Err(e) => {
                            return Err(LockError::Connection(Box::new(std::io::Error::new(
                                e.kind(),
                                format!(
                                    "failed to ensure lock directory '{}' exists: {}",
                                    parent.display(),
                                    e
                                ),
                            ))));
                        }
                    }
                }
            }

            // Open or create the file
            // We DON'T use truncate(true) here to avoid race conditions where a waiting
            // process might truncate a held lock file.
            let file_result = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&self.path);

            let file = match file_result {
                Ok(f) => f,
                Err(e) => {
                    match e.kind() {
                        ErrorKind::PermissionDenied | ErrorKind::IsADirectory => {
                            // The path might be a directory
                            if self.path.is_dir() {
                                return Err(LockError::InvalidName(format!(
                                    "Failed to create lock file '{}' because it is already the name of a directory",
                                    self.path.display()
                                )));
                            }

                            // If it's a file but we got PermissionDenied, it might be a transient
                            // error during concurrent creation/deletion, or it might be read-only.
                            if retry_count < MAX_RETRIES && e.kind() == ErrorKind::PermissionDenied
                            {
                                retry_count += 1;
                                continue;
                            }

                            return Err(LockError::Connection(Box::new(std::io::Error::new(
                                e.kind(),
                                format!(
                                    "failed to open lock file '{}': {}",
                                    self.path.display(),
                                    e
                                ),
                            ))));
                        }
                        ErrorKind::NotFound => {
                            // Transient error during concurrent creation/deletion
                            if retry_count < MAX_RETRIES {
                                retry_count += 1;
                                continue;
                            }
                            return Err(LockError::Connection(Box::new(e)));
                        }
                        _ => return Err(LockError::Connection(Box::new(e))),
                    }
                }
            };

            let lock_file = RwLock::new(file);

            // Try to acquire the lock - the handle will do the actual acquisition
            match FileLockHandle::try_new(lock_file, self.path.clone()) {
                Ok(handle) => return Ok(Some(handle)),
                Err(LockError::Backend(e)) => {
                    // Check if this is a "lock already held" error
                    // Most OS lock errors will fall into this category when the lock is held
                    let error_msg = e.to_string().to_lowercase();
                    if error_msg.contains("already held")
                        || error_msg.contains("would block")
                        || error_msg.contains("resource temporarily unavailable")
                        || error_msg.contains("access is denied")
                    // Windows error for locked file
                    {
                        // Lock is held by another process - this is expected
                        return Ok(None);
                    } else {
                        // Unexpected backend error
                        return Err(LockError::Backend(e));
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl DistributedLock for FileDistributedLock {
    type Handle = FileLockHandle;

    fn name(&self) -> &str {
        &self.name
    }

    #[instrument(skip(self), fields(lock.name = %self.name, lock.path = %self.path.display(), timeout = ?timeout, backend = "file"))]
    async fn acquire(&self, timeout: Option<Duration>) -> LockResult<Self::Handle> {
        let timeout_value = TimeoutValue::from(timeout);
        let start = std::time::Instant::now();
        Span::current().record("operation", "acquire");

        // Busy-wait with exponential backoff and jitter
        // Initial delay: 10ms (reduced from 50ms for faster initial retry)
        let mut sleep_duration = Duration::from_millis(10);
        const MAX_SLEEP: Duration = Duration::from_secs(1);
        const MIN_SLEEP: Duration = Duration::from_millis(5);
        const BACKOFF_MULTIPLIER: u32 = 2;

        loop {
            match self.try_acquire_internal().await {
                Ok(Some(handle)) => {
                    let elapsed = start.elapsed();
                    Span::current().record("acquired", true);
                    Span::current().record("elapsed_ms", elapsed.as_millis() as u64);
                    return Ok(handle);
                }
                Ok(None) => {
                    // Check timeout before sleeping
                    if !timeout_value.is_infinite() {
                        let elapsed = start.elapsed();
                        let timeout_duration = timeout_value.as_duration().unwrap();
                        if elapsed >= timeout_duration {
                            Span::current().record("acquired", false);
                            Span::current().record("error", "timeout");
                            return Err(LockError::Timeout(timeout_duration));
                        }

                        // Don't sleep longer than remaining timeout
                        let remaining = timeout_duration - elapsed;
                        if sleep_duration > remaining {
                            sleep_duration = remaining;
                        }
                    }

                    // Add jitter (±25%) to avoid thundering herd problem
                    // This helps when multiple processes are waiting for the same lock
                    let jitter_range = sleep_duration.as_millis() as u64 / 4;
                    let jitter = if jitter_range > 0 {
                        // Use a simple hash of the current time for pseudo-random jitter
                        // This avoids needing a random number generator dependency
                        let nanos = start.elapsed().as_nanos() as u64;
                        (nanos % (jitter_range * 2)).saturating_sub(jitter_range)
                    } else {
                        0
                    };

                    let sleep_with_jitter = sleep_duration
                        .checked_add(Duration::from_millis(jitter))
                        .unwrap_or(sleep_duration);

                    // Sleep before retry
                    tokio::time::sleep(sleep_with_jitter).await;

                    // Exponential backoff: double the sleep duration, but cap at MAX_SLEEP
                    sleep_duration = (sleep_duration * BACKOFF_MULTIPLIER)
                        .min(MAX_SLEEP)
                        .max(MIN_SLEEP);
                }
                Err(e) => return Err(e),
            }
        }
    }

    #[instrument(skip(self), fields(lock.name = %self.name, lock.path = %self.path.display(), backend = "file"))]
    async fn try_acquire(&self) -> LockResult<Option<Self::Handle>> {
        Span::current().record("operation", "try_acquire");
        let result = self.try_acquire_internal().await;
        match &result {
            Ok(Some(_)) => {
                Span::current().record("acquired", true);
            }
            Ok(None) => {
                Span::current().record("acquired", false);
                Span::current().record("reason", "lock_held");
            }
            Err(e) => {
                Span::current().record("acquired", false);
                Span::current().record("error", e.to_string());
            }
        }
        result
    }
}
