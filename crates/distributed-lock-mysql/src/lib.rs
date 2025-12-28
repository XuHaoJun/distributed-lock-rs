//! MySQL backend for distributed locks.
//!
//! Uses MySQL's GET_LOCK and RELEASE_LOCK functions for distributed locking
//! across multiple processes and machines.

pub mod connection;
pub mod handle;
pub mod lock;
pub mod name;
pub mod provider;
pub mod rw_lock;

pub use handle::MySqlLockHandle;
pub use provider::{MySqlLockProvider, MySqlLockProviderBuilder};
pub use rw_lock::{MySqlDistributedReaderWriterLock, MySqlReadLockHandle, MySqlWriteLockHandle};
