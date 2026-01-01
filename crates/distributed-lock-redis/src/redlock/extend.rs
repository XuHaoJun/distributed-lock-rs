//! RedLock extend algorithm implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use fred::prelude::*;
use tokio::time::Instant;

use super::helper::RedLockHelper;
use super::timeouts::RedLockTimeouts;

/// Extends a lock using the RedLock algorithm across multiple Redis servers.
///
/// Similar to acquire, requires majority consensus for successful extension.
///
/// # Arguments
///
/// * `try_extend_fn` - Function that attempts to extend the lock on a single client
/// * `clients` - List of Redis clients where lock was acquired
/// * `acquire_results` - Previous acquire results indicating which clients hold the lock
/// * `timeouts` - Timeout configuration
/// * `cancel_token` - Cancellation token
pub async fn extend_redlock<F, Fut>(
    try_extend_fn: F,
    clients: &[RedisClient],
    acquire_results: &[bool],
    timeouts: &RedLockTimeouts,
    cancel_token: &tokio::sync::watch::Receiver<bool>,
) -> LockResult<Option<bool>>
where
    F: Fn(&RedisClient) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = LockResult<bool>> + Send,
{
    // Only extend on clients where we successfully acquired
    let clients_to_extend: Vec<(usize, RedisClient)> = acquire_results
        .iter()
        .enumerate()
        .filter(|&(_, &success)| success)
        .map(|(idx, _)| (idx, clients[idx].clone()))
        .collect();

    if clients_to_extend.is_empty() {
        return Ok(Some(false)); // No locks to extend
    }

    let acquire_timeout = timeouts.acquire_timeout();
    let timeout_duration = acquire_timeout.as_duration();

    // Start extend attempts on all clients in parallel
    let mut extend_tasks: Vec<tokio::task::JoinHandle<LockResult<bool>>> = Vec::new();

    for (_, client) in &clients_to_extend {
        let client_clone = client.clone();
        let try_extend_fn_clone = try_extend_fn.clone();
        let task = tokio::spawn(async move { try_extend_fn_clone(&client_clone).await });
        extend_tasks.push(task);
    }

    // Wait for results with timeout
    let start = Instant::now();
    let mut success_count = 0;
    let mut fail_count = 0;
    let total_clients = clients_to_extend.len();

    // Wait for tasks to complete or timeout
    loop {
        // Check timeout
        if let Some(timeout_dur) = timeout_duration
            && start.elapsed() >= timeout_dur
        {
            // Timeout - return inconclusive
            return Ok(None);
        }

        // Check for cancellation
        if cancel_token.has_changed().unwrap_or(false) && *cancel_token.borrow() {
            return Err(LockError::Cancelled);
        }

        // Check completed tasks
        for task in extend_tasks.iter_mut() {
            if task.is_finished() {
                match task.await {
                    Ok(Ok(true)) => {
                        success_count += 1;
                        if RedLockHelper::has_sufficient_successes(success_count, total_clients) {
                            // We have majority - return success
                            return Ok(Some(true));
                        }
                    }
                    Ok(Ok(false)) => {
                        fail_count += 1;
                        if RedLockHelper::has_too_many_failures_or_faults(fail_count, total_clients)
                        {
                            // Can't achieve majority - return failure
                            return Ok(Some(false));
                        }
                    }
                    Ok(Err(e)) => {
                        // Error extending - treat as failure
                        fail_count += 1;
                        if RedLockHelper::has_too_many_failures_or_faults(fail_count, total_clients)
                        {
                            return Err(e);
                        }
                    }
                    Err(_) => {
                        // Task panicked - treat as failure
                        fail_count += 1;
                        if RedLockHelper::has_too_many_failures_or_faults(fail_count, total_clients)
                        {
                            return Ok(Some(false));
                        }
                    }
                }
            }
        }

        // If all tasks are done, check final result
        if success_count + fail_count >= total_clients {
            if RedLockHelper::has_sufficient_successes(success_count, total_clients) {
                return Ok(Some(true));
            } else {
                return Ok(Some(false));
            }
        }

        // Sleep briefly before checking again
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
