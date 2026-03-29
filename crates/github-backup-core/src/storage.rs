// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Storage abstraction for writing backup artefacts.

use std::path::Path;

use serde::Serialize;

use crate::error::CoreError;

/// Abstraction over writing backup artefacts to a persistent store.
///
/// The only production implementation is [`FsStorage`], which writes to the
/// real filesystem. Tests can substitute a no-op or in-memory implementation
/// to avoid touching the filesystem.
pub trait Storage: Send + Sync {
    /// Writes a serialisable value as a pretty-printed JSON file at `path`,
    /// creating parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError`] if directories cannot be created, the value
    /// cannot be serialised, or the file cannot be written.
    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), CoreError>;

    /// Writes raw bytes to `path`, creating parent directories as needed.
    ///
    /// Used for downloading release asset binaries.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError`] if directories cannot be created or the file
    /// cannot be written.
    fn write_bytes(&self, path: &Path, data: &[u8]) -> Result<(), CoreError>;

    /// Returns `true` if the given path already exists.
    fn exists(&self, path: &Path) -> bool;
}

/// Production [`Storage`] implementation backed by the real filesystem.
#[derive(Debug, Clone)]
pub struct FsStorage;

impl FsStorage {
    /// Creates a new [`FsStorage`].
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for FsStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl Storage for FsStorage {
    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), CoreError> {
        ensure_parent(path)?;
        let json = serde_json::to_string_pretty(value)?;
        std::fs::write(path, json.as_bytes()).map_err(|e| CoreError::io(path.display(), e))
    }

    fn write_bytes(&self, path: &Path, data: &[u8]) -> Result<(), CoreError> {
        ensure_parent(path)?;
        std::fs::write(path, data).map_err(|e| CoreError::io(path.display(), e))
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

fn ensure_parent(path: &Path) -> Result<(), CoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CoreError::io(parent.display(), e))?;
    }
    Ok(())
}

/// In-memory [`Storage`] for tests; collects written paths but does not touch
/// the filesystem.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    /// A [`Storage`] implementation that records writes in memory.
    #[derive(Debug, Clone, Default)]
    pub struct MemStorage {
        inner: Arc<Mutex<HashMap<PathBuf, Vec<u8>>>>,
    }

    impl MemStorage {
        /// Returns the stored bytes at `path`, or `None`.
        pub fn get(&self, path: &Path) -> Option<Vec<u8>> {
            self.inner.lock().unwrap().get(path).cloned()
        }

        /// Returns the number of paths that have been written.
        pub fn len(&self) -> usize {
            self.inner.lock().unwrap().len()
        }
    }

    impl Storage for MemStorage {
        fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), CoreError> {
            let json = serde_json::to_vec_pretty(value)?;
            self.inner.lock().unwrap().insert(path.to_path_buf(), json);
            Ok(())
        }

        fn write_bytes(&self, path: &Path, data: &[u8]) -> Result<(), CoreError> {
            self.inner
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), data.to_vec());
            Ok(())
        }

        fn exists(&self, path: &Path) -> bool {
            self.inner.lock().unwrap().contains_key(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn fs_storage_write_json_creates_file_and_parent_dirs() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("sub").join("data.json");
        let storage = FsStorage::new();

        let data = serde_json::json!({"key": "value"});
        storage.write_json(&path, &data).expect("write_json");

        assert!(path.exists());
        let contents = std::fs::read_to_string(&path).expect("read");
        assert!(contents.contains("\"key\""));
    }

    #[test]
    fn fs_storage_write_bytes_creates_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("asset.bin");
        let storage = FsStorage::new();

        storage.write_bytes(&path, b"hello").expect("write_bytes");

        assert_eq!(std::fs::read(&path).expect("read"), b"hello");
    }

    #[test]
    fn fs_storage_exists_returns_false_for_missing_path() {
        let storage = FsStorage::new();
        assert!(!storage.exists(Path::new("/nonexistent/path/file.json")));
    }

    #[test]
    fn fs_storage_exists_returns_true_for_existing_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("exists.txt");
        std::fs::write(&path, b"").expect("create file");
        let storage = FsStorage::new();
        assert!(storage.exists(&path));
    }
}
