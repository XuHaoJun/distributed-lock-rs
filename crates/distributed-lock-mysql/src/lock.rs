//! MySQL distributed lock implementation using GET_LOCK/RELEASE_LOCK.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::DistributedLock;

use sqlx::Row;

use crate::handle::MySqlLockHandle;
use crate::name::encode_lock_name;

/// A MySQL-based distributed mutual exclusion lock.
///
/// Uses MySQL's GET_LOCK and RELEASE_LOCK functions to provide distributed
/// locking across multiple processes and machines.
pub struct MySqlDistributedLock {
    /// The original lock name.
    name: String,
    /// The encoded lock name safe for MySQL.
    encoded_name: String,
    /// MySQL connection pool.
    pool: sqlx::MySqlPool,
    /// Keepalive cadence for long-held locks.
    keepalive_cadence: Option<Duration>,
}

impl MySqlDistributedLock {
    /// Creates a new MySQL distributed lock.
    pub(crate) fn new(
        name: String,
        pool: sqlx::MySqlPool,
        keepalive_cadence: Option<Duration>,
    ) -> Self {
        let encoded_name = encode_lock_name(&name);
        Self {
            name,
            encoded_name,
            pool,
            keepalive_cadence,
        }
    }
}

impl DistributedLock for MySqlDistributedLock {
    type Handle = MySqlLockHandle;

    fn name(&self) -> &str {
        &self.name
    }

    async fn acquire(&self, timeout: Option<Duration>) -> LockResult<Self::Handle> {
        let timeout_seconds = match timeout {
            Some(duration) => duration.as_secs_f64(),
            None => -1.0, // MySQL uses -1 for infinite timeout
        };

        // Get a dedicated connection for this lock
        let connection = self
            .pool
            .acquire()
            .await
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        // Guard to ensure we cleanup if the future is cancelled
        let mut guard = AcquireGuard {
            lock_name: self.encoded_name.clone(),
            connection: Some(connection),
        };

        let row = sqlx::query("SELECT GET_LOCK(?, ?)")
            .bind(&self.encoded_name)
            .bind(timeout_seconds)
            .fetch_one(&mut **guard.connection.as_mut().unwrap())
            .await
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        let lock_result: i64 = row
            .try_get(0)
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        match lock_result {
            1 => {
                // Lock acquired successfully
                Ok(MySqlLockHandle::new(
                    self.encoded_name.clone(),
                    guard.take_connection(), // Take connection safely
                    self.keepalive_cadence,
                ))
            }
            0 => {
                // Lock not acquired (timeout or deadlock)
                // We disarm the guard because we know we don't hold the lock,
                // so the connection is safe to return to the pool.
                let _ = guard.take_connection();

                Err(LockError::Timeout(
                    timeout.unwrap_or(Duration::from_secs(30)),
                ))
            }
            _ => {
                // Unexpected result
                let _ = guard.take_connection(); // Safe to return connection? Maybe tainted, but we assume no lock.
                Err(LockError::Connection(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Unexpected GET_LOCK result: {}", lock_result),
                ))))
            }
        }
    }

    async fn try_acquire(&self) -> LockResult<Option<Self::Handle>> {
        // Get a dedicated connection for this lock
        let connection = self
            .pool
            .acquire()
            .await
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        // Guard to ensure we cleanup if the future is cancelled
        let mut guard = AcquireGuard {
            lock_name: self.encoded_name.clone(),
            connection: Some(connection),
        };

        let row = sqlx::query("SELECT GET_LOCK(?, 0)")
            .bind(&self.encoded_name)
            .fetch_one(&mut **guard.connection.as_mut().unwrap())
            .await
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        let lock_result: i64 = row
            .try_get(0)
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        match lock_result {
            1 => {
                // Lock acquired successfully
                Ok(Some(MySqlLockHandle::new(
                    self.encoded_name.clone(),
                    guard.take_connection(),
                    self.keepalive_cadence,
                )))
            }
            0 => {
                // Lock not acquired
                let _ = guard.take_connection();
                Ok(None)
            }
            _ => {
                // Unexpected result
                let _ = guard.take_connection();
                Err(LockError::Connection(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Unexpected GET_LOCK result: {}", lock_result),
                ))))
            }
        }
    }
}

/// Guard that holds the connection during acquisition.
/// If dropped (e.g. cancelled), it spawns a task to release the lock/connection.
struct AcquireGuard {
    lock_name: String,
    connection: Option<sqlx::pool::PoolConnection<sqlx::MySql>>,
}

impl AcquireGuard {
    fn take_connection(&mut self) -> sqlx::pool::PoolConnection<sqlx::MySql> {
        self.connection.take().expect("Connection already taken")
    }
}

impl Drop for AcquireGuard {
    fn drop(&mut self) {
        if let Some(mut connection) = self.connection.take() {
            let lock_name = self.lock_name.clone();
            // We were dropped with the connection still inside.
            // This implies subsequent code (success/fail check) didn't run.
            // We might be holding the lock, or in the queue for it.
            // Best safety: Release the lock.
            tokio::spawn(async move {
                let _ = sqlx::query("SELECT RELEASE_LOCK(?)")
                    .bind(lock_name)
                    .execute(&mut *connection)
                    .await;
                // Connection returns to pool here
            });
        }
    }
}
