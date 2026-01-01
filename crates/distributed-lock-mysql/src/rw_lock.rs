//! MySQL distributed reader-writer lock implementation.

use std::time::Duration;

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::DistributedReaderWriterLock;

use sqlx::Row;

use crate::name::encode_lock_name;

/// State of a MySQL reader-writer lock stored in the database.
#[derive(Debug, Clone)]
struct LockState {
    /// Number of active readers.
    reader_count: i32,
    /// Whether a writer holds the lock (1 = held, 0 = not held).
    writer_held: i32,
    /// Version for optimistic locking.
    #[allow(dead_code)]
    version: i32,
}

/// A MySQL-based distributed reader-writer lock.
///
/// Uses a database table to track reader-writer state and transactions
/// to ensure atomic operations. This allows multiple readers to hold
/// the lock simultaneously while ensuring writers get exclusive access.
///
/// The table schema is:
/// ```sql
/// CREATE TABLE distributed_locks (
///     lock_name VARCHAR(255) PRIMARY KEY,
///     reader_count INT NOT NULL DEFAULT 0,
///     writer_held TINYINT(1) NOT NULL DEFAULT 0,
///     version INT NOT NULL DEFAULT 0,
///     created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
///     updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP
/// );
/// ```
pub struct MySqlDistributedReaderWriterLock {
    /// The original lock name.
    name: String,
    /// The encoded lock name used as database key.
    encoded_name: String,
    /// MySQL connection pool.
    pool: sqlx::MySqlPool,
    /// Keepalive cadence for long-held locks.
    keepalive_cadence: Option<Duration>,
}

impl MySqlDistributedReaderWriterLock {
    /// Creates a new MySQL distributed reader-writer lock.
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

    /// Ensures the lock table exists in the database.
    async fn ensure_table_exists(&self) -> LockResult<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS distributed_locks (
                lock_name VARCHAR(255) PRIMARY KEY,
                reader_count INT NOT NULL DEFAULT 0,
                writer_held TINYINT(1) NOT NULL DEFAULT 0,
                version INT NOT NULL DEFAULT 0,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| LockError::Connection(Box::new(e)))?;
        Ok(())
    }

    /// Gets the current lock state from the database.
    async fn get_lock_state(&self) -> LockResult<LockState> {
        self.ensure_table_exists().await?;

        let result = sqlx::query(
            "SELECT reader_count, writer_held, version FROM distributed_locks WHERE lock_name = ?",
        )
        .bind(&self.encoded_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| LockError::Connection(Box::new(e)))?;

        match result {
            Some(row) => Ok(LockState {
                reader_count: row
                    .try_get(0)
                    .map_err(|e| LockError::Connection(Box::new(e)))?,
                writer_held: row
                    .try_get(1)
                    .map_err(|e| LockError::Connection(Box::new(e)))?,
                version: row
                    .try_get(2)
                    .map_err(|e| LockError::Connection(Box::new(e)))?,
            }),
            None => Ok(LockState {
                reader_count: 0,
                writer_held: 0,
                version: 0,
            }),
        }
    }

    /// Attempts to acquire a read lock using database transactions.
    async fn try_acquire_read_internal(&self) -> LockResult<Option<MySqlReadLockHandle>> {
        self.ensure_table_exists().await?;

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        // Check if writer holds the lock
        let state = self.get_lock_state().await?;

        if state.writer_held != 0 {
            // Writer holds the lock, cannot acquire read lock
            transaction
                .rollback()
                .await
                .map_err(|e| LockError::Connection(Box::new(e)))?;
            return Ok(None);
        }

        // Insert or update the lock record to increment reader count
        let result = sqlx::query(
            r#"
            INSERT INTO distributed_locks (lock_name, reader_count, writer_held, version)
            VALUES (?, 1, 0, 1)
            ON DUPLICATE KEY UPDATE
                reader_count = reader_count + 1,
                version = version + 1
            "#,
        )
        .bind(&self.encoded_name)
        .execute(&mut *transaction)
        .await;

        match result {
            Ok(_) => {
                transaction
                    .commit()
                    .await
                    .map_err(|e| LockError::Connection(Box::new(e)))?;

                Ok(Some(MySqlReadLockHandle::new(
                    self.encoded_name.clone(),
                    self.pool.clone(),
                    self.keepalive_cadence,
                )))
            }
            Err(e) => {
                let error = LockError::Connection(Box::new(e));
                transaction
                    .rollback()
                    .await
                    .map_err(|rollback_e| LockError::Connection(Box::new(rollback_e)))?;
                Err(error)
            }
        }
    }

    /// Attempts to acquire a write lock using database transactions.
    async fn try_acquire_write_internal(&self) -> LockResult<Option<MySqlWriteLockHandle>> {
        self.ensure_table_exists().await?;

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|e| LockError::Connection(Box::new(e)))?;

        // Check if any readers or writers hold the lock
        let state = self.get_lock_state().await?;

        if state.reader_count > 0 || state.writer_held != 0 {
            // Lock is held by readers or writer
            transaction
                .rollback()
                .await
                .map_err(|e| LockError::Connection(Box::new(e)))?;
            return Ok(None);
        }

        // Acquire the write lock - first check if we can acquire it
        let check_result = sqlx::query(
            "SELECT reader_count, writer_held FROM distributed_locks WHERE lock_name = ?",
        )
        .bind(&self.encoded_name)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|e| LockError::Connection(Box::new(e)))?;

        let can_acquire = match check_result {
            Some(row) => {
                let reader_count: i32 = row
                    .try_get(0)
                    .map_err(|e| LockError::Connection(Box::new(e)))?;
                let writer_held: i32 = row
                    .try_get(1)
                    .map_err(|e| LockError::Connection(Box::new(e)))?;
                reader_count == 0 && writer_held == 0
            }
            None => true, // Lock doesn't exist yet, we can create it
        };

        if !can_acquire {
            transaction
                .rollback()
                .await
                .map_err(|e| LockError::Connection(Box::new(e)))?;
            return Ok(None);
        }

        // Now acquire the write lock
        let result = sqlx::query(
            r#"
            INSERT INTO distributed_locks (lock_name, reader_count, writer_held, version)
            VALUES (?, 0, 1, 1)
            ON DUPLICATE KEY UPDATE
                writer_held = VALUES(writer_held),
                version = version + 1
            "#,
        )
        .bind(&self.encoded_name)
        .execute(&mut *transaction)
        .await;

        match result {
            Ok(result) => {
                if result.rows_affected() > 0 {
                    transaction
                        .commit()
                        .await
                        .map_err(|e| LockError::Connection(Box::new(e)))?;

                    Ok(Some(MySqlWriteLockHandle::new(
                        self.encoded_name.clone(),
                        self.pool.clone(),
                        self.keepalive_cadence,
                    )))
                } else {
                    // Could not acquire (condition not met)
                    transaction
                        .rollback()
                        .await
                        .map_err(|e| LockError::Connection(Box::new(e)))?;
                    Ok(None)
                }
            }
            Err(e) => {
                let error = LockError::Connection(Box::new(e));
                transaction
                    .rollback()
                    .await
                    .map_err(|rollback_e| LockError::Connection(Box::new(rollback_e)))?;
                Err(error)
            }
        }
    }
}

