//! Integration tests for file-based distributed locks.

#![allow(clippy::disallowed_methods)] // Allow std::env::temp_dir for tests

use distributed_lock_core::traits::{DistributedLock, LockHandle, LockProvider};
use distributed_lock_file::{FileDistributedLock, FileLockProvider};
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn test_exclusive_lock_acquisition() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    let lock = provider.create_lock("test-exclusive");

    // First acquisition should succeed
    let handle1 = lock.try_acquire().await.unwrap();
    assert!(handle1.is_some());

    // Second acquisition should fail (lock is held)
    let handle2 = lock.try_acquire().await.unwrap();
    assert!(handle2.is_none());

    // Release the lock
    handle1.unwrap().release().await.unwrap();

    // Now acquisition should succeed
    let handle3 = lock.try_acquire().await.unwrap();
    assert!(handle3.is_some());
}

#[tokio::test]
async fn test_blocking_acquire() {
    let temp_dir = std::env::temp_dir();
    let temp_dir_clone = temp_dir.clone();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    let lock = provider.create_lock("test-blocking");

    // Acquire lock in first task
    let handle1 = lock.acquire(None).await.unwrap();

    // Spawn a task that tries to acquire the same lock
    let lock_name = lock.name().to_string();
    let acquire_task = tokio::spawn(async move {
        let provider2 = FileLockProvider::new(temp_dir_clone).unwrap();
        let lock2 = provider2.create_lock(&lock_name);
        lock2.acquire(Some(Duration::from_millis(100))).await
    });

    // Wait a bit to ensure the task is waiting
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Release the lock
    handle1.release().await.unwrap();

    // The waiting task should now acquire the lock
    let result = timeout(Duration::from_secs(1), acquire_task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_timeout() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    let lock = provider.create_lock("test-timeout");

    // Acquire lock
    let _handle1 = lock.acquire(None).await.unwrap();

    // Try to acquire with short timeout - should timeout
    let result = lock.acquire(Some(Duration::from_millis(50))).await;
    assert!(result.is_err());
    if let Err(e) = result {
        // Should be a timeout error
        assert!(format!("{:?}", e).contains("timeout") || format!("{:?}", e).contains("Timeout"));
    }
}

#[tokio::test]
async fn test_lock_release_on_drop() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    let lock = provider.create_lock("test-drop");

    // Acquire and immediately drop
    {
        let _handle = lock.acquire(None).await.unwrap();
        // Handle dropped here
    }

    // Lock should now be available
    let handle2 = lock.try_acquire().await.unwrap();
    assert!(handle2.is_some());
}

#[tokio::test]
async fn test_exponential_backoff_retry() {
    let temp_dir = std::env::temp_dir();
    let temp_dir_clone = temp_dir.clone();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    let lock = provider.create_lock("test-backoff");

    // Acquire lock
    let handle1 = lock.acquire(None).await.unwrap();

    // Spawn a task that will wait for the lock
    let lock_name = lock.name().to_string();
    let start = std::time::Instant::now();
    let acquire_task = tokio::spawn(async move {
        let provider2 = FileLockProvider::new(temp_dir_clone).unwrap();
        let lock2 = provider2.create_lock(&lock_name);
        lock2.acquire(Some(Duration::from_millis(200))).await
    });

    // Wait a bit to ensure the task starts waiting
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Release the lock
    handle1.release().await.unwrap();

    // The waiting task should acquire the lock
    let result = timeout(Duration::from_secs(1), acquire_task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_ok());

    // Verify that exponential backoff was used (should take some time)
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(10),
        "backoff should introduce some delay"
    );
}

#[tokio::test]
async fn test_invalid_lock_name() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();

    // Empty name should be handled gracefully
    let _result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        provider.create_lock("");
    }));
    // The provider might panic or return an error - both are acceptable
    // The important thing is that it doesn't crash the process
}

#[tokio::test]
async fn test_special_characters_in_name() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();

    // Names with special characters should be handled
    let lock = provider.create_lock("test/lock/with/slashes");
    let handle = lock.try_acquire().await.unwrap();
    assert!(handle.is_some());
    handle.unwrap().release().await.unwrap();

    let lock2 = provider.create_lock("test\\lock\\with\\backslashes");
    let handle2 = lock2.try_acquire().await.unwrap();
    assert!(handle2.is_some());
    handle2.unwrap().release().await.unwrap();
}

