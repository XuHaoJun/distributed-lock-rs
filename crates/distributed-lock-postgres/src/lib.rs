//! PostgreSQL backend for distributed locks.

pub mod key;
pub mod connection;
pub mod lock;
pub mod handle;
pub mod provider;
pub mod rw_lock;

pub use key::PostgresAdvisoryLockKey;
pub use provider::{PostgresLockProvider, PostgresLockProviderBuilder};
pub use rw_lock::{
    PostgresDistributedReaderWriterLock, PostgresReadLockHandle, PostgresWriteLockHandle,
};