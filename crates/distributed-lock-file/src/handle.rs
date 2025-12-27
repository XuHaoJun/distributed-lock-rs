//! File lock handle implementation.

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::instrument;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::LockHandle;

use fd_lock::{RwLock, RwLockWriteGuard};

/// Handle for a held file lock.
///
/// Dropping this handle releases the lock and deletes the lock file.
pub struct FileLockHandle {
    /// The locked file guard (released on drop).
    /// We use a custom wrapper to manage the lifetime.
    /// Wrapped in ManuallyDrop so we can move it out in release().
    inner: std::mem::ManuallyDrop<LockGuard>,
    /// Path to the lock file (for cleanup).
    path: PathBuf,
    /// Watch channel for lock lost detection (not supported for file locks).
    #[allow(dead_code)]
    lost_sender: Arc<watch::Sender<bool>>,
    lost_receiver: watch::Receiver<bool>,
}

/// Internal wrapper that holds both the RwLock and its guard.
/// This allows us to keep the guard alive while still being able to move the handle.
struct LockGuard {
    _lock_file: Arc<RwLock<std::fs::File>>,
    _guard: std::mem::ManuallyDrop<RwLockWriteGuard<'static, std::fs::File>>,
}

unsafe impl Send for LockGuard {}
unsafe impl Sync for LockGuard {}

impl LockGuard {
    fn new(lock_file: RwLock<std::fs::File>) -> LockResult<Self> {
        // Wrap in Arc first
        let lock_file_arc = Arc::new(lock_file);

        // Now acquire the lock guard by getting a mutable reference to the Arc's inner RwLock
        // This is safe because:
        // 1. We just created the Arc, so we're the only owner
        // 2. We'll keep the Arc alive for the lifetime of the guard
        let guard = unsafe {
            // Get a raw pointer to the RwLock inside the Arc
            let rwlock_ptr = Arc::as_ptr(&lock_file_arc) as *const RwLock<std::fs::File>
                as *mut RwLock<std::fs::File>;
            // Try to acquire the lock
            (*rwlock_ptr).try_write()
        }
        .map_err(|_| {
            LockError::Backend(Box::new(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "lock already held",
            )))
        })?;

        // Extend the guard's lifetime to 'static using unsafe
        // This is safe because:
        // 1. The guard borrows from lock_file_arc
        // 2. We keep lock_file_arc alive for the entire lifetime of LockGuard
        // 3. LockGuard is dropped before lock_file_arc, so the guard is dropped first
        let guard_box = Box::new(guard);
        let guard_ptr = Box::into_raw(guard_box) as *mut RwLockWriteGuard<'static, std::fs::File>;
        let guard = unsafe { *Box::from_raw(guard_ptr) };
        let guard = std::mem::ManuallyDrop::new(guard);

        Ok(Self {
            _lock_file: lock_file_arc,
            _guard: guard,
        })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Manually drop the guard to release the lock
        unsafe {
            std::mem::ManuallyDrop::drop(&mut self._guard);
        }
    }
}

impl FileLockHandle {
    /// Creates a new file lock handle by acquiring the lock.
    pub(crate) fn try_new(lock_file: RwLock<std::fs::File>, path: PathBuf) -> LockResult<Self> {
        let inner = LockGuard::new(lock_file)?;
        let (sender, receiver) = watch::channel(false);
        Ok(Self {
            inner: std::mem::ManuallyDrop::new(inner),
            path,
            lost_sender: Arc::new(sender),
            lost_receiver: receiver,
        })
    }
}

impl LockHandle for FileLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

    #[instrument(skip(self), fields(lock.path = %self.path.display(), backend = "file"))]
    async fn release(self) -> LockResult<()> {
        // Release the lock by manually dropping the inner guard
        // We need to extract the path before dropping self
        let path = self.path.clone();
        unsafe {
            std::mem::ManuallyDrop::drop(&mut std::mem::ManuallyDrop::new(self).inner);
        }

        // Try to delete the lock file (best effort)
        let _ = std::fs::remove_file(&path);

        Ok(())
    }
}

impl Drop for FileLockHandle {
    fn drop(&mut self) {
        // Release the lock by manually dropping the inner guard
        unsafe {
            std::mem::ManuallyDrop::drop(&mut self.inner);
        }

        // Try to delete the lock file (best effort)
        let _ = std::fs::remove_file(&self.path);
    }
}