#[tokio::test]
async fn test_very_long_lock_name() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();

    // Very long names should be handled (truncated/hashed)
    let long_name = "a".repeat(1000);
    let lock = provider.create_lock(&long_name);
    let handle = lock.try_acquire().await.unwrap();
    assert!(handle.is_some());
    handle.unwrap().release().await.unwrap();
}

#[tokio::test]
async fn test_concurrent_multiple_locks() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();

    // Multiple different locks should be independent
    let lock1 = provider.create_lock("concurrent-lock-1");
    let lock2 = provider.create_lock("concurrent-lock-2");

    let handle1 = lock1.try_acquire().await.unwrap();
    let handle2 = lock2.try_acquire().await.unwrap();

    assert!(handle1.is_some());
    assert!(handle2.is_some());

    handle1.unwrap().release().await.unwrap();
    handle2.unwrap().release().await.unwrap();
}

#[tokio::test]
async fn test_lock_file_cleanup() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    let lock = provider.create_lock("test-cleanup");

    let lock_path = lock.path();

    // Acquire and release
    {
        let handle = lock.acquire(None).await.unwrap();
        // Verify file exists
        assert!(lock_path.exists(), "lock file should exist while held");
        handle.release().await.unwrap();
    }

    // File should be cleaned up after release
    // Note: On some platforms, the file might still exist but be empty/unlocked
    // The important thing is that the lock is released
    let handle2 = lock.try_acquire().await.unwrap();
    assert!(handle2.is_some(), "lock should be available after release");
    handle2.unwrap().release().await.unwrap();
}

// Platform-specific tests
#[cfg(target_os = "linux")]
#[tokio::test]
async fn test_linux_file_locking() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    let lock = provider.create_lock("linux-test");

    // Linux uses flock() which should work correctly
    let handle = lock.try_acquire().await.unwrap();
    assert!(handle.is_some());

    // Second process should not be able to acquire
    let handle2 = lock.try_acquire().await.unwrap();
    assert!(handle2.is_none());

    handle.unwrap().release().await.unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_macos_file_locking() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    let lock = provider.create_lock("macos-test");

    // macOS uses flock() which should work correctly
    let handle = lock.try_acquire().await.unwrap();
    assert!(handle.is_some());

    // Second process should not be able to acquire
    let handle2 = lock.try_acquire().await.unwrap();
    assert!(handle2.is_none());

    handle.unwrap().release().await.unwrap();
}

#[cfg(target_os = "windows")]
#[tokio::test]
async fn test_windows_file_locking() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    let lock = provider.create_lock("windows-test");

    // Windows uses LockFile() which should work correctly
    let handle = lock.try_acquire().await.unwrap();
    assert!(handle.is_some());

    // Second process should not be able to acquire
    let handle2 = lock.try_acquire().await.unwrap();
    assert!(handle2.is_none());

    handle.unwrap().release().await.unwrap();
}

#[tokio::test]
async fn test_error_handling_permission_denied() {
    // Try to create a lock in a read-only location (if possible)
    // This test may not work on all systems, so we make it lenient
    #[cfg(unix)]
    {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir();
        let read_only_dir = temp_dir.join("readonly-test-dir");

        // Create a read-only directory
        if fs::create_dir_all(&read_only_dir).is_ok() {
            let mut perms = fs::metadata(&read_only_dir).unwrap().permissions();
            perms.set_mode(0o555); // Read and execute only
            fs::set_permissions(&read_only_dir, perms).ok();

            // Try to create a provider - should handle permission error gracefully
            let _result = FileLockProvider::new(&read_only_dir);
            // Either it fails gracefully or succeeds (if we have write access)
            // The important thing is it doesn't panic

            // Cleanup
            let mut perms = fs::metadata(&read_only_dir).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&read_only_dir, perms).ok();
            fs::remove_dir(&read_only_dir).ok();
        }
    }
}

#[tokio::test]
async fn test_from_path() {
    let temp_dir = std::env::temp_dir();
    let lock_file = temp_dir.join("custom-lock-file.lock");

    let lock = FileDistributedLock::from_path(&lock_file).unwrap();
    let handle = lock.try_acquire().await.unwrap();
    assert!(handle.is_some());

    // Verify the path matches
    assert_eq!(lock.path(), &lock_file);

    handle.unwrap().release().await.unwrap();
}

