// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Advisory lock file that prevents two backup processes from running
//! concurrently against the same output directory.
//!
//! # Design
//!
//! [`BackupLock`] creates `<owner-json-dir>/.backup.lock` using
//! `O_CREAT | O_EXCL`, which is atomic on POSIX filesystems.  The lock file
//! contains the current process PID so that a stale lock from a crashed
//! process can be detected.
//!
//! When the guard is dropped the lock file is removed.  If the process crashes
//! before the guard is dropped (power loss, SIGKILL, panic) the lock file will
//! remain on disk.  On the next run:
//!
//! - The PID stored in the file is read.
//! - If no process with that PID exists the stale lock is deleted and the new
//!   run proceeds.  Liveness is checked via `/proc/<pid>` on Linux and via
//!   `kill(pid, 0)` on other Unix platforms.
//! - If a process with that PID exists, the new run is aborted with an error.
//!
//! On non-Unix platforms (Windows) stale-PID detection falls back to a
//! permissive "delete and proceed" strategy; the atomic `O_CREAT | O_EXCL`
//! still prevents two *concurrent* processes from racing.

use std::path::{Path, PathBuf};

use tracing::{debug, warn};

/// RAII guard that holds a backup lock file for its lifetime.
///
/// Create via [`BackupLock::acquire`].  The lock file is removed when this
/// value is dropped.
#[derive(Debug)]
pub struct BackupLock {
    path: PathBuf,
}

/// Errors that can occur when acquiring a backup lock.
#[derive(Debug)]
pub enum LockError {
    /// Another backup process is already running.
    AlreadyRunning {
        /// PID of the existing process, if readable.
        pid: Option<u32>,
    },
    /// The lock file directory could not be created.
    DirCreate(std::io::Error),
    /// The lock file could not be written.
    Write(std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning { pid: Some(p) } => {
                write!(f, "another backup is already running (PID {p})")
            }
            Self::AlreadyRunning { pid: None } => {
                write!(f, "another backup is already running (lock file exists)")
            }
            Self::DirCreate(e) => write!(f, "could not create lock directory: {e}"),
            Self::Write(e) => write!(f, "could not write lock file: {e}"),
        }
    }
}

impl BackupLock {
    /// Acquires the lock for `json_dir`.
    ///
    /// Creates `json_dir/.backup.lock` exclusively.  If the file already
    /// exists but the recorded PID is no longer alive the stale lock is
    /// removed and acquisition proceeds normally.
    ///
    /// # Errors
    ///
    /// Returns [`LockError::AlreadyRunning`] if another live process holds the
    /// lock, or I/O errors if the directory/file cannot be created.
    pub fn acquire(json_dir: &Path) -> Result<Self, LockError> {
        if let Some(parent) = json_dir.parent() {
            std::fs::create_dir_all(parent).map_err(LockError::DirCreate)?;
        }
        std::fs::create_dir_all(json_dir).map_err(LockError::DirCreate)?;

        let path = json_dir.join(".backup.lock");
        let pid = std::process::id();
        let content = pid.to_string();

        // Attempt exclusive creation.
        match try_create_exclusive(&path, &content) {
            Ok(()) => {
                debug!(path = %path.display(), pid, "backup lock acquired");
                return Ok(Self { path });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Lock file exists.  Check if the owning process is alive.
            }
            Err(e) => return Err(LockError::Write(e)),
        }

        // Lock file exists — read the stored PID.
        let stored_pid = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok());

        if let Some(stale_pid) = stored_pid {
            if is_process_alive(stale_pid) {
                return Err(LockError::AlreadyRunning {
                    pid: Some(stale_pid),
                });
            }
            // Process is dead — stale lock.
            warn!(
                pid = stale_pid,
                path = %path.display(),
                "removing stale backup lock from dead process"
            );
            let _ = std::fs::remove_file(&path);
        } else {
            // Cannot determine staleness — remove cautiously and proceed.
            warn!(
                path = %path.display(),
                "backup lock file exists with unreadable PID; removing and proceeding"
            );
            let _ = std::fs::remove_file(&path);
        }

        // Retry now that the stale lock is gone.
        try_create_exclusive(&path, &content).map_err(LockError::Write)?;
        debug!(path = %path.display(), pid, "backup lock acquired (after stale removal)");
        Ok(Self { path })
    }
}

impl Drop for BackupLock {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.path) {
            // Warn but never panic in a Drop impl.
            warn!(
                path = %self.path.display(),
                error = %e,
                "failed to remove backup lock file"
            );
        } else {
            debug!(path = %self.path.display(), "backup lock released");
        }
    }
}

/// Creates `path` exclusively (fails if it already exists) and writes `content`.
fn try_create_exclusive(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

/// Returns `true` if a process with `pid` is currently running.
///
/// - **Linux**: checks `/proc/<pid>` (efficient, no syscall round-trip).
/// - **Other Unix** (macOS, FreeBSD, …): uses POSIX `kill(pid, 0)` which
///   probes process existence without delivering any signal.
/// - **Windows / other**: falls back to `false` (assume dead).
fn is_process_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // PID 0 is never a real user-space process.  kill(0, sig) is a POSIX
        // special case that signals the entire process group — it would return
        // 0 (success) for any running process, giving a false positive.
        if pid == 0 {
            return false;
        }
        // POSIX kill(pid, 0): returns 0 if the process exists and we have
        // permission to signal it; returns -1 (ESRCH) if not found.
        // pid_t is i32 on all Unix platforms we support.
        extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        // SAFETY: signal 0 is never delivered; this only checks existence.
        unsafe { kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn acquire_creates_lock_file() {
        let dir = tempdir().unwrap();
        let lock = BackupLock::acquire(dir.path()).expect("acquire lock");
        assert!(dir.path().join(".backup.lock").exists());
        drop(lock);
        assert!(!dir.path().join(".backup.lock").exists());
    }

    #[test]
    fn acquire_fails_when_lock_held_by_live_process() {
        let dir = tempdir().unwrap();
        let _lock = BackupLock::acquire(dir.path()).expect("first acquire");

        // Second acquire should fail because our own PID is alive.
        let result = BackupLock::acquire(dir.path());
        assert!(
            result.is_err(),
            "second acquire should fail while first lock is held"
        );
    }

    #[test]
    fn acquire_clears_stale_lock_with_dead_pid() {
        let dir = tempdir().unwrap();
        // Write a lock file with a PID that is guaranteed not to exist.
        // PID 0 is never a valid user-space process.
        std::fs::write(dir.path().join(".backup.lock"), b"0").unwrap();

        // Should succeed by removing the stale lock.
        let lock = BackupLock::acquire(dir.path()).expect("acquire after stale removal");
        drop(lock);
    }
}
