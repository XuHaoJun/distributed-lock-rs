//! RedLock release algorithm implementation.

use distributed_lock_core::error::{LockError, LockResult};
use fred::prelude::*;

use super::helper::RedLockHelper;

/// Releases a lock using the RedLock algorithm across multiple Redis servers.
///
/// Attempts to release on all clients where the lock was acquired. Requires
/// majority consensus for successful release (though release failures are
/// generally non-fatal).
///
/// # Arguments
///
/// * `try_release_fn` - Function that attempts to release the lock on a single client
/// * `clients` - List of Redis clients
/// * `acquire_results` - Previous acquire results indicating which clients hold the lock
pub async fn release_redlock<F, Fut>(
    try_release_fn: F,
    clients: &[RedisClient],
    acquire_results: &[bool],
) -> LockResult<()>
where
    F: Fn(&RedisClient) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = LockResult<()>> + Send,
{
    // Only release on clients where we successfully acquired
    let clients_to_release: Vec<RedisClient> = acquire_results
        .iter()
        .enumerate()
        .filter(|&(_, &success)| success)
        .map(|(idx, _)| clients[idx].clone())
        .collect();

    if clients_to_release.is_empty() {
        return Ok(()); // Nothing to release
    }

    // Start release attempts on all clients in parallel
    let mut release_tasks: Vec<tokio::task::JoinHandle<LockResult<()>>> = Vec::new();

    for client in &clients_to_release {
        let client_clone = client.clone();
        let try_release_fn_clone = try_release_fn.clone();
        let task = tokio::spawn(async move { try_release_fn_clone(&client_clone).await });
        release_tasks.push(task);
    }

    // Wait for results
    let mut success_count = 0;
    let mut fault_count = 0;
    let mut errors: Vec<LockError> = Vec::new();
    let total_clients = clients_to_release.len();

    // Wait for all tasks to complete
    for task in release_tasks {
        match task.await {
            Ok(Ok(())) => {
                success_count += 1;
            }
            Ok(Err(e)) => {
                fault_count += 1;
                errors.push(e);
                // Continue - we want to try releasing on all clients
            }
            Err(_) => {
                // Task panicked - treat as fault
                fault_count += 1;
            }
        }
    }

    // Check if we achieved majority release
    if RedLockHelper::has_sufficient_successes(success_count, total_clients) {
        // Majority released successfully - that's good enough
        Ok(())
    } else if RedLockHelper::has_too_many_failures_or_faults(fault_count, total_clients) {
        // Too many failures - return error
        if errors.is_empty() {
            Err(LockError::Backend(Box::new(std::io::Error::other(
                "failed to release lock on majority of servers",
            ))))
        } else {
            // Return first error
            Err(errors.into_iter().next().unwrap())
        }
    } else {
        // Some failures but not enough to be fatal - still OK
        Ok(())
    }
}
