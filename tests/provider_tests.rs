//! Tests for provider abstraction.

#![allow(clippy::disallowed_methods)] // Allow std::env::temp_dir for tests

use distributed_lock_core::traits::{DistributedLock, LockHandle, LockProvider, LockProviderExt};
use distributed_lock_file::FileLockProvider;
use std::time::Duration;

mod common;
use common::mock_provider::MockLockProvider;

/// Tests that any provider can be used with the same code.
async fn test_provider_abstraction<P: LockProvider>(provider: &P)
where
    P::Lock: DistributedLock,
{
    test_provider_abstraction_with_name(provider, "test-resource").await;
}

/// Tests that any provider can be used with the same code (with custom name).
async fn test_provider_abstraction_with_name<P: LockProvider>(provider: &P, name: &str)
where
    P::Lock: DistributedLock,
{
    // Create a lock using the provider
    let lock = provider.create_lock(name);

    // Try to acquire the lock
    let handle = lock.try_acquire().await.unwrap();
    assert!(handle.is_some());

    // Release the lock
    handle.unwrap().release().await.unwrap();

    // Now we should be able to acquire it again
    let handle2 = lock.try_acquire().await.unwrap();
    assert!(handle2.is_some());
}

/// Tests provider extension methods work with any provider.
async fn test_provider_extensions<P: LockProvider + LockProviderExt>(provider: &P)
where
    P::Lock: DistributedLock,
{
    test_provider_extensions_with_name(provider, "test-resource").await;
}

/// Tests provider extension methods work with any provider (with custom name).
async fn test_provider_extensions_with_name<P: LockProvider + LockProviderExt>(
    provider: &P,
    name: &str,
) where
    P::Lock: DistributedLock,
{
    // Test acquire_lock extension method
    let handle = provider
        .acquire_lock(name, Some(Duration::from_millis(100)))
        .await;
    assert!(handle.is_ok());

    // Test try_acquire_lock extension method
    let handle2 = provider.try_acquire_lock(name).await.unwrap();
    // Should be None because lock is held
    assert!(handle2.is_none());
}

#[tokio::test]
async fn test_file_provider_abstraction() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    test_provider_abstraction_with_name(&provider, "test-resource-abstraction").await;
}

#[tokio::test]
async fn test_mock_provider_abstraction() {
    let provider = MockLockProvider::new();
    test_provider_abstraction(&provider).await;
}

#[tokio::test]
async fn test_file_provider_extensions() {
    let temp_dir = std::env::temp_dir();
    let provider = FileLockProvider::new(temp_dir).unwrap();
    test_provider_extensions_with_name(&provider, "test-resource-extensions").await;
}

#[tokio::test]
async fn test_mock_provider_extensions() {
    let provider = MockLockProvider::new();
    test_provider_extensions(&provider).await;
}

#[tokio::test]
async fn test_provider_swappability() {
    // Test that we can write code that works with any provider
    async fn use_any_provider<P: LockProvider>(provider: &P)
    where
        P::Lock: DistributedLock,
    {
        let lock = provider.create_lock("shared-resource");
        let handle = lock.try_acquire().await.unwrap();
        assert!(handle.is_some());
        handle.unwrap().release().await.unwrap();
    }

    // Works with file provider
    let file_provider = FileLockProvider::new(std::env::temp_dir()).unwrap();
    use_any_provider(&file_provider).await;

    // Works with mock provider
    let mock_provider = MockLockProvider::new();
    use_any_provider(&mock_provider).await;
}
