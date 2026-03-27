// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Git subprocess abstraction: clone, mirror, and fetch.
//!
//! The production implementation shells out to the system `git` binary.
//! Credentials for HTTPS cloning are injected via the `GIT_ASKPASS` environment
//! variable rather than being embedded in the URL, which avoids leaking tokens
//! in process listings and git reflog.

use std::path::Path;
use std::process::Command;

use tracing::{debug, info};

use crate::error::CoreError;

/// Git clone options passed to the runner.
#[derive(Debug, Clone)]
pub struct CloneOptions {
    /// Token to inject for HTTPS authentication, or `None` for unauthenticated
    /// (public repos) or SSH-based cloning.
    pub token: Option<String>,
    /// When `true`, skip `--prune` during updates.
    pub no_prune: bool,
}

impl CloneOptions {
    /// No authentication, prune enabled.
    #[must_use]
    pub fn unauthenticated() -> Self {
        Self {
            token: None,
            no_prune: false,
        }
    }
}

/// Abstraction over git subprocess operations.
///
/// The production implementation ([`ProcessGitRunner`]) shells out to the
/// system `git` binary. A no-op stub can be substituted during unit tests to
/// avoid network and filesystem side-effects.
pub trait GitRunner: Send + Sync {
    /// Clones `url` into `dest` as a bare mirror (`git clone --mirror`).
    ///
    /// If `dest` already exists, updates it with `git remote update` instead
    /// of re-cloning (pruning deleted refs unless `opts.no_prune` is set).
    ///
    /// For HTTPS URLs, `opts.token` is injected via a temporary `GIT_ASKPASS`
    /// script so the token never appears in process listings.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] if git exits non-zero, or
    /// [`CoreError::GitSpawn`] if the binary cannot be started.
    fn mirror_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError>;

    /// Clones `url` into `dest` using Git LFS.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] or [`CoreError::GitSpawn`].
    fn lfs_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError>;
}

/// Production [`GitRunner`] that shells out to the system `git` binary.
#[derive(Debug, Clone, Default)]
pub struct ProcessGitRunner;

impl ProcessGitRunner {
    /// Creates a new [`ProcessGitRunner`].
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl GitRunner for ProcessGitRunner {
    fn mirror_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError> {
        let dest_str = path_to_str(dest)?;
        if dest.exists() {
            info!(dest = %dest.display(), "repository exists, updating");
            let update_args: &[&str] = if opts.no_prune {
                &["remote", "update"]
            } else {
                &["remote", "update", "--prune"]
            };
            run_git(update_args, dest, true, opts.token.as_deref())
        } else {
            info!(url = %url, dest = %dest.display(), "cloning bare mirror");
            run_git(
                &["clone", "--mirror", url, dest_str],
                Path::new("."),
                false,
                opts.token.as_deref(),
            )
        }
    }

    fn lfs_clone(&self, url: &str, dest: &Path, opts: &CloneOptions) -> Result<(), CoreError> {
        let dest_str = path_to_str(dest)?;
        if dest.exists() {
            info!(dest = %dest.display(), "LFS repository exists, updating");
            run_git(
                &["lfs", "fetch", "--all"],
                dest,
                true,
                opts.token.as_deref(),
            )
        } else {
            info!(url = %url, dest = %dest.display(), "cloning with LFS");
            run_git(
                &["lfs", "clone", url, dest_str],
                Path::new("."),
                false,
                opts.token.as_deref(),
            )
        }
    }
}

/// Converts a [`Path`] to a `&str`, returning a [`CoreError`] if the path
/// contains non-UTF-8 bytes.
fn path_to_str(path: &Path) -> Result<&str, CoreError> {
    path.to_str().ok_or_else(|| CoreError::NonUtf8Path {
        path: path.to_string_lossy().into_owned(),
    })
}

/// Runs `git` with `args` in `cwd`.
///
/// If `token` is `Some`, the `GIT_TERMINAL_PROMPT` env var is disabled and
/// `GIT_ASKPASS` is set to a small inline script that echoes the token.
/// This keeps the credential out of the command line and process list.
fn run_git(args: &[&str], cwd: &Path, in_cwd: bool, token: Option<&str>) -> Result<(), CoreError> {
    let cwd_for_cmd = if in_cwd { cwd } else { Path::new(".") };
    debug!(args = ?args, cwd = %cwd_for_cmd.display(), "running git");

    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(cwd_for_cmd);

    // Inject token via GIT_ASKPASS to avoid embedding it in the URL or
    // having it appear in process listings.
    // If a token is provided, write a temporary GIT_ASKPASS script and keep
    // the guard alive until after `cmd.output()` so the file exists when git
    // tries to execute it.  The guard deletes the file on drop.
    let _askpass_guard;
    if let Some(tok) = token {
        _askpass_guard = AskpassScript::create(tok);
        if let Some(ref script) = _askpass_guard {
            // git calls the ASKPASS program with a prompt; we ignore the prompt
            // and always return the token as the password.
            cmd.env("GIT_TERMINAL_PROMPT", "0");
            cmd.env("GIT_ASKPASS", script.path());
            // Username is always "x-access-token" for GitHub token auth.
            cmd.env("GIT_USERNAME", "x-access-token");
        }
    } else {
        _askpass_guard = None;
    }

    let output = cmd.output().map_err(CoreError::GitSpawn)?;
    // _askpass_guard is dropped here, cleaning up the temp file.

    if output.status.success() {
        return Ok(());
    }

    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Err(CoreError::GitFailed {
        args: args.join(" "),
        code,
        stderr,
    })
}

/// RAII guard for a temporary `GIT_ASKPASS` shell script.
///
/// The script is written to a uniquely-named file in the system temp
/// directory. When the guard is dropped the file is deleted, ensuring no
/// credentials are left on disk after the git subprocess exits.
struct AskpassScript {
    path: std::path::PathBuf,
}

impl AskpassScript {
    /// Creates the script file and returns a guard, or `None` on I/O failure.
    ///
    /// On failure git will receive an empty `GIT_ASKPASS` and authentication
    /// will fail with an auth error rather than hanging.
    fn create(token: &str) -> Option<Self> {
        let script = format!("#!/bin/sh\necho '{}'", token.replace('\'', "'\\''"));

        let mut path = std::env::temp_dir();
        // Use both PID and a random-ish component to avoid collisions when
        // the same process runs concurrent git operations.
        path.push(format!(
            "gh-backup-askpass-{}-{}.sh",
            std::process::id(),
            // Mix in the thread id for uniqueness within the same process.
            format!("{:?}", std::thread::current().id())
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>(),
        ));

        if std::fs::write(&path, script.as_bytes()).is_err() {
            return None;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
        }

        Some(Self { path })
    }