impl DistributedReaderWriterLock for MySqlDistributedReaderWriterLock {
    type ReadHandle = MySqlReadLockHandle;
    type WriteHandle = MySqlWriteLockHandle;

    fn name(&self) -> &str {
        &self.name
    }

    async fn acquire_read(&self, timeout: Option<Duration>) -> LockResult<Self::ReadHandle> {
        let start_time = std::time::Instant::now();

        loop {
            match self.try_acquire_read_internal().await? {
                Some(handle) => return Ok(handle),
                None => {
                    // Check if we've exceeded the timeout
                    if let Some(timeout_duration) = timeout
                        && start_time.elapsed() >= timeout_duration
                    {
                        return Err(LockError::Timeout(timeout_duration));
                    }

                    // Wait a bit before retrying
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    async fn try_acquire_read(&self) -> LockResult<Option<Self::ReadHandle>> {
        self.try_acquire_read_internal().await
    }

    async fn acquire_write(&self, timeout: Option<Duration>) -> LockResult<Self::WriteHandle> {
        let start_time = std::time::Instant::now();

        loop {
            match self.try_acquire_write_internal().await? {
                Some(handle) => return Ok(handle),
                None => {
                    // Check if we've exceeded the timeout
                    if let Some(timeout_duration) = timeout
                        && start_time.elapsed() >= timeout_duration
                    {
                        return Err(LockError::Timeout(timeout_duration));
                    }

                    // Wait a bit before retrying
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    async fn try_acquire_write(&self) -> LockResult<Option<Self::WriteHandle>> {
        self.try_acquire_write_internal().await
    }
}

/// Handle for a MySQL read lock.
pub struct MySqlReadLockHandle {
    lock_name: String,
    pool: sqlx::MySqlPool,
    lost_sender: tokio::sync::watch::Sender<bool>,
    lost_receiver: tokio::sync::watch::Receiver<bool>,
    keepalive_handle: Option<tokio::task::JoinHandle<()>>,
}

impl MySqlReadLockHandle {
    pub(crate) fn new(
        lock_name: String,
        pool: sqlx::MySqlPool,
        keepalive_cadence: Option<Duration>,
    ) -> Self {
        let (lost_token_tx, lost_receiver) = tokio::sync::watch::channel(false);

        let lost_token_tx_clone = lost_token_tx.clone();
        let keepalive_handle = keepalive_cadence.map(|cadence| {
            let pool_clone = pool.clone();
            let mut lost_token_rx_clone = lost_token_tx_clone.subscribe();

            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(cadence) => {
                            // Run a keepalive query
                            let result = sqlx::query("SELECT 1")
                                .execute(&pool_clone)
                                .await;

                            if result.is_err() {
                                // Connection failed, signal lock loss
                                let _ = lost_token_tx_clone.send(true);
                                break;
                            }
                        }
                        _ = lost_token_rx_clone.changed() => {
                            // Lock was released, stop keepalive
                            break;
                        }
                    }
                }
            })
        });

        Self {
            lock_name,
            pool,
            lost_sender: lost_token_tx,
            lost_receiver,
            keepalive_handle,
        }
    }
}

impl distributed_lock_core::traits::LockHandle for MySqlReadLockHandle {
    fn lost_token(&self) -> &tokio::sync::watch::Receiver<bool> {
        &self.lost_receiver
    }

    async fn release(self) -> LockResult<()> {
        // Stop keepalive task
        if let Some(handle) = &self.keepalive_handle {
            handle.abort();
        }

        // Signal that the lock is being released
        let _ = self.lost_sender.send(true);

        // Decrement reader count in database
        let result = sqlx::query(
            "UPDATE distributed_locks SET reader_count = GREATEST(reader_count - 1, 0), version = version + 1 WHERE lock_name = ?"
        )
        .bind(&self.lock_name)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(distributed_lock_core::error::LockError::Connection(
                Box::new(e),
            )),
        }
    }
}

impl Drop for MySqlReadLockHandle {
    fn drop(&mut self) {
        // Signal that the lock is lost (being dropped)
        let _ = self.lost_sender.send(true);

        // Stop keepalive task
        if let Some(handle) = self.keepalive_handle.take() {
            handle.abort();
        }
    }
}

/// Handle for a MySQL write lock.
pub struct MySqlWriteLockHandle {
    lock_name: String,
    pool: sqlx::MySqlPool,
    lost_sender: tokio::sync::watch::Sender<bool>,
    lost_receiver: tokio::sync::watch::Receiver<bool>,
    keepalive_handle: Option<tokio::task::JoinHandle<()>>,
}

impl MySqlWriteLockHandle {
    pub(crate) fn new(
        lock_name: String,
        pool: sqlx::MySqlPool,
        keepalive_cadence: Option<Duration>,
    ) -> Self {
        let (lost_token_tx, lost_receiver) = tokio::sync::watch::channel(false);

        let lost_token_tx_clone = lost_token_tx.clone();
        let keepalive_handle = keepalive_cadence.map(|cadence| {
            let pool_clone = pool.clone();
            let mut lost_token_rx_clone = lost_token_tx_clone.subscribe();

            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(cadence) => {
                            // Run a keepalive query
                            let result = sqlx::query("SELECT 1")
                                .execute(&pool_clone)
                                .await;

                            if result.is_err() {
                                // Connection failed, signal lock loss
                                let _ = lost_token_tx_clone.send(true);
                                break;
                            }
                        }
                        _ = lost_token_rx_clone.changed() => {
                            // Lock was released, stop keepalive
                            break;
                        }
                    }
                }
            })
        });

        Self {
            lock_name,
            pool,
            lost_sender: lost_token_tx,
            lost_receiver,
            keepalive_handle,
        }
    }
}

impl distributed_lock_core::traits::LockHandle for MySqlWriteLockHandle {
    fn lost_token(&self) -> &tokio::sync::watch::Receiver<bool> {
        &self.lost_receiver
    }

    async fn release(self) -> LockResult<()> {
        // Stop keepalive task
        if let Some(handle) = &self.keepalive_handle {
            handle.abort();
        }

        // Signal that the lock is being released
        let _ = self.lost_sender.send(true);

        // Release the write lock in database
        let result = sqlx::query(
            "UPDATE distributed_locks SET writer_held = 0, version = version + 1 WHERE lock_name = ?"
        )
        .bind(&self.lock_name)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(distributed_lock_core::error::LockError::Connection(
                Box::new(e),
            )),
        }
    }
}

impl Drop for MySqlWriteLockHandle {
    fn drop(&mut self) {
        // Signal that the lock is lost (being dropped)
        let _ = self.lost_sender.send(true);

        // Stop keepalive task
        if let Some(handle) = self.keepalive_handle.take() {
            handle.abort();
        }
    }
}
