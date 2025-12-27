//! Example: Using PostgreSQL distributed locks
//!
//! Run with: `cargo run --example postgres_lock`
//!
//! Requires a PostgreSQL database. Set POSTGRES_URL environment variable
//! or modify the connection string below.

use distributed_lock_core::prelude::*;
use distributed_lock_postgres::PostgresLockProvider;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get connection string from environment or use default
    let connection_string = std::env::var("POSTGRES_URL").unwrap_or_else(|_| {
        "postgresql://postgres:postgres@localhost/distributed_lock_test".to_string()
    });

    println!("Connecting to PostgreSQL...");
    let provider = PostgresLockProvider::builder()
        .connection_string(&connection_string)
        .build()
        .await?;

    println!("Created PostgreSQL lock provider");

    // Create a lock by name
    let lock = provider.create_lock("example-resource");
    println!("Created lock: {}", lock.name());

    // Acquire the lock with a timeout
    println!("Acquiring lock with 5 second timeout...");
    let handle = lock.acquire(Some(Duration::from_secs(5))).await?;
    println!("Lock acquired!");

    // Do some work while holding the lock
    println!("Doing critical work...");
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Release the lock
    handle.release().await?;
    println!("Lock released");

    // Example: Reader-writer lock
    println!("\nUsing reader-writer lock...");
    let rw_lock = provider.create_reader_writer_lock("cache");

    // Multiple readers can hold the lock
    {
        let read_handle = rw_lock.acquire_read(Some(Duration::from_secs(5))).await?;
        println!("Read lock acquired");
        // Read from cache...
        read_handle.release().await?;
        println!("Read lock released");
    }

    // Writers get exclusive access
    {
        let write_handle = rw_lock.acquire_write(Some(Duration::from_secs(5))).await?;
        println!("Write lock acquired");
        // Update cache...
        write_handle.release().await?;
        println!("Write lock released");
    }

    Ok(())
}
