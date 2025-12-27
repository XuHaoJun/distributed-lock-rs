//! Core traits and types for distributed locks.

pub mod error;
pub mod timeout;
pub mod traits;
pub mod prelude;

pub use error::{LockError, LockResult};
pub use prelude::*;
pub use traits::*;
