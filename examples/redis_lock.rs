//! Example: Using Redis distributed locks
//!
//! Run with: `cargo run --example redis_lock`
//!
//! Requires a Redis server. Set REDIS_URL environment variable
//! or modify the URL below.

use distributed_lock_redis::RedisLockProvider;
use distributed_lock_core::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get Redis URL from environment or use default
    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".to_string());

    println!("Connecting to Redis...");
    let provider = RedisLockProvider::builder()
        .add_server(&redis_url)
        .build()
        .await?;

    println!("Created Redis lock provider");

    // Create a lock by name
    let lock = provider.create_lock("example-resource");
    println!("Created lock: {}", lock.name());

    // Acquire the lock with a timeout
    // Note: Redis locks are automatically extended in the background
    println!("Acquiring lock with 5 second timeout...");
    let handle = lock.acquire(Some(Duration::from_secs(5))).await?;
    println!("Lock acquired! (will be automatically extended)");

    // Do some long-running work
    // The lock will be automatically extended every 10 seconds (default)
    println!("Doing long-running work...");
    tokio::time::sleep(Duration::from_secs(15)).await;
    println!("Work completed");

    // Release the lock
    handle.release().await?;
    println!("Lock released");

    // Example: Semaphore for rate limiting
    println!("\nUsing semaphore for rate limiting...");
    let semaphore = provider.create_semaphore("api-rate-limit", 5);

    // Acquire a ticket (allows up to 5 concurrent operations)
    let ticket = semaphore.acquire(Some(Duration::from_secs(10))).await?;
    println!("Semaphore ticket acquired ({} max concurrent)", semaphore.max_count());
    
    // Do rate-limited work
    println!("Making API call...");
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // Release the ticket
    ticket.release().await?;
    println!("Semaphore ticket released");

    Ok(())
}
