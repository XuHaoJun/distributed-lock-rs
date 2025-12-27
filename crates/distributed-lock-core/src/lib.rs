//! Core traits and types for distributed locks.

pub mod error;
pub mod prelude;
pub mod timeout;
pub mod traits;

pub use error::{LockError, LockResult};
pub use prelude::*;
