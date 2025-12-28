//! RedLock acquire algorithm implementation.

use distributed_lock_core::error::{LockError, LockResult};
use fred::prelude::*;

use super::helper::RedLockHelper;
use super::timeouts::RedLockTimeouts;

/// Result of a RedLock acquire operation.
#[derive(Debug)]
pub struct RedLockAcquireResult {
    /// Results indexed by client position (true = success, false = failed).
    pub acquire_results: Vec<bool>,
}

impl RedLockAcquireResult {
    /// Creates a new acquire result.
    pub fn new(acquire_results: Vec<bool>) -> Self {
        Self { acquire_results }
    }

    /// Checks if the acquire was successful (majority consensus).
    pub fn is_successful(&self, total_clients: usize) -> bool {
        let success_count = self.acquire_results.iter().filter(|&&v| v).count();
        RedLockHelper::has_sufficient_successes(success_count, total_clients)
    }

    /// Returns the number of successful acquisitions.
    pub fn success_count(&self) -> usize {
        self.acquire_results.iter().filter(|&&v| v).count()
    }
}

// ... imports ...
use tokio::task::JoinSet; // Add this import

// ...

/// Acquires a lock using the RedLock algorithm across multiple Redis servers.
///
/// The algorithm requires majority consensus: for N servers, we need at least
/// (N/2 + 1) successful acquisitions.
///
/// # Arguments
///
/// * `try_acquire_fn` - Function that attempts to acquire the lock on a single client
/// * `clients` - List of Redis clients to acquire on
/// * `timeouts` - Timeout configuration
/// * `cancel_token` - Cancellation token
pub async fn acquire_redlock<F, Fut>(
    try_acquire_fn: F,
    clients: &[RedisClient],
    timeouts: &RedLockTimeouts,
    cancel_token: &tokio::sync::watch::Receiver<bool>,
) -> LockResult<Option<RedLockAcquireResult>>
where
    F: Fn(&RedisClient) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = LockResult<bool>> + Send,
{
    if clients.is_empty() {
        return Err(LockError::InvalidName(
            "no Redis clients provided".to_string(),
        ));
    }

    // Single client case - simpler path
    if clients.len() == 1 {
        return acquire_single_client(try_acquire_fn, &clients[0], timeouts, cancel_token).await;
    }

    // Multi-client RedLock algorithm
    let acquire_timeout = timeouts.acquire_timeout();
    let timeout_duration = acquire_timeout.as_duration();

    // Use JoinSet to manage concurrent acquisition tasks
    let mut join_set = JoinSet::new();

    for (idx, client) in clients.iter().enumerate() {
        let client_clone = client.clone();
        let try_acquire_fn_clone = try_acquire_fn.clone();
        join_set.spawn(async move {
            let result = try_acquire_fn_clone(&client_clone).await;
            (idx, result)
        });
    }

    let mut results: Vec<Option<bool>> = vec![None; clients.len()];
    let mut success_count = 0;
    let mut fail_count = 0;

    // Create timeout future (if applicable)
    let timeout_fut = async {
        if let Some(dur) = timeout_duration {
            tokio::time::sleep(dur).await;
            true // Timed out
        } else {
            std::future::pending::<bool>().await;
            false // Never times out
        }
    };
    tokio::pin!(timeout_fut);

    // Create cancellation future
    let mut cancel_rx = cancel_token.clone();

    loop {
        tokio::select! {
            // 1. Check for cancellation
            _ = cancel_rx.changed() => {
               if *cancel_rx.borrow() {
                   // Abort remaining tasks automatically when join_set is dropped
                   return Err(LockError::Cancelled);
               }
            }

            // 2. Check for overall timeout
            _ = &mut timeout_fut => {
                // Abort remaining tasks
                return Ok(None);
            }

            // 3. Process completed tasks
            Some(join_result) = join_set.join_next() => {
                match join_result {
                    Ok((idx, lock_res)) => {
                        match lock_res {
                            Ok(true) => {
                                results[idx] = Some(true);
                                success_count += 1;
                                if RedLockHelper::has_sufficient_successes(success_count, clients.len()) {
                                    // Majority reached!
                                    // Fill remaining with false (unknown state but logically not holding)
                                    // Note: In RedLock, we technically might have acquired others,
                                    // but we only claim the ones we know about + default false.
                                    // Background release will clean up anyway.
                                    let final_results: Vec<bool> = results.iter().map(|r| r.unwrap_or(false)).collect();
                                    return Ok(Some(RedLockAcquireResult::new(final_results)));
                                }
                            }
                            Ok(false) => {
                                results[idx] = Some(false);
                                fail_count += 1;
                            }
                            Err(_) => {
                                // Error acquiring
                                results[idx] = Some(false);
                                fail_count += 1;
                            }
                        }
                    }
                    Err(_) => {
                        // Task panicked or cancelled
                        fail_count += 1;
                    }
                }

                // Check if too many failures to ever succeed
                if RedLockHelper::has_too_many_failures_or_faults(fail_count, clients.len()) {
                     return Ok(None);
                }
            }

            // 4. If all tasks finished and we are here (join_set empty), we failed
            else => {
                 return Ok(None);
            }
        }
    }
}

/// Acquires a lock on a single Redis client (simpler path).
async fn acquire_single_client<F, Fut>(
    try_acquire_fn: F,
    client: &RedisClient,
    timeouts: &RedLockTimeouts,
    cancel_token: &tokio::sync::watch::Receiver<bool>,
) -> LockResult<Option<RedLockAcquireResult>>
where
    F: Fn(&RedisClient) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = LockResult<bool>> + Send,
{
    let acquire_timeout = timeouts.acquire_timeout();
    let timeout_duration = acquire_timeout.as_duration();

    // Check for cancellation first
    if cancel_token.has_changed().unwrap_or(false) && *cancel_token.borrow() {
        return Err(LockError::Cancelled);
    }

    let acquire_future = try_acquire_fn(client);

    let result = if let Some(timeout_dur) = timeout_duration {
        match tokio::time::timeout(timeout_dur, acquire_future).await {
            Ok(Ok(true)) => true,
            Ok(Ok(false)) => return Ok(None),
            Ok(Err(e)) => return Err(e),
            Err(_) => return Ok(None), // Timeout
        }
    } else {
        // No timeout - wait indefinitely (but check cancellation)
        loop {
            let mut cancel_rx = cancel_token.clone();
            tokio::select! {
                result = try_acquire_fn(client) => {
                    match result {
                        Ok(true) => break true,
                        Ok(false) => return Ok(None),
                        Err(e) => return Err(e),
                    }
                }
                _ = cancel_rx.changed() => {
                    if *cancel_rx.borrow() {
                        return Err(LockError::Cancelled);
                    }
                    // Continue waiting
                }
            }
        }
    };

    Ok(Some(RedLockAcquireResult::new(vec![result])))
}
