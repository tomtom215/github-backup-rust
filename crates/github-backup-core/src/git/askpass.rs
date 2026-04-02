// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! RAII guard for a temporary `GIT_ASKPASS` credential helper script.
//!
//! The script echoes the provided token to stdout; git calls it with an
//! interactive prompt string that we intentionally ignore.
//!
//! Credentials are kept out of process arguments and environment variables by
//! routing them through a short-lived executable script that only exists for
//! the duration of the git subprocess call.
//!
//! # Platform behaviour
//!
//! | Platform | Script format | Extension | Permissions |
//! |----------|---------------|-----------|-------------|
//! | Unix     | `#!/bin/sh`   | `.sh`     | `0700` (owner-execute only) |
//! | Windows  | `@echo off`   | `.bat`    | No extra restriction needed; directory is process-private |
//!
//! On Unix the parent directory is created with mode `0700` so no other user
//! on the system can read the token even during the brief window between
//! `create` and the git subprocess completing.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Returns the path to the process-private directory used for askpass scripts.
///
/// The directory is created once per process.  On Unix it has mode `0700` so
/// only the current user can read files inside it.  All subsequent calls return
/// a reference to the same path.
fn askpass_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let mut path = std::env::temp_dir();
        path.push(format!("gh-backup-{}", std::process::id()));

        // Create the directory; ignore the error if it already exists.
        let _ = std::fs::create_dir_all(&path);

        // Restrict to owner-only on Unix so other users cannot read scripts.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
        }

        path
    })
}

/// Builds the platform-appropriate askpass script content for `token`.
///
/// - **Unix**: a POSIX shell script that echoes the token.  Single-quote
///   metacharacters in the token are escaped via the `'\''` idiom so the
///   shell never interprets token characters.
/// - **Windows**: a CMD batch file that echoes the token literally.  Any `%`
///   characters are doubled to prevent CMD variable expansion.
#[cfg(unix)]
fn script_content(token: &str) -> String {
    // Replace `'` with `'\''` so the shell does not interpret token chars.
    let escaped = token.replace('\'', "'\\''");
    format!("#!/bin/sh\necho '{}'\n", escaped)
}

#[cfg(windows)]
fn script_content(token: &str) -> String {
    // Prevent CMD variable expansion by doubling `%`; strip CR/LF that would
    // corrupt the echoed value.
    let escaped = token.replace('%', "%%").replace('\r', "").replace('\n', "");
    format!("@echo off\r\necho {}\r\n", escaped)
}

/// Returns the platform-appropriate file extension for askpass scripts.
#[cfg(unix)]
fn script_extension() -> &'static str {
    "sh"
}

#[cfg(windows)]
fn script_extension() -> &'static str {
    "bat"
}

/// RAII guard for a temporary `GIT_ASKPASS` script.
///
/// When dropped the script file is deleted, ensuring no credentials are
/// left on disk after the git subprocess exits — even on panic.
pub(super) struct AskpassScript {
    path: PathBuf,
}

impl AskpassScript {
    /// Creates the script file and returns a guard, or `None` on I/O failure.
    ///
    /// On failure git receives an empty `GIT_ASKPASS` and authentication
    /// will fail with an auth error rather than hanging indefinitely.
    pub(super) fn create(token: &str) -> Option<Self> {
        let content = script_content(token);

        // Use the process-private directory to prevent other users from
        // reading the token during the git subprocess window.
        let dir = askpass_dir();
        let mut path = dir.clone();
        // Use thread-id for collision-resistance when concurrent git operations
        // run within the same process.
        path.push(format!(
            "askpass-{}.{}",
            format!("{:?}", std::thread::current().id())
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>(),
            script_extension(),
        ));

        if std::fs::write(&path, content.as_bytes()).is_err() {
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
    fn askpass_script_is_in_private_dir() {
        let guard = AskpassScript::create("token").expect("create");
        // The script must be inside the process-private directory, NOT directly
        // in the system temp dir where other users could read it.
        let dir = askpass_dir();
        assert!(
            guard.path().starts_with(dir),
            "script must be in the private directory, not directly in the temp dir"
        );
    }

    #[cfg(unix)]
    #[test]
    fn askpass_private_dir_has_restricted_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = askpass_dir();
        let meta = std::fs::metadata(dir).expect("metadata");
        let mode = meta.permissions().mode();
        // Check that neither group nor others have any permission bits set.
        // We use (mode & 0o077) rather than asserting an exact 0o700 value
        // because macOS APFS may report sticky bits or ACL-influenced modes
        // that change the upper portion of the lower 9 bits while still
        // restricting access correctly.
        let group_other = mode & 0o077;
        assert_eq!(
            group_other,
            0,
            "private askpass dir must have no group/other permissions, got mode {:#o}",
            mode & 0o777
        );
    }

    #[cfg(unix)]
    #[test]
    fn script_content_escapes_single_quote() {
        let content = script_content("to'ken");
        assert!(
            content.contains("to'\\''ken"),
            "single quote must be escaped; got: {content:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn script_content_escapes_percent() {
        let content = script_content("tok%en");
        assert!(
            content.contains("tok%%en"),
            "percent must be doubled; got: {content:?}"
        );
    }

    #[test]
    fn script_has_correct_extension() {
        let guard = AskpassScript::create("token").expect("create");
        let ext = guard
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let expected = script_extension();
        assert_eq!(ext, expected, "script extension must be {expected}");
    }
}