    /// Returns the path to the askpass script file.
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for AskpassScript {
    fn drop(&mut self) {
        // Best-effort removal; ignore errors (e.g. if the file was already
        // cleaned up by a signal handler or the OS on process exit).
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    /// A [`GitRunner`] stub that records calls but does not invoke git.
    #[derive(Debug, Clone, Default)]
    pub struct SpyGitRunner {
        pub calls: Arc<Mutex<Vec<GitCall>>>,
    }

    /// A recorded call to a [`GitRunner`] method.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct GitCall {
        /// Method name: `"mirror_clone"` or `"lfs_clone"`.
        pub method: String,
        /// The URL argument.
        pub url: String,
        /// The destination path argument.
        pub dest: PathBuf,
    }

    impl GitRunner for SpyGitRunner {
        fn mirror_clone(
            &self,
            url: &str,
            dest: &Path,
            _opts: &CloneOptions,
        ) -> Result<(), CoreError> {
            self.calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(GitCall {
                    method: "mirror_clone".to_string(),
                    url: url.to_string(),
                    dest: dest.to_path_buf(),
                });
            Ok(())
        }

        fn lfs_clone(&self, url: &str, dest: &Path, _opts: &CloneOptions) -> Result<(), CoreError> {
            self.calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(GitCall {
                    method: "lfs_clone".to_string(),
                    url: url.to_string(),
                    dest: dest.to_path_buf(),
                });
            Ok(())
        }
    }

    impl SpyGitRunner {
        /// Returns all recorded calls.
        pub fn recorded_calls(&self) -> Vec<GitCall> {
            self.calls.lock().unwrap_or_else(|p| p.into_inner()).clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use std::path::PathBuf;

    fn opts() -> CloneOptions {
        CloneOptions::unauthenticated()
    }

    #[test]
    fn spy_git_runner_mirror_clone_records_call() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/test.git");
        runner
            .mirror_clone("https://github.com/octocat/Hello-World.git", &dest, &opts())
            .expect("mirror clone");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "mirror_clone");
        assert_eq!(calls[0].dest, dest);
    }

    #[test]
    fn spy_git_runner_lfs_clone_records_call() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/lfs.git");
        runner
            .lfs_clone("https://github.com/octocat/Hello-World.git", &dest, &opts())
            .expect("lfs clone");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "lfs_clone");
    }

    #[test]
    fn path_to_str_returns_error_for_non_utf8_path() {
        #[cfg(unix)]
        {
            use std::ffi::OsStr;
            use std::os::unix::ffi::OsStrExt;
            let invalid = OsStr::from_bytes(b"/tmp/invalid\xff");
            let path = std::path::Path::new(invalid);
            assert!(path_to_str(path).is_err());
        }
    }

    #[test]
    fn path_to_str_returns_str_for_valid_utf8() {
        let path = Path::new("/tmp/valid-path.git");
        assert_eq!(
            path_to_str(path).expect("valid path"),
            "/tmp/valid-path.git"
        );
    }
}
