//! Integration tests for MongoDB-based distributed locks.

use distributed_lock_core::traits::{DistributedLock, LockHandle};
use distributed_lock_mongo::MongoDistributedLock;
use mongodb::Client;
use std::time::Duration;

/// Helper to get MongoDB URI from environment or use default.
fn get_mongo_uri() -> String {
    std::env::var("MONGODB_URI").unwrap_or_else(|_| "mongodb://localhost:27017".to_string())
}

#[tokio::test]
#[ignore] // Requires MongoDB server running
async fn test_mongo_lock_acquire_release() {
    let uri = get_mongo_uri();
    let client = Client::with_uri_str(&uri)
        .await
        .expect("Failed to connect to MongoDB");
    let database = client.database("test_distributed_locks");

    // Ensure collection is clean-ish or use random name
    let lock_name = uuid::Uuid::new_v4().to_string();

    let lock = MongoDistributedLock::new(lock_name.clone(), database.clone(), None, None);

    // 1. Acquire
    let handle = lock.acquire(Some(Duration::from_secs(5))).await;
    assert!(handle.is_ok(), "Failed to acquire lock: {:?}", handle.err());
    let handle = handle.unwrap();

    // 2. Try acquire again (should fail/timeout)
    let lock2 = MongoDistributedLock::new(lock_name.clone(), database.clone(), None, None);

    let handle2 = lock2
        .try_acquire()
        .await
        .expect("Failed to call try_acquire");
    assert!(handle2.is_none(), "Should not be able to acquire held lock");

    // 3. Release
    handle.release().await.expect("Failed to release lock");

    // 4. Acquire again (should succeed)
    let handle3 = lock2.acquire(Some(Duration::from_secs(5))).await;
    assert!(
        handle3.is_ok(),
        "Failed to re-acquire lock: {:?}",
        handle3.err()
    );

    handle3
        .unwrap()
        .release()
        .await
        .expect("Failed to release lock 3");
}