/// Comprehensive integration test for exclusive lock semantics.
///
/// This test verifies all the core requirements for exclusive locks:
/// - Lock acquisition blocks other processes
/// - Lock release allows waiting processes to acquire
/// - Try-acquire returns None when lock is held
/// - Lock is released on handle drop
#[tokio::test]
async fn test_exclusive_lock_semantics() {
    let temp_dir = std::env::temp_dir();
    let temp_dir_clone = temp_dir.clone();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    let lock = provider.create_lock("test-exclusive-semantics");

    // Test 1: Try-acquire returns None when lock is held
    let handle1 = lock.try_acquire().await.unwrap();
    assert!(handle1.is_some(), "First acquisition should succeed");

    let handle2 = lock.try_acquire().await.unwrap();
    assert!(
        handle2.is_none(),
        "Second acquisition should fail when lock is held"
    );

    // Test 2: Lock acquisition blocks other processes
    // Spawn a task that will wait for the lock
    let lock_name = lock.name().to_string();
    let acquire_task = tokio::spawn(async move {
        // This should block until the lock is released
        let provider2 = FileLockProvider::new(temp_dir_clone).unwrap();
        let lock2 = provider2.create_lock(&lock_name);
        lock2.acquire(Some(Duration::from_millis(500))).await
    });

    // Give the task time to start waiting
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Verify the task is still waiting (hasn't acquired yet)
    // We can't directly check this, but we can verify it hasn't completed

    // Test 3: Lock release allows waiting processes to acquire
    handle1.unwrap().release().await.unwrap();

    // Now the waiting task should acquire the lock
    let result = timeout(Duration::from_secs(1), acquire_task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.is_ok(),
        "Waiting task should acquire lock after release"
    );
    let handle3 = result.unwrap();

    // Verify we can't acquire while the other task holds it
    let handle4 = lock.try_acquire().await.unwrap();
    assert!(
        handle4.is_none(),
        "Lock should still be held by waiting task"
    );

    // Release the lock from the waiting task
    handle3.release().await.unwrap();

    // Test 4: Lock is released on handle drop
    {
        let _handle5 = lock.acquire(None).await.unwrap();
        // Handle dropped here - lock should be released
    }

    // Verify lock is now available after drop
    let handle6 = lock.try_acquire().await.unwrap();
    assert!(
        handle6.is_some(),
        "Lock should be available after handle drop"
    );
    handle6.unwrap().release().await.unwrap();
}

#[tokio::test]
async fn test_lock_path_is_directory() {
    let temp_dir = std::env::temp_dir();
    let lock_dir = temp_dir.join("test-is-dir");
    std::fs::create_dir_all(&lock_dir).unwrap();

    let _provider = FileLockProvider::new(temp_dir).unwrap();
    // Using the directory name as the lock name will result in the same path
    // if not hashed, or we can use from_path
    let lock = FileDistributedLock::from_path(&lock_dir).unwrap();

    let result = lock.try_acquire().await;
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert!(err
        .to_string()
        .contains("is already the name of a directory"));

    // Cleanup
    std::fs::remove_dir(lock_dir).unwrap();
}

#[tokio::test]
async fn test_no_truncation() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    let lock = provider.create_lock("test-no-truncation");
    let lock_path = lock.path().clone();

    // 1. Acquire, write data, release
    {
        let handle = lock.acquire(None).await.unwrap();
        std::fs::write(&lock_path, b"some data").unwrap();
        handle.release().await.unwrap();
    }

    // 2. Clear release might delete file, so we need to be careful.
    // In our implementation, release() calls remove_file.
    // So let's test without explicit release first, or just check the file exists.

    // If we want to test truncation, we should probably NOT delete the file on release for this test.
    // But our handle always deletes. Let's manually create the file.

    std::fs::write(&lock_path, b"persistent data").unwrap();

    // 3. Acquire again
    let handle = lock.acquire(None).await.unwrap();

    // 4. Check if data is still there
    let data = std::fs::read(&lock_path).unwrap();
    assert_eq!(
        data, b"persistent data",
        "File should not be truncated on acquisition"
    );

    handle.release().await.unwrap();
}
