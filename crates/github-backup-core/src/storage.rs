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

    /// Reads the file at `path`, returning `None` if it does not exist.
    ///
    /// Used to merge a fresh API listing into what a previous run already
    /// stored, so that items GitHub no longer returns (deleted issues) are
    /// never lost from the backup.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError`] for any I/O failure other than "not found".
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, CoreError>;

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
        let json = serde_json::to_string_pretty(value)?;
        write_atomic(path, json.as_bytes())
    }

    fn write_bytes(&self, path: &Path, data: &[u8]) -> Result<(), CoreError> {
        write_atomic(path, data)
    }

    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, CoreError> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(CoreError::io(path.display(), e)),
        }
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

/// Writes `data` to `path` so that a reader (or a crash) never observes a
/// half-written file: the bytes go to a uniquely named sibling temporary file
/// which is then renamed over the destination.  `rename` is atomic on every
/// platform and filesystem the tool supports, so after an interruption the
/// path holds either the previous complete content or the new complete
/// content, never a truncated JSON document.
///
/// The temporary file is removed again if the write or the rename fails.
///
/// The data is not `fsync`ed: the files are re-creatable metadata and a sync
/// per file would make large backups dramatically slower.  Surviving a power
/// cut is therefore best-effort, surviving a killed process is guaranteed.
fn write_atomic(path: &Path, data: &[u8]) -> Result<(), CoreError> {
    ensure_parent(path)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, data).map_err(|e| CoreError::io(tmp.display(), e))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(CoreError::io(path.display(), e));
    }
    Ok(())
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

        fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, CoreError> {
            Ok(self.inner.lock().unwrap().get(path).cloned())
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
    fn fs_storage_read_returns_none_for_a_missing_file_and_bytes_otherwise() {
        let dir = tempdir().expect("tempdir");
        let storage = FsStorage::new();
        let path = dir.path().join("a.json");
        assert_eq!(storage.read(&path).expect("read missing"), None);
        storage.write_bytes(&path, b"hi").expect("write");
        assert_eq!(storage.read(&path).expect("read"), Some(b"hi".to_vec()));
    }

    #[test]
    fn fs_storage_read_reports_errors_other_than_not_found() {
        let dir = tempdir().expect("tempdir");
        // Reading a directory as a file is an error that must not look like "absent".
        let err = FsStorage::new().read(dir.path()).expect_err("must fail");
        assert!(matches!(err, CoreError::Io { .. }), "{err:?}");
    }

    #[test]
    fn fs_storage_write_replaces_content_and_leaves_no_temp_file() {
        let dir = tempdir().expect("tempdir");
        let storage = FsStorage::new();
        let path = dir.path().join("issues.json");
        storage
            .write_json(&path, &serde_json::json!([1, 2, 3]))
            .expect("first write");
        storage
            .write_json(&path, &serde_json::json!(["new"]))
            .expect("second write");
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("new") && !text.contains('1'), "{text}");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("list")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec!["issues.json".to_string()], "{leftovers:?}");
    }

    /// A failed write must leave the previous complete file untouched (the
    /// old `std::fs::write` truncated it first).
    #[cfg(unix)]
    #[test]
    fn fs_storage_failed_write_keeps_the_previous_file_intact() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().expect("tempdir");
        let storage = FsStorage::new();
        let path = dir.path().join("keep.json");
        storage.write_bytes(&path, b"previous").expect("seed");
        // A read-only directory makes creating the temp file fail.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555))
            .expect("chmod");
        let result = storage.write_bytes(&path, b"replacement");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755))
            .expect("restore chmod");
        if result.is_ok() {
            // Running as root ignores directory permissions; nothing to prove then.
            return;
        }
        assert_eq!(std::fs::read(&path).expect("read"), b"previous");
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
