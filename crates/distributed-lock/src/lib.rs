//! Distributed locks for Rust with multiple backend support.
//!
//! This crate provides distributed synchronization primitives (mutex locks,
//! reader-writer locks, semaphores) that work across processes and machines.
//! Multiple backend implementations are available: PostgreSQL, Redis, and file system.
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use distributed_lock::*;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a provider (example: file backend)
//!     let provider = FileLockProvider::builder()
//!         .build("/tmp/locks")?;
//!
//!     // Create a lock by name
//!     let lock = provider.create_lock("my-resource");
//!
//!     // Acquire the lock with a timeout
//!     let handle = lock.acquire(Some(Duration::from_secs(5))).await?;
//!
//!     // Critical section - we have exclusive access
//!     println!("Doing critical work...");
//!
//!     // Release the lock (also happens automatically on drop)
//!     handle.release().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Backends
//!
//! ## File System Backend
//!
//! Uses OS-level file locking. Simple and requires no external services.
//!
//! ```rust,no_run
//! use distributed_lock::FileLockProvider;
//!
//! let provider = FileLockProvider::builder()
//!     .build("/tmp/locks")?;
//! ```
//!
//! ## PostgreSQL Backend
//!
//! Uses PostgreSQL advisory locks. Production-ready with connection pooling.
//!
//! ```rust,no_run
//! use distributed_lock::PostgresLockProvider;
//!
//! let provider = PostgresLockProvider::builder()
//!     .connection_string("postgresql://user:pass@localhost/db")
//!     .build()
//!     .await?;
//! ```
//!
//! ## Redis Backend
//!
//! Uses Redis with RedLock algorithm for multi-server deployments. Supports
//! semaphores and automatic lease extension.
//!
//! ```rust,no_run
//! use distributed_lock::RedisLockProvider;
//!
//! let provider = RedisLockProvider::builder()
//!     .add_server("redis://localhost:6379")
//!     .build()
//!     .await?;
//! ```
//!
//! # Features
//!
//! - **Exclusive Locks**: Mutual exclusion across processes
//! - **Reader-Writer Locks**: Multiple readers or single writer
//! - **Semaphores**: Limit concurrent access to N processes
//! - **Async/Await**: Full async support with tokio
//! - **Backend Agnostic**: Swap backends without changing application code
//! - **Handle Loss Detection**: Detect when locks are lost due to connection issues
//!
//! # Crate Organization
//!
//! This is a meta-crate that re-exports types from:
//! - `distributed-lock-core`: Core traits and types
//! - `distributed-lock-file`: File system backend
//! - `distributed-lock-postgres`: PostgreSQL backend
//! - `distributed-lock-redis`: Redis backend
//!
//! For fine-grained control, you can depend on individual crates instead.

// Re-export core types and traits
pub use distributed_lock_core::*;

// Re-export file backend
#[allow(ambiguous_glob_reexports)]
pub use distributed_lock_file::*;

// Re-export postgres backend
#[allow(ambiguous_glob_reexports)]
pub use distributed_lock_postgres::*;

// Re-export redis backend
#[allow(ambiguous_glob_reexports)]
pub use distributed_lock_redis::*;
