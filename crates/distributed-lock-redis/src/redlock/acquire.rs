//! RedLock acquire algorithm implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use fred::prelude::*;
use tokio::time::Instant;

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

    // Start acquire attempts on all clients in parallel
    let mut acquire_tasks: Vec<tokio::task::JoinHandle<LockResult<bool>>> = Vec::new();

    for client in clients {
        let client_clone = client.clone();
        let try_acquire_fn_clone = try_acquire_fn.clone();
        let task = tokio::spawn(async move { try_acquire_fn_clone(&client_clone).await });
        acquire_tasks.push(task);
    }

    // Wait for results with timeout
    let start = Instant::now();
    let mut results: Vec<Option<bool>> = vec![None; clients.len()];
    let mut success_count = 0;
    let mut fail_count = 0;

    // Wait for tasks to complete or timeout
    loop {
        // Check timeout
        if let Some(timeout_dur) = timeout_duration {
            if start.elapsed() >= timeout_dur {
                // Timeout - return failure
                return Ok(None);
            }
        }

        // Check for cancellation
        if cancel_token.has_changed().unwrap_or(false) && *cancel_token.borrow() {
            return Err(LockError::Cancelled);
        }

        // Check completed tasks
        for (idx, task) in acquire_tasks.iter_mut().enumerate() {
            if results[idx].is_some() {
                continue; // Already processed
            }

            if task.is_finished() {
                match task.await {
                    Ok(Ok(true)) => {
                        results[idx] = Some(true);
                        success_count += 1;
                        if RedLockHelper::has_sufficient_successes(success_count, clients.len()) {
                            // We have majority - fill remaining with false and return success
                            for r in results.iter_mut() {
                                if r.is_none() {
                                    *r = Some(false);
                                }
                            }
                            return Ok(Some(RedLockAcquireResult::new(
                                results.into_iter().map(|r| r.unwrap_or(false)).collect(),
                            )));
                        }
                    }
                    Ok(Ok(false)) => {
                        results[idx] = Some(false);
                        fail_count += 1;
                        if RedLockHelper::has_too_many_failures_or_faults(fail_count, clients.len())
                        {
                            // Can't achieve majority - return failure
                            return Ok(None);
                        }
                    }
                    Ok(Err(e)) => {
                        // Error acquiring - treat as failure
                        results[idx] = Some(false);
                        fail_count += 1;
                        if RedLockHelper::has_too_many_failures_or_faults(fail_count, clients.len())
                        {
                            return Err(e);
                        }
                    }
                    Err(_) => {
                        // Task panicked - treat as failure
                        results[idx] = Some(false);
                        fail_count += 1;
                        if RedLockHelper::has_too_many_failures_or_faults(fail_count, clients.len())
                        {
                            return Ok(None);
                        }
                    }
                }
            }
        }

        // If all tasks are done, check final result
        if results.iter().all(|r| r.is_some()) {
            let result = RedLockAcquireResult::new(
                results.into_iter().map(|r| r.unwrap_or(false)).collect(),
            );
            if result.is_successful(clients.len()) {
                return Ok(Some(result));
            } else {
                return Ok(None);
            }
        }

        // Sleep briefly before checking again
        tokio::time::sleep(Duration::from_millis(10)).await;
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
