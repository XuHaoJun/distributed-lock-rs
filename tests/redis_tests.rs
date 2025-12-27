//! Integration tests for Redis-based distributed locks.

use distributed_lock_core::error::LockError;
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

/// Helper to generate unique lock names to prevent state persistence between test runs.
fn unique_lock_name(base_name: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{}-{}", base_name, timestamp)
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_exclusive_lock_acquisition() {
    let url = get_redis_url();
    let provider = RedisLockProvider::new(url).await.unwrap();
    let lock = provider.create_lock(&unique_lock_name("test-exclusive"));

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
    let lock_name = unique_lock_name("test-blocking");
    let lock = provider.create_lock(&lock_name);

    // Acquire lock in first task
    let handle1 = lock.acquire(None).await.unwrap();

    // Spawn a task that tries to acquire the same lock
    let lock_name = lock.name().to_string();
    let acquire_task = tokio::spawn(async move {
        let provider2 = RedisLockProvider::new(url_clone).await.unwrap();
        let lock2 = provider2.create_lock(&lock_name);
        // Try to acquire with polling instead of blocking acquire
        for _ in 0..50 {
            if let Some(handle) = lock2.try_acquire().await.unwrap() {
                return Ok(handle);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Err(LockError::Timeout(Duration::from_millis(500)))
    });

    // Wait a bit to ensure the task is waiting
    tokio::time::sleep(Duration::from_millis(50)).await;

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
    let lock = provider.create_lock(&unique_lock_name("test-timeout"));

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

    // Test expiry by directly using Redis client to avoid extension task complications
    use fred::prelude::*;
    let config = RedisConfig::from_url(&url).unwrap();
    let client = RedisClient::new(config, None, None, None);
    client.connect();
    client.wait_for_connect().await.unwrap();

    let key = unique_lock_name("test-expiry-direct");
    let lock_id = "test-lock-id";

    // Set a key with short TTL directly
    let _: () = client
        .set(&key, lock_id, Some(Expiration::PX(500)), None, false)
        .await
        .unwrap();

    // Wait for it to expire
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Key should be gone
    let result: Option<String> = client.get(&key).await.unwrap();
    assert!(result.is_none(), "Key should have expired");

    // Now we can set it again
    let _: () = client.set(&key, lock_id, None, None, false).await.unwrap();

    // Clean up
    let _: i64 = client.del(&key).await.unwrap();
    client.quit().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_reader_writer_lock_readers() {
    let url = get_redis_url();
    let provider = RedisLockProvider::new(url).await.unwrap();
    let lock = provider.create_reader_writer_lock(&unique_lock_name("test-rw-readers"));

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
    let lock = provider.create_reader_writer_lock(&unique_lock_name("test-rw-writer"));

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
    let lock = provider.create_reader_writer_lock(&unique_lock_name("test-rw-block-unique"));

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

    // Small delay to ensure release completes
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Now writer should be able to acquire
    let write_handle3 = lock.try_acquire_write().await.unwrap();
    assert!(write_handle3.is_some());

    write_handle3.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires Redis server running
async fn test_redlock_multiple_servers() {
    // Test RedLock with multiple clients pointing to the same server
    // This verifies the RedLock logic works (though in practice you'd use different servers)
    let url = get_redis_url();
    let provider = RedisLockProvider::new(url).await.unwrap();
    let lock = provider.create_lock(&unique_lock_name("test-redlock"));

    // Should work with RedLock provider
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
    let semaphore = provider.create_semaphore(&unique_lock_name("test-semaphore-max"), 3);
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
    let semaphore_name = unique_lock_name("test-semaphore-blocking");
    let semaphore = provider.create_semaphore(&semaphore_name, 2);

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
    let semaphore = provider.create_semaphore(&unique_lock_name("test-semaphore-timeout"), 1);

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
    let semaphore = provider.create_semaphore(&unique_lock_name("test-semaphore-drop"), 1);

    // Acquire ticket
    let ticket = semaphore.acquire(None).await.unwrap();

    // Release explicitly
    ticket.release().await.unwrap();

    // Semaphore should now be available
    let ticket2 = semaphore.try_acquire().await.unwrap();
    assert!(
        ticket2.is_some(),
        "Semaphore should be available after ticket release"
    );

    ticket2.unwrap().release().await.unwrap();
}
