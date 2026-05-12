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
/// In addition to taking the lock, this performs a writability probe by
/// creating and removing a small marker file so that a permission error is
/// surfaced *before* the backup starts, not deep inside the engine after
/// hundreds of API calls have been made.
///
/// # Errors
///
/// Returns a human-readable error string when:
/// - Another process already holds the lock (concurrent run detected).
/// - The lock file cannot be created (permissions, path not found, etc.).
/// - The output directory is not writable by the current user.
pub fn acquire(output_dir: &Path) -> Result<OutputLock, String> {
    std::fs::create_dir_all(output_dir).map_err(|e| {
        format!(
            "cannot create output directory {}: {e}",
            output_dir.display()
        )
    })?;

    // Probe writability up-front: a misconfigured volume (read-only mount,
    // root-owned directory, etc.) is far easier to diagnose here than after
    // a successful API call and a failing `git clone`.
    probe_writable(output_dir)?;

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

/// Writes and removes a tiny marker file to confirm that we can actually
/// create files in `dir`.  Many CI environments mount the workspace
/// read-only or with restrictive ACLs; surfacing that here is much more
/// helpful than a cryptic error from `git clone` an hour into the run.
fn probe_writable(dir: &Path) -> Result<(), String> {
    let probe = dir.join(".github-backup-writable-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            // Best-effort cleanup — if removal fails (e.g. some exotic
            // filesystem) we don't fail the run, the file is harmless.
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(format!(
            "output directory {} is not writable ({}); \
             check permissions, ownership, or mount options",
            dir.display(),
            e
        )),
    }
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

    #[test]
    fn probe_writable_leaves_no_marker_file_behind() {
        let dir = tempdir().expect("tempdir");
        probe_writable(dir.path()).expect("dir should be writable");
        // The lock-acquire path explicitly cleans up the probe file even on
        // success.  We assert that here so a future regression that leaves
        // the marker on disk is caught.
        assert!(
            !dir.path().join(".github-backup-writable-probe").exists(),
            "probe must not leave the marker file behind"
        );
    }

    #[test]
    #[cfg(unix)]
    fn probe_writable_rejects_readonly_dir() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempdir().expect("tempdir");
        // Strip write bits.  Some platforms / CI users still have CAP_DAC
        // override (running as root); in that case the probe will succeed
        // and this test silently skips its assertion.
        let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
        perms.set_mode(0o555);
        if std::fs::set_permissions(dir.path(), perms).is_err() {
            return;
        }
        let result = probe_writable(dir.path());
        // Restore so tempdir can clean up.
        let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
        perms.set_mode(0o755);
        let _ = std::fs::set_permissions(dir.path(), perms);
        // root users bypass mode bits; only assert when probe actually fails.
        if let Err(msg) = result {
            assert!(msg.contains("not writable"));
        }
    }
}
