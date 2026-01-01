//! RedLock helper functions.

use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::Rng;

/// Helper functions for RedLock algorithm.
pub struct RedLockHelper;

impl RedLockHelper {
    /// Checks if we have sufficient successes for majority consensus.
    ///
    /// For N servers, we need at least (N/2 + 1) successes.
    pub fn has_sufficient_successes(success_count: usize, database_count: usize) -> bool {
        let threshold = (database_count / 2) + 1;
        success_count >= threshold
    }

    /// Checks if we have too many failures/faults to achieve majority.
    ///
    /// For odd N: need (N/2 + 1) failures to rule out majority.
    /// For even N: need (N/2 + 1) failures to rule out majority.
    pub fn has_too_many_failures_or_faults(
        failure_or_fault_count: usize,
        database_count: usize,
    ) -> bool {
        let threshold = (database_count / 2) + (database_count % 2);
        failure_or_fault_count >= threshold
    }

    /// Generates a unique lock ID.
    ///
    /// Format: `{process_id}_{counter}_{random_uuid}`
    pub fn create_lock_id() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

        // Get process ID
        let pid = process::id();

        // Generate random component
        let mut rng = rand::thread_rng();
        let random: u64 = rng.r#gen();

        format!("{}_{}_{:016x}", pid, counter, random)
    }

    /// Gets current time in milliseconds since Unix epoch.
    pub fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }
}
