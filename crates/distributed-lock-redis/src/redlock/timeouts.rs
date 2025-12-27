//! RedLock timeout calculations.

use std::time::Duration;

use distributed_lock_core::timeout::TimeoutValue;

/// Timeout configuration for RedLock algorithm.
///
/// Calculates acquire timeout based on expiry and minimum validity time.
#[derive(Debug, Clone)]
pub struct RedLockTimeouts {
    /// Lock expiry time (TTL set on Redis keys).
    pub expiry: Duration,
    /// Minimum validity time required after acquisition.
    ///
    /// This accounts for clock drift in multi-server scenarios.
    pub min_validity: Duration,
}

impl RedLockTimeouts {
    /// Creates a new timeout configuration.
    pub fn new(expiry: Duration, min_validity: Duration) -> Self {
        Self {
            expiry,
            min_validity,
        }
    }

    /// Calculates the acquire timeout.
    ///
    /// This is the maximum time we can spend acquiring the lock while still
    /// ensuring at least `min_validity` time remains on the lock after acquisition.
    pub fn acquire_timeout(&self) -> TimeoutValue {
        if self.expiry > self.min_validity {
            TimeoutValue::from(Some(self.expiry - self.min_validity))
        } else {
            TimeoutValue::ZERO
        }
    }
}
