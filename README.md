# distributed-lock-rs

[![CI](https://github.com/yourusername/distributed-lock-rs/workflows/CI/badge.svg)](https://github.com/yourusername/distributed-lock-rs/actions)
[![Crates.io](https://img.shields.io/crates/v/distributed-lock.svg)](https://crates.io/crates/distributed-lock)
[![Documentation](https://docs.rs/distributed-lock/badge.svg)](https://docs.rs/distributed-lock)
[![MSRV](https://img.shields.io/badge/rustc-1.85+-blue.svg)](https://www.rust-lang.org)

Distributed locks for Rust with multiple backend support. This library provides distributed synchronization primitives (mutex locks, reader-writer locks, semaphores) that work across processes and machines.

## Features

- 🔒 **Exclusive Locks**: Mutual exclusion across processes
- 📖 **Reader-Writer Locks**: Multiple readers or single writer
- 🎫 **Semaphores**: Limit concurrent access to N processes
- ⚡ **Async/Await**: Full async support with tokio
- 🔄 **Backend Agnostic**: Swap backends without changing application code
- 🔍 **Handle Loss Detection**: Detect when locks are lost due to connection issues
- 🚀 **Production Ready**: Connection pooling, automatic lease extension, RedLock support

## Backends

### PostgreSQL
Uses PostgreSQL advisory locks. Production-ready with connection pooling and transaction-scoped locks.

```toml
[dependencies]
distributed-lock-postgres = "0.1"
tokio = { version = "1", features = ["full"] }
```

### Redis
Uses Redis with RedLock algorithm for multi-server deployments. Supports semaphores and automatic lease extension.

```toml
[dependencies]
distributed-lock-redis = "0.1"
tokio = { version = "1", features = ["full"] }
```

### File System
Uses OS-level file locking. Simple and requires no external services.

```toml
[dependencies]
distributed-lock-file = "0.1"
tokio = { version = "1", features = ["full"] }
```

### Meta-Crate (All Backends)
Or use the meta-crate to get all backends:

```toml
[dependencies]
distributed-lock = "0.1"
tokio = { version = "1", features = ["full"] }
```

## Quick Start

### Basic Usage

```rust
use distributed_lock::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a provider (example: file backend)
    let provider = FileLockProvider::builder()
        .build("/tmp/locks")?;

    // Create a lock by name
    let lock = provider.create_lock("my-resource");

    // Acquire the lock with a timeout
    let handle = lock.acquire(Some(Duration::from_secs(5))).await?;

    // Critical section - we have exclusive access
    println!("Doing critical work...");

    // Release the lock (also happens automatically on drop)
    handle.release().await?;

    Ok(())
}
```

### PostgreSQL Example

```rust
use distributed_lock_postgres::PostgresLockProvider;
use distributed_lock_core::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = PostgresLockProvider::builder()
        .connection_string("postgresql://user:pass@localhost/db")
        .build()
        .await?;

    let lock = provider.create_lock("my-resource");
    let handle = lock.acquire(Some(Duration::from_secs(5))).await?;

    // Critical section
    do_work().await?;

    handle.release().await?;
    Ok(())
}
```

### Redis Example

```rust
use distributed_lock_redis::RedisLockProvider;
use distributed_lock_core::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = RedisLockProvider::builder()
        .add_server("redis://localhost:6379")
        .build()
        .await?;

    let lock = provider.create_lock("my-resource");
    let handle = lock.acquire(Some(Duration::from_secs(5))).await?;

    // Lock is automatically extended in the background
    do_long_running_work().await?;

    handle.release().await?;
    Ok(())
}
```

### Reader-Writer Locks

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

### Semaphores

```rust
use distributed_lock_redis::RedisLockProvider;
use distributed_lock_core::prelude::*;
use std::time::Duration;

async fn rate_limit_example(provider: &RedisLockProvider) -> Result<(), LockError> {
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

## Try-Acquire Pattern

Use `try_acquire` when you want to check if a lock is available without waiting:

```rust
match lock.try_acquire().await? {
    Some(handle) => {
        // We got the lock!
        do_work().await;
        handle.release().await?;
    }
    None => {
        // Someone else has it
        println!("Lock unavailable");
    }
}
```

## Error Handling

```rust
use distributed_lock_core::prelude::*;

match lock.acquire(Some(Duration::from_secs(5))).await {
    Ok(handle) => {
        // Got the lock
        handle.release().await?;
    }
    Err(LockError::Timeout(duration)) => {
        eprintln!("Timed out after {:?}", duration);
    }
    Err(LockError::Connection(e)) => {
        eprintln!("Connection error: {}", e);
    }
    Err(e) => {
        eprintln!("Error: {}", e);
    }
}
```

## Architecture

This library is organized as a Cargo workspace with separate crates:

- `distributed-lock-core`: Core traits and types
- `distributed-lock-file`: File system backend
- `distributed-lock-postgres`: PostgreSQL backend
- `distributed-lock-redis`: Redis backend
- `distributed-lock`: Meta-crate re-exporting all backends

Each backend implements the same trait interfaces, allowing you to swap backends without changing application code.

## Requirements

- Rust 1.85 or later
- Tokio async runtime (for async operations)
- Backend-specific requirements:
  - **PostgreSQL**: PostgreSQL 9.5+ with `pg_advisory_lock` support
  - **Redis**: Redis 2.6+ (Redis 3.0+ recommended)
  - **File**: Any POSIX-compliant filesystem

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Acknowledgments

This library is inspired by the [DistributedLock](https://github.com/madelson/DistributedLock) C# library, ported to Rust with idiomatic patterns and async/await support.
