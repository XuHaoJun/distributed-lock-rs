# Quickstart: distributed-lock-rs

This guide shows how to use `distributed-lock-rs` for distributed synchronization in Rust applications.

## Installation

Add the crate for your chosen backend to `Cargo.toml`:

```toml
# For PostgreSQL
[dependencies]
distributed-lock-postgres = "0.1"
tokio = { version = "1", features = ["full"] }

# For Redis
[dependencies]
distributed-lock-redis = "0.1"
tokio = { version = "1", features = ["full"] }

# For file system locks
[dependencies]
distributed-lock-file = "0.1"
tokio = { version = "1", features = ["full"] }

# Or all backends via meta-crate
[dependencies]
distributed-lock = "0.1"
tokio = { version = "1", features = ["full"] }
```

---

## Basic Usage

### PostgreSQL Locks

```rust
use distributed_lock_postgres::PostgresLockProvider;
use distributed_lock_core::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create provider (manages connection pooling)
    let provider = PostgresLockProvider::builder()
        .connection_string("postgresql://localhost/mydb")
        .build()
        .await?;

    // Create a lock by name
    let lock = provider.create_lock("my-resource");

    // Acquire with 5 second timeout
    let handle = lock.acquire(Some(Duration::from_secs(5))).await?;

    println!("Lock acquired! Doing critical work...");
    
    // Critical section - only one process can be here
    do_critical_work().await?;

    // Explicit release (also happens on drop)
    handle.release().await?;
    println!("Lock released!");

    Ok(())
}

async fn do_critical_work() -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(())
}
```

### Redis Locks

```rust
use distributed_lock_redis::RedisLockProvider;
use distributed_lock_core::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Single Redis server
    let provider = RedisLockProvider::builder()
        .url("redis://localhost:6379")
        .build()
        .await?;

    let lock = provider.create_lock("my-resource");
    let handle = lock.acquire(Some(Duration::from_secs(5))).await?;

    // Lock is automatically extended in the background
    do_long_running_work().await?;

    handle.release().await?;
    Ok(())
}

async fn do_long_running_work() -> Result<(), Box<dyn std::error::Error>> {
    // Even if this takes longer than the lock expiry (30s default),
    // the library automatically extends the lock
    tokio::time::sleep(Duration::from_secs(60)).await;
    Ok(())
}
```

### File Locks

```rust
use distributed_lock_file::FileLockProvider;
use distributed_lock_core::prelude::*;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // File locks work synchronously too
    let provider = FileLockProvider::builder()
        .directory("/var/lock/myapp")
        .build()?;

    let lock = provider.create_lock("my-resource");

    // Blocking acquire (no async runtime needed)
    let handle = lock.acquire_blocking(Some(Duration::from_secs(5)))?;

    println!("Lock acquired!");
    std::thread::sleep(Duration::from_secs(2));

    drop(handle); // Releases on drop
    println!("Lock released!");

    Ok(())
}
```

---

## Try-Acquire Pattern

Use `try_acquire` when you want to check if a lock is available without waiting:

```rust
use distributed_lock_core::prelude::*;

async fn maybe_do_work(lock: &impl DistributedLock) -> Result<bool, LockError> {
    match lock.try_acquire().await? {
        Some(handle) => {
            // We got the lock!
            do_work().await;
            handle.release().await?;
            Ok(true)
        }
        None => {
            // Someone else has it
            println!("Lock unavailable, skipping work");
            Ok(false)
        }
    }
}
```

---

## Reader-Writer Locks

For resources that can be read concurrently but need exclusive writes:

```rust
use distributed_lock_postgres::PostgresLockProvider;
use distributed_lock_core::prelude::*;

async fn cache_example(provider: &PostgresLockProvider) -> Result<(), LockError> {
    let rw_lock = provider.create_reader_writer_lock("cache");

    // Multiple readers can hold the lock simultaneously
    {
        let read_handle = rw_lock.acquire_read(None).await?;
        let data = read_from_cache().await;
        read_handle.release().await?;
    }

    // Writers get exclusive access
    {
        let write_handle = rw_lock.acquire_write(None).await?;
        update_cache().await;
        write_handle.release().await?;
    }

    Ok(())
}
```

---

## Semaphores for Rate Limiting

Limit concurrent access to a resource:

