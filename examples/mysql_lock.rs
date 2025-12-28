//! Example demonstrating MySQL-based distributed locks.

use distributed_lock_core::prelude::*;
use distributed_lock_mysql::MySqlLockProvider;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get MySQL connection string from environment or use default
    let url = std::env::var("MYSQL_URL")
        .unwrap_or_else(|_| "mysql://root:password@localhost:3306/mysql".to_string());

    println!("Connecting to MySQL at: {}", url);

    // Create a lock provider
    let provider = MySqlLockProvider::new(url).await?;
    println!("Connected successfully!");

    // Create a lock
    let lock = provider.create_lock("example-lock");
    println!("Created lock: {}", lock.name());

    // Try to acquire the lock
    println!("Attempting to acquire lock...");
    match lock.try_acquire().await {
        Ok(Some(handle)) => {
            println!("Lock acquired successfully!");
            println!("Holding lock for 2 seconds...");

            // Hold the lock for a bit
            tokio::time::sleep(Duration::from_secs(2)).await;

            // Release the lock
            handle.release().await?;
            println!("Lock released!");
        }
        Ok(None) => {
            println!("Lock is currently held by another process");
        }
        Err(e) => {
            eprintln!("Error acquiring lock: {}", e);
        }
    }

    Ok(())
}
