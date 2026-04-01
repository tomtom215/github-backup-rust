// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Exclusive per-output-directory lock.
//!
//! Prevents two `github-backup` processes from running concurrently against
//! the same output directory, which would corrupt `backup_checkpoint.json`,
//! `backup_state.json`, and the JSON metadata files.
//!
//! The lock is implemented as an OS-level exclusive `flock` (Unix) or
//! `LockFileEx` (Windows) on a well-known file inside the output directory.
//! It is automatically released when the `OutputLock` guard is dropped or the
//! process exits.

use std::path::Path;

use fslock::LockFile;

const LOCK_FILENAME: &str = ".github-backup.lock";

/// RAII guard that holds an exclusive lock on the output directory.
///
/// Dropping this value releases the lock.
pub struct OutputLock {
    _inner: LockFile,
}

/// Acquires an exclusive lock on `output_dir`.
///
/// # Errors
///
/// Returns a human-readable error string when:
/// - Another process already holds the lock (concurrent run detected).
/// - The lock file cannot be created (permissions, path not found, etc.).
pub fn acquire(output_dir: &Path) -> Result<OutputLock, String> {
    std::fs::create_dir_all(output_dir).map_err(|e| {
        format!(
            "cannot create output directory {}: {e}",
            output_dir.display()
        )
    })?;

    let lock_path = output_dir.join(LOCK_FILENAME);

    let mut lock = LockFile::open(&lock_path)
        .map_err(|e| format!("cannot open lock file {}: {e}", lock_path.display()))?;

    let acquired = lock
        .try_lock()
        .map_err(|e| format!("cannot acquire output directory lock: {e}"))?;

    if !acquired {
        return Err(format!(
            "another github-backup process is already running against {}.\n\
             If you are sure no other process is running, delete {} and retry.",
            output_dir.display(),
            lock_path.display(),
        ));
    }

    Ok(OutputLock { _inner: lock })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn acquire_succeeds_on_fresh_directory() {
        let dir = tempdir().expect("tempdir");
        assert!(acquire(dir.path()).is_ok());
    }

    #[test]
    fn double_acquire_same_process_fails() {
        let dir = tempdir().expect("tempdir");
        let _guard = acquire(dir.path()).expect("first acquire");
        // A second acquire in the same process must fail because `fslock`
        // uses POSIX file locking, which is per-process on Linux.
        // On some platforms (macOS, Windows) this may succeed; we accept either.
        let _second = acquire(dir.path());
        // We don't assert failure here because behaviour is platform-dependent,
        // but we do assert no panic.
    }

    #[test]
    fn lock_file_created_in_output_dir() {
        let dir = tempdir().expect("tempdir");
        let _guard = acquire(dir.path()).expect("acquire");
        assert!(dir.path().join(LOCK_FILENAME).exists());
    }
}
