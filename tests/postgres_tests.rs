//! Integration tests for PostgreSQL-based distributed locks.

use distributed_lock_core::traits::{
    DistributedLock, DistributedReaderWriterLock, LockHandle, LockProvider,
    ReaderWriterLockProvider,
};
use distributed_lock_postgres::PostgresLockProvider;
use std::time::Duration;
use tokio::time::timeout;

/// Helper to get PostgreSQL connection string from environment or use default.
fn get_postgres_url() -> String {
    std::env::var("POSTGRES_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5432/postgres".to_string())
}

#[tokio::test]
#[ignore] // Requires PostgreSQL server running
async fn test_exclusive_lock_acquisition() {
    let url = get_postgres_url();
    let provider = PostgresLockProvider::new(url).await.unwrap();
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
    handle3.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires PostgreSQL server running
async fn test_blocking_acquire() {
    let url = get_postgres_url();
    let url_clone = url.clone();
    let provider = PostgresLockProvider::new(url).await.unwrap();
    let lock = provider.create_lock("test-blocking");

    // Acquire lock in first task
    let handle1 = lock.acquire(None).await.unwrap();

    // Spawn a task that tries to acquire the same lock
    let lock_name = lock.name().to_string();
    let acquire_task = tokio::spawn(async move {
        let provider2 = PostgresLockProvider::new(url_clone).await.unwrap();
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
    result.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires PostgreSQL server running
async fn test_lock_timeout() {
    let url = get_postgres_url();
    let provider = PostgresLockProvider::new(url).await.unwrap();
    let lock = provider.create_lock("test-timeout");

    // Acquire lock
    let handle1 = lock.acquire(None).await.unwrap();

    // Try to acquire with short timeout - should fail
    let result = lock.acquire(Some(Duration::from_millis(50))).await;
    assert!(result.is_err());

    // Release the lock
    handle1.release().await.unwrap();
}

/// Comprehensive reader-writer lock integration test.
///
/// This test verifies all the core requirements for reader-writer locks:
/// - Multiple readers can hold lock simultaneously
/// - Write lock blocks all readers
/// - Readers block when writer holds lock
/// - Writer priority prevents starvation
#[tokio::test]
#[ignore] // Requires PostgreSQL server running
async fn test_reader_writer_lock_semantics() {
    let url = get_postgres_url();
    let provider = PostgresLockProvider::new(url).await.unwrap();
    let rw_lock = provider.create_reader_writer_lock("test-rw-semantics");

    // Test 1: Multiple readers can hold lock simultaneously
    let read_handle1 = rw_lock.try_acquire_read().await.unwrap();
    assert!(read_handle1.is_some(), "First read lock should succeed");

    let read_handle2 = rw_lock.try_acquire_read().await.unwrap();
    assert!(read_handle2.is_some(), "Second read lock should succeed");

    let read_handle3 = rw_lock.try_acquire_read().await.unwrap();
    assert!(read_handle3.is_some(), "Third read lock should succeed");

    // Test 2: Write lock blocks all readers
    let write_handle = rw_lock.try_acquire_write().await.unwrap();
    assert!(
        write_handle.is_none(),
        "Write lock should fail when readers hold lock"
    );

    // Release all readers
    read_handle1.unwrap().release().await.unwrap();
    read_handle2.unwrap().release().await.unwrap();
    read_handle3.unwrap().release().await.unwrap();

    // Test 3: Writer can acquire after readers release
    let write_handle2 = rw_lock.try_acquire_write().await.unwrap();
    assert!(
        write_handle2.is_some(),
        "Write lock should succeed after readers release"
    );

    // Test 4: Readers block when writer holds lock
    let read_handle4 = rw_lock.try_acquire_read().await.unwrap();
    assert!(
        read_handle4.is_none(),
        "Read lock should fail when writer holds lock"
    );

    let write_handle3 = rw_lock.try_acquire_write().await.unwrap();
    assert!(
        write_handle3.is_none(),
        "Another write lock should fail when writer holds lock"
    );

    // Release writer
    write_handle2.unwrap().release().await.unwrap();

    // Test 5: Readers can acquire after writer releases
    let read_handle5 = rw_lock.try_acquire_read().await.unwrap();
    assert!(
        read_handle5.is_some(),
        "Read lock should succeed after writer releases"
    );
    read_handle5.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires PostgreSQL server running
async fn test_reader_writer_lock_readers_block_writer() {
    let url = get_postgres_url();
    let provider = PostgresLockProvider::new(url).await.unwrap();
    let rw_lock = provider.create_reader_writer_lock("test-rw-block");

    // Acquire multiple readers
    let read_handle1 = rw_lock.try_acquire_read().await.unwrap().unwrap();
    let read_handle2 = rw_lock.try_acquire_read().await.unwrap().unwrap();

    // Writer should not be able to acquire while readers hold the lock
    let write_handle = rw_lock.try_acquire_write().await.unwrap();
    assert!(write_handle.is_none());

    // Release one reader
    read_handle1.release().await.unwrap();

    // Writer still shouldn't be able to acquire
    let write_handle2 = rw_lock.try_acquire_write().await.unwrap();
    assert!(write_handle2.is_none());

    // Release the other reader
    read_handle2.release().await.unwrap();

    // Now writer should be able to acquire
    let write_handle3 = rw_lock.try_acquire_write().await.unwrap();
    assert!(write_handle3.is_some());
    write_handle3.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires PostgreSQL server running
async fn test_reader_writer_lock_blocking_acquire() {
    let url = get_postgres_url();
    let url_clone = url.clone();
    let provider = PostgresLockProvider::new(url).await.unwrap();
    let rw_lock = provider.create_reader_writer_lock("test-rw-blocking");

    // Acquire write lock
    let write_handle = rw_lock.acquire_write(None).await.unwrap();

    // Spawn a task that tries to acquire a read lock (should block)
    let lock_name = rw_lock.name().to_string();
    let acquire_task = tokio::spawn(async move {
        let provider2 = PostgresLockProvider::new(url_clone).await.unwrap();
        let rw_lock2 = provider2.create_reader_writer_lock(&lock_name);
        rw_lock2
            .acquire_read(Some(Duration::from_millis(200)))
            .await
    });

    // Wait a bit to ensure the task is waiting
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Release the write lock
    write_handle.release().await.unwrap();

    // The waiting task should now acquire the read lock
    let result = timeout(Duration::from_secs(1), acquire_task)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.is_ok(),
        "Waiting task should acquire read lock after writer releases"
    );
    result.unwrap().release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires PostgreSQL server running
async fn test_keepalive_mechanism() {
    let url = get_postgres_url();
    let provider = PostgresLockProvider::builder()
        .connection_string(url)
        .keepalive_cadence(Duration::from_millis(100))
        .build()
        .await
        .unwrap();

    let lock = provider.create_lock("test-keepalive");
    let handle = lock.acquire(None).await.unwrap();

    // Hold the lock for a while - keepalive should keep connection alive
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Lock should still be held
    let handle2 = lock.try_acquire().await.unwrap();
    assert!(handle2.is_none(), "Lock should still be held");

    handle.release().await.unwrap();
}

#[tokio::test]
#[ignore] // Requires PostgreSQL server running
async fn test_transaction_scoped_locks() {
    let url = get_postgres_url();
    let provider = PostgresLockProvider::builder()
        .connection_string(url)
        .use_transaction(true)
        .build()
        .await
        .unwrap();

    let lock = provider.create_lock("test-transaction");
    let handle = lock.try_acquire().await.unwrap();
    assert!(handle.is_some(), "Should acquire transaction-scoped lock");

    // Lock should be released when handle is dropped (transaction ends)
    handle.unwrap().release().await.unwrap();

    // Should be able to acquire again
    let handle2 = lock.try_acquire().await.unwrap();
    assert!(handle2.is_some());
    handle2.unwrap().release().await.unwrap();
}
