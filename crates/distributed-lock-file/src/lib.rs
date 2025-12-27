//! File system backend for distributed locks.

pub mod handle;
pub mod lock;
pub mod name;
pub mod provider;

pub use handle::FileLockHandle;
pub use lock::FileDistributedLock;
pub use provider::{FileLockProvider, FileLockProviderBuilder};
