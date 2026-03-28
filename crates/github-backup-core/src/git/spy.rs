// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! [`SpyGitRunner`] — a test-only [`GitRunner`] stub.
//!
//! Records every call made to it without executing any git processes.
//! Useful for unit-testing code that calls the [`GitRunner`] trait without
//! hitting the filesystem or network.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{CloneOptions, GitRunner};
use crate::error::CoreError;

/// A [`GitRunner`] stub that records calls but does not invoke git.
///
/// All methods succeed immediately and push a [`GitCall`] entry to the
/// shared call log so tests can assert on what was called.
#[derive(Debug, Clone, Default)]
pub struct SpyGitRunner {
    /// All recorded git operation calls.
    pub calls: Arc<Mutex<Vec<GitCall>>>,
}

/// A recorded call to a [`GitRunner`] method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCall {
    /// Method name, e.g. `"mirror_clone"`, `"full_clone"`, `"push_mirror"`.
    pub method: String,
    /// The URL or remote argument.
    pub url: String,
    /// The destination or source path argument.
    pub dest: PathBuf,
}

impl GitRunner for SpyGitRunner {
    fn mirror_clone(&self, url: &str, dest: &Path, _opts: &CloneOptions) -> Result<(), CoreError> {
        self.record("mirror_clone", url, dest);
        Ok(())
    }

    fn bare_clone(&self, url: &str, dest: &Path, _opts: &CloneOptions) -> Result<(), CoreError> {
        self.record("bare_clone", url, dest);
        Ok(())
    }

    fn full_clone(&self, url: &str, dest: &Path, _opts: &CloneOptions) -> Result<(), CoreError> {
        self.record("full_clone", url, dest);
        Ok(())
    }

    fn shallow_clone(
        &self,
        url: &str,
        dest: &Path,
        _opts: &CloneOptions,
        _depth: u32,
    ) -> Result<(), CoreError> {
        self.record("shallow_clone", url, dest);
        Ok(())
    }

    fn lfs_clone(&self, url: &str, dest: &Path, _opts: &CloneOptions) -> Result<(), CoreError> {
        self.record("lfs_clone", url, dest);
        Ok(())
    }

    fn push_mirror(
        &self,
        src: &Path,
        remote_url: &str,
        _opts: &CloneOptions,
    ) -> Result<(), CoreError> {
        self.record("push_mirror", remote_url, src);
        Ok(())
    }
}

impl SpyGitRunner {
    fn record(&self, method: &str, url: &str, dest: &Path) {
        self.calls
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(GitCall {
                method: method.to_string(),
                url: url.to_string(),
                dest: dest.to_path_buf(),
            });
    }

    /// Returns all recorded calls.
    pub fn recorded_calls(&self) -> Vec<GitCall> {
        self.calls.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> CloneOptions {
        CloneOptions::unauthenticated()
    }

    #[test]
    fn mirror_clone_records_call() {
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
    fn bare_clone_records_call() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/bare.git");
        runner
            .bare_clone("https://github.com/octocat/Hello-World.git", &dest, &opts())
            .expect("bare clone");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "bare_clone");
    }

    #[test]
    fn full_clone_records_call() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/full");
        runner
            .full_clone("https://github.com/octocat/Hello-World.git", &dest, &opts())
            .expect("full clone");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "full_clone");
    }

    #[test]
    fn shallow_clone_records_call() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/shallow.git");
        runner
            .shallow_clone(
                "https://github.com/octocat/Hello-World.git",
                &dest,
                &opts(),
                10,
            )
            .expect("shallow clone");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "shallow_clone");
    }

    #[test]
    fn lfs_clone_records_call() {
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
    fn push_mirror_records_call() {
        let runner = SpyGitRunner::default();
        let src = PathBuf::from("/tmp/local.git");
        runner
            .push_mirror(&src, "https://gitea.example.com/user/repo.git", &opts())
            .expect("push mirror");

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "push_mirror");
    }

    #[test]
    fn multiple_calls_all_recorded() {
        let runner = SpyGitRunner::default();
        let dest = PathBuf::from("/tmp/repo.git");
        runner
            .mirror_clone("https://github.com/a/b.git", &dest, &opts())
            .unwrap();
        runner
            .bare_clone("https://github.com/c/d.git", &dest, &opts())
            .unwrap();
        assert_eq!(runner.recorded_calls().len(), 2);
    }
}
