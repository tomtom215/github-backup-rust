// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! RAII guard for a temporary `GIT_ASKPASS` shell script.
//!
//! The script is written to a uniquely-named file in the system temp directory.
//! When the guard is dropped the file is deleted, ensuring no credentials are
//! left on disk after the git subprocess exits — even on panic.

/// RAII guard for a temporary `GIT_ASKPASS` shell script.
///
/// The script echoes the provided token to stdout; git calls it with an
/// interactive prompt string that we intentionally ignore.
///
/// Credentials are kept out of process arguments and environment variables by
/// routing them through a short-lived executable script that only exists for
/// the duration of the git subprocess call.
pub(super) struct AskpassScript {
    path: std::path::PathBuf,
}

impl AskpassScript {
    /// Creates the script file and returns a guard, or `None` on I/O failure.
    ///
    /// On failure, git receives an empty `GIT_ASKPASS` and authentication
    /// will fail with an auth error rather than hanging indefinitely.
    pub(super) fn create(token: &str) -> Option<Self> {
        // Single-quote–safe token embedding: replace `'` with `'\''` so the
        // shell does not interpret token characters.
        let script = format!("#!/bin/sh\necho '{}'", token.replace('\'', "'\\''"));

        let mut path = std::env::temp_dir();
        // Use PID + thread-id for a collision-resistant filename when
        // concurrent git operations run within the same process.
        path.push(format!(
            "gh-backup-askpass-{}-{}.sh",
            std::process::id(),
            format!("{:?}", std::thread::current().id())
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>(),
        ));

        if std::fs::write(&path, script.as_bytes()).is_err() {
            return None;
        }

        // Mark the script executable on Unix; without this git cannot invoke it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
        }

        Some(Self { path })
    }

    /// Returns the path to the askpass script file.
    pub(super) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for AskpassScript {
    fn drop(&mut self) {
        // Best-effort removal; ignore errors (e.g. if already cleaned up by
        // a signal handler or the OS on process exit).
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn askpass_script_creates_file() {
        let guard = AskpassScript::create("test_token");
        assert!(guard.is_some(), "script creation should succeed");
        let guard = guard.unwrap();
        assert!(guard.path().exists(), "script file should exist");
    }

    #[test]
    fn askpass_script_deleted_on_drop() {
        let path = {
            let guard = AskpassScript::create("test_token").expect("create");
            guard.path().to_path_buf()
        };
        assert!(!path.exists(), "script file should be deleted after drop");
    }

    #[test]
    fn askpass_script_path_is_in_temp_dir() {
        let guard = AskpassScript::create("token").expect("create");
        let temp = std::env::temp_dir();
        assert!(guard.path().starts_with(&temp));
    }
}
