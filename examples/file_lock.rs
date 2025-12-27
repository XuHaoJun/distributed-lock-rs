//! Example: Using file-based distributed locks
//!
//! Run with: `cargo run --example file_lock`

use distributed_lock_core::prelude::*;
use distributed_lock_file::FileLockProvider;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a file lock provider
    let provider = FileLockProvider::builder()
        .directory("/tmp/distributed-locks")
        .build()?;

    println!("Created file lock provider");

    // Create a lock by name
    let lock = provider.create_lock("example-resource");
    println!("Created lock: {}", lock.name());

    // Try to acquire the lock
    match lock.try_acquire().await? {
        Some(handle) => {
            println!("Lock acquired successfully!");

            // Do some work while holding the lock
            tokio::time::sleep(Duration::from_secs(2)).await;
            println!("Work completed");

            // Release the lock
            handle.release().await?;
            println!("Lock released");
        }
        None => {
            println!("Lock is currently held by another process");
        }
    }

    // Acquire with timeout
    println!("\nAcquiring lock with 5 second timeout...");
    let handle = lock.acquire(Some(Duration::from_secs(5))).await?;
    println!("Lock acquired!");

    // Lock is automatically released when handle is dropped
    drop(handle);
    println!("Lock released (via drop)");

    Ok(())
}
