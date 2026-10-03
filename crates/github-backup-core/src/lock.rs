// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Advisory lock that prevents two backup processes from running concurrently
//! against the same owner directory.
//!
//! # Design
//!
//! [`BackupLock`] holds an OS-level exclusive lock (`flock` on Unix,
//! `LockFileEx` on Windows, via `fslock`) on `<owner-json-dir>/.backup.lock`.
//! The kernel releases such a lock the moment the holding process dies — however
//! it dies (`SIGKILL`, OOM kill, power loss, container stop) — so there is **no
//! stale-lock state to detect and nothing to clean up by hand**.  An earlier
//! design stored a PID in the file and probed whether that PID was alive, which
//! reported a lock as held whenever an unrelated process (or PID 1 in a fresh
//! container) happened to reuse the number.
//!
//! The file itself is left in place when the guard is dropped; removing it would
//! race with a process that has just opened it.  It records the PID of the last
//! holder purely so the "already running" message can name it.

use std::io::Write;
use std::path::{Path, PathBuf};

use fslock::LockFile;
use tracing::debug;

/// RAII guard that holds the lock for its lifetime.
///
/// Create via [`BackupLock::acquire`].  The lock is released when this value is
/// dropped or the process exits.
#[derive(Debug)]
pub struct BackupLock {
    path: PathBuf,
    _file: LockFile,
}

/// Errors that can occur when acquiring a backup lock.
#[derive(Debug)]
pub enum LockError {
    /// Another backup process is already running.
    AlreadyRunning {
        /// PID recorded by the holder, if readable (informational only).
        pid: Option<u32>,
    },
    /// The lock file directory could not be created.
    DirCreate(std::io::Error),
    /// The lock file could not be opened or locked.
    Write(std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRunning { pid: Some(p) } => {
                write!(f, "another backup is already running (PID {p})")
            }
            Self::AlreadyRunning { pid: None } => {
                write!(f, "another backup is already running (lock file is held)")
            }
            Self::DirCreate(e) => write!(f, "could not create lock directory: {e}"),
            Self::Write(e) => write!(f, "could not write lock file: {e}"),
        }
    }
}

impl BackupLock {
    /// Acquires the lock for `json_dir`.
    ///
    /// # Errors
    ///
    /// Returns [`LockError::AlreadyRunning`] if another live process holds the
    /// lock, or I/O errors if the directory/file cannot be created or locked.
    pub fn acquire(json_dir: &Path) -> Result<Self, LockError> {
        std::fs::create_dir_all(json_dir).map_err(LockError::DirCreate)?;

        let path = json_dir.join(".backup.lock");
        let mut file = LockFile::open(&path).map_err(LockError::Write)?;
        let acquired = file.try_lock().map_err(LockError::Write)?;
        if !acquired {
            let pid = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok());
            return Err(LockError::AlreadyRunning { pid });
        }

        // Informational: who holds it now.
        let pid = std::process::id();
        if let Err(e) = write_pid(&path, pid) {
            debug!(path = %path.display(), error = %e, "could not record the PID in the lock file");
        }
        debug!(path = %path.display(), pid, "backup lock acquired");
        Ok(Self { path, _file: file })
    }

    /// Path of the lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Overwrites the lock file's content with `pid`.
fn write_pid(path: &Path, pid: u32) -> std::io::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)?;
    f.write_all(pid.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn acquire_creates_the_lock_file_and_records_the_pid() {
        let dir = tempdir().unwrap();
        let lock = BackupLock::acquire(dir.path()).expect("acquire lock");
        let recorded = std::fs::read_to_string(lock.path()).unwrap();
        assert_eq!(recorded, std::process::id().to_string());
    }

    #[test]
    fn a_second_acquire_fails_while_the_first_is_held_and_names_the_holder() {
        let dir = tempdir().unwrap();
        let _lock = BackupLock::acquire(dir.path()).expect("first acquire");
        match BackupLock::acquire(dir.path()) {
            Err(LockError::AlreadyRunning { pid }) => {
                assert_eq!(pid, Some(std::process::id()));
            }
            other => panic!("expected AlreadyRunning, got {other:?}"),
        }
    }

    #[test]
    fn the_lock_is_free_again_after_the_guard_is_dropped() {
        let dir = tempdir().unwrap();
        drop(BackupLock::acquire(dir.path()).expect("first"));
        BackupLock::acquire(dir.path()).expect("re-acquire after release");
    }

    /// Regression (audit K6, e2e S9e): a lock file naming a PID that is alive
    /// but unrelated — an old holder's PID reused, or PID 1 in a new container —
    /// must not block a new run.  Only a held OS lock does.
    #[test]
    fn a_leftover_file_naming_a_live_pid_does_not_block() {
        let dir = tempdir().unwrap();
        for pid in ["1", &std::process::id().to_string(), "0", "garbage", ""] {
            std::fs::write(dir.path().join(".backup.lock"), pid).unwrap();
            BackupLock::acquire(dir.path())
                .unwrap_or_else(|e| panic!("leftover file {pid:?} must not block: {e}"));
        }
    }

    #[test]
    fn lock_error_display_includes_pid_when_known() {
        let err = LockError::AlreadyRunning { pid: Some(1234) };
        let s = format!("{err}");
        assert!(s.contains("1234"), "should contain the pid: {s}");
        assert!(s.contains("already running"), "should describe error: {s}");
    }

    #[test]
    fn lock_error_display_handles_unknown_pid() {
        let err = LockError::AlreadyRunning { pid: None };
        let s = format!("{err}");
        assert!(s.contains("already running"));
        assert!(!s.contains("PID "), "should not mention a PID number: {s}");
    }

    #[test]
    fn lock_error_display_dir_create_includes_inner() {
        let inner = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let s = format!("{}", LockError::DirCreate(inner));
        assert!(s.contains("lock directory"), "got: {s}");
        assert!(s.contains("denied"), "should propagate inner error: {s}");
    }

    #[test]
    fn lock_error_display_write_includes_inner() {
        let s = format!("{}", LockError::Write(std::io::Error::other("boom")));
        assert!(s.contains("lock file"), "got: {s}");
        assert!(s.contains("boom"), "should propagate inner error: {s}");
    }
}
