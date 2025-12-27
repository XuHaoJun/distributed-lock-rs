//! Example: Using the meta-crate (all backends)
//!
//! Run with: `cargo run --example meta_crate`
//!
//! This example shows how to use the meta-crate which re-exports
//! all backend implementations.

use distributed_lock::*;
use distributed_lock_core::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Example: Using distributed-lock meta-crate\n");

    // File backend example
    println!("=== File Backend ===");
    let file_provider = FileLockProvider::builder()
        .build("/tmp/distributed-locks")?;
    
    let file_lock = file_provider.create_lock("example");
    if let Some(handle) = file_lock.try_acquire().await? {
        println!("File lock acquired");
        handle.release().await?;
    }

    // PostgreSQL backend example (if available)
    if let Ok(postgres_url) = std::env::var("POSTGRES_URL") {
        println!("\n=== PostgreSQL Backend ===");
        if let Ok(postgres_provider) = PostgresLockProvider::builder()
            .connection_string(&postgres_url)
            .build()
            .await
        {
            let pg_lock = postgres_provider.create_lock("example");
            if let Some(handle) = pg_lock.try_acquire().await? {
                println!("PostgreSQL lock acquired");
                handle.release().await?;
            }
        }
    }

    // Redis backend example (if available)
    if let Ok(redis_url) = std::env::var("REDIS_URL") {
        println!("\n=== Redis Backend ===");
        if let Ok(redis_provider) = RedisLockProvider::builder()
            .add_server(&redis_url)
            .build()
            .await
        {
            let redis_lock = redis_provider.create_lock("example");
            if let Some(handle) = redis_lock.try_acquire().await? {
                println!("Redis lock acquired");
                handle.release().await?;
            }
        }
    }

    println!("\nAll examples completed!");
    Ok(())
}
