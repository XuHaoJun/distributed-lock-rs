//! Integration tests for Redis-based distributed locks.

use distributed_lock_core::traits::{
    DistributedLock, DistributedReaderWriterLock, DistributedSemaphore, LockHandle, LockProvider,
    ReaderWriterLockProvider, SemaphoreProvider,
};
use distributed_lock_redis::RedisLockProvider;
use std::time::Duration;
use tokio::time::timeout;

/// Helper to get Redis URL from environment or use default.
fn get_redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string())
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_exclusive_lock_acquisition() {
    let url = get_redis_url();
    let provider = RedisLockProvider::new(url).await.unwrap();
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
#[ignore] // Requires Redis server running
async fn test_blocking_acquire() {
    let url = get_redis_url();
    let url_clone = url.clone();
    let provider = RedisLockProvider::new(url).await.unwrap();
    let lock = provider.create_lock("test-blocking");

    // Acquire lock in first task
    let handle1 = lock.acquire(None).await.unwrap();

    // Spawn a task that tries to acquire the same lock
    let lock_name = lock.name().to_string();
    let acquire_task = tokio::spawn(async move {
        let provider2 = RedisLockProvider::new(url_clone).await.unwrap();
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
#[ignore] // Requires Redis server running
async fn test_lock_timeout() {
    let url = get_redis_url();
    let provider = RedisLockProvider::new(url).await.unwrap();
    let lock = provider.create_lock("test-timeout");

    // Acquire lock
    let handle1 = lock.acquire(None).await.unwrap();

    // Try to acquire with short timeout - should fail
    let result = lock.acquire(Some(Duration::from_millis(50))).await;
    assert!(result.is_err());

    // Release the lock
    handle1.release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_lock_expiry() {
    let url = get_redis_url();
    let provider = RedisLockProvider::builder()
        .url(url)
        .expiry(Duration::from_millis(200))
        .build()
        .await
        .unwrap();
    let lock = provider.create_lock("test-expiry");

    // Acquire lock
    let _handle1 = lock.acquire(None).await.unwrap();

    // Wait for lock to expire (longer than expiry time)
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Lock should have expired, so we can acquire it
    let handle2 = lock.try_acquire().await.unwrap();
    assert!(handle2.is_some());

    // Clean up
    handle2.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_reader_writer_lock_readers() {
    let url = get_redis_url();
    let provider = RedisLockProvider::new(url).await.unwrap();
    let lock = provider.create_reader_writer_lock("test-rw-readers");

    // Multiple readers should be able to acquire simultaneously
    let handle1 = lock.try_acquire_read().await.unwrap();
    assert!(handle1.is_some());

    let handle2 = lock.try_acquire_read().await.unwrap();
    assert!(handle2.is_some());

    let handle3 = lock.try_acquire_read().await.unwrap();
    assert!(handle3.is_some());

    // Release all readers
    handle1.unwrap().release().await.unwrap();
    handle2.unwrap().release().await.unwrap();
    handle3.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_reader_writer_lock_writer_exclusive() {
    let url = get_redis_url();
    let provider = RedisLockProvider::new(url).await.unwrap();
    let lock = provider.create_reader_writer_lock("test-rw-writer");

    // Writer should be able to acquire
    let write_handle = lock.try_acquire_write().await.unwrap();
    assert!(write_handle.is_some());

    // While writer holds lock, readers should fail
    let read_handle = lock.try_acquire_read().await.unwrap();
    assert!(read_handle.is_none());

    // Another writer should also fail
    let write_handle2 = lock.try_acquire_write().await.unwrap();
    assert!(write_handle2.is_none());

    // Release writer
    write_handle.unwrap().release().await.unwrap();

    // Now readers should be able to acquire
    let read_handle2 = lock.try_acquire_read().await.unwrap();
    assert!(read_handle2.is_some());

    read_handle2.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_reader_writer_lock_readers_block_writer() {
    let url = get_redis_url();
    let provider = RedisLockProvider::new(url).await.unwrap();
    let lock = provider.create_reader_writer_lock("test-rw-block");

    // Acquire multiple readers
    let read_handle1 = lock.try_acquire_read().await.unwrap().unwrap();
    let read_handle2 = lock.try_acquire_read().await.unwrap().unwrap();

    // Writer should not be able to acquire while readers hold the lock
    let write_handle = lock.try_acquire_write().await.unwrap();
    assert!(write_handle.is_none());

    // Release one reader
    read_handle1.release().await.unwrap();

    // Writer still shouldn't be able to acquire
    let write_handle2 = lock.try_acquire_write().await.unwrap();
    assert!(write_handle2.is_none());

    // Release the other reader
    read_handle2.release().await.unwrap();

    // Now writer should be able to acquire
    let write_handle3 = lock.try_acquire_write().await.unwrap();
    assert!(write_handle3.is_some());

    write_handle3.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_redlock_multiple_servers() {
    // This test requires multiple Redis instances
    // For now, we'll test with a single server but verify the RedLock logic works
    let url = get_redis_url();
    let provider = RedisLockProvider::builder()
        .url(url.clone())
        .url(url.clone()) // Same server twice for testing
        .url(url)
        .build()
        .await
        .unwrap();
    let lock = provider.create_lock("test-redlock");

    // Should work with multiple servers configured
    let handle = lock.try_acquire().await.unwrap();
    assert!(handle.is_some());

    handle.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_semaphore_max_count() {
    let url = get_redis_url();
    let provider = RedisLockProvider::new(url).await.unwrap();

    // Create a semaphore with max_count=3
    let semaphore = provider.create_semaphore("test-semaphore-max", 3);
    assert_eq!(semaphore.max_count(), 3);

    // Should be able to acquire 3 tickets
    let ticket1 = semaphore.try_acquire().await.unwrap();
    assert!(ticket1.is_some());

    let ticket2 = semaphore.try_acquire().await.unwrap();
    assert!(ticket2.is_some());

    let ticket3 = semaphore.try_acquire().await.unwrap();
    assert!(ticket3.is_some());

    // 4th acquisition should fail (max_count reached)
    let ticket4 = semaphore.try_acquire().await.unwrap();
    assert!(
        ticket4.is_none(),
        "Should not acquire 4th ticket when max_count=3"
    );

    // Release one ticket
    ticket1.unwrap().release().await.unwrap();

    // Now should be able to acquire again
    let ticket5 = semaphore.try_acquire().await.unwrap();
    assert!(ticket5.is_some(), "Should acquire ticket after release");

    // Clean up
    ticket2.unwrap().release().await.unwrap();
    ticket3.unwrap().release().await.unwrap();
    ticket5.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_semaphore_blocking_acquire() {
    let url = get_redis_url();
    let url_clone = url.clone();
    let provider = RedisLockProvider::new(url).await.unwrap();

    // Create a semaphore with max_count=2
    let semaphore = provider.create_semaphore("test-semaphore-blocking", 2);

    // Acquire both tickets
    let ticket1 = semaphore.acquire(None).await.unwrap();
    let ticket2 = semaphore.acquire(None).await.unwrap();

    // Spawn a task that tries to acquire (should block)
    let semaphore_name = semaphore.name().to_string();
    let acquire_task = tokio::spawn(async move {
        let provider2 = RedisLockProvider::new(url_clone).await.unwrap();
        let semaphore2 = provider2.create_semaphore(&semaphore_name, 2);
        semaphore2.acquire(Some(Duration::from_millis(200))).await
    });

    // Give the task time to start waiting
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Release one ticket - the waiting task should acquire it
    ticket1.release().await.unwrap();

    // The waiting task should now acquire the lock
    let result = timeout(Duration::from_secs(1), acquire_task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.is_ok(),
        "Waiting task should acquire ticket after release"
    );

    // Clean up
    ticket2.release().await.unwrap();
    result.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_semaphore_timeout() {
    let url = get_redis_url();
    let provider = RedisLockProvider::new(url).await.unwrap();

    // Create a semaphore with max_count=1
    let semaphore = provider.create_semaphore("test-semaphore-timeout", 1);

    // Acquire the only ticket
    let ticket1 = semaphore.acquire(None).await.unwrap();

    // Try to acquire with short timeout - should timeout
    let result = semaphore.acquire(Some(Duration::from_millis(50))).await;
    assert!(result.is_err(), "Should timeout when semaphore is full");

    if let Err(e) = result {
        assert!(format!("{:?}", e).contains("timeout") || format!("{:?}", e).contains("Timeout"));
    }

    // Clean up
    ticket1.release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_semaphore_release_on_drop() {
    let url = get_redis_url();
    let provider = RedisLockProvider::new(url).await.unwrap();

    // Create a semaphore with max_count=1
    let semaphore = provider.create_semaphore("test-semaphore-drop", 1);

    // Acquire and immediately drop
    {
        let _ticket = semaphore.acquire(None).await.unwrap();
        // Ticket dropped here
    }

    // Semaphore should now be available
    let ticket2 = semaphore.try_acquire().await.unwrap();
    assert!(
        ticket2.is_some(),
        "Semaphore should be available after ticket drop"
    );

    ticket2.unwrap().release().await.unwrap();
}
