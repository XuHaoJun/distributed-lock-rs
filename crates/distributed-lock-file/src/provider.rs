//! File lock provider implementation.

use std::path::{Path, PathBuf};

use distributed_lock_core::error::{LockError, LockResult};
use distributed_lock_core::traits::LockProvider;

use crate::lock::FileDistributedLock;

/// Builder for file-based lock provider configuration.
pub struct FileLockProviderBuilder {
    directory: Option<PathBuf>,
}

impl FileLockProviderBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Self { directory: None }
    }

    /// Sets the directory for lock files.
    ///
    /// The directory will be created if it doesn't exist.
    pub fn directory(mut self, path: impl Into<PathBuf>) -> Self {
        self.directory = Some(path.into());
        self
    }

    /// Builds the provider.
    ///
    /// # Errors
    ///
    /// Returns an error if no directory is specified or if the directory
    /// cannot be created.
    pub fn build(self) -> LockResult<FileLockProvider> {
        let directory = self
            .directory
            .ok_or_else(|| LockError::InvalidName("directory not specified".to_string()))?;

        // Ensure directory exists
        std::fs::create_dir_all(&directory)
            .map_err(|e| LockError::InvalidName(format!("failed to create directory: {e}")))?;

        Ok(FileLockProvider { directory })
    }
}

impl Default for FileLockProviderBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Provider for file-based distributed locks.
///
/// Uses OS-level file locking to provide mutual exclusion across processes.
/// Works on any POSIX system (Linux, macOS) and Windows.
pub struct FileLockProvider {
    directory: PathBuf,
}

impl FileLockProvider {
    /// Returns a new builder for configuring the provider.
    pub fn builder() -> FileLockProviderBuilder {
        FileLockProviderBuilder::new()
    }

    /// Creates a provider using the specified directory.
    ///
    /// Convenience method for simple use cases.
    pub fn new(directory: impl Into<PathBuf>) -> LockResult<Self> {
        Self::builder().directory(directory).build()
    }

    /// Returns the directory where lock files are stored.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl LockProvider for FileLockProvider {
    type Lock = FileDistributedLock;

    fn create_lock(&self, name: &str) -> Self::Lock {
        FileDistributedLock::new(&self.directory, name)
            .expect("failed to create lock from provider")
    }
}
