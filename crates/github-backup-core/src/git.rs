// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Git subprocess abstraction: clone, mirror, and fetch.

use std::path::Path;
use std::process::Command;

use tracing::{debug, info};

use crate::error::CoreError;

/// Abstraction over git subprocess operations.
///
/// The production implementation ([`ProcessGitRunner`]) shells out to the
/// system `git` binary. A no-op stub can be substituted during unit tests to
/// avoid network and filesystem side-effects.
pub trait GitRunner: Send + Sync {
    /// Clones `url` into `dest` as a bare mirror (`git clone --mirror`).
    ///
    /// If `dest` already exists, updates it with `git remote update --prune`
    /// instead of re-cloning.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] if git exits non-zero, or
    /// [`CoreError::GitSpawn`] if the binary cannot be started.
    fn mirror_clone(&self, url: &str, dest: &Path) -> Result<(), CoreError>;

    /// Clones `url` into `dest` using LFS.
    ///
    /// Equivalent to `git lfs clone url dest`.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::GitFailed`] or [`CoreError::GitSpawn`].
    fn lfs_clone(&self, url: &str, dest: &Path) -> Result<(), CoreError>;
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
    fn mirror_clone(&self, url: &str, dest: &Path) -> Result<(), CoreError> {
        if dest.exists() {
            info!(dest = %dest.display(), "repository exists, updating");
            run_git(&["remote", "update", "--prune"], dest, true)
        } else {
            info!(url = %url, dest = %dest.display(), "cloning bare mirror");
            run_git(
                &["clone", "--mirror", url, dest.to_str().unwrap_or("")],
                Path::new("."),
                false,
            )
        }
    }

    fn lfs_clone(&self, url: &str, dest: &Path) -> Result<(), CoreError> {
        if dest.exists() {
            info!(dest = %dest.display(), "LFS repository exists, updating");
            run_git(&["lfs", "fetch", "--all"], dest, true)
        } else {
            info!(url = %url, dest = %dest.display(), "cloning with LFS");
            run_git(
                &["lfs", "clone", url, dest.to_str().unwrap_or("")],
                Path::new("."),
                false,
            )
        }
    }
}

/// Runs `git` with `args` in `cwd`.
///
/// When `in_cwd` is `true` the first argument specifies a repo that has
/// already been cloned and `cwd` should be the repository directory.
fn run_git(args: &[&str], cwd: &Path, in_cwd: bool) -> Result<(), CoreError> {
    let cwd_for_cmd = if in_cwd { cwd } else { Path::new(".") };
    debug!(args = ?args, cwd = %cwd_for_cmd.display(), "running git");

    let output = Command::new("git")
        .args(args)
        .current_dir(cwd_for_cmd)
        .output()
        .map_err(CoreError::GitSpawn)?;

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
        fn mirror_clone(&self, url: &str, dest: &Path) -> Result<(), CoreError> {
            self.calls.lock().unwrap().push(GitCall {
                method: "mirror_clone".to_string(),
                url: url.to_string(),
                dest: dest.to_path_buf(),
            });
            Ok(())
        }

        fn lfs_clone(&self, url: &str, dest: &Path) -> Result<(), CoreError> {
            self.calls.lock().unwrap().push(GitCall {
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
            self.calls.lock().unwrap().clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn spy_git_runner_mirror_clone_records_call() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/test.git");
        runner
            .mirror_clone("https://github.com/octocat/Hello-World.git", &dest)
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
            .lfs_clone("https://github.com/octocat/Hello-World.git", &dest)
            .expect("lfs clone");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "lfs_clone");
    }
}