```rust
use distributed_lock_redis::RedisLockProvider;
use distributed_lock_core::prelude::*;

async fn database_access(provider: &RedisLockProvider) -> Result<(), LockError> {
    // Allow at most 5 concurrent database connections
    let semaphore = provider.create_semaphore("db-pool", 5);

    // Acquire a "ticket"
    let ticket = semaphore.acquire(Some(Duration::from_secs(10))).await?;

    // Use the limited resource
    query_database().await?;

    // Release the ticket
    ticket.release().await?;
    Ok(())
}
```

---

## Handling Lock Loss

For long-held locks, monitor if the underlying connection is lost:

```rust
use distributed_lock_core::prelude::*;
use tokio::select;

async fn resilient_work(lock: &impl DistributedLock) -> Result<(), LockError> {
    let handle = lock.acquire(None).await?;

    loop {
        select! {
            // Monitor for lock loss
            _ = handle.lost_token().changed() => {
                eprintln!("Lock was lost! Aborting work.");
                return Err(LockError::LockLost("Connection died".into()));
            }
            
            // Do incremental work
            result = do_work_chunk() => {
                if result.is_done() {
                    break;
                }
            }
        }
    }

    handle.release().await?;
    Ok(())
}
```

---

## Provider Abstraction for Testing

Write backend-agnostic code using traits:

```rust
use distributed_lock_core::prelude::*;

// Generic function works with any backend
async fn protected_operation<P>(provider: &P) -> Result<(), LockError>
where
    P: LockProvider,
{
    let lock = provider.create_lock("operation");
    let handle = lock.acquire(Some(Duration::from_secs(5))).await?;
    
    // Do work...
    
    handle.release().await
}

// In production:
// let provider = PostgresLockProvider::new("postgresql://...").await?;
// protected_operation(&provider).await?;

// In tests:
// let provider = InMemoryLockProvider::new();
// protected_operation(&provider).await?;
```

---

## RedLock for High Availability

For Redis, use multiple servers for fault tolerance:

```rust
use distributed_lock_redis::RedisLockProvider;

async fn high_availability_setup() -> Result<RedisLockProvider, LockError> {
    // RedLock algorithm: acquire on majority (2 of 3) for lock to be valid
    RedisLockProvider::builder()
        .urls(&[
            "redis://redis1.example.com:6379",
            "redis://redis2.example.com:6379",
            "redis://redis3.example.com:6379",
        ])
        .expiry(Duration::from_secs(30))
        .extension_cadence(Duration::from_secs(10))
        .build()
        .await
}
```

---

## Error Handling

```rust
use distributed_lock_core::prelude::*;

async fn handle_errors(lock: &impl DistributedLock) {
    match lock.acquire(Some(Duration::from_secs(5))).await {
        Ok(handle) => {
            // Got the lock
            let _ = handle.release().await;
        }
        Err(LockError::Timeout(duration)) => {
            eprintln!("Timed out after {:?}", duration);
        }
        Err(LockError::Cancelled) => {
            eprintln!("Operation was cancelled");
        }
        Err(LockError::Deadlock(msg)) => {
            eprintln!("Deadlock detected: {}", msg);
        }
        Err(LockError::Connection(e)) => {
            eprintln!("Connection error: {}", e);
        }
        Err(e) => {
            eprintln!("Other error: {}", e);
        }
    }
}
```

---

## Configuration Reference

### PostgreSQL Options

| Option | Default | Description |
|--------|---------|-------------|
| `connection_string` | Required | PostgreSQL connection URL |
| `use_transaction_scope` | `false` | Lock released when transaction ends |
| `keepalive_cadence` | Disabled | Issue periodic queries to keep connection alive |
| `use_multiplexing` | `true` | Share connections between locks |

### Redis Options

| Option | Default | Description |
|--------|---------|-------------|
| `url` / `urls` | Required | Redis server URL(s) |
| `expiry` | 30s | Initial lock TTL |
| `extension_cadence` | 10s | How often to extend held locks |
| `min_validity` | 27s (90%) | Minimum TTL after acquire for success |
| `busy_wait_sleep_range` | 10-200ms | Sleep range between retries |

### File Options

| Option | Default | Description |
|--------|---------|-------------|
| `directory` | Required | Directory for lock files |

---

## Next Steps

- Read the [API Documentation](https://docs.rs/distributed-lock) for complete reference
- See [Examples](examples/) for more use cases
- Check backend-specific documentation for advanced options
