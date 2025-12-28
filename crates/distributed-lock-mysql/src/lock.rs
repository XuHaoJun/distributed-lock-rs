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
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        let result = sqlx::query("SELECT GET_LOCK(?, ?)")
            .bind(&self.encoded_name)
            .bind(timeout_seconds)
            .fetch_one(&mut *connection)
            .await;

        match result {
            Ok(row) => {
                let lock_result: i64 = row
                    .try_get(0)
                    .map_err(|e| LockError::Connection(Box::new(e)))?;
                match lock_result {
                    1 => {
                        // Lock acquired successfully
                        Ok(MySqlLockHandle::new(
                            self.encoded_name.clone(),
                            connection,
                            self.keepalive_cadence,
                        ))
                    }
                    0 => {
                        // Lock not acquired (timeout or deadlock)
                        // For infinite timeout (-1), this shouldn't happen, so use a default
                        Err(LockError::Timeout(
                            timeout.unwrap_or(Duration::from_secs(30)),
                        ))
                    }
                    _ => {
                        // Unexpected result
                        Err(LockError::Connection(Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Unexpected GET_LOCK result: {}", lock_result),
                        ))))
                    }
                }
            }
            Err(e) => Err(LockError::Connection(Box::new(e))),
        }
    }

    async fn try_acquire(&self) -> LockResult<Option<Self::Handle>> {
        // Get a dedicated connection for this lock
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        let result = sqlx::query("SELECT GET_LOCK(?, 0)")
            .bind(&self.encoded_name)
            .fetch_one(&mut *connection)
            .await;

        match result {
            Ok(row) => {
                let lock_result: i64 = row
                    .try_get(0)
                    .map_err(|e| LockError::Connection(Box::new(e)))?;
                match lock_result {
                    1 => {
                        // Lock acquired successfully
                        Ok(Some(MySqlLockHandle::new(
                            self.encoded_name.clone(),
                            connection,
                            self.keepalive_cadence,
                        )))
                    }
                    0 => {
                        // Lock not acquired
                        Ok(None)
                    }
                    _ => {
                        // Unexpected result
                        Err(LockError::Connection(Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Unexpected GET_LOCK result: {}", lock_result),
                        ))))
                    }
                }
            }
            Err(e) => Err(LockError::Connection(Box::new(e))),
        }
    }
}
