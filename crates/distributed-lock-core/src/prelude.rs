//! Convenience prelude for distributed lock types.

pub use crate::error::{LockError, LockResult};
pub use crate::traits::{
    DistributedLock, DistributedReaderWriterLock, DistributedSemaphore, LockHandle,
    LockProvider, LockProviderExt, ReaderWriterLockProvider, SemaphoreProvider,
};
