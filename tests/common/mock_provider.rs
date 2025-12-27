//! Mock provider for testing provider abstraction.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)] // Allow std::sync::Mutex and std::env::temp_dir for test mocks

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::{DistributedLock, LockHandle, LockProvider};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

/// Mock lock handle for testing.
#[allow(dead_code)] // Fields used for trait implementation
pub struct MockLockHandle {
    name: String,
    held: Arc<Mutex<bool>>,
    lost_receiver: watch::Receiver<bool>,
}

impl LockHandle for MockLockHandle {
    fn lost_token(&self) -> &watch::Receiver<bool> {
        &self.lost_receiver
    }

    async fn release(self) -> LockResult<()> {
        // Release the lock
        let mut held = self.held.lock().unwrap();
        *held = false;
        Ok(())
    }
}

/// Mock distributed lock for testing.
#[allow(dead_code)] // Fields used for trait implementation
pub struct MockDistributedLock {
    name: String,
    held: Arc<Mutex<bool>>,
    lost_sender: Arc<watch::Sender<bool>>,
}

impl DistributedLock for MockDistributedLock {
    type Handle = MockLockHandle;

    fn name(&self) -> &str {
        &self.name
    }

    async fn acquire(&self, timeout: Option<Duration>) -> LockResult<Self::Handle> {
        // Check if lock is already held
        let was_held = {
            let held = self.held.lock().unwrap();
            *held
        };

        if was_held {
            if let Some(timeout) = timeout {
                tokio::time::sleep(timeout).await;
                // Check again after timeout
                let held = self.held.lock().unwrap();
                if *held {
                    return Err(LockError::Timeout(timeout));
                }
            } else {
                // Wait indefinitely (for testing, we'll just return an error)
                return Err(LockError::Backend(Box::new(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "lock held",
                ))));
            }
        }

        // Acquire the lock
        {
            let mut held = self.held.lock().unwrap();
            *held = true;
        }

        let (_sender, receiver) = watch::channel(false);
        Ok(MockLockHandle {
            name: self.name.clone(),
            held: self.held.clone(),
            lost_receiver: receiver,
        })
    }

    async fn try_acquire(&self) -> LockResult<Option<Self::Handle>> {
        let mut held = self.held.lock().unwrap();
        if *held {
            return Ok(None);
        }

        *held = true;
        let (_sender, receiver) = watch::channel(false);
        Ok(Some(MockLockHandle {
            name: self.name.clone(),
            held: self.held.clone(),
            lost_receiver: receiver,
        }))
    }
}

/// Mock provider for testing provider abstraction.
pub struct MockLockProvider {
    locks: Arc<Mutex<std::collections::HashMap<String, Arc<Mutex<bool>>>>>,
}

impl MockLockProvider {
    /// Creates a new mock provider.
    pub fn new() -> Self {
        Self {
            locks: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }
}

impl Default for MockLockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LockProvider for MockLockProvider {
    type Lock = MockDistributedLock;

    fn create_lock(&self, name: &str) -> Self::Lock {
        let mut locks = self.locks.lock().unwrap();
        let held = locks
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(false)))
            .clone();

        let (sender, _receiver) = watch::channel(false);
        MockDistributedLock {
            name: name.to_string(),
            held,
            lost_sender: Arc::new(sender),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_provider_creates_locks() {
        let provider = MockLockProvider::new();
        let lock = provider.create_lock("test-lock");
        assert_eq!(lock.name(), "test-lock");
    }

    #[tokio::test]
    async fn test_mock_lock_try_acquire() {
        let provider = MockLockProvider::new();
        let lock = provider.create_lock("test-lock");

        // First acquisition should succeed
        let handle1 = lock.try_acquire().await.unwrap();
        assert!(handle1.is_some());

        // Second acquisition should fail
        let handle2 = lock.try_acquire().await.unwrap();
        assert!(handle2.is_none());
    }
}
